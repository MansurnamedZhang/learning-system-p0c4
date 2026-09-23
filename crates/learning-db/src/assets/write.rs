use super::{AssetMedia, AssetRecord, AssetStore, ResourceInput, SourceSegmentRef, read};
use crate::{authorization, request, storage};
use learning_assets::VerifiedBlob;
use learning_core::{
    AssetRef, ContentError, Principal, ResourceVersionRef, canonical_json, hex_digest,
};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

fn validate_name(name: &str) -> Result<(), ContentError> {
    if name.trim().is_empty() || name.chars().count() > 255 || name.contains('\0') {
        return Err(ContentError::Invalid("invalid_asset_display_name".into()));
    }
    Ok(())
}

fn validate_media(media: &AssetMedia) -> Result<(), ContentError> {
    validate_name(&media.original_file_name)?;
    let mut parts = media.media_type.split('/');
    let valid_part = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".+_-".contains(&b))
    };
    if media.media_type.len() > 200
        || !parts.next().is_some_and(valid_part)
        || !parts.next().is_some_and(valid_part)
        || parts.next().is_some()
    {
        return Err(ContentError::Invalid("invalid_asset_media_type".into()));
    }
    Ok(())
}

async fn begin_write(pool: &sqlx::PgPool) -> Result<Transaction<'_, Postgres>, ContentError> {
    let mut tx = pool.begin().await.map_err(storage)?;
    sqlx::query("SET LOCAL statement_timeout='15s'")
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    Ok(tx)
}

impl AssetStore {
    pub async fn register_verified(
        &self,
        actor: Principal,
        space: Uuid,
        request_id: Uuid,
        verified: VerifiedBlob,
        media: AssetMedia,
    ) -> Result<AssetRecord, ContentError> {
        validate_media(&media)?;
        let byte_size = i64::try_from(verified.size_bytes())
            .map_err(|_| ContentError::Invalid("asset_too_large".into()))?;
        let digest = hex_digest(
            canonical_json(&json!({
                "domain": "asset-register-v1",
                "space_id": space,
                "sha256": verified.sha256(),
                "byte_size": byte_size,
                "storage_key": verified.storage_key(),
                "media_type": media.media_type,
                "original_file_name": media.original_file_name,
            }))
            .as_bytes(),
        );
        let mut tx = request::begin(&self.pool, actor, request_id).await?;
        authorization::lock_grants(&mut tx, actor, &[(space, true)]).await?;
        if request::check(&mut tx, actor, request_id, &digest, "asset_register").await? {
            // A historical receipt does not make corrupt bytes readable again.
            self.files.verify(&verified).map_err(storage)?;
            let old = read::receipt(&mut tx, actor, request_id).await?;
            tx.commit().await.map_err(storage)?;
            return Ok(old);
        }
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO public.asset(space_id,id,sha256,byte_size,storage_key,media_type,original_file_name,status) VALUES($1,$2,$3,$4,$5,$6,$7,'ready')")
            .bind(space)
            .bind(id)
            .bind(verified.sha256())
            .bind(byte_size)
            .bind(verified.storage_key())
            .bind(&media.media_type)
            .bind(&media.original_file_name)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        request::register(&mut tx, actor, request_id, &digest, "asset_register").await?;
        sqlx::query("INSERT INTO public.upload_receipt(actor_id,request_id,space_id,asset_id) VALUES($1,$2,$3,$4)")
            .bind(actor.actor_id)
            .bind(request_id)
            .bind(space)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        // The token proves finalization occurred, but the file can go missing or
        // be changed before this transaction. Recheck after all DB writes and
        // immediately before commit; an error rolls back both rows and receipt.
        self.files.verify(&verified).map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        Ok(AssetRecord {
            reference: AssetRef {
                space_id: space,
                asset_id: id,
            },
            sha256: verified.sha256().into(),
            byte_size,
            storage_key: verified.storage_key().into(),
            media,
        })
    }

    pub async fn link_resource_version(
        &self,
        actor: Principal,
        resource: ResourceInput,
        asset: AssetRef,
    ) -> Result<ResourceVersionRef, ContentError> {
        validate_name(&resource.display_name)?;
        if asset.space_id != resource.space_id {
            return Err(ContentError::NotFound);
        }
        let mut tx = begin_write(&self.pool).await?;
        authorization::lock_grants(&mut tx, actor, &[(resource.space_id, true)]).await?;
        let ready: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.asset WHERE space_id=$1 AND id=$2 AND status='ready')")
            .bind(resource.space_id)
            .bind(asset.asset_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(storage)?;
        if !ready {
            return Err(ContentError::NotFound);
        }
        let resource_id = resource.resource_id.unwrap_or_else(Uuid::new_v4);
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!(
                "learning/asset/resource/{}:{resource_id}",
                resource.space_id
            ))
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        let version_no = if resource.resource_id.is_some() {
            let saved_name: Option<String> = sqlx::query_scalar(
                "SELECT display_name FROM public.resource WHERE space_id=$1 AND id=$2",
            )
            .bind(resource.space_id)
            .bind(resource_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(storage)?;
            match saved_name {
                None => return Err(ContentError::NotFound),
                Some(saved) if saved != resource.display_name => {
                    return Err(ContentError::Invalid("resource_name_mismatch".into()));
                }
                Some(_) => {}
            }
            sqlx::query_scalar::<_, Option<i32>>("SELECT max(version_no) FROM public.resource_version WHERE space_id=$1 AND resource_id=$2")
                .bind(resource.space_id)
                .bind(resource_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(storage)?
                .unwrap_or(0)
                .checked_add(1)
                .ok_or(ContentError::Storage)?
        } else {
            sqlx::query("INSERT INTO public.resource(space_id,id,display_name) VALUES($1,$2,$3)")
                .bind(resource.space_id)
                .bind(resource_id)
                .bind(&resource.display_name)
                .execute(&mut *tx)
                .await
                .map_err(storage)?;
            1
        };
        let version_id = Uuid::new_v4();
        sqlx::query("INSERT INTO public.resource_version(space_id,resource_id,id,asset_id,version_no) VALUES($1,$2,$3,$4,$5)")
            .bind(resource.space_id)
            .bind(resource_id)
            .bind(version_id)
            .bind(asset.asset_id)
            .bind(version_no)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        Ok(ResourceVersionRef {
            space_id: resource.space_id,
            resource_id,
            version_id,
        })
    }

    pub async fn add_source_segment(
        &self,
        actor: Principal,
        version: ResourceVersionRef,
        selector: Value,
    ) -> Result<SourceSegmentRef, ContentError> {
        if !selector.is_object() || selector.to_string().len() > 16_000 {
            return Err(ContentError::Invalid("invalid_source_selector".into()));
        }
        let mut tx = begin_write(&self.pool).await?;
        authorization::lock_grants(&mut tx, actor, &[(version.space_id, true)]).await?;
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.resource_version WHERE space_id=$1 AND resource_id=$2 AND id=$3)")
            .bind(version.space_id)
            .bind(version.resource_id)
            .bind(version.version_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(storage)?;
        if !exists {
            return Err(ContentError::NotFound);
        }
        let segment_id = Uuid::new_v4();
        sqlx::query("INSERT INTO public.source_segment(space_id,resource_id,resource_version_id,id,selector) VALUES($1,$2,$3,$4,$5)")
            .bind(version.space_id)
            .bind(version.resource_id)
            .bind(version.version_id)
            .bind(segment_id)
            .bind(&selector)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        Ok(SourceSegmentRef {
            space_id: version.space_id,
            resource_id: version.resource_id,
            version_id: version.version_id,
            segment_id,
        })
    }
}

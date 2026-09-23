use super::{AssetMedia, AssetRecord, AssetStore};
use crate::{references, request, storage};
use learning_core::{AssetRef, AssetUseRef, ContentError, ExactRef, Principal};
use sqlx::{Postgres, Transaction};
use std::fs::File;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
pub(super) struct AssetRow {
    pub space_id: Uuid,
    pub id: Uuid,
    pub sha256: String,
    pub byte_size: i64,
    pub storage_key: String,
    pub media_type: String,
    pub original_file_name: String,
}

impl AssetRow {
    pub(super) fn record(self) -> AssetRecord {
        AssetRecord {
            reference: AssetRef {
                space_id: self.space_id,
                asset_id: self.id,
            },
            sha256: self.sha256,
            byte_size: self.byte_size,
            storage_key: self.storage_key,
            media: AssetMedia {
                media_type: self.media_type,
                original_file_name: self.original_file_name,
            },
        }
    }
}

async fn authorized_row(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    use_ref: AssetUseRef,
) -> Result<Option<AssetRow>, ContentError> {
    let row = match use_ref {
        AssetUseRef::Block(block) => {
            // Match the existing exact-reference read contract: a visible root
            // with a hidden necessary dependency is not readable either.
            if !references::Session::default()
                .authorize(tx, actor, &ExactRef::Block(block.clone()))
                .await?
            {
                return Ok(None);
            }
            sqlx::query_as::<_, AssetRow>(
                "SELECT a.space_id,a.id,a.sha256,a.byte_size,a.storage_key,a.media_type,a.original_file_name \
                 FROM public.block_asset_use u \
                 JOIN public.block_revision r ON (r.space_id,r.block_id,r.id)=(u.space_id,u.block_id,u.revision_id) \
                 JOIN public.space_grant g ON g.space_id=u.space_id AND g.actor_id=$1 \
                 JOIN public.asset a ON (a.space_id,a.id)=(u.space_id,u.asset_id) \
                 WHERE u.block_id=$2 AND u.revision_id=$3 AND a.status='ready'",
            )
            .bind(actor.actor_id)
            .bind(block.block_id)
            .bind(block.revision_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(storage)?
        }
        AssetUseRef::Resource(version) => {
            sqlx::query_as::<_, AssetRow>(
                "SELECT a.space_id,a.id,a.sha256,a.byte_size,a.storage_key,a.media_type,a.original_file_name \
                 FROM public.resource_version v \
                 JOIN public.space_grant g ON g.space_id=v.space_id AND g.actor_id=$1 \
                 JOIN public.asset a ON (a.space_id,a.id)=(v.space_id,v.asset_id) \
                 WHERE v.space_id=$2 AND v.resource_id=$3 AND v.id=$4 AND a.status='ready'",
            )
            .bind(actor.actor_id)
            .bind(version.space_id)
            .bind(version.resource_id)
            .bind(version.version_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(storage)?
        }
    };
    Ok(row)
}

impl AssetStore {
    /// Resolve an exact, authorized block revision or resource version. A
    /// hidden use and a nonexistent use are indistinguishable to callers.
    pub async fn read_for_use(
        &self,
        actor: Principal,
        use_ref: AssetUseRef,
    ) -> Result<Option<AssetRecord>, ContentError> {
        let mut tx = request::begin_read(&self.pool).await?;
        let row = authorized_row(&mut tx, actor, use_ref).await?;
        tx.commit().await.map_err(storage)?;
        Ok(row.map(AssetRow::record))
    }

    /// Authorize before touching the file store; then rehash the exact opened
    /// handle. An unavailable file is an error, never an empty/hidden asset.
    pub async fn open_for_use(
        &self,
        actor: Principal,
        use_ref: AssetUseRef,
    ) -> Result<Option<File>, ContentError> {
        let mut tx = request::begin_read(&self.pool).await?;
        let row = authorized_row(&mut tx, actor, use_ref).await?;
        let Some(row) = row else {
            tx.commit().await.map_err(storage)?;
            return Ok(None);
        };
        let file = self
            .files
            .open_record(&row.storage_key, &row.sha256, row.byte_size)
            .map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        Ok(Some(file))
    }
}

pub(super) async fn receipt(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    request: Uuid,
) -> Result<AssetRecord, ContentError> {
    let row: Option<AssetRow> = sqlx::query_as(
        "SELECT a.space_id,a.id,a.sha256,a.byte_size,a.storage_key,a.media_type,a.original_file_name \
         FROM public.upload_receipt r JOIN public.asset a ON (a.space_id,a.id)=(r.space_id,r.asset_id) \
         WHERE r.actor_id=$1 AND r.request_id=$2",
    )
    .bind(actor.actor_id)
    .bind(request)
    .fetch_optional(&mut **tx)
    .await
    .map_err(storage)?;
    row.map(AssetRow::record).ok_or(ContentError::Storage)
}

use super::{ReleaseStore, manifest::collect, read};
use crate::{overlay::model, request, storage};
use learning_core::*;
use uuid::Uuid;
pub(super) async fn load_with_manifest(
    tx: &mut model::Tx<'_>,
    actor: Principal,
    id: Uuid,
) -> Result<(EvidenceRelease, super::manifest::Manifest), ContentError> {
    let basic = read::load(tx, actor, id).await?;
    let (version, sha): (i32, Option<String>) =
        sqlx::query_as("SELECT contract_version,manifest_sha256 FROM release WHERE id=$1")
            .bind(id)
            .fetch_one(&mut **tx)
            .await
            .map_err(storage)?;
    if version != 2 {
        crate::composition::closure::load(tx, actor, &basic.roots, None).await?;
        return Err(ContentError::Invalid("unsupported_release_version".into()));
    }
    let readings:Vec<(Uuid,Uuid)>=sqlx::query_as("SELECT view_id,view_revision_id FROM release_reading WHERE release_id=$1 ORDER BY view_id,view_revision_id").bind(id).fetch_all(&mut **tx).await.map_err(storage)?;
    let readings: Vec<_> = readings
        .into_iter()
        .map(|(view_id, revision_id)| ReadingRef {
            view_id,
            revision_id,
        })
        .collect();
    let manifest = collect(tx, actor, &basic.roots, &readings).await?;
    let stored_objects:Vec<(String,Uuid,Uuid)>=sqlx::query_as("SELECT kind,object_id,revision_id FROM release_manifest_object WHERE release_id=$1 ORDER BY kind,object_id,revision_id").bind(id).fetch_all(&mut **tx).await.map_err(storage)?;
    let stored_compositions:Vec<(Uuid,Uuid,Uuid)>=sqlx::query_as("SELECT composition_id,revision_id,space_id FROM release_manifest_composition WHERE release_id=$1 ORDER BY composition_id,revision_id").bind(id).fetch_all(&mut **tx).await.map_err(storage)?;
    let expected_compositions: Vec<_> = manifest
        .compositions
        .iter()
        .map(|(r, s)| (r.composition_id, r.revision_id, *s))
        .collect();
    let digest = manifest.digest();
    if stored_objects != manifest.objects
        || stored_compositions != expected_compositions
        || sha.as_deref() != Some(&digest)
    {
        return Err(ContentError::Storage);
    }
    Ok((
        EvidenceRelease {
            release_id: id,
            space_id: basic.space_id,
            roots: basic.roots,
            author_id: basic.author_id,
            reason: basic.reason,
            created_at: basic.created_at,
            readings,
            manifest_sha256: digest,
        },
        manifest,
    ))
}
pub(super) async fn load(
    tx: &mut model::Tx<'_>,
    actor: Principal,
    id: Uuid,
) -> Result<EvidenceRelease, ContentError> {
    load_with_manifest(tx, actor, id)
        .await
        .map(|(release, _)| release)
}
impl ReleaseStore {
    pub async fn read_evidence(
        &self,
        actor: Principal,
        id: Uuid,
    ) -> Result<Option<EvidenceRelease>, ContentError> {
        let mut tx = request::begin_read(&self.pool).await?;
        match load(&mut tx, actor, id).await {
            Ok(r) => {
                tx.commit().await.map_err(storage)?;
                Ok(Some(r))
            }
            Err(ContentError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

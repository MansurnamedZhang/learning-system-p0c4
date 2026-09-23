mod evidence_read;
mod evidence_write;
mod manifest;
mod read;
mod write;
use crate::{composition::closure, storage};
use learning_core::{CompositionRef, ContentError, Principal, ReadingRef};
use sqlx::PgPool;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
#[derive(Clone)]
pub struct ReleaseStore {
    pub(crate) pool: PgPool,
}
impl ReleaseStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Check the same full fixed release closure used by B3 before impact scope
/// membership is derived. No manifest row becomes a direct citation.
pub(crate) struct ImpactRelease {
    pub roots: Vec<CompositionRef>,
    pub readings: Vec<ReadingRef>,
    pub manifest_objects: Vec<(String, Uuid, Uuid)>,
    pub manifest_compositions: Vec<CompositionRef>,
}
pub(crate) async fn load_for_impact(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    id: Uuid,
) -> Result<ImpactRelease, ContentError> {
    let release = read::load(tx, actor, id).await?;
    let version: i32 =
        sqlx::query_scalar("SELECT contract_version FROM public.release WHERE id=$1")
            .bind(id)
            .fetch_one(&mut **tx)
            .await
            .map_err(storage)?;
    if version == 2 {
        let (evidence, manifest) = evidence_read::load_with_manifest(tx, actor, id).await?;
        Ok(ImpactRelease {
            roots: evidence.roots,
            readings: evidence.readings,
            manifest_objects: manifest.objects,
            manifest_compositions: manifest.compositions.into_keys().collect(),
        })
    } else if version == 1 {
        closure::load(tx, actor, &release.roots, None).await?;
        Ok(ImpactRelease {
            roots: release.roots,
            readings: vec![],
            manifest_objects: vec![],
            manifest_compositions: vec![],
        })
    } else {
        Err(ContentError::Storage)
    }
}

fn publication_token(composition: uuid::Uuid, release: Option<uuid::Uuid>) -> String {
    learning_core::hex_digest(learning_core::canonical_json(&serde_json::json!({"domain":"publication-basis-v1","composition_id":composition,"last_release_id":release})).as_bytes())
}

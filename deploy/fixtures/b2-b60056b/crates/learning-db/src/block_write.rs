//! Transaction-only primitive shared by content and personal-reading writes.
use crate::{COLUMNS, RevisionRow, storage};
use learning_core::{ContentError, Principal, Revision, TextDraft};
use sqlx::{Postgres, Transaction, types::Json};
use uuid::Uuid;
pub(crate) struct PendingRevision<'a> {
    pub block_id: Uuid,
    pub space_id: Uuid,
    pub revision_id: Uuid,
    pub parent: Option<Uuid>,
    pub actor: Principal,
    pub draft: &'a TextDraft,
    pub reason: &'a str,
}
pub(crate) async fn insert_revision(
    tx: &mut Transaction<'_, Postgres>,
    r: PendingRevision<'_>,
) -> Result<Revision, ContentError> {
    let query = format!(
        "INSERT INTO public.block_revision AS r(id,space_id,block_id,parent_revision_id,content,content_sha256,author_id,reason) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) RETURNING {COLUMNS}"
    );
    let row = sqlx::query_as::<_, RevisionRow>(&query)
        .bind(r.revision_id)
        .bind(r.space_id)
        .bind(r.block_id)
        .bind(r.parent)
        .bind(Json(r.draft))
        .bind(r.draft.digest())
        .bind(r.actor.actor_id)
        .bind(r.reason)
        .fetch_one(&mut **tx)
        .await
        .map_err(storage)?;
    Ok(row.into())
}

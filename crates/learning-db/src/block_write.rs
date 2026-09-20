//! Transaction-only primitive shared by content and personal-reading writes.
use crate::{COLUMNS, RevisionRow, storage};
use learning_core::{
    BlockRef, ContentDraft, ContentError, ContentRevision, ExactRef, Principal, Revision, TextDraft,
};
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
    insert_content(
        tx,
        PendingContent {
            block_id: r.block_id,
            space_id: r.space_id,
            revision_id: r.revision_id,
            parent: r.parent,
            actor: r.actor,
            draft: &ContentDraft::V1(r.draft.clone()),
            reason: r.reason,
        },
    )
    .await?
    .try_into()
}
pub(crate) struct PendingContent<'a> {
    pub block_id: Uuid,
    pub space_id: Uuid,
    pub revision_id: Uuid,
    pub parent: Option<Uuid>,
    pub actor: Principal,
    pub draft: &'a ContentDraft,
    pub reason: &'a str,
}
pub(crate) async fn insert_content(
    tx: &mut Transaction<'_, Postgres>,
    r: PendingContent<'_>,
) -> Result<ContentRevision, ContentError> {
    let query = format!(
        "INSERT INTO public.block_revision AS r(id,space_id,block_id,parent_revision_id,content,content_sha256,author_id,reason,contract_version) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) RETURNING {COLUMNS}"
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
        .bind(r.draft.contract_version() as i32)
        .fetch_one(&mut **tx)
        .await
        .map_err(storage)?;
    crate::references::insert(
        tx,
        &ExactRef::Block(BlockRef {
            block_id: r.block_id,
            revision_id: r.revision_id,
        }),
        &r.draft.dependencies(),
    )
    .await?;
    row.decode()
}

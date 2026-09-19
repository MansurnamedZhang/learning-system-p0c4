use crate::{COLUMNS, ContentStore, RevisionRow, storage};
use learning_core::{
    CONTRACT_VERSION, ContentError, CreateCommand, Principal, ReviseCommand, Revision, TextDraft,
    canonical_json, hex_digest,
};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction, types::Json};
use uuid::Uuid;

impl ContentStore {
    pub async fn create(
        &self,
        actor: Principal,
        space_id: Uuid,
        command: CreateCommand,
    ) -> Result<Revision, ContentError> {
        command.validate()?;
        let digest = request_digest(
            "create",
            space_id,
            serde_json::to_value(&command).map_err(storage)?,
        );
        let mut tx = crate::request::begin(&self.pool, actor, command.request_id).await?;
        authorize(&mut tx, actor, space_id).await?;
        if let Some(revision) = receipt(&mut tx, actor, command.request_id, &digest).await? {
            tx.commit().await.map_err(storage)?;
            return Ok(revision);
        }
        let block_id = Uuid::new_v4();
        let revision_id = Uuid::new_v4();
        sqlx::query("INSERT INTO public.block(id,space_id,head_revision_id) VALUES ($1,$2,$3)")
            .bind(block_id)
            .bind(space_id)
            .bind(revision_id)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        let pending = PendingRevision {
            block_id,
            space_id,
            revision_id,
            parent: None,
            actor,
            draft: &command.draft,
            reason: &command.reason,
        };
        let revision = insert_revision(&mut tx, pending).await?;
        insert_receipt(&mut tx, actor, command.request_id, &digest, revision_id).await?;
        tx.commit().await.map_err(storage)?;
        Ok(revision)
    }

    pub async fn revise(
        &self,
        actor: Principal,
        block_id: Uuid,
        command: ReviseCommand,
    ) -> Result<Revision, ContentError> {
        command.validate()?;
        let digest = request_digest(
            "revise",
            block_id,
            serde_json::to_value(&command).map_err(storage)?,
        );
        let mut tx = crate::request::begin(&self.pool, actor, command.request_id).await?;
        let space_id: Uuid = sqlx::query_scalar("SELECT space_id FROM public.block WHERE id=$1")
            .bind(block_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(storage)?
            .ok_or(ContentError::NotFound)?;
        authorize(&mut tx, actor, space_id).await?;
        // Receipt lookup precedes stale-base detection: a retry must return its original result.
        if let Some(revision) = receipt(&mut tx, actor, command.request_id, &digest).await? {
            tx.commit().await.map_err(storage)?;
            return Ok(revision);
        }
        let head: Uuid =
            sqlx::query_scalar("SELECT head_revision_id FROM public.block WHERE id=$1 FOR UPDATE")
                .bind(block_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(storage)?;
        if head != command.base_revision_id {
            return Err(ContentError::Conflict {
                current_revision_id: head,
            });
        }
        let revision_id = Uuid::new_v4();
        let pending = PendingRevision {
            block_id,
            space_id,
            revision_id,
            parent: Some(head),
            actor,
            draft: &command.draft,
            reason: &command.reason,
        };
        let revision = insert_revision(&mut tx, pending).await?;
        sqlx::query("UPDATE public.block SET head_revision_id=$1 WHERE id=$2")
            .bind(revision_id)
            .bind(block_id)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        insert_receipt(&mut tx, actor, command.request_id, &digest, revision_id).await?;
        tx.commit().await.map_err(storage)?;
        Ok(revision)
    }
}

fn request_digest(operation: &str, target: Uuid, command: Value) -> String {
    hex_digest(canonical_json(&json!({"operation":operation,"target":target,"contract_version":CONTRACT_VERSION,"command":command})).as_bytes())
}

async fn authorize(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    space: Uuid,
) -> Result<(), ContentError> {
    crate::authorization::lock_grants(tx, actor, &[(space, true)]).await
}

async fn receipt(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    request: Uuid,
    digest: &str,
) -> Result<Option<Revision>, ContentError> {
    let registered = crate::request::check(tx, actor, request, digest, "content_v1").await?;
    let existing:Option<(String,Uuid)>=sqlx::query_as("SELECT request_sha256,revision_id FROM public.mutation_receipt WHERE actor_id=$1 AND request_id=$2")
        .bind(actor.actor_id).bind(request).fetch_optional(&mut **tx).await.map_err(storage)?;
    let Some((prior, id)) = existing else {
        return if registered {
            Err(ContentError::Storage)
        } else {
            Ok(None)
        };
    };
    if prior != digest {
        return Err(ContentError::IdempotencyConflict);
    }
    // Caller holds the current grant lock, and a matching digest fixes the target.
    let row = sqlx::query_as::<_, RevisionRow>(&format!(
        "SELECT {COLUMNS} FROM public.block_revision r WHERE r.id=$1"
    ))
    .bind(id)
    .fetch_one(&mut **tx)
    .await
    .map_err(storage)?;
    Ok(Some(row.into()))
}

struct PendingRevision<'a> {
    block_id: Uuid,
    space_id: Uuid,
    revision_id: Uuid,
    parent: Option<Uuid>,
    actor: Principal,
    draft: &'a TextDraft,
    reason: &'a str,
}
async fn insert_revision(
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
async fn insert_receipt(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    request: Uuid,
    digest: &str,
    revision: Uuid,
) -> Result<(), ContentError> {
    crate::request::register(tx, actor, request, digest, "content_v1").await?;
    sqlx::query("INSERT INTO public.mutation_receipt(actor_id,request_id,request_sha256,revision_id) VALUES ($1,$2,$3,$4)")
        .bind(actor.actor_id).bind(request).bind(digest).bind(revision).execute(&mut **tx).await.map_err(storage)?;
    Ok(())
}

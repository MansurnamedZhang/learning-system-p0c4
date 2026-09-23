use super::VersionedContentStore;
use crate::{
    authorization,
    block_write::{PendingContent, insert_content},
    references, request, storage,
};
use learning_core::*;
use serde_json::json;
use uuid::Uuid;

impl VersionedContentStore {
    pub async fn create(
        &self,
        actor: Principal,
        space: Uuid,
        command: CreateContent,
    ) -> Result<ContentRevision, ContentError> {
        command.validate()?;
        let digest=hex_digest(canonical_json(&json!({"domain":"content-versioned-request-v1","operation":"create","actor_id":actor.actor_id,"space_id":space,"command":command})).as_bytes());
        self.save(
            actor,
            space,
            None,
            command.request_id,
            command.draft,
            command.reason,
            digest,
        )
        .await
    }
    pub async fn revise(
        &self,
        actor: Principal,
        block: Uuid,
        command: ReviseContent,
    ) -> Result<ContentRevision, ContentError> {
        command.validate()?;
        let digest=hex_digest(canonical_json(&json!({"domain":"content-versioned-request-v1","operation":"revise","actor_id":actor.actor_id,"block_id":block,"command":command})).as_bytes());
        // This identity lookup yields no public data. Grants and all fixed
        // references are checked together inside the request transaction.
        let space = sqlx::query_scalar("SELECT space_id FROM public.block WHERE id=$1")
            .bind(block)
            .fetch_optional(&self.pool)
            .await
            .map_err(storage)?
            .ok_or(ContentError::NotFound)?;
        self.save(
            actor,
            space,
            Some((block, command.base_revision_id)),
            command.request_id,
            command.draft,
            command.reason,
            digest,
        )
        .await
    }
    #[allow(clippy::too_many_arguments)]
    async fn save(
        &self,
        actor: Principal,
        space: Uuid,
        base: Option<(Uuid, Uuid)>,
        request_id: Uuid,
        draft: ContentDraft,
        reason: String,
        digest: String,
    ) -> Result<ContentRevision, ContentError> {
        let mut tx = request::begin(&self.pool, actor, request_id).await?;
        let operation = if matches!(&draft, ContentDraft::V3(_)) {
            "content_v3"
        } else {
            "content_v2"
        };
        let roots: Vec<_> = draft.dependencies().into_iter().map(|d| d.target).collect();
        let discovered = references::load(&mut tx, actor, &roots).await?;
        let mut spaces = discovered.spaces();
        spaces.push((space, true));
        authorization::lock_grants(&mut tx, actor, &spaces).await?;
        let checked = references::load(&mut tx, actor, &roots).await?;
        if checked.spaces() != discovered.spaces() {
            return Err(ContentError::Invalid(
                "reference_authorization_changed".into(),
            ));
        }
        if request::check(&mut tx, actor, request_id, &digest, operation).await? {
            let (block_id,revision_id):(Uuid,Uuid)=sqlx::query_as("SELECT r.block_id,r.id FROM public.mutation_receipt m JOIN public.block_revision r ON r.id=m.revision_id WHERE m.actor_id=$1 AND m.request_id=$2")
                .bind(actor.actor_id).bind(request_id).fetch_one(&mut *tx).await.map_err(storage)?;
            let result = references::project(
                &mut tx,
                actor,
                &ExactRef::Block(BlockRef {
                    block_id,
                    revision_id,
                }),
            )
            .await?
            .ok_or(ContentError::NotFound)?
            .block()?;
            tx.commit().await.map_err(storage)?;
            return Ok(result);
        }
        let (block, parent) = if let Some((block, expected)) = base {
            let head: Uuid = sqlx::query_scalar(
                "SELECT head_revision_id FROM public.block WHERE id=$1 AND space_id=$2 FOR UPDATE",
            )
            .bind(block)
            .bind(space)
            .fetch_optional(&mut *tx)
            .await
            .map_err(storage)?
            .ok_or(ContentError::NotFound)?;
            if head != expected {
                references::load(
                    &mut tx,
                    actor,
                    &[ExactRef::Block(BlockRef {
                        block_id: block,
                        revision_id: head,
                    })],
                )
                .await?;
                return Err(ContentError::Conflict {
                    current_revision_id: head,
                });
            }
            (block, Some(head))
        } else {
            (Uuid::new_v4(), None)
        };
        let revision_id = Uuid::new_v4();
        if parent.is_none() {
            sqlx::query("INSERT INTO public.block(id,space_id,head_revision_id) VALUES($1,$2,$3)")
                .bind(block)
                .bind(space)
                .bind(revision_id)
                .execute(&mut *tx)
                .await
                .map_err(storage)?;
        }
        insert_content(
            &mut tx,
            PendingContent {
                block_id: block,
                space_id: space,
                revision_id,
                parent,
                actor,
                draft: &draft,
                reason: &reason,
            },
        )
        .await?;
        if parent.is_some() {
            sqlx::query("UPDATE public.block SET head_revision_id=$1 WHERE id=$2")
                .bind(revision_id)
                .bind(block)
                .execute(&mut *tx)
                .await
                .map_err(storage)?;
        }
        request::register(&mut tx, actor, request_id, &digest, operation).await?;
        sqlx::query("INSERT INTO public.mutation_receipt(actor_id,request_id,request_sha256,revision_id) VALUES($1,$2,$3,$4)").bind(actor.actor_id).bind(request_id).bind(digest).bind(revision_id).execute(&mut *tx).await.map_err(storage)?;
        let result = references::project(
            &mut tx,
            actor,
            &ExactRef::Block(BlockRef {
                block_id: block,
                revision_id,
            }),
        )
        .await?
        .ok_or(ContentError::NotFound)?
        .block()?;
        tx.commit().await.map_err(storage)?;
        Ok(result)
    }
}

use super::ReadingStore;
use crate::{
    authorization,
    block_write::{PendingRevision, insert_revision},
    overlay::{
        anchor::invalid,
        edit,
        model::{self, Layer, Tx},
        persist,
    },
    request, storage,
};
use learning_core::*;
use uuid::Uuid;

impl ReadingStore {
    pub async fn create(
        &self,
        actor: Principal,
        space: Uuid,
        command: CreateReading,
    ) -> Result<ReadingSaved, ContentError> {
        command.validate()?;
        let digest = reading_request_digest("reading_create", actor, space, &command);
        let mut tx = request::begin(&self.pool, actor, command.request_id).await?;
        let owner: Option<Uuid> = sqlx::query_scalar("SELECT owner_id FROM space WHERE id=$1")
            .bind(space)
            .fetch_optional(&mut *tx)
            .await
            .map_err(storage)?;
        if owner != Some(actor.actor_id) {
            return Err(ContentError::NotFound);
        }
        let (mut spaces, source) = model::snapshot(&mut tx, actor, &command.base).await?;
        let source = source.ok_or(ContentError::NotFound)?;
        if source
            .compositions
            .iter()
            .find(|c| c.reference == command.base)
            .ok_or(ContentError::Storage)?
            .kind
            != CompositionKind::Document
        {
            return invalid("root_must_be_document");
        }
        spaces.push((space, true));
        // A replay may reference content no longer present in the current layer.
        if let Some(saved) = replay(
            &mut tx,
            actor,
            command.request_id,
            &digest,
            "reading_create",
            &spaces,
        )
        .await?
        {
            tx.commit().await.map_err(storage)?;
            return Ok(saved);
        }
        authorization::lock_grants(&mut tx, actor, &spaces).await?;
        let id = Uuid::new_v4();
        let view_id = Uuid::new_v4();
        let mut layer = Layer {
            id,
            space,
            revision: Uuid::new_v4(),
            view: ReadingRef {
                view_id,
                revision_id: Uuid::new_v4(),
            },
            data: EditableReading {
                base: command.base,
                title: command.title,
                groups: vec![],
            },
        };
        let access = model::access(&mut tx, actor, &layer).await?;
        sqlx::query("INSERT INTO overlay(id,space_id,owner_id,root_composition_id,head_revision_id) VALUES($1,$2,$3,$4,$5)").bind(id).bind(space).bind(actor.actor_id).bind(layer.data.base.composition_id).bind(layer.revision).execute(&mut *tx).await.map_err(storage)?;
        sqlx::query("INSERT INTO reading_view(id,overlay_id,head_revision_id) VALUES($1,$2,$3)")
            .bind(view_id)
            .bind(id)
            .bind(layer.view.revision_id)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        let saved =
            persist::save(&mut tx, actor, &mut layer, None, &command.reason, &access).await?;
        persist::record(
            &mut tx,
            actor,
            command.request_id,
            &digest,
            "reading_create",
            &saved,
        )
        .await?;
        tx.commit().await.map_err(storage)?;
        Ok(saved)
    }

    pub async fn edit(
        &self,
        actor: Principal,
        id: Uuid,
        command: EditReading,
    ) -> Result<ReadingSaved, ContentError> {
        command.validate()?;
        let digest = command.digest(actor, id);
        let mut tx = request::begin(&self.pool, actor, command.request_id).await?;
        let mut layer =
            model::load(&mut tx, actor, id, Some(command.expected_overlay_revision)).await?;
        if let Some(saved) = replay(
            &mut tx,
            actor,
            command.request_id,
            &digest,
            "reading_edit",
            &[(layer.space, true)],
        )
        .await?
        {
            tx.commit().await.map_err(storage)?;
            return Ok(saved);
        }
        let mut access = model::access(&mut tx, actor, &layer).await?;
        if !access.complete(&layer) {
            return Err(ContentError::NotFound);
        }
        access.spaces.push((layer.space, true));
        let extra = match &command.edit {
            ReadingEdit::InsertExisting { block, .. }
            | ReadingEdit::AdoptExisting { block, .. } => vec![block.clone()],
            ReadingEdit::ReviseSelected { changes } => changes
                .iter()
                .map(|c| BlockRef {
                    block_id: c.block_id,
                    revision_id: c.base_revision_id,
                })
                .collect(),
            _ => vec![],
        };
        let (extra_blocks, extra_spaces) =
            model::blocks_with_spaces(&mut tx, actor, extra.clone()).await?;
        if extra_blocks.len() != extra.len() {
            return Err(ContentError::NotFound);
        }
        access.spaces.extend(extra_spaces);
        access.spaces.extend(extra_blocks.values().map(|(s, _)| {
            (
                *s,
                matches!(command.edit, ReadingEdit::ReviseSelected { .. }),
            )
        }));
        authorization::lock_grants(&mut tx, actor, &access.spaces).await?;
        access = model::access(&mut tx, actor, &layer).await?;
        if !access.complete(&layer)
            || model::blocks(&mut tx, actor, extra).await?.len() != extra_blocks.len()
        {
            return Err(ContentError::NotFound);
        }
        model::lock_heads(
            &mut tx,
            &layer,
            command.expected_overlay_revision,
            command.expected_reading_view_revision,
        )
        .await?;
        let original = layer.data.digest();
        let mut changed = vec![];
        let mut mapping = vec![];
        match &command.edit {
            ReadingEdit::InsertNew { drafts, target } => {
                for draft in drafts {
                    let block_id = Uuid::new_v4();
                    let revision_id = Uuid::new_v4();
                    sqlx::query("INSERT INTO block(id,space_id,head_revision_id) VALUES($1,$2,$3)")
                        .bind(block_id)
                        .bind(layer.space)
                        .bind(revision_id)
                        .execute(&mut *tx)
                        .await
                        .map_err(storage)?;
                    insert_revision(
                        &mut tx,
                        PendingRevision {
                            block_id,
                            space_id: layer.space,
                            revision_id,
                            parent: None,
                            actor,
                            draft,
                            reason: &command.reason,
                        },
                    )
                    .await?;
                    changed.push(BlockRef {
                        block_id,
                        revision_id,
                    });
                }
                edit::insert(
                    &mut layer.data,
                    target,
                    changed
                        .iter()
                        .cloned()
                        .map(|block| Placement {
                            placement_id: Uuid::new_v4(),
                            block,
                        })
                        .collect(),
                )?;
            }
            ReadingEdit::InsertExisting { block, target } => edit::insert(
                &mut layer.data,
                target,
                vec![Placement {
                    placement_id: Uuid::new_v4(),
                    block: block.clone(),
                }],
            )?,
            ReadingEdit::AdoptExisting {
                block,
                selected_placements,
            } => edit::select(&mut layer.data, selected_placements, block)?,
            ReadingEdit::ReviseSelected { changes } => {
                let mut changes: Vec<_> = changes.iter().collect();
                changes.sort_by_key(|c| c.block_id);
                for c in changes {
                    let head: Uuid = sqlx::query_scalar(
                        "SELECT head_revision_id FROM block WHERE id=$1 FOR UPDATE",
                    )
                    .bind(c.block_id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(storage)?;
                    if head != c.base_revision_id {
                        crate::references::load(
                            &mut tx,
                            actor,
                            &[ExactRef::Block(BlockRef {
                                block_id: c.block_id,
                                revision_id: head,
                            })],
                        )
                        .await?;
                        return Err(ContentError::Conflict {
                            current_revision_id: head,
                        });
                    }
                    let (space, old) = extra_blocks
                        .get(&BlockRef {
                            block_id: c.block_id,
                            revision_id: head,
                        })
                        .ok_or(ContentError::NotFound)?;
                    if c.draft.digest() == old.content_sha256 {
                        return invalid("no_change");
                    }
                    let revision_id = Uuid::new_v4();
                    insert_revision(
                        &mut tx,
                        PendingRevision {
                            block_id: c.block_id,
                            space_id: *space,
                            revision_id,
                            parent: Some(head),
                            actor,
                            draft: &c.draft,
                            reason: &command.reason,
                        },
                    )
                    .await?;
                    sqlx::query("UPDATE block SET head_revision_id=$1 WHERE id=$2")
                        .bind(revision_id)
                        .bind(c.block_id)
                        .execute(&mut *tx)
                        .await
                        .map_err(storage)?;
                    let block = BlockRef {
                        block_id: c.block_id,
                        revision_id,
                    };
                    edit::select(&mut layer.data, &c.selected_placements, &block)?;
                    changed.push(block);
                }
            }
            other => mapping = edit::structure(&mut layer.data, other)?,
        }
        if layer.data.digest() == original {
            return invalid("no_change");
        }
        let access = model::access(&mut tx, actor, &layer).await?;
        let mut saved = persist::save(
            &mut tx,
            actor,
            &mut layer,
            Some((
                command.expected_overlay_revision,
                command.expected_reading_view_revision,
            )),
            &command.reason,
            &access,
        )
        .await?;
        saved.changed_blocks = changed;
        for (source, result) in mapping {
            sqlx::query("INSERT INTO placement_manual_decision(overlay_id,overlay_revision_id,source_group_id,result_group_id) VALUES($1,$2,$3,$4)").bind(id).bind(saved.overlay.revision_id).bind(source).bind(result).execute(&mut *tx).await.map_err(storage)?;
        }
        persist::record(
            &mut tx,
            actor,
            command.request_id,
            &digest,
            "reading_edit",
            &saved,
        )
        .await?;
        tx.commit().await.map_err(storage)?;
        Ok(saved)
    }
}

async fn replay(
    tx: &mut Tx<'_>,
    actor: Principal,
    request_id: Uuid,
    digest: &str,
    operation: &str,
    required: &[(Uuid, bool)],
) -> Result<Option<ReadingSaved>, ContentError> {
    let Some(saved) = persist::receipt(tx, actor, request_id).await? else {
        if request::check(tx, actor, request_id, digest, operation).await? {
            return Err(ContentError::Storage);
        }
        return Ok(None);
    };
    let layer = model::load(
        tx,
        actor,
        saved.overlay.overlay_id,
        Some(saved.overlay.revision_id),
    )
    .await?;
    let mut access = model::access(tx, actor, &layer).await?;
    if !access.complete(&layer) {
        return Err(ContentError::NotFound);
    }
    let (blocks, block_spaces) =
        model::blocks_with_spaces(tx, actor, saved.changed_blocks.clone()).await?;
    if blocks.len() != saved.changed_blocks.len() {
        return Err(ContentError::NotFound);
    }
    access.spaces.extend_from_slice(required);
    access.spaces.extend(block_spaces);
    access
        .spaces
        .extend(blocks.values().map(|(s, _)| (*s, false)));
    authorization::lock_grants(tx, actor, &access.spaces).await?;
    if !model::access(tx, actor, &layer).await?.complete(&layer)
        || model::blocks(tx, actor, saved.changed_blocks.clone())
            .await?
            .len()
            != blocks.len()
    {
        return Err(ContentError::NotFound);
    }
    request::check(tx, actor, request_id, digest, operation).await?;
    Ok(Some(saved))
}

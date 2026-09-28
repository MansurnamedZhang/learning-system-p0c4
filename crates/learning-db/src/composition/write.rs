use super::{CompositionStore, closure};
use crate::{authorization, request, storage};
use learning_core::*;
use uuid::Uuid;

impl CompositionStore {
    pub async fn save(
        &self,
        actor: Principal,
        space: Uuid,
        command: SaveComposition,
    ) -> Result<CompositionRevision, ContentError> {
        command.validate()?;
        let digest = command.digest(space);
        let mut tx = request::begin(&self.pool, actor, command.request_id).await?;
        let id = command.composition_id.unwrap_or_else(Uuid::new_v4);
        let base = match command.base_revision_id {
            Some(revision_id) => {
                let reference = CompositionRef {
                    composition_id: id,
                    revision_id,
                };
                let (owner, _) = closure::load_composition(&mut tx, actor, &reference).await?;
                if owner != space {
                    return Err(ContentError::NotFound);
                }
                Some(reference)
            }
            None => None,
        };
        let bases: Vec<_> = base.iter().cloned().collect();
        let previous = closure::load(&mut tx, actor, &bases, None).await?;
        let proposed = closure::load(&mut tx, actor, &[], Some((id, &command.nodes))).await?;
        authorization::lock_grants(
            &mut tx,
            actor,
            &closure::merge_spaces(&[&previous, &proposed], space),
        )
        .await?;
        let previous = closure::load(&mut tx, actor, &bases, None).await?;
        let proposed = closure::load(&mut tx, actor, &[], Some((id, &command.nodes))).await?;
        if request::check(
            &mut tx,
            actor,
            command.request_id,
            &digest,
            "composition_save",
        )
        .await?
        {
            let saved:Option<(Uuid,Uuid)>=sqlx::query_as("SELECT r.composition_id,r.id FROM public.composition_receipt c JOIN public.composition_revision r ON r.id=c.revision_id WHERE c.actor_id=$1 AND c.request_id=$2").bind(actor.actor_id).bind(command.request_id).fetch_optional(&mut *tx).await.map_err(storage)?;
            let (composition_id, revision_id) = saved.ok_or(ContentError::Storage)?;
            let reference = CompositionRef {
                composition_id,
                revision_id,
            };
            let mut checked =
                closure::load(&mut tx, actor, std::slice::from_ref(&reference), None).await?;
            let result = checked
                .compositions
                .remove(&reference)
                .ok_or(ContentError::Storage)?
                .1;
            tx.commit().await.map_err(storage)?;
            return Ok(result);
        }
        if let Some(base) = &base {
            let (head,kind):(Uuid,String)=sqlx::query_as("SELECT head_revision_id,kind FROM public.composition WHERE space_id=$1 AND id=$2 FOR UPDATE").bind(space).bind(id).fetch_optional(&mut *tx).await.map_err(storage)?.ok_or(ContentError::NotFound)?;
            if head != base.revision_id {
                closure::load(
                    &mut tx,
                    actor,
                    &[CompositionRef {
                        composition_id: id,
                        revision_id: head,
                    }],
                    None,
                )
                .await?;
                return Err(ContentError::Conflict {
                    current_revision_id: head,
                });
            }
            let expected = match command.kind {
                CompositionKind::Document => "document",
                CompositionKind::Section => "section",
            };
            if kind != expected {
                return Err(ContentError::Invalid("composition_kind_immutable".into()));
            }
        }
        let mut nodes = Vec::with_capacity(command.nodes.len());
        for node in &command.nodes {
            let occurrence_id = if let Some(occurrence_id) = node.occurrence_id {
                let prior = base
                    .as_ref()
                    .and_then(|b| previous.compositions.get(b))
                    .and_then(|(_, r)| r.nodes.iter().find(|n| n.occurrence_id == occurrence_id));
                if !prior.is_some_and(|n| closure::same_identity(&n.target, &node.target)) {
                    return Err(ContentError::Invalid("occurrence_identity".into()));
                }
                occurrence_id
            } else {
                Uuid::new_v4()
            };
            nodes.push(Occurrence {
                occurrence_id,
                target: node.target.clone(),
            });
        }
        let revision_id = Uuid::new_v4();
        let kind = match command.kind {
            CompositionKind::Document => "document",
            CompositionKind::Section => "section",
        };
        if base.is_none() {
            sqlx::query("INSERT INTO public.composition(id,space_id,kind,head_revision_id) VALUES($1,$2,$3,$4)").bind(id).bind(space).bind(kind).bind(revision_id).execute(&mut *tx).await.map_err(storage)?;
        }
        let hash = composition_digest(command.kind, &command.title, &nodes);
        let created_at=sqlx::query_scalar("INSERT INTO public.composition_revision(id,space_id,composition_id,parent_revision_id,kind,title,content_sha256,author_id,reason) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9) RETURNING created_at").bind(revision_id).bind(space).bind(id).bind(command.base_revision_id).bind(kind).bind(&command.title).bind(&hash).bind(actor.actor_id).bind(&command.reason).fetch_one(&mut *tx).await.map_err(storage)?;
        for (position, node) in nodes.iter().enumerate() {
            let target_space = proposed.target_space(&node.target)?;
            let (bs, b, br, cs, c, cr) = match &node.target {
                NodeTarget::Block(r) => (
                    Some(target_space),
                    Some(r.block_id),
                    Some(r.revision_id),
                    None,
                    None,
                    None,
                ),
                NodeTarget::Composition(r) => (
                    None,
                    None,
                    None,
                    Some(target_space),
                    Some(r.composition_id),
                    Some(r.revision_id),
                ),
            };
            sqlx::query("INSERT INTO public.composition_occurrence(composition_revision_id,space_id,composition_id,occurrence_id,position,block_space_id,block_id,block_revision_id,child_space_id,child_composition_id,child_revision_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)").bind(revision_id).bind(space).bind(id).bind(node.occurrence_id).bind(position as i32).bind(bs).bind(b).bind(br).bind(cs).bind(c).bind(cr).execute(&mut *tx).await.map_err(storage)?;
        }
        sqlx::query("UPDATE public.composition SET head_revision_id=$1 WHERE id=$2")
            .bind(revision_id)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        request::register(
            &mut tx,
            actor,
            command.request_id,
            &digest,
            "composition_save",
        )
        .await?;
        sqlx::query("INSERT INTO public.composition_receipt(actor_id,request_id,revision_id) VALUES($1,$2,$3)").bind(actor.actor_id).bind(command.request_id).bind(revision_id).execute(&mut *tx).await.map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        Ok(CompositionRevision {
            reference: CompositionRef {
                composition_id: id,
                revision_id,
            },
            parent_revision_id: command.base_revision_id,
            kind: command.kind,
            title: command.title,
            nodes,
            content_sha256: hash,
            author_id: actor.actor_id,
            reason: command.reason,
            created_at,
        })
    }
}

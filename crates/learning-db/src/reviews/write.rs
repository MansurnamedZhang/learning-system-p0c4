use super::ReviewStore;
use crate::{
    references,
    relations::{check_dependencies, enum_text, lock_dependencies, scope},
    request, storage,
};
use learning_core::*;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

impl ReviewStore {
    pub async fn append(
        &self,
        actor: Principal,
        command: AppendEpistemicReview,
    ) -> Result<EpistemicReview, ContentError> {
        command.validate()?;
        let c = &command;
        let digest = hex_digest(canonical_json(&json!({"domain":"epistemic-review-request-v1","operation":"epistemic_review","actor_id":actor.actor_id,"command":c})).as_bytes());
        let mut tx = request::begin(&self.pool, actor, c.request_id).await?;
        let receipt: Option<(Uuid, Uuid)> = sqlx::query_as("SELECT r.stream_id,r.id FROM public.epistemic_review_receipt m JOIN public.epistemic_review r ON r.id=m.review_id WHERE m.actor_id=$1 AND m.request_id=$2")
            .bind(actor.actor_id).bind(c.request_id).fetch_optional(&mut *tx).await.map_err(storage)?;
        let deps = c.dependencies();
        let mut roots: Vec<_> = deps.iter().map(|d| d.target.clone()).collect();
        if let Some((stream_id, review_id)) = receipt {
            roots.push(ExactRef::EpistemicReview(EpistemicReviewRef {
                stream_id,
                review_id,
            }));
        }
        let allowed = lock_dependencies(&mut tx, actor, &c.scope, &roots).await?;
        // Replay checks current authorization of the original fixed record, but
        // never applies creation rules against a newer relation review head.
        if request::check(&mut tx, actor, c.request_id, &digest, "epistemic_review").await? {
            let (stream_id, review_id) = receipt.ok_or(ContentError::Storage)?;
            let value = references::project(
                &mut tx,
                actor,
                &ExactRef::EpistemicReview(EpistemicReviewRef {
                    stream_id,
                    review_id,
                }),
            )
            .await?
            .ok_or(ContentError::NotFound)?
            .epistemic_review()?;
            tx.commit().await.map_err(storage)?;
            return Ok(value);
        }
        let identities: BTreeSet<_> = c.relations.iter().map(|r| r.relation.relation_id).collect();
        for id in identities {
            sqlx::query("SELECT id FROM public.relation WHERE id=$1 FOR UPDATE")
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(storage)?;
        }
        let selected: BTreeSet<_> = c.relations.iter().map(|r| r.relation.clone()).collect();
        let mut heads = BTreeMap::new();
        for r in selected {
            let head: Option<Uuid> = sqlx::query_scalar("SELECT head_review_id FROM public.relation_review_head WHERE relation_id=$1 AND relation_revision_id=$2 FOR UPDATE")
                .bind(r.relation_id).bind(r.revision_id).fetch_optional(&mut *tx).await.map_err(storage)?;
            let state: Option<String> = if let Some(head) = head {
                sqlx::query_scalar("SELECT state FROM public.relation_review WHERE id=$1")
                    .bind(head)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(storage)?
            } else {
                None
            };
            heads.insert(r, state);
        }
        let closure = check_dependencies(&mut tx, actor, &roots, &allowed).await?;
        validate_judgment(c, &closure, &heads)?;
        let (space, overlay) = scope(&c.scope);
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!(
                "learning/epistemic/{}:{}:{}:{}:{}",
                space,
                overlay.map_or_else(|| "space".into(), |id| id.to_string()),
                c.target.block_id,
                c.target.revision_id,
                actor.actor_id
            ))
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        let stream: Option<(Uuid, Uuid)> = sqlx::query_as("SELECT id,head_review_id FROM public.epistemic_stream WHERE space_id=$1 AND overlay_id IS NOT DISTINCT FROM $2 AND target_block_id=$3 AND target_revision_id=$4 AND actor_id=$5 FOR UPDATE")
            .bind(space).bind(overlay).bind(c.target.block_id).bind(c.target.revision_id).bind(actor.actor_id).fetch_optional(&mut *tx).await.map_err(storage)?;
        let previous = stream.map(|(stream_id, review_id)| EpistemicReviewRef {
            stream_id,
            review_id,
        });
        if previous != c.expected_previous {
            if let Some(current) = previous {
                let mut conflict_roots = roots.clone();
                conflict_roots.push(ExactRef::EpistemicReview(current.clone()));
                check_dependencies(&mut tx, actor, &conflict_roots, &allowed).await?;
                return Err(ContentError::Conflict {
                    current_revision_id: current.review_id,
                });
            }
            return Err(ContentError::Invalid(
                "epistemic_predecessor_mismatch".into(),
            ));
        }
        let review_id = Uuid::new_v4();
        let stream_id = stream.map_or_else(Uuid::new_v4, |(s, _)| s);
        if stream.is_none() {
            let target_space = closure
                .objects
                .get(&ExactRef::Block(c.target.clone()))
                .ok_or(ContentError::Storage)?
                .0;
            sqlx::query("INSERT INTO public.epistemic_stream(id,space_id,overlay_id,target_space_id,target_block_id,target_revision_id,actor_id,head_review_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
                .bind(stream_id).bind(space).bind(overlay).bind(target_space).bind(c.target.block_id).bind(c.target.revision_id).bind(actor.actor_id).bind(review_id).execute(&mut *tx).await.map_err(storage)?;
        }
        sqlx::query("INSERT INTO public.epistemic_review(id,space_id,stream_id,previous_review_id,state,relations,evidence,conditions,explanation,reviewer_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
            .bind(review_id).bind(space).bind(stream_id).bind(previous.map(|r| r.review_id)).bind(enum_text(json!(c.state))?).bind(json!(c.relations)).bind(json!(c.evidence)).bind(&c.conditions).bind(&c.explanation).bind(actor.actor_id).execute(&mut *tx).await.map_err(storage)?;
        let reference = EpistemicReviewRef {
            stream_id,
            review_id,
        };
        references::insert(
            &mut tx,
            &ExactRef::EpistemicReview(reference.clone()),
            &deps,
        )
        .await?;
        if stream.is_some() {
            sqlx::query("UPDATE public.epistemic_stream SET head_review_id=$1 WHERE id=$2")
                .bind(review_id)
                .bind(stream_id)
                .execute(&mut *tx)
                .await
                .map_err(storage)?;
        }
        request::register(&mut tx, actor, c.request_id, &digest, "epistemic_review").await?;
        sqlx::query("INSERT INTO public.epistemic_review_receipt(actor_id,request_id,review_id) VALUES($1,$2,$3)").bind(actor.actor_id).bind(c.request_id).bind(review_id).execute(&mut *tx).await.map_err(storage)?;
        let value = references::project(&mut tx, actor, &ExactRef::EpistemicReview(reference))
            .await?
            .ok_or(ContentError::NotFound)?
            .epistemic_review()?;
        tx.commit().await.map_err(storage)?;
        Ok(value)
    }
}

fn validate_judgment(
    c: &AppendEpistemicReview,
    closure: &references::AuthorizedClosure,
    heads: &BTreeMap<RelationRef, Option<String>>,
) -> Result<(), ContentError> {
    let target = closure
        .objects
        .get(&ExactRef::Block(c.target.clone()))
        .ok_or(ContentError::Storage)?
        .1
        .clone()
        .block()?;
    let intent = match target.draft {
        ContentDraft::V1(d) => d.intent,
        ContentDraft::V2(d) => d.intent,
    };
    if !matches!(intent, Intent::Conjecture | Intent::Conclusion) {
        return Err(ContentError::Invalid(
            "invalid_epistemic_target_intent".into(),
        ));
    }
    let direction = match c.state {
        EpistemicState::SupportedWithinScope => Some(RelationType::Supports),
        EpistemicState::RefutedWithinScope => Some(RelationType::Opposes),
        _ => None,
    };
    let mut has_basis = false;
    for selection in &c.relations {
        let relation = closure
            .objects
            .get(&ExactRef::Relation(selection.relation.clone()))
            .ok_or(ContentError::Storage)?
            .1
            .clone()
            .relation()?;
        if matches!(
            relation.relation_type,
            RelationType::Supports | RelationType::Opposes
        ) {
            if relation.to != c.target || !c.evidence.contains(&relation.from) {
                return Err(ContentError::Invalid(
                    "epistemic_evidence_target_mismatch".into(),
                ));
            }
        } else if relation.from != c.target && relation.to != c.target {
            return Err(ContentError::Invalid(
                "epistemic_relation_target_mismatch".into(),
            ));
        }
        if direction == Some(relation.relation_type)
            && let Some(review) = &selection.review
        {
            let review = closure
                .objects
                .get(&ExactRef::RelationReview(review.clone()))
                .ok_or(ContentError::Storage)?
                .1
                .clone()
                .relation_review()?;
            if review.state == RelationReviewState::Reviewed
                && heads.get(&selection.relation).and_then(Option::as_deref) == Some("reviewed")
            {
                has_basis = true;
            }
        }
    }
    if direction.is_some() && !has_basis {
        return Err(ContentError::Invalid(
            "epistemic_reviewed_basis_required".into(),
        ));
    }
    Ok(())
}

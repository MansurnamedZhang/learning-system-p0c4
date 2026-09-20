use super::{RelationStore, check_dependencies, enum_text, lock_dependencies, scope};
use crate::{references, request, storage};
use learning_core::*;
use serde_json::json;
use uuid::Uuid;

impl RelationStore {
    pub async fn review(
        &self,
        actor: Principal,
        command: ReviewRelation,
    ) -> Result<RelationReview, ContentError> {
        command.validate()?;
        let c = &command;
        let digest=hex_digest(canonical_json(&json!({"domain":"relation-review-request-v1","operation":"relation_review","actor_id":actor.actor_id,"command":c})).as_bytes());
        let mut tx = request::begin(&self.pool, actor, c.request_id).await?;
        let target = ExactRef::Relation(c.relation.clone());
        let relation = references::project(&mut tx, actor, &target)
            .await?
            .ok_or(ContentError::NotFound)?
            .relation()?;
        let receipt:Option<(Uuid,Uuid,Uuid)>=sqlx::query_as("SELECT r.relation_id,r.relation_revision_id,r.id FROM public.relation_review_receipt m JOIN public.relation_review r ON r.id=m.review_id WHERE m.actor_id=$1 AND m.request_id=$2")
            .bind(actor.actor_id).bind(c.request_id).fetch_optional(&mut *tx).await.map_err(storage)?;
        let mut roots = vec![target.clone()];
        if let Some((relation_id, revision_id, review_id)) = receipt {
            roots.push(ExactRef::RelationReview(RelationReviewRef {
                relation: RelationRef {
                    relation_id,
                    revision_id,
                },
                review_id,
            }));
        }
        let allowed = lock_dependencies(&mut tx, actor, &relation.scope, &roots).await?;
        if request::check(&mut tx, actor, c.request_id, &digest, "relation_review").await? {
            let (relation_id, revision_id, review_id) = receipt.ok_or(ContentError::Storage)?;
            let reference = RelationReviewRef {
                relation: RelationRef {
                    relation_id,
                    revision_id,
                },
                review_id,
            };
            let result = references::project(&mut tx, actor, &ExactRef::RelationReview(reference))
                .await?
                .ok_or(ContentError::NotFound)?
                .relation_review()?;
            tx.commit().await.map_err(storage)?;
            return Ok(result);
        }
        sqlx::query("SELECT id FROM public.relation WHERE id=$1 FOR UPDATE")
            .bind(c.relation.relation_id)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        check_dependencies(&mut tx, actor, &roots, &allowed).await?;
        let head:Option<Uuid>=sqlx::query_scalar("SELECT head_review_id FROM public.relation_review_head WHERE relation_id=$1 AND relation_revision_id=$2 FOR UPDATE")
            .bind(c.relation.relation_id).bind(c.relation.revision_id).fetch_optional(&mut *tx).await.map_err(storage)?;
        if head != c.expected_previous {
            if let Some(head) = head {
                let mut current_roots = roots.clone();
                current_roots.push(ExactRef::RelationReview(RelationReviewRef {
                    relation: c.relation.clone(),
                    review_id: head,
                }));
                check_dependencies(&mut tx, actor, &current_roots, &allowed).await?;
                return Err(ContentError::Conflict {
                    current_revision_id: head,
                });
            }
            return Err(ContentError::Invalid(
                "relation_review_predecessor_mismatch".into(),
            ));
        }
        if c.state == RelationReviewState::Reviewed
            && matches!(
                relation.relation_type,
                RelationType::Supports | RelationType::Opposes
            )
            && (relation.rationale.trim().is_empty()
                || relation.conditions.trim().is_empty()
                || c.explanation.trim().is_empty())
        {
            return Err(ContentError::Invalid(
                "reviewed_relation_requires_explanation".into(),
            ));
        }
        let review = Uuid::new_v4();
        let space = scope(&relation.scope).0;
        if head.is_none() {
            sqlx::query("INSERT INTO public.relation_review_head(relation_id,relation_revision_id,head_review_id) VALUES($1,$2,$3)").bind(c.relation.relation_id).bind(c.relation.revision_id).bind(review).execute(&mut *tx).await.map_err(storage)?;
        }
        sqlx::query("INSERT INTO public.relation_review(id,space_id,relation_id,relation_revision_id,previous_review_id,state,explanation,reviewer_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
            .bind(review).bind(space).bind(c.relation.relation_id).bind(c.relation.revision_id).bind(head).bind(enum_text(json!(c.state))?).bind(&c.explanation).bind(actor.actor_id).execute(&mut *tx).await.map_err(storage)?;
        let reference = RelationReviewRef {
            relation: c.relation.clone(),
            review_id: review,
        };
        references::insert(
            &mut tx,
            &ExactRef::RelationReview(reference.clone()),
            &[Dependency {
                role: DependencyRole::Target,
                target,
            }],
        )
        .await?;
        if head.is_some() {
            sqlx::query("UPDATE public.relation_review_head SET head_review_id=$1 WHERE relation_id=$2 AND relation_revision_id=$3").bind(review).bind(c.relation.relation_id).bind(c.relation.revision_id).execute(&mut *tx).await.map_err(storage)?;
        }
        request::register(&mut tx, actor, c.request_id, &digest, "relation_review").await?;
        sqlx::query("INSERT INTO public.relation_review_receipt(actor_id,request_id,review_id) VALUES($1,$2,$3)").bind(actor.actor_id).bind(c.request_id).bind(review).execute(&mut *tx).await.map_err(storage)?;
        let result = references::project(&mut tx, actor, &ExactRef::RelationReview(reference))
            .await?
            .ok_or(ContentError::NotFound)?
            .relation_review()?;
        tx.commit().await.map_err(storage)?;
        Ok(result)
    }
}

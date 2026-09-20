use super::Session;
use crate::{COLUMNS, RevisionRow, storage};
use learning_core::*;
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

#[derive(Clone)]
pub(crate) enum AuthorizedObject {
    Block(ContentRevision),
    Relation(RelationRevision),
    RelationReview(RelationReview),
    EpistemicReview(EpistemicReview),
}
impl AuthorizedObject {
    pub fn block(self) -> Result<ContentRevision, ContentError> {
        match self {
            Self::Block(r) => Ok(r),
            _ => Err(ContentError::Storage),
        }
    }
    pub(super) fn previous(&self) -> Option<ExactRef> {
        match self {
            Self::Block(r) => r.parent_revision_id.map(|id| {
                ExactRef::Block(BlockRef {
                    block_id: r.block_id,
                    revision_id: id,
                })
            }),
            Self::Relation(r) => r.parent_revision_id.map(|id| {
                ExactRef::Relation(RelationRef {
                    relation_id: r.reference.relation_id,
                    revision_id: id,
                })
            }),
            Self::RelationReview(r) => r.previous_review_id.map(|id| {
                ExactRef::RelationReview(RelationReviewRef {
                    relation: r.reference.relation.clone(),
                    review_id: id,
                })
            }),
            Self::EpistemicReview(r) => r.previous.clone().map(ExactRef::EpistemicReview),
        }
    }
    pub(super) fn redact_previous(&mut self) {
        match self {
            Self::Block(r) => r.parent_revision_id = None,
            Self::Relation(r) => r.parent_revision_id = None,
            Self::RelationReview(r) => r.previous_review_id = None,
            Self::EpistemicReview(r) => r.previous = None,
        }
    }
}
pub(crate) async fn project(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    root: &ExactRef,
) -> Result<Option<AuthorizedObject>, ContentError> {
    Session::default().object(tx, actor, root).await
}
fn scope(space_id: Uuid, overlay_id: Option<Uuid>) -> RelationScope {
    match overlay_id {
        Some(overlay_id) => RelationScope::PersonalOverlay {
            space_id,
            overlay_id,
        },
        None => RelationScope::Space { space_id },
    }
}
fn value(r: &sqlx::postgres::PgRow, field: &str) -> serde_json::Value {
    serde_json::Value::String(r.get(field))
}
pub(super) async fn materialize(
    tx: &mut Transaction<'_, Postgres>,
    r: &ExactRef,
) -> Result<AuthorizedObject, ContentError> {
    Ok(match r {
        ExactRef::Block(r) => {
            let row = sqlx::query_as::<_, RevisionRow>(&format!(
                "SELECT {COLUMNS} FROM public.block_revision r WHERE r.block_id=$1 AND r.id=$2"
            ))
            .bind(r.block_id)
            .bind(r.revision_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(storage)?;
            AuthorizedObject::Block(row.decode()?)
        }
        ExactRef::Relation(reference) => {
            let r=sqlx::query("SELECT r.*,s.overlay_id,s.type,s.origin FROM public.relation_revision r JOIN public.relation s ON s.id=r.relation_id WHERE r.relation_id=$1 AND r.id=$2")
                .bind(reference.relation_id).bind(reference.revision_id).fetch_one(&mut **tx).await.map_err(storage)?;
            AuthorizedObject::Relation(RelationRevision {
                reference: reference.clone(),
                scope: scope(r.get("space_id"), r.get("overlay_id")),
                relation_type: serde_json::from_value(value(&r, "type")).map_err(storage)?,
                origin: serde_json::from_value(value(&r, "origin")).map_err(storage)?,
                parent_revision_id: r.get("parent_revision_id"),
                from: BlockRef {
                    block_id: r.get("from_block_id"),
                    revision_id: r.get("from_revision_id"),
                },
                to: BlockRef {
                    block_id: r.get("to_block_id"),
                    revision_id: r.get("to_revision_id"),
                },
                rationale: r.get("rationale"),
                conditions: r.get("conditions"),
                content_sha256: r.get("content_sha256"),
                author_id: r.get("author_id"),
                created_at: r.get("created_at"),
            })
        }
        ExactRef::RelationReview(reference) => {
            let r=sqlx::query("SELECT * FROM public.relation_review WHERE relation_id=$1 AND relation_revision_id=$2 AND id=$3")
                .bind(reference.relation.relation_id).bind(reference.relation.revision_id).bind(reference.review_id).fetch_one(&mut **tx).await.map_err(storage)?;
            AuthorizedObject::RelationReview(RelationReview {
                reference: reference.clone(),
                previous_review_id: r.get("previous_review_id"),
                state: serde_json::from_value(value(&r, "state")).map_err(storage)?,
                explanation: r.get("explanation"),
                reviewer_id: r.get("reviewer_id"),
                created_at: r.get("created_at"),
            })
        }
        ExactRef::EpistemicReview(reference) => {
            let r=sqlx::query("SELECT r.*,s.overlay_id,s.target_block_id,s.target_revision_id FROM public.epistemic_review r JOIN public.epistemic_stream s ON s.id=r.stream_id WHERE r.stream_id=$1 AND r.id=$2")
                .bind(reference.stream_id).bind(reference.review_id).fetch_one(&mut **tx).await.map_err(storage)?;
            AuthorizedObject::EpistemicReview(EpistemicReview {
                reference: reference.clone(),
                scope: scope(r.get("space_id"), r.get("overlay_id")),
                target: BlockRef {
                    block_id: r.get("target_block_id"),
                    revision_id: r.get("target_revision_id"),
                },
                previous: r
                    .get::<Option<Uuid>, _>("previous_review_id")
                    .map(|review_id| EpistemicReviewRef {
                        stream_id: reference.stream_id,
                        review_id,
                    }),
                state: serde_json::from_value(value(&r, "state")).map_err(storage)?,
                relations: r
                    .get::<sqlx::types::Json<Vec<RelationSelection>>, _>("relations")
                    .0,
                evidence: r.get::<sqlx::types::Json<Vec<BlockRef>>, _>("evidence").0,
                conditions: r.get("conditions"),
                explanation: r.get("explanation"),
                reviewer_id: r.get("reviewer_id"),
                created_at: r.get("created_at"),
            })
        }
    })
}

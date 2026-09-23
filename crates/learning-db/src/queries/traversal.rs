use super::{consumers, scope};
use crate::{references, storage};
use learning_core::*;
use sqlx::{Postgres, Transaction};
use std::collections::BTreeMap;
use uuid::Uuid;

const BATCH: i64 = 128;
const INTERNAL_SCAN_LIMIT: usize = 100_000;

fn exact(node: &ImpactNode) -> Option<ExactRef> {
    Some(match node {
        ImpactNode::Block(r) => ExactRef::Block(r.clone()),
        ImpactNode::Relation(r) => ExactRef::Relation(r.clone()),
        ImpactNode::RelationReview(r) => ExactRef::RelationReview(r.clone()),
        ImpactNode::EpistemicReview(r) => ExactRef::EpistemicReview(r.clone()),
        _ => return None,
    })
}

/// Reverse necessary references. This is separate from the already accepted
/// direct query so each traversal hop and each candidate has its own complete
/// B3 closure validation without changing that API's operational semantics.
pub(super) async fn necessary_neighbors(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    current: &ImpactNode,
    fixed: &scope::FixedScope,
    budget: &mut VisibleWorkBudget,
) -> Result<Vec<ImpactStep>, consumers::DirectError> {
    let Some(target) = exact(current) else {
        return Ok(vec![]);
    };
    if !fixed
        .members
        .contains_key(&ImpactStart::from(target.clone()))
    {
        return Ok(vec![]);
    }
    let mut target_session = references::Session::default();
    if !target_session.authorize(tx, actor, &target).await? {
        return Ok(vec![]);
    }
    let (kind, object, revision) = references::key(&target);
    let mut after: Option<(String, Uuid, Uuid, i32)> = None;
    let mut scanned = 0usize;
    let mut groups = BTreeMap::new();
    loop {
        let rows: Vec<(String, Uuid, Uuid, i32, String, Option<Uuid>)> = sqlx::query_as(
            "SELECT d.source_kind,d.source_object_id,d.source_revision_id,d.position,d.role,rr.relation_id \
             FROM public.reference_dependency d \
             JOIN public.reference_object s ON (s.kind,s.object_id,s.revision_id)=(d.source_kind,d.source_object_id,d.source_revision_id) \
             JOIN public.space_grant g ON g.space_id=s.space_id AND g.actor_id=$1 \
             LEFT JOIN public.relation_review rr ON d.source_kind='relation_review' AND rr.relation_revision_id=d.source_object_id AND rr.id=d.source_revision_id \
             WHERE (d.target_kind,d.target_object_id,d.target_revision_id)=($2,$3,$4) \
               AND ($5::text IS NULL OR (d.source_kind,d.source_object_id,d.source_revision_id,d.position)>($5,$6,$7,$8)) \
               AND (d.source_kind='block' OR \
                    (d.source_kind='relation' AND EXISTS(SELECT 1 FROM public.relation r LEFT JOIN public.overlay o ON o.id=r.overlay_id WHERE r.id=d.source_object_id AND (r.overlay_id IS NULL OR o.owner_id=$1))) OR \
                    (d.source_kind='relation_review' AND EXISTS(SELECT 1 FROM public.relation r LEFT JOIN public.overlay o ON o.id=r.overlay_id WHERE r.id=rr.relation_id AND (r.overlay_id IS NULL OR o.owner_id=$1))) OR \
                    (d.source_kind='epistemic_review' AND EXISTS(SELECT 1 FROM public.epistemic_review er JOIN public.epistemic_stream es ON es.id=er.stream_id LEFT JOIN public.overlay o ON o.id=es.overlay_id WHERE er.stream_id=d.source_object_id AND er.id=d.source_revision_id AND (es.overlay_id IS NULL OR o.owner_id=$1)))) \
             ORDER BY d.source_kind,d.source_object_id,d.source_revision_id,d.position LIMIT $9",
        )
        .bind(actor.actor_id)
        .bind(kind)
        .bind(object)
        .bind(revision)
        .bind(after.as_ref().map(|v| v.0.as_str()))
        .bind(after.as_ref().map(|v| v.1))
        .bind(after.as_ref().map(|v| v.2))
        .bind(after.as_ref().map(|v| v.3))
        .bind(BATCH)
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| consumers::DirectError::Storage(storage(e)))?;
        let count = rows.len();
        for (source_kind, source_object, source_revision, position, role, relation_id) in rows {
            scanned += 1;
            if scanned > INTERNAL_SCAN_LIMIT {
                return Err(consumers::DirectError::Storage(ContentError::Storage));
            }
            after = Some((
                source_kind.clone(),
                source_object,
                source_revision,
                position,
            ));
            let source = match source_kind.as_str() {
                "block" => ExactRef::Block(BlockRef {
                    block_id: source_object,
                    revision_id: source_revision,
                }),
                "relation" => ExactRef::Relation(RelationRef {
                    relation_id: source_object,
                    revision_id: source_revision,
                }),
                "relation_review" => ExactRef::RelationReview(RelationReviewRef {
                    relation: RelationRef {
                        relation_id: relation_id
                            .ok_or(consumers::DirectError::Storage(ContentError::Storage))?,
                        revision_id: source_object,
                    },
                    review_id: source_revision,
                }),
                "epistemic_review" => ExactRef::EpistemicReview(EpistemicReviewRef {
                    stream_id: source_object,
                    review_id: source_revision,
                }),
                _ => return Err(consumers::DirectError::Storage(ContentError::Storage)),
            };
            if !fixed
                .members
                .contains_key(&ImpactStart::from(source.clone()))
            {
                continue;
            }
            let mut candidate_session = references::Session::default();
            if !candidate_session.authorize(tx, actor, &source).await? {
                continue;
            }
            let role: DependencyRole = serde_json::from_value(serde_json::Value::String(role))
                .map_err(|_| consumers::DirectError::Storage(ContentError::Storage))?;
            let position = u32::try_from(position)
                .map_err(|_| consumers::DirectError::Storage(ContentError::Storage))?;
            consumers::append(
                &mut groups,
                budget,
                ImpactStep {
                    from: current.clone(),
                    to: consumers::node(&ImpactStart::from(source)),
                    family: ImpactFamily::Necessary,
                    dependency_role: Some(role),
                    dependency_position: Some(position),
                    direction: None,
                    relation_type: None,
                    lineage_type: None,
                    provenance: ImpactProvenance::Stored,
                    location: None,
                    reason: ImpactReason::RequiresExactRevision,
                },
            )?;
        }
        if count < BATCH as usize {
            break;
        }
    }
    let mut steps = groups
        .into_values()
        .flat_map(|group: ImpactConsumerGroup| group.explanations)
        .flat_map(|explanation| explanation.steps)
        .collect::<Vec<_>>();
    steps.sort_by_key(|step| serde_json::to_string(step).expect("step serializes"));
    Ok(steps)
}

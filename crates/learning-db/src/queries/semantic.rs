use super::{consumers, scope};
use crate::{reading::selection, references, storage};
use learning_core::*;
use sqlx::{Postgres, Transaction};
use std::collections::BTreeSet;
use uuid::Uuid;

const BATCH: i64 = 128;
const INTERNAL_SCAN_LIMIT: usize = 100_000;

fn relation_entry(
    endpoint: &BlockRef,
    relation: &RelationRevision,
    provenance: ImpactProvenance,
) -> Option<ImpactStep> {
    let direction = if relation.from == *endpoint {
        TraversalDirection::SavedForward
    } else if relation.to == *endpoint {
        TraversalDirection::SavedReverse
    } else {
        return None;
    };
    Some(ImpactStep {
        from: ImpactNode::Block(endpoint.clone()),
        to: ImpactNode::Relation(relation.reference.clone()),
        family: ImpactFamily::Semantic,
        dependency_role: None,
        dependency_position: None,
        direction: Some(direction),
        relation_type: Some(relation.relation_type),
        lineage_type: None,
        provenance,
        location: None,
        reason: ImpactReason::SuggestReview,
    })
}

fn push_visible(
    steps: &mut Vec<ImpactStep>,
    seen_edges: &mut BTreeSet<(ImpactNode, ImpactNode)>,
    budget: &mut VisibleWorkBudget,
    step: ImpactStep,
) -> Result<(), consumers::DirectError> {
    step.validate()?;
    if steps.contains(&step) {
        return Ok(());
    }
    budget
        .charge_node(step.to.clone())
        .map_err(|_| consumers::DirectError::Budget)?;
    if seen_edges.insert((step.from.clone(), step.to.clone())) {
        budget
            .charge_edge()
            .map_err(|_| consumers::DirectError::Budget)?;
    } else {
        budget
            .charge_step()
            .map_err(|_| consumers::DirectError::Budget)?;
    }
    steps.push(step);
    Ok(())
}

async fn block_neighbors(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    endpoint: &BlockRef,
    fixed: &scope::FixedScope,
    impact_scope: &ImpactScope,
    budget: &mut VisibleWorkBudget,
) -> Result<Vec<ImpactStep>, consumers::DirectError> {
    let mut endpoint_session = references::Session::default();
    if !endpoint_session
        .authorize(tx, actor, &ExactRef::Block(endpoint.clone()))
        .await?
    {
        return Ok(vec![]);
    }
    let mut steps = vec![];
    let mut seen_edges = BTreeSet::new();
    let fixed_provenance = match impact_scope {
        ImpactScope::Reading { .. } => ImpactProvenance::FixedReadingSelection,
        ImpactScope::Release { .. } => ImpactProvenance::FixedReleaseManifest,
    };
    for view in &fixed.reading_views {
        let (_, choices) = selection::choices(tx, view).await?;
        for choice in choices.selections {
            let mut candidate_session = references::Session::default();
            let Some(object) = candidate_session
                .object(tx, actor, &ExactRef::Relation(choice.relation))
                .await?
            else {
                continue;
            };
            let relation = object.relation()?;
            if let Some(step) = relation_entry(endpoint, &relation, fixed_provenance) {
                push_visible(&mut steps, &mut seen_edges, budget, step)?;
            }
        }
    }
    // Work-state discovery is restricted to an actually displayed Reading
    // endpoint. Historical Release membership never upgrades to a current head.
    if matches!(impact_scope, ImpactScope::Reading { .. })
        && fixed.members.get(&ImpactStart::Block(endpoint.clone()))
            == Some(&ImpactMembership::Displayed)
    {
        let mut after: Option<(Uuid, Uuid, i32)> = None;
        let mut scanned = 0usize;
        loop {
            let rows: Vec<(Uuid, Uuid, i32)> = sqlx::query_as(
                "SELECT d.source_object_id,d.source_revision_id,d.position \
                 FROM public.reference_dependency d \
                 JOIN public.relation r ON r.id=d.source_object_id AND r.head_revision_id=d.source_revision_id \
                 JOIN public.space_grant g ON g.space_id=r.space_id AND g.actor_id=$1 \
                 WHERE d.target_kind='block' AND d.target_object_id=$2 AND d.target_revision_id=$3 \
                   AND d.source_kind='relation' AND d.role='target' \
                   AND (r.overlay_id IS NULL OR EXISTS(SELECT 1 FROM public.overlay o \
                       WHERE o.id=r.overlay_id AND o.owner_id=$1 AND o.space_id=r.space_id)) \
                   AND ($4::uuid IS NULL OR (d.source_object_id,d.source_revision_id,d.position)>($4,$5,$6)) \
                 ORDER BY d.source_object_id,d.source_revision_id,d.position LIMIT $7",
            )
            .bind(actor.actor_id)
            .bind(endpoint.block_id)
            .bind(endpoint.revision_id)
            .bind(after.map(|v| v.0))
            .bind(after.map(|v| v.1))
            .bind(after.map(|v| v.2))
            .bind(BATCH)
            .fetch_all(&mut **tx)
            .await
            .map_err(storage)?;
            let count = rows.len();
            for (relation_id, revision_id, position) in rows {
                scanned += 1;
                if scanned > INTERNAL_SCAN_LIMIT {
                    return Err(consumers::DirectError::Storage(ContentError::Storage));
                }
                after = Some((relation_id, revision_id, position));
                let head_review: Option<String> = sqlx::query_scalar(
                    "SELECT rr.state FROM public.relation_review_head h \
                     JOIN public.relation_review rr ON rr.id=h.head_review_id \
                     WHERE h.relation_id=$1 AND h.relation_revision_id=$2",
                )
                .bind(relation_id)
                .bind(revision_id)
                .fetch_optional(&mut **tx)
                .await
                .map_err(storage)?;
                if head_review.as_deref() == Some("withdrawn") {
                    continue;
                }
                let reference = RelationRef {
                    relation_id,
                    revision_id,
                };
                let mut candidate_session = references::Session::default();
                let Some(object) = candidate_session
                    .object(tx, actor, &ExactRef::Relation(reference))
                    .await?
                else {
                    continue;
                };
                let relation = object.relation()?;
                if let Some(step) =
                    relation_entry(endpoint, &relation, ImpactProvenance::DynamicWorking)
                {
                    push_visible(&mut steps, &mut seen_edges, budget, step)?;
                }
            }
            if count < BATCH as usize {
                break;
            }
        }
    }
    Ok(steps)
}

async fn relation_neighbors(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    reference: &RelationRef,
    fixed: &scope::FixedScope,
    impact_scope: &ImpactScope,
    budget: &mut VisibleWorkBudget,
) -> Result<Vec<ImpactStep>, consumers::DirectError> {
    let mut session = references::Session::default();
    let Some(object) = session
        .object(tx, actor, &ExactRef::Relation(reference.clone()))
        .await?
    else {
        return Ok(vec![]);
    };
    let relation = object.relation()?;
    let mut eligible = vec![];
    for view in &fixed.reading_views {
        let (_, choices) = selection::choices(tx, view).await?;
        if choices
            .selections
            .iter()
            .any(|choice| choice.relation == *reference)
        {
            eligible.push(match impact_scope {
                ImpactScope::Reading { .. } => ImpactProvenance::FixedReadingSelection,
                ImpactScope::Release { .. } => ImpactProvenance::FixedReleaseManifest,
            });
            break;
        }
    }
    if matches!(impact_scope, ImpactScope::Reading { .. })
        && [&relation.from, &relation.to].iter().any(|endpoint| {
            fixed.members.get(&ImpactStart::Block((*endpoint).clone()))
                == Some(&ImpactMembership::Displayed)
        })
    {
        let head: Option<Uuid> =
            sqlx::query_scalar("SELECT head_revision_id FROM public.relation WHERE id=$1")
                .bind(reference.relation_id)
                .fetch_optional(&mut **tx)
                .await
                .map_err(storage)?;
        if head == Some(reference.revision_id) {
            let head_review: Option<String> = sqlx::query_scalar(
                "SELECT rr.state FROM public.relation_review_head h \
                 JOIN public.relation_review rr ON rr.id=h.head_review_id \
                 WHERE h.relation_id=$1 AND h.relation_revision_id=$2",
            )
            .bind(reference.relation_id)
            .bind(reference.revision_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(storage)?;
            if head_review.as_deref() != Some("withdrawn") {
                eligible.push(ImpactProvenance::DynamicWorking);
            }
        }
    }
    if eligible.is_empty() {
        return Ok(vec![]);
    }
    let mut steps = vec![];
    let mut seen_edges = BTreeSet::new();
    for (endpoint, direction) in [
        (&relation.from, TraversalDirection::SavedReverse),
        (&relation.to, TraversalDirection::SavedForward),
    ] {
        // A readable endpoint outside fixed membership is still an
        // explanatory terminal. The traversal driver will not expand it.
        let mut endpoint_session = references::Session::default();
        if !endpoint_session
            .authorize(tx, actor, &ExactRef::Block(endpoint.clone()))
            .await?
        {
            continue;
        }
        for provenance in &eligible {
            push_visible(
                &mut steps,
                &mut seen_edges,
                budget,
                ImpactStep {
                    from: ImpactNode::Relation(reference.clone()),
                    to: ImpactNode::Block(endpoint.clone()),
                    family: ImpactFamily::Semantic,
                    dependency_role: None,
                    dependency_position: None,
                    direction: Some(direction),
                    relation_type: Some(relation.relation_type),
                    lineage_type: None,
                    provenance: *provenance,
                    location: None,
                    reason: ImpactReason::SuggestReview,
                },
            )?;
        }
    }
    Ok(steps)
}

pub(super) async fn neighbors(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    current: &ImpactNode,
    fixed: &scope::FixedScope,
    impact_scope: &ImpactScope,
    budget: &mut VisibleWorkBudget,
) -> Result<Vec<ImpactStep>, consumers::DirectError> {
    match current {
        ImpactNode::Block(endpoint) => {
            block_neighbors(tx, actor, endpoint, fixed, impact_scope, budget).await
        }
        ImpactNode::Relation(reference) => {
            relation_neighbors(tx, actor, reference, fixed, impact_scope, budget).await
        }
        _ => Ok(vec![]),
    }
}

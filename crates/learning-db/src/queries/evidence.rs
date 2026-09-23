use super::{QueryStore, scope};
use crate::{reading::selection, references, request, storage};
use learning_core::*;
use sqlx::{Postgres, Transaction};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

const BATCH: i64 = 128;
const INTERNAL_SCAN_LIMIT: usize = 100_000;

enum BuildError {
    Budget,
    Storage(ContentError),
}
impl From<ContentError> for BuildError {
    fn from(value: ContentError) -> Self {
        Self::Storage(internal(value))
    }
}

type Edge = (ImpactNode, ImpactNode, ImpactFamily);
fn charge_edge(
    budget: &mut VisibleWorkBudget,
    seen: &mut BTreeSet<Edge>,
    from: ImpactNode,
    to: ImpactNode,
    family: ImpactFamily,
) -> Result<(), BuildError> {
    if seen.insert((from, to, family)) {
        budget.charge_edge().map_err(|_| BuildError::Budget)?;
    }
    Ok(())
}

fn internal(error: ContentError) -> ContentError {
    match error {
        ContentError::Invalid(code) if code == "reference_budget_exceeded" => ContentError::Storage,
        other => other,
    }
}

fn source(
    view: ReadingRef,
    scope: &ImpactScope,
    review: Option<ReviewProjection<RelationReview>>,
) -> EvidenceSource {
    match scope {
        ImpactScope::Reading { .. } => EvidenceSource::FixedReadingSelection { view, review },
        ImpactScope::Release { .. } => EvidenceSource::FixedReleaseSelection { view, review },
    }
}

fn add_assertion(
    groups: &mut BTreeMap<RelationRef, EvidenceAssertion>,
    budget: &mut VisibleWorkBudget,
    edges: &mut BTreeSet<Edge>,
    fixed: &scope::FixedScope,
    endpoint: &BlockRef,
    relation: RelationRevision,
    provenance: EvidenceSource,
) -> Result<(), BuildError> {
    let (direction, other) = if relation.from == *endpoint {
        (TraversalDirection::SavedForward, &relation.to)
    } else if relation.to == *endpoint {
        (TraversalDirection::SavedReverse, &relation.from)
    } else {
        return Err(BuildError::Storage(ContentError::Storage));
    };
    let other_in_scope = fixed
        .members
        .contains_key(&ImpactStart::Block(other.clone()));
    let relation_node = ImpactNode::Relation(relation.reference.clone());
    if let Some(group) = groups.get(&relation.reference) {
        if group.relation != relation || group.entry_direction != direction {
            return Err(BuildError::Storage(ContentError::Storage));
        }
        if group.sources.contains(&provenance) {
            return Ok(());
        }
    } else {
        budget
            .charge_node(relation_node.clone())
            .map_err(|_| BuildError::Budget)?;
        budget
            .charge_node(ImpactNode::Block(other.clone()))
            .map_err(|_| BuildError::Budget)?;
        charge_edge(
            budget,
            edges,
            ImpactNode::Block(endpoint.clone()),
            relation_node.clone(),
            ImpactFamily::Semantic,
        )?;
        groups.insert(
            relation.reference.clone(),
            EvidenceAssertion {
                relation: relation.clone(),
                entry_direction: direction,
                other_in_scope,
                sources: vec![],
            },
        );
    }
    match &provenance {
        EvidenceSource::FixedReadingSelection { view, review }
        | EvidenceSource::FixedReleaseSelection { view, review } => {
            let reading_node = ImpactNode::Reading {
                view_id: view.view_id,
                revision_id: view.revision_id,
            };
            budget
                .charge_node(reading_node.clone())
                .map_err(|_| BuildError::Budget)?;
            charge_edge(
                budget,
                edges,
                reading_node.clone(),
                relation_node.clone(),
                ImpactFamily::ReviewSelection,
            )?;
            if let Some(ReviewProjection::Available(review)) = review {
                let review_node = ImpactNode::RelationReview(review.reference.clone());
                budget
                    .charge_node(review_node.clone())
                    .map_err(|_| BuildError::Budget)?;
                charge_edge(
                    budget,
                    edges,
                    reading_node,
                    review_node.clone(),
                    ImpactFamily::ReviewSelection,
                )?;
                charge_edge(
                    budget,
                    edges,
                    review_node,
                    relation_node.clone(),
                    ImpactFamily::Necessary,
                )?;
            }
        }
        EvidenceSource::DynamicWorking { review } => {
            if let Some(ReviewProjection::Available(review)) = review {
                let review_node = ImpactNode::RelationReview(review.reference.clone());
                budget
                    .charge_node(review_node.clone())
                    .map_err(|_| BuildError::Budget)?;
                charge_edge(
                    budget,
                    edges,
                    review_node,
                    relation_node,
                    ImpactFamily::Necessary,
                )?;
            }
        }
    }
    groups
        .get_mut(&relation.reference)
        .ok_or(BuildError::Storage(ContentError::Storage))?
        .sources
        .push(provenance);
    Ok(())
}

async fn review_projection(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    session: &mut references::Session,
    reference: RelationReviewRef,
) -> Result<Option<ReviewProjection<RelationReview>>, ContentError> {
    let root = ExactRef::RelationReview(reference);
    if !references::directly_visible(tx, actor, &root).await? {
        return Ok(None);
    }
    Ok(Some(match session.object(tx, actor, &root).await? {
        Some(object) => ReviewProjection::Available(object.relation_review()?),
        None => ReviewProjection::Incomplete,
    }))
}

#[derive(Default)]
struct CollectedEvidence {
    assertions: BTreeMap<RelationRef, EvidenceAssertion>,
    judgments: BTreeMap<EvidenceCursor, EvidenceJudgment>,
}

async fn fixed_selections(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    query: &EvidenceQuery,
    fixed: &scope::FixedScope,
    collected: &mut CollectedEvidence,
    budget: &mut VisibleWorkBudget,
    edges: &mut BTreeSet<Edge>,
) -> Result<(), BuildError> {
    let mut any_incomplete = false;
    for view in &fixed.reading_views {
        let (_, choices) = selection::choices(tx, view).await?;
        for choice in choices.selections {
            // Scope authorization may already hold thousands of unrelated
            // displayed objects. Authorize each candidate's necessary
            // closure independently in this same repeatable-read snapshot.
            let mut candidate_session = references::Session::default();
            let root = ExactRef::Relation(choice.relation.clone());
            // A selected review cannot stand alone when its relation is hidden.
            let Some(object) = candidate_session.object(tx, actor, &root).await? else {
                continue;
            };
            let relation = object.relation()?;
            if relation.from != query.endpoint && relation.to != query.endpoint {
                continue;
            }
            let review = if let Some(reference) = choice.review {
                review_projection(tx, actor, &mut candidate_session, reference).await?
            } else {
                None
            };
            add_assertion(
                &mut collected.assertions,
                budget,
                edges,
                fixed,
                &query.endpoint,
                relation,
                source(view.clone(), &query.scope, review),
            )?;
        }
        for reference in choices.epistemic_reviews {
            let mut candidate_session = references::Session::default();
            let root = ExactRef::EpistemicReview(reference.clone());
            if !references::directly_visible(tx, actor, &root).await? {
                continue;
            }
            // The selected ID routes internally only. An incomplete review has
            // neither identity nor source in the public result or cursor.
            let target: Option<(Uuid, Uuid)> = sqlx::query_as(
                "SELECT s.target_block_id,s.target_revision_id FROM public.epistemic_stream s \
                 JOIN public.epistemic_review r ON r.stream_id=s.id \
                 WHERE s.id=$1 AND r.id=$2",
            )
            .bind(reference.stream_id)
            .bind(reference.review_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(storage)?;
            let Some((block_id, revision_id)) = target else {
                return Err(BuildError::Storage(ContentError::Storage));
            };
            if (block_id, revision_id) != (query.endpoint.block_id, query.endpoint.revision_id) {
                continue;
            }
            match candidate_session.object(tx, actor, &root).await? {
                Some(object) => {
                    let review = object.epistemic_review()?;
                    let key = EvidenceCursor::JudgmentAvailable {
                        stream_id: reference.stream_id,
                        review_id: reference.review_id,
                    };
                    let already_selected = matches!(
                        collected.judgments.get(&key),
                        Some(EvidenceJudgment::Available { views, .. }) if views.contains(view)
                    );
                    if already_selected {
                        continue;
                    }
                    let reading_node = ImpactNode::Reading {
                        view_id: view.view_id,
                        revision_id: view.revision_id,
                    };
                    let review_node = ImpactNode::EpistemicReview(review.reference.clone());
                    budget
                        .charge_node(reading_node.clone())
                        .map_err(|_| BuildError::Budget)?;
                    budget
                        .charge_node(review_node.clone())
                        .map_err(|_| BuildError::Budget)?;
                    charge_edge(
                        budget,
                        edges,
                        reading_node,
                        review_node.clone(),
                        ImpactFamily::ReviewSelection,
                    )?;
                    charge_edge(
                        budget,
                        edges,
                        review_node,
                        ImpactNode::Block(query.endpoint.clone()),
                        ImpactFamily::Necessary,
                    )?;
                    match collected.judgments.get_mut(&key) {
                        Some(EvidenceJudgment::Available {
                            views,
                            review: prior,
                        }) => {
                            if prior.as_ref() != &review {
                                return Err(BuildError::Storage(ContentError::Storage));
                            }
                            views.push(view.clone());
                            views.sort();
                        }
                        None => {
                            collected.judgments.insert(
                                key,
                                EvidenceJudgment::Available {
                                    views: vec![view.clone()],
                                    review: Box::new(review),
                                },
                            );
                        }
                        Some(EvidenceJudgment::Incomplete) => {
                            return Err(BuildError::Storage(ContentError::Storage));
                        }
                    }
                }
                None => any_incomplete = true,
            }
        }
    }
    if any_incomplete {
        budget.charge_step().map_err(|_| BuildError::Budget)?;
        collected.judgments.insert(
            EvidenceCursor::JudgmentIncomplete,
            EvidenceJudgment::Incomplete,
        );
    }
    Ok(())
}

async fn dynamic_heads(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    query: &EvidenceQuery,
    fixed: &scope::FixedScope,
    groups: &mut BTreeMap<RelationRef, EvidenceAssertion>,
    budget: &mut VisibleWorkBudget,
    edges: &mut BTreeSet<Edge>,
) -> Result<(), BuildError> {
    let mut after: Option<(Uuid, Uuid, i32)> = None;
    let mut scanned = 0usize;
    loop {
        // Exact target is the leading key; grant/overlay prefilter precedes
        // bounded keyset access and full necessary-closure authorization.
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
        ).bind(actor.actor_id).bind(query.endpoint.block_id).bind(query.endpoint.revision_id)
            .bind(after.map(|v| v.0)).bind(after.map(|v| v.1)).bind(after.map(|v| v.2))
            .bind(BATCH).fetch_all(&mut **tx).await.map_err(storage)?;
        let count = rows.len();
        for (relation_id, revision_id, position) in rows {
            scanned += 1;
            if scanned > INTERNAL_SCAN_LIMIT {
                return Err(BuildError::Storage(ContentError::Storage));
            }
            after = Some((relation_id, revision_id, position));
            let reference = RelationRef {
                relation_id,
                revision_id,
            };
            let head_review: Option<(Uuid, String)> = sqlx::query_as(
                "SELECT rr.id,rr.state FROM public.relation_review_head h \
                 JOIN public.relation_review rr ON rr.id=h.head_review_id \
                 WHERE h.relation_id=$1 AND h.relation_revision_id=$2",
            )
            .bind(relation_id)
            .bind(revision_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(storage)?;
            if head_review
                .as_ref()
                .is_some_and(|(_, state)| state == "withdrawn")
            {
                continue;
            }
            let mut candidate_session = references::Session::default();
            let Some(object) = candidate_session
                .object(tx, actor, &ExactRef::Relation(reference.clone()))
                .await?
            else {
                continue;
            };
            let relation = object.relation()?;
            let review = if let Some((review_id, _)) = head_review {
                review_projection(
                    tx,
                    actor,
                    &mut candidate_session,
                    RelationReviewRef {
                        relation: reference,
                        review_id,
                    },
                )
                .await?
            } else {
                None
            };
            add_assertion(
                groups,
                budget,
                edges,
                fixed,
                &query.endpoint,
                relation,
                EvidenceSource::DynamicWorking { review },
            )?;
        }
        if count < BATCH as usize {
            break;
        }
    }
    Ok(())
}

enum PageItem {
    Assertion(Box<EvidenceAssertion>),
    Judgment(EvidenceJudgment),
}

impl QueryStore {
    pub async fn evidence(
        &self,
        actor: Principal,
        query: EvidenceQuery,
    ) -> Result<Option<EvidenceResult>, ContentError> {
        query.validate()?;
        let mut tx = request::begin_read(&self.pool).await?;
        let mut session = references::Session::default();
        let fixed = match scope::load(&mut tx, actor, &query.scope, &mut session).await {
            Ok(fixed) => fixed,
            Err(ContentError::NotFound) => return Ok(None),
            Err(error) => return Err(internal(error)),
        };
        let Some(membership) = fixed
            .members
            .get(&ImpactStart::Block(query.endpoint.clone()))
        else {
            return Ok(None);
        };
        if !session
            .authorize(&mut tx, actor, &ExactRef::Block(query.endpoint.clone()))
            .await
            .map_err(internal)?
        {
            return Ok(None);
        }
        if query.include_dynamic && *membership != ImpactMembership::Displayed {
            return Err(ContentError::Invalid(
                "dynamic_requires_displayed_endpoint".into(),
            ));
        }
        let budget_exceeded = || {
            EvidenceResult::new(
                query.endpoint.clone(),
                query.scope.clone(),
                vec![],
                vec![],
                EvidencePageStatus::BudgetExceeded,
            )
        };
        let mut budget = VisibleWorkBudget::new(query.work_limit);
        if budget
            .charge_node(ImpactNode::Block(query.endpoint.clone()))
            .is_err()
        {
            return Ok(Some(budget_exceeded()));
        }
        let mut collected = CollectedEvidence::default();
        let mut edges = BTreeSet::new();
        let fixed_result = fixed_selections(
            &mut tx,
            actor,
            &query,
            &fixed,
            &mut collected,
            &mut budget,
            &mut edges,
        )
        .await;
        match fixed_result {
            Ok(()) => {}
            Err(BuildError::Budget) => return Ok(Some(budget_exceeded())),
            Err(BuildError::Storage(error)) => return Err(error),
        }
        if query.include_dynamic {
            match dynamic_heads(
                &mut tx,
                actor,
                &query,
                &fixed,
                &mut collected.assertions,
                &mut budget,
                &mut edges,
            )
            .await
            {
                Ok(()) => {}
                Err(BuildError::Budget) => return Ok(Some(budget_exceeded())),
                Err(BuildError::Storage(error)) => return Err(error),
            }
        }
        let mut items = BTreeMap::<EvidenceCursor, PageItem>::new();
        for (reference, assertion) in collected.assertions {
            items.insert(
                EvidenceCursor::Assertion {
                    relation_id: reference.relation_id,
                    revision_id: reference.revision_id,
                },
                PageItem::Assertion(Box::new(assertion)),
            );
        }
        for (key, judgment) in collected.judgments {
            items.insert(key, PageItem::Judgment(judgment));
        }
        let mut eligible = items
            .into_iter()
            .filter(|(key, _)| query.after.as_ref().is_none_or(|after| key > after))
            .peekable();
        let mut assertions = vec![];
        let mut judgments = vec![];
        let mut last = None;
        let mut group_bytes = 0usize;
        while assertions.len() + judgments.len() < usize::from(query.limit) {
            let Some((key, item)) = eligible.next() else {
                break;
            };
            let payload = match &item {
                PageItem::Assertion(v) => serde_json::to_vec(v),
                PageItem::Judgment(v) => serde_json::to_vec(v),
            }
            .map_err(storage)?;
            let paths = match &item {
                PageItem::Assertion(value) => value.sources.len(),
                PageItem::Judgment(EvidenceJudgment::Available { views, .. }) => views.len(),
                PageItem::Judgment(EvidenceJudgment::Incomplete) => 1,
            };
            for _ in 0..paths {
                if budget.charge_path().is_err() {
                    return Ok(Some(budget_exceeded()));
                }
            }
            if budget.charge_payload_bytes(payload.len()).is_err() {
                return Ok(Some(budget_exceeded()));
            }
            group_bytes = group_bytes
                .checked_add(payload.len())
                .ok_or(ContentError::Storage)?;
            match item {
                PageItem::Assertion(v) => assertions.push(*v),
                PageItem::Judgment(v) => judgments.push(v),
            }
            last = Some(key);
        }
        let status = if eligible.peek().is_some() {
            EvidencePageStatus::Truncated {
                after: last.ok_or(ContentError::Storage)?,
            }
        } else {
            EvidencePageStatus::Complete
        };
        let result =
            EvidenceResult::new(query.endpoint, query.scope, assertions, judgments, status);
        let full_bytes = serde_json::to_vec(&result).map_err(storage)?.len();
        let framing_bytes = full_bytes
            .checked_sub(group_bytes)
            .ok_or(ContentError::Storage)?;
        if budget.charge_payload_bytes(framing_bytes).is_err() {
            return Ok(Some(EvidenceResult::new(
                result.endpoint.clone(),
                result.actual_scope.clone(),
                vec![],
                vec![],
                EvidencePageStatus::BudgetExceeded,
            )));
        }
        tx.commit().await.map_err(storage)?;
        Ok(Some(result))
    }
}

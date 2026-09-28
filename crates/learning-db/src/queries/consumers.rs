use super::scope;
use crate::{composition::closure, references, request, storage};
use learning_core::*;
use sqlx::{PgPool, Postgres, Transaction};
use std::collections::BTreeMap;
use uuid::Uuid;

const BATCH: i64 = 128;
const INTERNAL_SCAN_LIMIT: usize = 100_000;

#[derive(Clone)]
pub struct QueryStore {
    pub(super) pool: PgPool,
}
impl QueryStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// One fixed scope, one authorization snapshot, and only visible groups.
    pub async fn direct(
        &self,
        actor: Principal,
        query: ImpactQuery,
    ) -> Result<Option<ImpactResult>, ContentError> {
        query.validate()?;
        if query
            .families
            .iter()
            .any(|f| !matches!(f, ImpactFamily::Structural | ImpactFamily::Necessary))
        {
            return Err(ContentError::Invalid(
                "impact_family_not_implemented".into(),
            ));
        }
        let mut tx = request::begin_read(&self.pool).await?;
        let mut session = references::Session::default();
        let fixed = match scope::load(&mut tx, actor, &query.scope, &mut session).await {
            Ok(fixed) => fixed,
            Err(ContentError::NotFound) => return Ok(None),
            Err(error) => return Err(internal_error(error)),
        };
        let Some(membership) = fixed.members.get(&query.start).copied() else {
            return Ok(None);
        };
        match &query.start {
            ImpactStart::Composition(reference) => {
                match closure::load_with_session(
                    &mut tx,
                    actor,
                    std::slice::from_ref(reference),
                    None,
                    &mut session,
                )
                .await
                {
                    Ok(_) => {}
                    Err(ContentError::NotFound) => return Ok(None),
                    Err(error) => return Err(internal_error(error)),
                }
            }
            _ => {
                if !session
                    .authorize(
                        &mut tx,
                        actor,
                        &query.start.exact().ok_or(ContentError::Storage)?,
                    )
                    .await
                    .map_err(internal_error)?
                {
                    return Ok(None);
                }
            }
        }
        let context = ImpactPageContext {
            start: query.start.clone(),
            start_membership: membership,
            actual_scope: query.scope.clone(),
        };
        let mut budget = VisibleWorkBudget::new(query.work_limit);
        let start_node = node(&query.start);
        if budget.charge_node(start_node.clone()).is_err() {
            return Ok(Some(ImpactResult::budget_exceeded(
                query.start,
                membership,
                query.scope,
            )));
        }
        let mut groups = BTreeMap::<ImpactNode, ImpactConsumerGroup>::new();
        let result = async {
            if query.families.contains(&ImpactFamily::Structural) {
                match &query.start {
                    ImpactStart::Block(block) => {
                        structural(
                            &mut tx,
                            actor,
                            block,
                            &fixed,
                            &mut session,
                            &mut budget,
                            &mut groups,
                        )
                        .await?
                    }
                    ImpactStart::Composition(composition) => {
                        structural_composition(
                            &mut tx,
                            actor,
                            composition,
                            &query.scope,
                            &fixed,
                            &mut budget,
                            &mut groups,
                        )
                        .await?
                    }
                    _ => {}
                }
            }
            if query.families.contains(&ImpactFamily::Necessary)
                && let Some(exact) = query.start.exact()
            {
                necessary(&mut tx, actor, &exact, &fixed, &mut budget, &mut groups).await?;
            }
            Ok::<(), DirectError>(())
        }
        .await;
        match result {
            Err(DirectError::Budget) => {
                return Ok(Some(ImpactResult::budget_exceeded(
                    query.start,
                    membership,
                    query.scope,
                )));
            }
            Err(DirectError::Storage(e)) => return Err(e),
            Ok(()) => {}
        }
        let page = paginate_visible_groups(
            groups.into_values().collect(),
            query.after.as_ref(),
            query.limit,
            &mut budget,
            &context,
        )
        .unwrap_or_else(|_| {
            ImpactResult::budget_exceeded(query.start.clone(), membership, query.scope.clone())
        });
        tx.commit().await.map_err(storage)?;
        Ok(Some(page))
    }
}

pub(super) enum DirectError {
    Budget,
    Storage(ContentError),
}
impl From<ContentError> for DirectError {
    fn from(e: ContentError) -> Self {
        Self::Storage(internal_error(e))
    }
}
pub(super) fn internal_error(error: ContentError) -> ContentError {
    match error {
        ContentError::Invalid(code) if code == "reference_budget_exceeded" => ContentError::Storage,
        other => other,
    }
}
pub(super) fn node(reference: &ImpactStart) -> ImpactNode {
    match reference {
        ImpactStart::Block(r) => ImpactNode::Block(r.clone()),
        ImpactStart::Composition(r) => ImpactNode::Composition(r.clone()),
        ImpactStart::Relation(r) => ImpactNode::Relation(r.clone()),
        ImpactStart::RelationReview(r) => ImpactNode::RelationReview(r.clone()),
        ImpactStart::EpistemicReview(r) => ImpactNode::EpistemicReview(r.clone()),
    }
}
pub(super) fn append(
    groups: &mut BTreeMap<ImpactNode, ImpactConsumerGroup>,
    budget: &mut VisibleWorkBudget,
    step: ImpactStep,
) -> Result<(), DirectError> {
    step.validate().map_err(DirectError::Storage)?;
    let consumer = step.to.clone();
    budget
        .charge_node(consumer.clone())
        .map_err(|_| DirectError::Budget)?;
    budget.charge_edge().map_err(|_| DirectError::Budget)?;
    let group = groups
        .entry(consumer.clone())
        .or_insert_with(|| ImpactConsumerGroup {
            consumer,
            locations: vec![],
            explanations: vec![],
        });
    if let Some(location) = &step.location {
        group.locations.push(location.clone());
    }
    group
        .explanations
        .push(ImpactExplanation { steps: vec![step] });
    Ok(())
}
fn structural_step(start: &BlockRef, to: ImpactNode, location: ImpactLocation) -> ImpactStep {
    ImpactStep {
        from: ImpactNode::Block(start.clone()),
        to,
        family: ImpactFamily::Structural,
        dependency_role: None,
        dependency_position: None,
        direction: None,
        relation_type: None,
        lineage_type: None,
        provenance: ImpactProvenance::Stored,
        location: Some(location),
        reason: ImpactReason::ReferencesOldRevision,
    }
}
pub(super) async fn structural(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    start: &BlockRef,
    fixed: &scope::FixedScope,
    session: &mut references::Session,
    budget: &mut VisibleWorkBudget,
    groups: &mut BTreeMap<ImpactNode, ImpactConsumerGroup>,
) -> Result<(), DirectError> {
    let mut after: Option<(Uuid, Uuid)> = None;
    let mut scanned = 0usize;
    loop {
        // The exact target is the leading index key. A keyset and SQL LIMIT
        // bound each reverse access, even with a dense hidden candidate set.
        let rows: Vec<(Uuid, Uuid, Uuid)> = sqlx::query_as(
            "SELECT o.composition_id,o.composition_revision_id,o.occurrence_id \
             FROM public.composition_occurrence o \
             JOIN public.space_grant g ON g.space_id=o.space_id AND g.actor_id=$1 \
             WHERE o.block_revision_id=$2 AND o.block_id=$3 \
               AND ($4::uuid IS NULL OR (o.composition_revision_id,o.occurrence_id)>($4,$5)) \
             ORDER BY o.composition_revision_id,o.occurrence_id LIMIT $6",
        )
        .bind(actor.actor_id)
        .bind(start.revision_id)
        .bind(start.block_id)
        .bind(after.map(|v| v.0))
        .bind(after.map(|v| v.1))
        .bind(BATCH)
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| DirectError::Storage(storage(e)))?;
        let count = rows.len();
        for (composition_id, revision_id, occurrence_id) in rows {
            scanned += 1;
            if scanned > INTERNAL_SCAN_LIMIT {
                return Err(DirectError::Storage(ContentError::Storage));
            }
            after = Some((revision_id, occurrence_id));
            let parent = CompositionRef {
                composition_id,
                revision_id,
            };
            let Some(prefixes) = fixed.composition_paths.get(&parent) else {
                continue;
            };
            let Some(composition) = fixed.compositions.get(&parent) else {
                return Err(DirectError::Storage(ContentError::Storage));
            };
            if !composition.nodes.iter().any(|n| {
                n.occurrence_id == occurrence_id && n.target == NodeTarget::Block(start.clone())
            }) {
                return Err(DirectError::Storage(ContentError::Storage));
            }
            // The full root closure was authorized while resolving scope;
            // checking this exact block again keeps the candidate boundary local.
            if !session
                .authorize(tx, actor, &ExactRef::Block(start.clone()))
                .await?
            {
                continue;
            }
            for prefix in prefixes {
                let mut path = prefix.clone();
                path.push(occurrence_id);
                append(
                    groups,
                    budget,
                    structural_step(
                        start,
                        ImpactNode::Composition(parent.clone()),
                        ImpactLocation::Occurrence { path },
                    ),
                )?;
            }
        }
        if count < BATCH as usize {
            break;
        }
    }
    if let Some(locations) = fixed.reading_locations.get(start) {
        for (reading, location) in locations {
            let mut step = structural_step(start, reading.clone(), location.clone());
            step.provenance = ImpactProvenance::FixedReadingSelection;
            append(groups, budget, step)?;
        }
    }
    Ok(())
}

pub(super) async fn structural_composition(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    start: &CompositionRef,
    scope: &ImpactScope,
    fixed: &scope::FixedScope,
    budget: &mut VisibleWorkBudget,
    groups: &mut BTreeMap<ImpactNode, ImpactConsumerGroup>,
) -> Result<(), DirectError> {
    let mut after: Option<(Uuid, Uuid)> = None;
    let mut scanned = 0usize;
    loop {
        let rows: Vec<(Uuid, Uuid, Uuid)> = sqlx::query_as(
            "SELECT o.composition_id,o.composition_revision_id,o.occurrence_id \
             FROM public.composition_occurrence o \
             JOIN public.space_grant g ON g.space_id=o.space_id AND g.actor_id=$1 \
             WHERE o.child_revision_id=$2 AND o.child_composition_id=$3 \
               AND ($4::uuid IS NULL OR (o.composition_revision_id,o.occurrence_id)>($4,$5)) \
             ORDER BY o.composition_revision_id,o.occurrence_id LIMIT $6",
        )
        .bind(actor.actor_id)
        .bind(start.revision_id)
        .bind(start.composition_id)
        .bind(after.map(|v| v.0))
        .bind(after.map(|v| v.1))
        .bind(BATCH)
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| DirectError::Storage(storage(e)))?;
        let count = rows.len();
        for (composition_id, revision_id, occurrence_id) in rows {
            scanned += 1;
            if scanned > INTERNAL_SCAN_LIMIT {
                return Err(DirectError::Storage(ContentError::Storage));
            }
            after = Some((revision_id, occurrence_id));
            let parent = CompositionRef {
                composition_id,
                revision_id,
            };
            let Some(prefixes) = fixed.composition_paths.get(&parent) else {
                continue;
            };
            let Some(composition) = fixed.compositions.get(&parent) else {
                return Err(DirectError::Storage(ContentError::Storage));
            };
            if !composition.nodes.iter().any(|n| {
                n.occurrence_id == occurrence_id
                    && n.target == NodeTarget::Composition(start.clone())
            }) {
                return Err(DirectError::Storage(ContentError::Storage));
            }
            for prefix in prefixes {
                let mut path = prefix.clone();
                path.push(occurrence_id);
                let location = ImpactLocation::Occurrence { path };
                append(
                    groups,
                    budget,
                    ImpactStep {
                        from: ImpactNode::Composition(start.clone()),
                        to: ImpactNode::Composition(parent.clone()),
                        family: ImpactFamily::Structural,
                        dependency_role: None,
                        dependency_position: None,
                        direction: None,
                        relation_type: None,
                        lineage_type: None,
                        provenance: ImpactProvenance::Stored,
                        location: Some(location),
                        reason: ImpactReason::ReferencesOldRevision,
                    },
                )?;
            }
        }
        if count < BATCH as usize {
            break;
        }
    }
    // release_root is an explicit fixed edge. The recursive manifest alone is not.
    if let ImpactScope::Release { release_id } = scope {
        let root: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.release_root WHERE release_id=$1 AND composition_id=$2 AND revision_id=$3)")
            .bind(release_id).bind(start.composition_id).bind(start.revision_id)
            .fetch_one(&mut **tx).await.map_err(|e| DirectError::Storage(storage(e)))?;
        if root {
            append(
                groups,
                budget,
                ImpactStep {
                    from: ImpactNode::Composition(start.clone()),
                    to: ImpactNode::Release {
                        release_id: *release_id,
                    },
                    family: ImpactFamily::Structural,
                    dependency_role: None,
                    dependency_position: None,
                    direction: None,
                    relation_type: None,
                    lineage_type: None,
                    provenance: ImpactProvenance::Stored,
                    location: None,
                    reason: ImpactReason::ReferencesOldRevision,
                },
            )?;
        }
    }
    if let Some(readings) = fixed.reading_bases.get(start) {
        for reading in readings {
            append(
                groups,
                budget,
                ImpactStep {
                    from: ImpactNode::Composition(start.clone()),
                    to: reading.clone(),
                    family: ImpactFamily::Structural,
                    dependency_role: None,
                    dependency_position: None,
                    direction: None,
                    relation_type: None,
                    lineage_type: None,
                    provenance: ImpactProvenance::FixedReadingSelection,
                    location: None,
                    reason: ImpactReason::ReferencesOldRevision,
                },
            )?;
        }
    }
    Ok(())
}

async fn necessary(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    start: &ExactRef,
    fixed: &scope::FixedScope,
    budget: &mut VisibleWorkBudget,
    groups: &mut BTreeMap<ImpactNode, ImpactConsumerGroup>,
) -> Result<(), DirectError> {
    let (kind, object, revision) = references::key(start);
    let mut after: Option<(String, Uuid, Uuid, i32)> = None;
    let mut scanned = 0usize;
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
             ORDER BY d.source_kind,d.source_object_id,d.source_revision_id,d.position LIMIT $9")
            .bind(actor.actor_id).bind(kind).bind(object).bind(revision)
            .bind(after.as_ref().map(|v| v.0.as_str())).bind(after.as_ref().map(|v| v.1))
            .bind(after.as_ref().map(|v| v.2)).bind(after.as_ref().map(|v| v.3)).bind(BATCH)
            .fetch_all(&mut **tx).await.map_err(|e| DirectError::Storage(storage(e)))?;
        let count = rows.len();
        for (source_kind, source_object, source_revision, position, role, relation_id) in rows {
            scanned += 1;
            if scanned > INTERNAL_SCAN_LIMIT {
                return Err(DirectError::Storage(ContentError::Storage));
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
                            .ok_or(DirectError::Storage(ContentError::Storage))?,
                        revision_id: source_object,
                    },
                    review_id: source_revision,
                }),
                "epistemic_review" => ExactRef::EpistemicReview(EpistemicReviewRef {
                    stream_id: source_object,
                    review_id: source_revision,
                }),
                _ => return Err(DirectError::Storage(ContentError::Storage)),
            };
            if !fixed
                .members
                .contains_key(&ImpactStart::from(source.clone()))
            {
                continue;
            }
            // The fixed-scope loader uses its own operation session. Each
            // visible reverse candidate still needs its complete B3 closure
            // (including independent cyclic-root depth validation), but
            // unrelated candidates must not accumulate one B3 work budget.
            let mut candidate_session = references::Session::default();
            if !candidate_session.authorize(tx, actor, &source).await? {
                continue;
            }
            let role: DependencyRole = serde_json::from_value(serde_json::Value::String(role))
                .map_err(|_| DirectError::Storage(ContentError::Storage))?;
            let position =
                u32::try_from(position).map_err(|_| DirectError::Storage(ContentError::Storage))?;
            append(
                groups,
                budget,
                ImpactStep {
                    from: node(&start.clone().into()),
                    to: node(&source.into()),
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
    Ok(())
}

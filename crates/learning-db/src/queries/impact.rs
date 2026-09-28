use super::{QueryStore, consumers, lineage_impact, review_selection, scope, semantic, traversal};
use crate::{composition::closure, overlay::model, references, request, storage};
use learning_core::*;
use sqlx::{Postgres, Transaction};
use std::collections::{BTreeMap, VecDeque};
use uuid::Uuid;

struct Path {
    nodes: Vec<ImpactNode>,
    steps: Vec<ImpactStep>,
    origin_location: Option<ImpactLocation>,
    composition_path: Option<Vec<Uuid>>,
}

fn continues_same_occurrence(path: &Path, step: &ImpactStep) -> bool {
    if !matches!(step.from, ImpactNode::Composition(_))
        || !matches!(step.to, ImpactNode::Composition(_))
    {
        return true;
    }
    let Some(previous) = &path.composition_path else {
        return true;
    };
    let Some(ImpactLocation::Occurrence { path: parent }) = &step.location else {
        return false;
    };
    previous.len() == parent.len() + 1 && previous.starts_with(parent)
}

fn belongs_to_reading_source(path: &Path, step: &ImpactStep, fixed: &scope::FixedScope) -> bool {
    if !matches!(step.from, ImpactNode::Composition(_))
        || !matches!(step.to, ImpactNode::Reading { .. })
    {
        return true;
    }
    match &path.origin_location {
        None => true, // The exact base composition itself is the query start.
        Some(ImpactLocation::Occurrence { path }) => fixed
            .reading_source_paths
            .get(&step.to)
            .is_some_and(|source_paths| source_paths.contains(path)),
        Some(_) => false,
    }
}

async fn structural_neighbors(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    current: &ImpactNode,
    fixed: &scope::FixedScope,
    impact_scope: &ImpactScope,
    budget: &mut VisibleWorkBudget,
) -> Result<Vec<ImpactStep>, consumers::DirectError> {
    let mut found = BTreeMap::new();
    match current {
        ImpactNode::Block(reference) => {
            let mut session = references::Session::default();
            consumers::structural(
                tx,
                actor,
                reference,
                fixed,
                &mut session,
                budget,
                &mut found,
            )
            .await?;
        }
        ImpactNode::Composition(reference) => {
            let mut session = references::Session::default();
            match closure::load_with_session(
                tx,
                actor,
                std::slice::from_ref(reference),
                None,
                &mut session,
            )
            .await
            {
                Ok(_) => {}
                Err(ContentError::NotFound) => return Ok(vec![]),
                Err(error) => {
                    return Err(consumers::DirectError::Storage(consumers::internal_error(
                        error,
                    )));
                }
            }
            consumers::structural_composition(
                tx,
                actor,
                reference,
                impact_scope,
                fixed,
                budget,
                &mut found,
            )
            .await?;
        }
        ImpactNode::Reading {
            view_id,
            revision_id,
        } => {
            if let ImpactScope::Release { release_id } = impact_scope {
                let view = ReadingRef {
                    view_id: *view_id,
                    revision_id: *revision_id,
                };
                if fixed.reading_views.contains(&view) {
                    // The view's full necessary display closure is checked
                    // again before it becomes a bridge to the release.
                    let layer = model::load_view(tx, actor, view).await?;
                    let mut session = references::Session::default();
                    model::access_with_session(tx, actor, &layer, &mut session)
                        .await
                        .map_err(consumers::internal_error)?;
                    let selected: bool = sqlx::query_scalar(
                        "SELECT EXISTS(SELECT 1 FROM public.release_reading \
                         WHERE release_id=$1 AND view_id=$2 AND view_revision_id=$3)",
                    )
                    .bind(release_id)
                    .bind(view_id)
                    .bind(revision_id)
                    .fetch_one(&mut **tx)
                    .await
                    .map_err(storage)?;
                    if selected {
                        consumers::append(
                            &mut found,
                            budget,
                            ImpactStep {
                                from: current.clone(),
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
            }
        }
        _ => {}
    }
    let mut steps = found
        .into_values()
        .flat_map(|group: ImpactConsumerGroup| group.explanations)
        .flat_map(|explanation| explanation.steps)
        .collect::<Vec<_>>();
    steps.sort_by_key(|step| serde_json::to_string(step).expect("step serializes"));
    Ok(steps)
}

impl QueryStore {
    /// Traverse authorized impacts in one fixed, read-only snapshot.
    pub async fn traverse(
        &self,
        actor: Principal,
        query: ImpactQuery,
    ) -> Result<Option<ImpactResult>, ContentError> {
        query.validate()?;
        if query.families.iter().any(|family| {
            !matches!(
                family,
                ImpactFamily::Structural
                    | ImpactFamily::Necessary
                    | ImpactFamily::Semantic
                    | ImpactFamily::ReviewSelection
                    | ImpactFamily::Lineage
            )
        }) {
            return Err(ContentError::Invalid(
                "impact_family_not_implemented".into(),
            ));
        }
        let mut tx = request::begin_read(&self.pool).await?;
        let fixed = match scope::load(
            &mut tx,
            actor,
            &query.scope,
            &mut references::Session::default(),
        )
        .await
        {
            Ok(fixed) => fixed,
            Err(ContentError::NotFound) => return Ok(None),
            Err(error) => return Err(consumers::internal_error(error)),
        };
        let Some(membership) = fixed.members.get(&query.start).copied() else {
            return Ok(None);
        };
        let mut start_session = references::Session::default();
        match &query.start {
            ImpactStart::Composition(reference) => {
                match closure::load_with_session(
                    &mut tx,
                    actor,
                    std::slice::from_ref(reference),
                    None,
                    &mut start_session,
                )
                .await
                {
                    Ok(_) => {}
                    Err(ContentError::NotFound) => return Ok(None),
                    Err(error) => return Err(consumers::internal_error(error)),
                }
            }
            _ => {
                if !start_session
                    .authorize(
                        &mut tx,
                        actor,
                        &query.start.exact().ok_or(ContentError::Storage)?,
                    )
                    .await
                    .map_err(consumers::internal_error)?
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
        let exceeded =
            || ImpactResult::budget_exceeded(query.start.clone(), membership, query.scope.clone());
        let start = consumers::node(&query.start);
        let mut budget = VisibleWorkBudget::new(query.work_limit);
        if budget.charge_node(start.clone()).is_err() {
            return Ok(Some(exceeded()));
        }
        let mut queue = VecDeque::from([Path {
            nodes: vec![start],
            steps: vec![],
            origin_location: None,
            composition_path: None,
        }]);
        let mut neighbor_cache = BTreeMap::<ImpactNode, Vec<ImpactStep>>::new();
        let mut groups = BTreeMap::<ImpactNode, ImpactConsumerGroup>::new();
        while let Some(path) = queue.pop_front() {
            if path.steps.len() >= usize::from(query.max_depth) {
                continue;
            }
            let current = path.nodes.last().ok_or(ContentError::Storage)?;
            if let ImpactNode::Block(reference) = current
                && !fixed
                    .members
                    .contains_key(&ImpactStart::Block(reference.clone()))
            {
                // A readable semantic counterpart outside this fixed scope is
                // display-only; it cannot bridge to any further consumer.
                continue;
            }
            if !neighbor_cache.contains_key(current) {
                let neighbors = async {
                    let mut steps = vec![];
                    if query.families.contains(&ImpactFamily::Structural) {
                        steps.extend(
                            structural_neighbors(
                                &mut tx,
                                actor,
                                current,
                                &fixed,
                                &query.scope,
                                &mut budget,
                            )
                            .await?,
                        );
                    }
                    if query.families.contains(&ImpactFamily::Necessary) {
                        steps.extend(
                            traversal::necessary_neighbors(
                                &mut tx,
                                actor,
                                current,
                                &fixed,
                                &mut budget,
                            )
                            .await?,
                        );
                    }
                    if query.families.contains(&ImpactFamily::Semantic) {
                        steps.extend(
                            semantic::neighbors(
                                &mut tx,
                                actor,
                                current,
                                &fixed,
                                &query.scope,
                                &mut budget,
                            )
                            .await?,
                        );
                    }
                    if query.families.contains(&ImpactFamily::ReviewSelection) {
                        steps.extend(
                            review_selection::neighbors(
                                &mut tx,
                                actor,
                                current,
                                &fixed,
                                &query.scope,
                                &mut budget,
                            )
                            .await?,
                        );
                    }
                    if query.families.contains(&ImpactFamily::Lineage) {
                        steps.extend(
                            lineage_impact::neighbors(&mut tx, actor, current, &fixed, &mut budget)
                                .await?,
                        );
                    }
                    steps.sort_by_key(|step| serde_json::to_string(step).expect("step serializes"));
                    Ok::<_, consumers::DirectError>(steps)
                }
                .await;
                let neighbors = match neighbors {
                    Ok(neighbors) => neighbors,
                    Err(consumers::DirectError::Budget) => return Ok(Some(exceeded())),
                    Err(consumers::DirectError::Storage(error)) => return Err(error),
                };
                neighbor_cache.insert(current.clone(), neighbors);
            }
            for cached_step in neighbor_cache.get(current).ok_or(ContentError::Storage)? {
                let step = cached_step.clone();
                if matches!(current, ImpactNode::Relation(_))
                    && step.family == ImpactFamily::Semantic
                    && let Some(incoming) = path.steps.last()
                    && incoming.family == ImpactFamily::Semantic
                    && incoming.provenance != step.provenance
                {
                    continue;
                }
                if path.nodes.contains(&step.to)
                    || !continues_same_occurrence(&path, &step)
                    || !belongs_to_reading_source(&path, &step, &fixed)
                {
                    continue;
                }
                if budget.charge_step().is_err() {
                    return Ok(Some(exceeded()));
                }
                let mut next = Path {
                    nodes: path.nodes.clone(),
                    steps: path.steps.clone(),
                    origin_location: path
                        .origin_location
                        .clone()
                        .or_else(|| step.location.clone()),
                    composition_path: path.composition_path.clone(),
                };
                if let Some(ImpactLocation::Occurrence { path: occurrence }) = &step.location {
                    next.composition_path = Some(occurrence.clone());
                }
                next.nodes.push(step.to.clone());
                next.steps.push(step.clone());
                let group = groups
                    .entry(step.to.clone())
                    .or_insert_with(|| ImpactConsumerGroup {
                        consumer: step.to.clone(),
                        locations: vec![],
                        explanations: vec![],
                    });
                if let Some(location) = &next.origin_location {
                    group.locations.push(location.clone());
                }
                group.explanations.push(ImpactExplanation {
                    steps: next.steps.clone(),
                });
                queue.push_back(next);
            }
        }
        let result = paginate_visible_groups(
            groups.into_values().collect(),
            query.after.as_ref(),
            query.limit,
            &mut budget,
            &context,
        )
        .unwrap_or_else(|_| exceeded());
        tx.commit().await.map_err(storage)?;
        Ok(Some(result))
    }
}

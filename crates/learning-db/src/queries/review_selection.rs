use super::{consumers, scope};
use crate::{reading::selection, references};
use learning_core::*;
use sqlx::{Postgres, Transaction};
use std::collections::BTreeMap;

fn selected(current: &ImpactNode, choices: &ReadingSelections) -> bool {
    match current {
        ImpactNode::Relation(reference) => choices
            .selections
            .iter()
            .any(|choice| choice.relation == *reference),
        ImpactNode::RelationReview(reference) => choices
            .selections
            .iter()
            .any(|choice| choice.review.as_ref() == Some(reference)),
        ImpactNode::EpistemicReview(reference) => choices.epistemic_reviews.contains(reference),
        _ => false,
    }
}

fn root(current: &ImpactNode) -> Option<ExactRef> {
    match current {
        ImpactNode::Relation(reference) => Some(ExactRef::Relation(reference.clone())),
        ImpactNode::RelationReview(reference) => Some(ExactRef::RelationReview(reference.clone())),
        ImpactNode::EpistemicReview(reference) => {
            Some(ExactRef::EpistemicReview(reference.clone()))
        }
        _ => None,
    }
}

pub(super) async fn neighbors(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    current: &ImpactNode,
    fixed: &scope::FixedScope,
    impact_scope: &ImpactScope,
    budget: &mut VisibleWorkBudget,
) -> Result<Vec<ImpactStep>, consumers::DirectError> {
    let Some(root) = root(current) else {
        return Ok(vec![]);
    };
    let member = ImpactStart::from(root.clone());
    if fixed.members.get(&member) != Some(&ImpactMembership::Selected) {
        return Ok(vec![]);
    }
    // Recheck the whole B3 dependency closure on this hop, even when a
    // previous selected object in the same scope was already authorized.
    let mut session = references::Session::default();
    if !session.authorize(tx, actor, &root).await? {
        return Ok(vec![]);
    }
    let provenance = match impact_scope {
        ImpactScope::Reading { .. } => ImpactProvenance::FixedReadingSelection,
        ImpactScope::Release { .. } => ImpactProvenance::FixedReleaseManifest,
    };
    let mut groups = BTreeMap::new();
    for view in &fixed.reading_views {
        let (_, choices) = selection::choices(tx, view).await?;
        if !selected(current, &choices) {
            continue;
        }
        consumers::append(
            &mut groups,
            budget,
            ImpactStep {
                from: current.clone(),
                to: ImpactNode::Reading {
                    view_id: view.view_id,
                    revision_id: view.revision_id,
                },
                family: ImpactFamily::ReviewSelection,
                dependency_role: None,
                dependency_position: None,
                direction: None,
                relation_type: None,
                lineage_type: None,
                provenance,
                location: None,
                reason: ImpactReason::SelectedEvidence,
            },
        )?;
    }
    Ok(groups
        .into_values()
        .flat_map(|group: ImpactConsumerGroup| group.explanations)
        .flat_map(|explanation| explanation.steps)
        .collect())
}

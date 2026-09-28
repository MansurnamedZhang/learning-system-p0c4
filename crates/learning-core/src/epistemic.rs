//! Immutable human judgments; no truth field is added to block content.
use crate::{
    BlockRef, ContentError, Dependency, DependencyRole, EpistemicReviewRef, ExactRef,
    RelationScope, RelationSelection,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpistemicState {
    Untested,
    Testing,
    Inconclusive,
    SupportedWithinScope,
    RefutedWithinScope,
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpistemicReview {
    pub reference: EpistemicReviewRef,
    pub scope: RelationScope,
    pub target: BlockRef,
    pub previous: Option<EpistemicReviewRef>,
    pub state: EpistemicState,
    pub relations: Vec<RelationSelection>,
    pub evidence: Vec<BlockRef>,
    pub conditions: String,
    pub explanation: String,
    pub reviewer_id: Uuid,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppendEpistemicReview {
    pub request_id: Uuid,
    pub scope: RelationScope,
    pub target: BlockRef,
    pub expected_previous: Option<EpistemicReviewRef>,
    pub state: EpistemicState,
    pub relations: Vec<RelationSelection>,
    pub evidence: Vec<BlockRef>,
    pub conditions: String,
    pub explanation: String,
}
impl AppendEpistemicReview {
    pub fn validate(&self) -> Result<(), ContentError> {
        if self.relations.len() > 256 || self.evidence.len() > 256 {
            return Err(ContentError::Invalid(
                "epistemic_selection_limit_exceeded".into(),
            ));
        }
        for text in [&self.conditions, &self.explanation] {
            if text.len() > 10000 || text.contains('\0') {
                return Err(ContentError::Invalid("invalid_epistemic_text".into()));
            }
        }
        if matches!(
            self.state,
            EpistemicState::SupportedWithinScope | EpistemicState::RefutedWithinScope
        ) && (self.conditions.trim().is_empty() || self.explanation.trim().is_empty())
        {
            return Err(ContentError::Invalid(
                "epistemic_conditions_and_explanation_required".into(),
            ));
        }
        let mut relations = std::collections::BTreeSet::new();
        for selection in &self.relations {
            if !relations.insert(&selection.relation) {
                return Err(ContentError::Invalid("duplicate_relation_selection".into()));
            }
            if selection
                .review
                .as_ref()
                .is_some_and(|r| r.relation != selection.relation)
            {
                return Err(ContentError::Invalid(
                    "relation_review_target_mismatch".into(),
                ));
            }
        }
        let mut evidence = std::collections::BTreeSet::new();
        if self.evidence.iter().any(|r| !evidence.insert(r)) {
            return Err(ContentError::Invalid("duplicate_epistemic_evidence".into()));
        }
        Ok(())
    }
    /// Audit predecessors are intentionally excluded from necessary evidence.
    pub fn dependencies(&self) -> Vec<Dependency> {
        let mut deps = vec![Dependency {
            role: DependencyRole::Target,
            target: ExactRef::Block(self.target.clone()),
        }];
        for selection in &self.relations {
            deps.push(Dependency {
                role: DependencyRole::SelectedRelation,
                target: ExactRef::Relation(selection.relation.clone()),
            });
            if let Some(review) = &selection.review {
                deps.push(Dependency {
                    role: DependencyRole::SelectedReview,
                    target: ExactRef::RelationReview(review.clone()),
                });
            }
        }
        deps.extend(self.evidence.iter().map(|r| Dependency {
            role: DependencyRole::Basis,
            target: ExactRef::Block(r.clone()),
        }));
        deps
    }
}

/// Shared explicit source runs only. An unassigned singleton does not assert
/// independence, confidence, or any judgment about the evidence's truth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceGroup {
    pub source_run: Option<BlockRef>,
    pub evidence: Vec<BlockRef>,
}

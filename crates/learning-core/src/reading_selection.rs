use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

/// Stored exact choices. Public reading responses use authorized projections instead.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadingSelections {
    pub selections: Vec<RelationSelection>,
    pub epistemic_reviews: Vec<EpistemicReviewRef>,
}
impl ReadingSelections {
    pub fn validate(&self) -> Result<(), ContentError> {
        if self.selections.len() > 256 || self.epistemic_reviews.len() > 256 {
            return Err(ContentError::Invalid(
                "reading_selection_limit_exceeded".into(),
            ));
        }
        let mut seen = BTreeSet::new();
        for s in &self.selections {
            if !seen.insert(&s.relation) {
                return Err(ContentError::Invalid("duplicate_relation_selection".into()));
            }
            if s.review.as_ref().is_some_and(|r| r.relation != s.relation) {
                return Err(ContentError::Invalid(
                    "relation_review_target_mismatch".into(),
                ));
            }
        }
        if self.epistemic_reviews.iter().collect::<BTreeSet<_>>().len()
            != self.epistemic_reviews.len()
        {
            return Err(ContentError::Invalid(
                "duplicate_epistemic_selection".into(),
            ));
        }
        Ok(())
    }
    pub fn references(&self) -> Vec<ExactRef> {
        let mut roots = vec![];
        for s in &self.selections {
            roots.push(ExactRef::Relation(s.relation.clone()));
            if let Some(r) = &s.review {
                roots.push(ExactRef::RelationReview(r.clone()));
            }
        }
        roots.extend(
            self.epistemic_reviews
                .iter()
                .cloned()
                .map(ExactRef::EpistemicReview),
        );
        roots
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "RawSelectRelations")]
pub struct SelectRelations {
    pub request_id: Uuid,
    pub expected_overlay_revision: Uuid,
    pub expected_reading_view_revision: Uuid,
    pub selections: Vec<RelationSelection>,
    pub epistemic_reviews: Vec<EpistemicReviewRef>,
    pub reason: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSelectRelations {
    request_id: Uuid,
    expected_overlay_revision: Uuid,
    expected_reading_view_revision: Uuid,
    selections: Vec<RelationSelection>,
    epistemic_reviews: Vec<EpistemicReviewRef>,
    reason: String,
}
impl TryFrom<RawSelectRelations> for SelectRelations {
    type Error = ContentError;
    fn try_from(r: RawSelectRelations) -> Result<Self, Self::Error> {
        let value = Self {
            request_id: r.request_id,
            expected_overlay_revision: r.expected_overlay_revision,
            expected_reading_view_revision: r.expected_reading_view_revision,
            selections: r.selections,
            epistemic_reviews: r.epistemic_reviews,
            reason: r.reason,
        };
        value.validate()?;
        Ok(value)
    }
}
impl SelectRelations {
    pub fn choices(&self) -> ReadingSelections {
        ReadingSelections {
            selections: self.selections.clone(),
            epistemic_reviews: self.epistemic_reviews.clone(),
        }
    }
    pub fn validate(&self) -> Result<(), ContentError> {
        crate::overlay::validate_reason(&self.reason)?;
        if self.reason.trim().is_empty() {
            return Err(ContentError::Invalid("invalid_reason".into()));
        }
        self.choices().validate()
    }
    pub fn digest(&self, actor: Principal, overlay: Uuid) -> String {
        hex_digest(canonical_json(&serde_json::json!({"domain":"reading-request-v2","operation":"reading_select","actor_id":actor.actor_id,"overlay_id":overlay,"command":self})).as_bytes())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SelectedRelationProjection {
    pub relation: RelationRevision,
    pub review: Option<ReviewProjection<RelationReview>>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ReadingEvidence {
    pub selections: Vec<SelectedRelationProjection>,
    pub epistemic_reviews: Vec<ReviewProjection<EpistemicReview>>,
}

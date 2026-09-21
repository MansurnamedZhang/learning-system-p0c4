//! Fixed semantic relations and independent, append-only human reviews.
use crate::{BlockRef, ContentError, RelationRef, RelationReviewRef};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RelationScope {
    Space { space_id: Uuid },
    PersonalOverlay { space_id: Uuid, overlay_id: Uuid },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationType {
    Annotates,
    Questions,
    Answers,
    InspiredBy,
    Supports,
    Opposes,
    Tests,
    RelatedTo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationOrigin {
    UserAsserted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationReviewState {
    Unreviewed,
    Reviewed,
    NeedsRecheck,
    Withdrawn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationRevision {
    pub reference: RelationRef,
    pub scope: RelationScope,
    #[serde(rename = "type")]
    pub relation_type: RelationType,
    pub origin: RelationOrigin,
    pub parent_revision_id: Option<Uuid>,
    pub from: BlockRef,
    pub to: BlockRef,
    pub rationale: String,
    pub conditions: String,
    pub content_sha256: String,
    pub author_id: Uuid,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationReview {
    pub reference: RelationReviewRef,
    pub previous_review_id: Option<Uuid>,
    pub state: RelationReviewState,
    pub explanation: String,
    pub reviewer_id: Uuid,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", content = "review", rename_all = "snake_case")]
pub enum ReviewProjection<T> {
    Available(T),
    Incomplete,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveRelation {
    pub request_id: Uuid,
    pub scope: RelationScope,
    pub relation_id: Option<Uuid>,
    pub expected_revision: Option<Uuid>,
    #[serde(rename = "type")]
    pub relation_type: RelationType,
    pub from: BlockRef,
    pub to: BlockRef,
    pub rationale: String,
    pub conditions: String,
}
impl SaveRelation {
    pub fn validate(&self) -> Result<(), ContentError> {
        if self.relation_id.is_some() != self.expected_revision.is_some() {
            return Err(ContentError::Invalid("relation_cas_pair_required".into()));
        }
        if self.from.block_id == self.to.block_id {
            return Err(ContentError::Invalid(
                "relation_endpoints_must_differ".into(),
            ));
        }
        if self.rationale.chars().count() > 1000 || self.rationale.contains('\0') {
            return Err(ContentError::Invalid("invalid_relation_rationale".into()));
        }
        validate_text(&self.conditions)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewRelation {
    pub request_id: Uuid,
    pub relation: RelationRef,
    pub expected_previous: Option<Uuid>,
    pub state: RelationReviewState,
    pub explanation: String,
}
impl ReviewRelation {
    pub fn validate(&self) -> Result<(), ContentError> {
        validate_text(&self.explanation)
    }
}
fn validate_text(value: &str) -> Result<(), ContentError> {
    if value.len() > 10000 || value.contains('\0') {
        return Err(ContentError::Invalid("invalid_relation_text".into()));
    }
    Ok(())
}

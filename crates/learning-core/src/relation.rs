//! Persisted relation contracts. Commands and authorization live in later store layers.
use crate::{BlockRef, RelationRef, RelationReviewRef};
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewProjection<T> {
    Available(T),
    Incomplete,
    Unavailable,
}

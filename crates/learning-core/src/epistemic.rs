//! Immutable human judgments; no truth field is added to block content.
use crate::{BlockRef, EpistemicReviewRef, RelationScope, RelationSelection};
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

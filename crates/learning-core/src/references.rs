use crate::BlockRef;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationRef {
    pub relation_id: Uuid,
    pub revision_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationReviewRef {
    pub relation: RelationRef,
    pub review_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpistemicReviewRef {
    pub stream_id: Uuid,
    pub review_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExactRef {
    Block(BlockRef),
    Relation(RelationRef),
    RelationReview(RelationReviewRef),
    EpistemicReview(EpistemicReviewRef),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyRole {
    Basis,
    Target,
    RequiresContext,
    SourceRun,
    SelectedRelation,
    SelectedReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dependency {
    pub role: DependencyRole,
    pub target: ExactRef,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationSelection {
    pub relation: RelationRef,
    pub review: Option<RelationReviewRef>,
}

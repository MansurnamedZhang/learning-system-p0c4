//! Local, explicit evidence projected from a fixed scope and current grants.
use crate::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawEvidenceQuery")]
pub struct EvidenceQuery {
    pub endpoint: BlockRef,
    pub scope: ImpactScope,
    #[serde(default)]
    pub include_dynamic: bool,
    #[serde(default = "default_limit")]
    pub limit: u16,
    #[serde(default = "default_work")]
    pub work_limit: u32,
    #[serde(default)]
    pub after: Option<EvidenceCursor>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEvidenceQuery {
    endpoint: BlockRef,
    scope: ImpactScope,
    #[serde(default)]
    include_dynamic: bool,
    #[serde(default = "default_limit")]
    limit: u16,
    #[serde(default = "default_work")]
    work_limit: u32,
    #[serde(default)]
    after: Option<EvidenceCursor>,
}
impl TryFrom<RawEvidenceQuery> for EvidenceQuery {
    type Error = ContentError;
    fn try_from(raw: RawEvidenceQuery) -> Result<Self, Self::Error> {
        let value = Self {
            endpoint: raw.endpoint,
            scope: raw.scope,
            include_dynamic: raw.include_dynamic,
            limit: raw.limit,
            work_limit: raw.work_limit,
            after: raw.after,
        };
        value.validate()?;
        Ok(value)
    }
}
const fn default_limit() -> u16 {
    50
}
const fn default_work() -> u32 {
    4096
}
impl EvidenceQuery {
    pub fn validate(&self) -> Result<(), ContentError> {
        if self.limit == 0
            || self.limit > MAX_IMPACT_CONSUMERS
            || self.work_limit == 0
            || self.work_limit > MAX_VISIBLE_WORK
        {
            return Err(ContentError::Invalid("invalid_evidence_limits".into()));
        }
        if self.include_dynamic && matches!(self.scope, ImpactScope::Release { .. }) {
            return Err(ContentError::Invalid("dynamic_requires_reading".into()));
        }
        let valid_cursor = match &self.after {
            None | Some(EvidenceCursor::JudgmentIncomplete) => true,
            Some(EvidenceCursor::Assertion {
                relation_id,
                revision_id,
            }) => !relation_id.is_nil() && !revision_id.is_nil(),
            Some(EvidenceCursor::JudgmentAvailable {
                stream_id,
                review_id,
            }) => !stream_id.is_nil() && !review_id.is_nil(),
        };
        if !valid_cursor {
            return Err(ContentError::Invalid("invalid_evidence_cursor".into()));
        }
        Ok(())
    }
}

/// Every variant contains only an already visible identity. The anonymous
/// marker deliberately has no field from its hidden selected review.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum EvidenceCursor {
    Assertion {
        relation_id: uuid::Uuid,
        revision_id: uuid::Uuid,
    },
    JudgmentAvailable {
        stream_id: uuid::Uuid,
        review_id: uuid::Uuid,
    },
    JudgmentIncomplete,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum CursorWire {
    Assertion(AssertionWire),
    JudgmentAvailable(JudgmentAvailableWire),
    JudgmentIncomplete(JudgmentIncompleteWire),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AssertionWire {
    #[serde(rename = "type")]
    kind: String,
    relation_id: uuid::Uuid,
    revision_id: uuid::Uuid,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JudgmentAvailableWire {
    #[serde(rename = "type")]
    kind: String,
    stream_id: uuid::Uuid,
    review_id: uuid::Uuid,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JudgmentIncompleteWire {
    #[serde(rename = "type")]
    kind: String,
}
impl<'de> Deserialize<'de> for EvidenceCursor {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (kind, value) = match CursorWire::deserialize(deserializer)? {
            CursorWire::Assertion(raw) => (
                raw.kind,
                Self::Assertion {
                    relation_id: raw.relation_id,
                    revision_id: raw.revision_id,
                },
            ),
            CursorWire::JudgmentAvailable(raw) => (
                raw.kind,
                Self::JudgmentAvailable {
                    stream_id: raw.stream_id,
                    review_id: raw.review_id,
                },
            ),
            CursorWire::JudgmentIncomplete(raw) => (raw.kind, Self::JudgmentIncomplete),
        };
        let expected = match &value {
            Self::Assertion { .. } => "assertion",
            Self::JudgmentAvailable { .. } => "judgment_available",
            Self::JudgmentIncomplete => "judgment_incomplete",
        };
        if kind != expected {
            return Err(serde::de::Error::custom("invalid_evidence_cursor"));
        }
        Ok(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EvidenceSource {
    FixedReadingSelection {
        view: ReadingRef,
        review: Option<ReviewProjection<RelationReview>>,
    },
    FixedReleaseSelection {
        view: ReadingRef,
        review: Option<ReviewProjection<RelationReview>>,
    },
    DynamicWorking {
        review: Option<ReviewProjection<RelationReview>>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvidenceAssertion {
    pub relation: RelationRevision,
    pub entry_direction: TraversalDirection,
    pub other_in_scope: bool,
    pub sources: Vec<EvidenceSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EvidenceJudgment {
    Available {
        views: Vec<ReadingRef>,
        review: Box<EpistemicReview>,
    },
    Incomplete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EvidencePageStatus {
    Complete,
    Truncated { after: EvidenceCursor },
    BudgetExceeded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvidenceResult {
    pub endpoint: BlockRef,
    pub actual_scope: ImpactScope,
    assertions: Vec<EvidenceAssertion>,
    judgments: Vec<EvidenceJudgment>,
    status: EvidencePageStatus,
}
impl EvidenceResult {
    pub fn new(
        endpoint: BlockRef,
        actual_scope: ImpactScope,
        assertions: Vec<EvidenceAssertion>,
        judgments: Vec<EvidenceJudgment>,
        status: EvidencePageStatus,
    ) -> Self {
        let (assertions, judgments) = if matches!(status, EvidencePageStatus::BudgetExceeded) {
            (vec![], vec![])
        } else {
            (assertions, judgments)
        };
        Self {
            endpoint,
            actual_scope,
            assertions,
            judgments,
            status,
        }
    }
    pub fn assertions(&self) -> &[EvidenceAssertion] {
        &self.assertions
    }
    pub fn judgments(&self) -> &[EvidenceJudgment] {
        &self.judgments
    }
    pub fn status(&self) -> &EvidencePageStatus {
        &self.status
    }
}

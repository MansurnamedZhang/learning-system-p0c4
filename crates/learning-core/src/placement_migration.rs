use crate::overlay::{checked_deserialize, invalid_reading, unique_ids, validate_reason};
use crate::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[derive(Debug, Clone, Serialize)]
pub struct ProposeMigration {
    pub request_id: Uuid,
    pub expected_overlay_revision: Uuid,
    pub expected_reading_view_revision: Uuid,
    pub target: CompositionRef,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum GroupDecision {
    Exact { group_id: Uuid },
    AcceptCandidate { group_id: Uuid },
    Manual { group_id: Uuid, anchor: GapAnchor },
    KeepUnplaced { group_id: Uuid },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MergeOrder {
    pub source_group_ids: Vec<Uuid>,
    pub placement_ids: Vec<Uuid>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum MigrationAction {
    Reject,
    Adopt {
        groups: Vec<GroupDecision>,
        merges: Vec<MergeOrder>,
    },
}
#[derive(Debug, Clone, Serialize)]
pub struct DecideMigration {
    pub request_id: Uuid,
    pub proposal_id: Uuid,
    pub expected_overlay_revision: Uuid,
    pub expected_reading_view_revision: Uuid,
    pub action: MigrationAction,
    pub reason: String,
}
impl GroupDecision {
    pub fn group_id(&self) -> Uuid {
        match self {
            Self::Exact { group_id }
            | Self::AcceptCandidate { group_id }
            | Self::Manual { group_id, .. }
            | Self::KeepUnplaced { group_id } => *group_id,
        }
    }
}
impl ProposeMigration {
    pub fn validate(&self) -> Result<(), ContentError> {
        validate_reason(&self.reason)
    }
}
impl DecideMigration {
    pub fn validate(&self) -> Result<(), ContentError> {
        validate_reason(&self.reason)?;
        if let MigrationAction::Adopt { groups, merges } = &self.action {
            if !groups.is_empty() {
                unique_ids(
                    &groups
                        .iter()
                        .map(GroupDecision::group_id)
                        .collect::<Vec<_>>(),
                    512,
                )?;
            }
            if merges.len() > 512 {
                return invalid_reading("group_limit");
            }
            for g in groups {
                if let GroupDecision::Manual { anchor, .. } = g {
                    anchor.validate()?;
                }
            }
            let mut seen = std::collections::BTreeSet::new();
            for m in merges {
                unique_ids(&m.source_group_ids, 512)?;
                unique_ids(&m.placement_ids, 2048)?;
                if m.source_group_ids.len() < 2
                    || m.source_group_ids.iter().any(|id| !seen.insert(*id))
                {
                    return invalid_reading("invalid_merge");
                }
            }
        }
        Ok(())
    }
    pub fn digest(&self, actor: Principal, target: Uuid) -> String {
        let mut c = self.clone();
        if let MigrationAction::Adopt { groups, merges } = &mut c.action {
            groups.sort_by_key(GroupDecision::group_id);
            for m in merges.iter_mut() {
                m.source_group_ids.sort();
            }
            merges.sort_by(|a, b| a.source_group_ids.cmp(&b.source_group_ids));
        }
        reading_request_digest("migration_decide", actor, target, &c)
    }
}
checked_deserialize!(ProposeMigration {
    request_id: Uuid,
    expected_overlay_revision: Uuid,
    expected_reading_view_revision: Uuid,
    target: CompositionRef,
    reason: String
});
checked_deserialize!(DecideMigration {
    request_id: Uuid,
    proposal_id: Uuid,
    expected_overlay_revision: Uuid,
    expected_reading_view_revision: Uuid,
    action: MigrationAction,
    reason: String
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationClass {
    Exact,
    Candidate,
    Unresolved,
}
#[derive(Debug, Clone, Serialize)]
pub struct MigrationGroupProjection {
    pub group_id: Uuid,
    pub placement_ids: Vec<Uuid>,
    pub old_anchor: Option<GapAnchor>,
    pub suggested_anchor: Option<GapAnchor>,
    pub classification: Option<MigrationClass>,
    pub reason_code: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct MigrationDecisionSummary {
    pub decision_id: Uuid,
    pub adopted: bool,
    pub author_id: Uuid,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
#[derive(Debug, Clone, Serialize)]
pub struct MigrationProjection {
    pub proposal_id: Uuid,
    pub overlay: OverlayRef,
    pub target: Option<CompositionRef>,
    pub groups: Vec<MigrationGroupProjection>,
    pub decisions: Vec<MigrationDecisionSummary>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationDecided {
    pub proposal_id: Uuid,
    pub decision_id: Uuid,
    pub adopted: Option<ReadingSaved>,
}

//! Versioned input and state vocabulary for durable work.
use crate::{BlockRef, ContentError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobInput {
    actor_id: Uuid,
    space_id: Uuid,
    block: BlockRef,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawJobInput {
    version: u32,
    kind: String,
    actor_id: Uuid,
    space_id: Uuid,
    block_id: Uuid,
    revision_id: Uuid,
}

impl JobInput {
    pub fn asset_integrity(
        actor_id: Uuid,
        space_id: Uuid,
        block: BlockRef,
    ) -> Result<Self, ContentError> {
        if [actor_id, space_id, block.block_id, block.revision_id].contains(&Uuid::nil()) {
            return Err(ContentError::Invalid("invalid_job_input".into()));
        }
        Ok(Self {
            actor_id,
            space_id,
            block,
        })
    }

    pub fn business_key(&self) -> String {
        format!(
            "asset-integrity:v1:{}:{}:{}",
            self.space_id, self.block.block_id, self.block.revision_id
        )
    }

    pub fn to_value(&self) -> Value {
        json!({
            "version": 1,
            "kind": "asset_integrity",
            "actor_id": self.actor_id,
            "space_id": self.space_id,
            "block_id": self.block.block_id,
            "revision_id": self.block.revision_id,
        })
    }

    pub fn from_value(value: Value) -> Result<Self, ContentError> {
        let raw: RawJobInput = serde_json::from_value(value)
            .map_err(|_| ContentError::Invalid("invalid_job_input".into()))?;
        if raw.version != 1 || raw.kind != "asset_integrity" {
            return Err(ContentError::Invalid("unknown_job_input_version".into()));
        }
        Self::asset_integrity(
            raw.actor_id,
            raw.space_id,
            BlockRef {
                block_id: raw.block_id,
                revision_id: raw.revision_id,
            },
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    RetryWait,
    Succeeded,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub fn parse(name: &str) -> Result<Self, ContentError> {
        match name {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "retry_wait" => Ok(Self::RetryWait),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(ContentError::Invalid("unknown_job_status".into())),
        }
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        match self {
            Self::Queued => matches!(next, Self::Running | Self::Cancelled),
            Self::Running => matches!(
                next,
                Self::Running | Self::RetryWait | Self::Succeeded | Self::Failed | Self::Cancelled
            ),
            Self::RetryWait => matches!(next, Self::Running | Self::Cancelled),
            Self::Succeeded | Self::Failed | Self::Cancelled => false,
        }
    }
}

//! Versioned inputs; the C2 asset wire representation and business key are frozen.
use crate::{
    BlockRef, ContentError, SNAPSHOT_MAX_OBJECTS, SnapshotRequest, canonical_json, hex_digest,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobInput {
    AssetIntegrity {
        actor_id: Uuid,
        space_id: Uuid,
        block: BlockRef,
    },
    SnapshotExport {
        actor_id: Uuid,
        space_id: Uuid,
        request: SnapshotRequest,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAssetInput {
    version: u32,
    kind: String,
    actor_id: Uuid,
    space_id: Uuid,
    block_id: Uuid,
    revision_id: Uuid,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSnapshotInput {
    version: u32,
    kind: String,
    actor_id: Uuid,
    space_id: Uuid,
    request: SnapshotRequest,
}

impl JobInput {
    pub fn asset_integrity(
        actor_id: Uuid,
        space_id: Uuid,
        block: BlockRef,
    ) -> Result<Self, ContentError> {
        if [actor_id, space_id, block.block_id, block.revision_id].contains(&Uuid::nil()) {
            return Err(invalid());
        }
        Ok(Self::AssetIntegrity {
            actor_id,
            space_id,
            block,
        })
    }
    pub fn snapshot_export(
        actor_id: Uuid,
        space_id: Uuid,
        request: SnapshotRequest,
    ) -> Result<Self, ContentError> {
        if [
            actor_id,
            space_id,
            request.reading.view_id,
            request.reading.revision_id,
        ]
        .contains(&Uuid::nil())
            || request
                .resource_versions
                .len()
                .saturating_add(request.source_segments.len())
                > SNAPSHOT_MAX_OBJECTS
            || request
                .resource_versions
                .iter()
                .any(|r| [r.space_id, r.resource_id, r.version_id].contains(&Uuid::nil()))
            || request.source_segments.iter().any(|r| {
                [r.space_id, r.resource_id, r.version_id, r.segment_id].contains(&Uuid::nil())
            })
        {
            return Err(invalid());
        }
        Ok(Self::SnapshotExport {
            actor_id,
            space_id,
            request,
        })
    }
    pub fn business_key(&self) -> String {
        match self {
            Self::AssetIntegrity {
                space_id, block, ..
            } => format!(
                "asset-integrity:v1:{space_id}:{}:{}",
                block.block_id, block.revision_id
            ),
            Self::SnapshotExport {
                actor_id,
                space_id,
                request,
            } => format!(
                "snapshot-export:v1:{actor_id}:{space_id}:{}",
                hex_digest(canonical_json(&json!(request)).as_bytes())
            ),
        }
    }
    pub fn actor_id(&self) -> Uuid {
        match self {
            Self::AssetIntegrity { actor_id, .. } | Self::SnapshotExport { actor_id, .. } => {
                *actor_id
            }
        }
    }
    pub fn space_id(&self) -> Uuid {
        match self {
            Self::AssetIntegrity { space_id, .. } | Self::SnapshotExport { space_id, .. } => {
                *space_id
            }
        }
    }
    pub fn asset_block(&self) -> Option<&BlockRef> {
        match self {
            Self::AssetIntegrity { block, .. } => Some(block),
            Self::SnapshotExport { .. } => None,
        }
    }
    pub fn to_value(&self) -> Value {
        match self {
            Self::AssetIntegrity {
                actor_id,
                space_id,
                block,
            } => {
                json!({"version":1,"kind":"asset_integrity","actor_id":actor_id,"space_id":space_id,"block_id":block.block_id,"revision_id":block.revision_id})
            }
            Self::SnapshotExport {
                actor_id,
                space_id,
                request,
            } => {
                json!({"version":1,"kind":"snapshot_export","actor_id":actor_id,"space_id":space_id,"request":request})
            }
        }
    }
    pub fn from_value(value: Value) -> Result<Self, ContentError> {
        if value.get("kind").and_then(Value::as_str) == Some("snapshot_export") {
            let raw: RawSnapshotInput =
                serde_json::from_value(value.clone()).map_err(|_| invalid())?;
            if raw.version != 1 || raw.kind != "snapshot_export" {
                return Err(invalid());
            }
            let input = Self::snapshot_export(raw.actor_id, raw.space_id, raw.request)?;
            // C3 accepts only canonical UUID spellings, strict nested objects and the
            // exact wire types also accepted by the database CHECK.
            if input.to_value() != value {
                return Err(invalid());
            }
            Ok(input)
        } else {
            let raw: RawAssetInput = serde_json::from_value(value).map_err(|_| invalid())?;
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
}
fn invalid() -> ContentError {
    ContentError::Invalid("invalid_job_input".into())
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

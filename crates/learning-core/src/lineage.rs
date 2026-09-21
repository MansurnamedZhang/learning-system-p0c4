//! Explicit content transformations. Provenance is separate from semantic basis.
use crate::{BlockRef, ContentDraft, ContentError};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineageOperation {
    Derive,
    Split,
    Merge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineageCommand {
    pub request_id: Uuid,
    pub operation: LineageOperation,
    pub inputs: Vec<BlockRef>,
    pub outputs: Vec<ContentDraft>,
    pub reason: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VersionedOutput {
    contract_version: u32,
    draft: serde_json::Value,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCommand {
    request_id: Uuid,
    operation: LineageOperation,
    #[serde(deserialize_with = "bounded_members")]
    inputs: Vec<BlockRef>,
    #[serde(deserialize_with = "bounded_members")]
    outputs: Vec<VersionedOutput>,
    reason: String,
}
// Bound the member vectors while decoding, before constructing an unbounded
// command. Individual output bodies use ContentDraft's existing size limits.
fn bounded_members<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Members<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Members<T> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("at most 32 lineage members")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Self::Value, A::Error> {
            let mut members = Vec::new();
            while let Some(member) = sequence.next_element()? {
                if members.len() == 32 {
                    return Err(serde::de::Error::custom("invalid_lineage_shape"));
                }
                members.push(member);
            }
            Ok(members)
        }
    }
    deserializer.deserialize_seq(Members(std::marker::PhantomData))
}
impl Serialize for LineageCommand {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let outputs = self
            .outputs
            .iter()
            .map(|draft| {
                Ok(VersionedOutput {
                    contract_version: draft.contract_version(),
                    draft: serde_json::to_value(draft).map_err(serde::ser::Error::custom)?,
                })
            })
            .collect::<Result<Vec<_>, S::Error>>()?;
        RawCommand {
            request_id: self.request_id,
            operation: self.operation,
            inputs: self.inputs.clone(),
            outputs,
            reason: self.reason.clone(),
        }
        .serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for LineageCommand {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawCommand::deserialize(deserializer)?;
        let outputs = raw
            .outputs
            .into_iter()
            .map(|d| {
                ContentDraft::decode(d.contract_version, d.draft).map_err(serde::de::Error::custom)
            })
            .collect::<Result<Vec<_>, D::Error>>()?;
        let command = Self {
            request_id: raw.request_id,
            operation: raw.operation,
            inputs: raw.inputs,
            outputs,
            reason: raw.reason,
        };
        command.validate().map_err(serde::de::Error::custom)?;
        Ok(command)
    }
}
impl LineageCommand {
    pub fn validate(&self) -> Result<(), ContentError> {
        let valid = match self.operation {
            LineageOperation::Derive => self.inputs.len() == 1 && self.outputs.len() == 1,
            LineageOperation::Split => {
                self.inputs.len() == 1 && (2..=32).contains(&self.outputs.len())
            }
            LineageOperation::Merge => {
                (2..=32).contains(&self.inputs.len()) && self.outputs.len() == 1
            }
        };
        if !valid
            || self
                .inputs
                .iter()
                .map(|r| r.block_id)
                .collect::<BTreeSet<_>>()
                .len()
                != self.inputs.len()
        {
            return Err(ContentError::Invalid("invalid_lineage_shape".into()));
        }
        if self.reason.trim().is_empty()
            || self.reason.chars().count() > 1000
            || self.reason.contains('\0')
        {
            return Err(ContentError::Invalid("invalid_lineage_reason".into()));
        }
        for output in &self.outputs {
            output.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineageSaved {
    pub operation_id: Uuid,
    pub operation: LineageOperation,
    pub inputs: Vec<BlockRef>,
    pub outputs: Vec<BlockRef>,
    pub author_id: Uuid,
    pub created_at: DateTime<Utc>,
}

/// Read-only graph edges have no editable relation identity or head.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemLineageType {
    DerivedFrom,
    SplitFrom,
    MergedFrom,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SystemLineageEdge {
    pub operation_id: Uuid,
    pub relation_type: SystemLineageType,
    pub from: BlockRef,
    pub to: BlockRef,
}
impl LineageSaved {
    pub fn system_relations(&self) -> Vec<SystemLineageEdge> {
        let relation_type = match self.operation {
            LineageOperation::Derive => SystemLineageType::DerivedFrom,
            LineageOperation::Split => SystemLineageType::SplitFrom,
            LineageOperation::Merge => SystemLineageType::MergedFrom,
        };
        self.outputs
            .iter()
            .flat_map(|output| {
                self.inputs.iter().map(move |input| SystemLineageEdge {
                    operation_id: self.operation_id,
                    relation_type,
                    from: output.clone(),
                    to: input.clone(),
                })
            })
            .collect()
    }
}

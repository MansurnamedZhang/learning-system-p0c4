use crate::{ContentError, Revision, canonical_json, hex_digest};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockRef {
    pub block_id: Uuid,
    pub revision_id: Uuid,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompositionRef {
    pub composition_id: Uuid,
    pub revision_id: Uuid,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeTarget {
    Block(BlockRef),
    Composition(CompositionRef),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompositionKind {
    Document,
    Section,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeDraft {
    pub occurrence_id: Option<Uuid>,
    pub target: NodeTarget,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Occurrence {
    pub occurrence_id: Uuid,
    pub target: NodeTarget,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "RawSaveComposition")]
pub struct SaveComposition {
    pub request_id: Uuid,
    pub composition_id: Option<Uuid>,
    pub base_revision_id: Option<Uuid>,
    pub kind: CompositionKind,
    pub title: String,
    pub nodes: Vec<NodeDraft>,
    pub reason: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSaveComposition {
    pub request_id: Uuid,
    pub composition_id: Option<Uuid>,
    pub base_revision_id: Option<Uuid>,
    pub kind: CompositionKind,
    pub title: String,
    pub nodes: Vec<NodeDraft>,
    pub reason: String,
}
impl TryFrom<RawSaveComposition> for SaveComposition {
    type Error = ContentError;
    fn try_from(r: RawSaveComposition) -> Result<Self, Self::Error> {
        let value = Self {
            request_id: r.request_id,
            composition_id: r.composition_id,
            base_revision_id: r.base_revision_id,
            kind: r.kind,
            title: r.title,
            nodes: r.nodes,
            reason: r.reason,
        };
        value.validate()?;
        Ok(value)
    }
}
impl SaveComposition {
    pub fn validate(&self) -> Result<(), ContentError> {
        if self.composition_id.is_some() != self.base_revision_id.is_some() {
            return invalid("composition_base_pair");
        }
        if self.title.chars().count() > 300 || self.title.contains('\0') {
            return invalid("composition_title");
        }
        validate_assembly_reason(&self.reason)?;
        if self.nodes.len() > 512 {
            return invalid("composition_nodes_limit");
        }
        let mut seen = HashSet::new();
        for id in self.nodes.iter().filter_map(|n| n.occurrence_id) {
            if !seen.insert(id) {
                return invalid("duplicate_occurrence");
            }
        }
        Ok(())
    }
    pub fn digest(&self, space: Uuid) -> String {
        hex_digest(canonical_json(&json!({"domain":"assembly-request-v1","operation":"composition_save","space_id":space,"command":self})).as_bytes())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompositionRevision {
    pub reference: CompositionRef,
    pub parent_revision_id: Option<Uuid>,
    pub kind: CompositionKind,
    pub title: String,
    pub nodes: Vec<Occurrence>,
    pub content_sha256: String,
    pub author_id: Uuid,
    pub reason: String,
    pub created_at: DateTime<Utc>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompositionSnapshot {
    pub root: CompositionRef,
    pub compositions: Vec<CompositionRevision>,
    pub blocks: Vec<Revision>,
}
pub fn composition_digest(kind: CompositionKind, title: &str, nodes: &[Occurrence]) -> String {
    hex_digest(
        canonical_json(&json!({"domain":"composition-v1","kind":kind,"title":title,"nodes":nodes}))
            .as_bytes(),
    )
}
pub(crate) fn invalid<T>(code: &str) -> Result<T, ContentError> {
    Err(ContentError::Invalid(code.into()))
}
pub(crate) fn validate_assembly_reason(reason: &str) -> Result<(), ContentError> {
    if reason.is_empty() || reason.chars().count() > 1000 || reason.contains('\0') {
        return invalid("assembly_reason");
    }
    Ok(())
}

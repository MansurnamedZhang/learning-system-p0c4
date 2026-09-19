use crate::composition::{invalid, validate_assembly_reason};
use crate::{CompositionRef, ContentError, canonical_json, hex_digest};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use uuid::Uuid;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishRoot {
    pub composition_id: Uuid,
    pub revision_id: Uuid,
    pub expected_head_revision_id: Uuid,
    pub expected_publication_token: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "RawPublishCommand")]
pub struct PublishCommand {
    pub request_id: Uuid,
    pub roots: Vec<PublishRoot>,
    pub reason: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPublishCommand {
    request_id: Uuid,
    roots: Vec<PublishRoot>,
    reason: String,
}
impl TryFrom<RawPublishCommand> for PublishCommand {
    type Error = ContentError;
    fn try_from(r: RawPublishCommand) -> Result<Self, Self::Error> {
        let value = Self {
            request_id: r.request_id,
            roots: r.roots,
            reason: r.reason,
        };
        value.validate()?;
        Ok(value)
    }
}
impl PublishCommand {
    pub fn validate(&self) -> Result<(), ContentError> {
        validate_assembly_reason(&self.reason)?;
        if self.roots.is_empty() || self.roots.len() > 16 {
            return invalid("release_roots_limit");
        }
        let mut seen = HashSet::new();
        for root in &self.roots {
            if root.expected_publication_token.len() != 64
                || !root
                    .expected_publication_token
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return invalid("publication_token");
            }
            if !seen.insert(root.composition_id) {
                return invalid("duplicate_release_root");
            }
        }
        Ok(())
    }
    pub fn digest(&self, space: Uuid) -> String {
        let mut normalized = self.clone();
        normalized.roots.sort_by_key(|r| r.composition_id);
        hex_digest(canonical_json(&json!({"domain":"assembly-request-v1","operation":"release_publish","space_id":space,"command":normalized})).as_bytes())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub release_id: Uuid,
    pub space_id: Uuid,
    pub roots: Vec<CompositionRef>,
    pub author_id: Uuid,
    pub reason: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicationState {
    pub composition_id: Uuid,
    pub head_revision_id: Uuid,
    pub published: Option<CompositionRef>,
    pub publication_token: String,
}

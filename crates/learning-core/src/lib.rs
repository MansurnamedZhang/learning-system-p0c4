use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intent { Knowledge, Note, Question, Idea, Conjecture, Observation, Evidence, Conclusion }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind { Text }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextFormat { Markdown }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextDraft {
    pub kind: BlockKind,
    pub intent: Intent,
    pub language: String,
    pub title: String,
    pub payload: TextPayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextPayload {
    pub format: TextFormat,
    pub text: String,
}

impl TextDraft {
    pub fn validate(&self) -> Result<(), String> {
        if self.title.chars().count() > 300 { return Err("标题最多 300 字".into()); }
        if self.payload.text.len() > 200_000 { return Err("正文最多 200,000 字节".into()); }
        if self.language.is_empty() || self.language.len() > 35 {
            return Err("语言标记长度应为 1–35 字节".into());
        }
        Ok(())
    }
    pub fn digest(&self) -> String {
        // Serialize through sorted maps so object field order is never semantic.
        let value = serde_json::to_value(self).expect("text serialization is infallible");
        let canonical = canonical_json(&value);
        hex_digest(canonical.as_bytes())
    }
}

pub fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let sorted: std::collections::BTreeMap<_, _> = map.iter().collect();
            let parts: Vec<_> = sorted.into_iter().map(|(k, v)|
                format!("{}:{}", serde_json::to_string(k).unwrap(), canonical_json(v))).collect();
            format!("{{{}}}", parts.join(","))
        }
        serde_json::Value::Array(items) => format!("[{}]", items.iter().map(canonical_json).collect::<Vec<_>>().join(",")),
        other => other.to_string(),
    }
}

pub fn hex_digest(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }

#[derive(Debug, Clone, Copy)]
pub struct Principal { pub id: Uuid }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revision {
    pub block_id: Uuid,
    pub revision_id: Uuid,
    pub parent_revision_id: Option<Uuid>,
    pub draft: TextDraft,
    pub content_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateCommand {
    pub request_id: Uuid,
    pub draft: TextDraft,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviseCommand {
    pub request_id: Uuid,
    pub base_revision_id: Uuid,
    pub draft: TextDraft,
    pub reason: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ContentError {
    #[error("{0}")] Invalid(String),
    #[error("内容不存在或不可访问")] NotFound,
    #[error("基础修订已过期")] Conflict { current_revision_id: Uuid },
    #[error("幂等请求标识已用于其它内容")] IdempotencyConflict,
    #[error("保存服务暂时不可用")] Storage,
}

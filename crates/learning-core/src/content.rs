use crate::{ContentError, content_digest};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const CONTRACT_VERSION: u32 = 1;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intent {
    Knowledge,
    Note,
    Question,
    Idea,
    Conjecture,
    Observation,
    Evidence,
    Conclusion,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    Text,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextFormat {
    Markdown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawTextDraft")]
pub struct TextDraft {
    pub kind: BlockKind,
    pub intent: Intent,
    pub language: String,
    pub title: String,
    pub payload: TextPayload,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTextDraft {
    kind: BlockKind,
    intent: Intent,
    language: String,
    title: String,
    payload: TextPayload,
}
impl TryFrom<RawTextDraft> for TextDraft {
    type Error = ContentError;
    fn try_from(raw: RawTextDraft) -> Result<Self, Self::Error> {
        let draft = Self {
            kind: raw.kind,
            intent: raw.intent,
            language: raw.language,
            title: raw.title,
            payload: raw.payload,
        };
        draft.validate()?;
        Ok(draft)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextPayload {
    pub format: TextFormat,
    pub text: String,
}
impl TextDraft {
    pub fn validate(&self) -> Result<(), ContentError> {
        if self.title.chars().count() > 300 {
            return invalid("标题最多 300 个 Unicode 标量");
        }
        if self.payload.text.len() > 200_000 {
            return invalid("正文最多 200,000 个 UTF-8 字节");
        }
        if self.language.is_empty() || self.language.len() > 35 || !self.language.is_ascii() {
            return invalid("language 必须为 1–35 个 ASCII 字符");
        }
        if [&self.title, &self.language, &self.payload.text]
            .iter()
            .any(|s| s.contains('\0'))
        {
            return invalid("正文及元数据不支持 U+0000");
        }
        Ok(())
    }
    pub fn digest(&self) -> String {
        content_digest(self)
    }
}
fn invalid<T>(message: &str) -> Result<T, ContentError> {
    Err(ContentError::Invalid(message.to_owned()))
}
fn validate_reason(reason: &str) -> Result<(), ContentError> {
    if reason.is_empty() || reason.chars().count() > 1_000 || reason.contains('\0') {
        return invalid("修改理由须为 1–1,000 个 Unicode 标量且不含 U+0000");
    }
    Ok(())
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revision {
    pub block_id: Uuid,
    pub revision_id: Uuid,
    pub parent_revision_id: Option<Uuid>,
    pub draft: TextDraft,
    pub content_sha256: String,
    pub author_id: Uuid,
    pub reason: String,
    pub created_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateCommand {
    pub request_id: Uuid,
    pub draft: TextDraft,
    pub reason: String,
}
impl CreateCommand {
    pub fn validate(&self) -> Result<(), ContentError> {
        self.draft.validate()?;
        validate_reason(&self.reason)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviseCommand {
    pub request_id: Uuid,
    pub base_revision_id: Uuid,
    pub draft: TextDraft,
    pub reason: String,
}
impl ReviseCommand {
    pub fn validate(&self) -> Result<(), ContentError> {
        self.draft.validate()?;
        validate_reason(&self.reason)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageCursor {
    pub created_at: DateTime<Utc>,
    pub id: Uuid,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevisionPage {
    pub items: Vec<Revision>,
    pub next_cursor: Option<PageCursor>,
}

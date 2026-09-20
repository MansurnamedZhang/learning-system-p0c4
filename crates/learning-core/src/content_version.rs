use crate::{
    BlockRef, ContentError, Dependency, DependencyRole, ExactRef, Intent, RelationSelection,
    TextDraft, TextPayload, canonical_json, hex_digest,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

fn invalid<T>(message: &str) -> Result<T, ContentError> {
    Err(ContentError::Invalid(message.to_owned()))
}

fn validate_versioned_reason(reason: &str) -> Result<(), ContentError> {
    if reason.trim().is_empty() || reason.chars().count() > 1_000 || reason.contains('\0') {
        return invalid("修改理由须为 1–1,000 个 Unicode 标量且不含 U+0000");
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyV2 {
    Text(TextPayload),
    Reference { target: BlockRef },
    RelationView { selections: Vec<RelationSelection> },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum RawBodyV2 {
    Text { payload: TextPayload },
    Reference { target: BlockRef },
    RelationView { selections: Vec<RelationSelection> },
}

impl From<BodyV2> for RawBodyV2 {
    fn from(value: BodyV2) -> Self {
        match value {
            BodyV2::Text(payload) => Self::Text { payload },
            BodyV2::Reference { target } => Self::Reference { target },
            BodyV2::RelationView { selections } => Self::RelationView { selections },
        }
    }
}

impl From<RawBodyV2> for BodyV2 {
    fn from(value: RawBodyV2) -> Self {
        match value {
            RawBodyV2::Text { payload } => Self::Text(payload),
            RawBodyV2::Reference { target } => Self::Reference { target },
            RawBodyV2::RelationView { selections } => Self::RelationView { selections },
        }
    }
}

impl<'de> Deserialize<'de> for BodyV2 {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        RawBodyV2::deserialize(deserializer).map(Into::into)
    }
}

impl Serialize for RawBodyV2 {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Text { payload } => {
                #[derive(Serialize)]
                struct Value<'a> {
                    kind: &'static str,
                    payload: &'a TextPayload,
                }
                Value {
                    kind: "text",
                    payload,
                }
                .serialize(serializer)
            }
            Self::Reference { target } => {
                #[derive(Serialize)]
                struct Value<'a> {
                    kind: &'static str,
                    target: &'a BlockRef,
                }
                Value {
                    kind: "reference",
                    target,
                }
                .serialize(serializer)
            }
            Self::RelationView { selections } => {
                #[derive(Serialize)]
                struct Value<'a> {
                    kind: &'static str,
                    selections: &'a [RelationSelection],
                }
                Value {
                    kind: "relation_view",
                    selections,
                }
                .serialize(serializer)
            }
        }
    }
}

impl Serialize for BodyV2 {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        RawBodyV2::from(self.clone()).serialize(serializer)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawContentV2")]
pub struct ContentV2 {
    pub intent: Intent,
    pub language: String,
    pub title: String,
    pub body: BodyV2,
    pub basis_refs: Vec<ExactRef>,
    pub requires_context: Vec<BlockRef>,
    pub source_run: Option<BlockRef>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawContentV2 {
    intent: Intent,
    language: String,
    title: String,
    body: BodyV2,
    basis_refs: Vec<ExactRef>,
    requires_context: Vec<BlockRef>,
    source_run: Option<BlockRef>,
}

impl TryFrom<RawContentV2> for ContentV2 {
    type Error = ContentError;

    fn try_from(raw: RawContentV2) -> Result<Self, Self::Error> {
        let value = Self {
            intent: raw.intent,
            language: raw.language,
            title: raw.title,
            body: raw.body,
            basis_refs: raw.basis_refs,
            requires_context: raw.requires_context,
            source_run: raw.source_run,
        };
        value.validate()?;
        Ok(value)
    }
}

impl ContentV2 {
    pub fn validate(&self) -> Result<(), ContentError> {
        if self.title.chars().count() > 300 {
            return invalid("标题最多 300 个 Unicode 标量");
        }
        if self.language.is_empty() || self.language.len() > 35 || !self.language.is_ascii() {
            return invalid("language 必须为 1–35 个 ASCII 字符");
        }
        if self.title.contains('\0') || self.language.contains('\0') {
            return invalid("正文及元数据不支持 U+0000");
        }
        if let BodyV2::Text(payload) = &self.body {
            if payload.text.len() > 200_000 {
                return invalid("正文最多 200,000 个 UTF-8 字节");
            }
            if payload.text.contains('\0') {
                return invalid("正文及元数据不支持 U+0000");
            }
        }
        let mut basis = BTreeSet::new();
        if self
            .basis_refs
            .iter()
            .any(|reference| !basis.insert(reference))
        {
            return invalid("duplicate_reference");
        }
        if let BodyV2::RelationView { selections } = &self.body {
            if selections.len() > 256 {
                return invalid("reference_budget_exceeded");
            }
            let mut relations = BTreeSet::new();
            if selections
                .iter()
                .any(|selection| !relations.insert(&selection.relation))
            {
                return invalid("duplicate_reference");
            }
            if selections.iter().any(|selection| {
                selection
                    .review
                    .as_ref()
                    .is_some_and(|review| review.relation != selection.relation)
            }) {
                return invalid("review_relation_mismatch");
            }
        }
        if self.dependencies().len() > 256 {
            return invalid("reference_budget_exceeded");
        }
        Ok(())
    }

    pub fn dependencies(&self) -> Vec<Dependency> {
        let mut result = Vec::new();
        result.extend(self.basis_refs.iter().cloned().map(|target| Dependency {
            role: DependencyRole::Basis,
            target,
        }));
        result.extend(
            self.requires_context
                .iter()
                .cloned()
                .map(|target| Dependency {
                    role: DependencyRole::RequiresContext,
                    target: ExactRef::Block(target),
                }),
        );
        if let Some(target) = self.source_run.clone() {
            result.push(Dependency {
                role: DependencyRole::SourceRun,
                target: ExactRef::Block(target),
            });
        }
        match &self.body {
            BodyV2::Reference { target } => result.push(Dependency {
                role: DependencyRole::Target,
                target: ExactRef::Block(target.clone()),
            }),
            BodyV2::RelationView { selections } => {
                for selection in selections {
                    result.push(Dependency {
                        role: DependencyRole::SelectedRelation,
                        target: ExactRef::Relation(selection.relation.clone()),
                    });
                    if let Some(review) = &selection.review {
                        result.push(Dependency {
                            role: DependencyRole::SelectedReview,
                            target: ExactRef::RelationReview(review.clone()),
                        });
                    }
                }
            }
            BodyV2::Text(_) => {}
        }
        result
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentDraft {
    V1(TextDraft),
    V2(ContentV2),
}

impl ContentDraft {
    pub fn decode(version: u32, value: serde_json::Value) -> Result<Self, ContentError> {
        match version {
            1 => serde_json::from_value(value)
                .map(Self::V1)
                .map_err(|error| ContentError::Invalid(error.to_string())),
            2 => serde_json::from_value(value)
                .map(Self::V2)
                .map_err(|error| ContentError::Invalid(error.to_string())),
            _ => invalid("unsupported_content_version"),
        }
    }

    pub fn contract_version(&self) -> u32 {
        match self {
            Self::V1(_) => 1,
            Self::V2(_) => 2,
        }
    }

    pub fn validate(&self) -> Result<(), ContentError> {
        match self {
            Self::V1(value) => value.validate(),
            Self::V2(value) => value.validate(),
        }
    }

    pub fn digest(&self) -> String {
        match self {
            Self::V1(value) => value.digest(),
            Self::V2(value) => {
                let mut stable = value.clone();
                if let BodyV2::RelationView { selections } = &mut stable.body {
                    selections.sort();
                }
                hex_digest(canonical_json(&serde_json::json!({"domain":"content-v2","contract_version":2,"draft":stable})).as_bytes())
            }
        }
    }

    pub fn dependencies(&self) -> Vec<Dependency> {
        match self {
            Self::V1(_) => Vec::new(),
            Self::V2(value) => value.dependencies(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentRevision {
    pub block_id: Uuid,
    pub revision_id: Uuid,
    pub parent_revision_id: Option<Uuid>,
    pub draft: ContentDraft,
    pub content_sha256: String,
    pub author_id: Uuid,
    pub reason: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReferencePreview {
    pub revision: ContentRevision,
    pub target: Option<PreviewTarget>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum PreviewTarget {
    Embedded(Box<ReferencePreview>),
    Link(BlockRef),
}

impl From<crate::Revision> for ContentRevision {
    fn from(r: crate::Revision) -> Self {
        Self {
            block_id: r.block_id,
            revision_id: r.revision_id,
            parent_revision_id: r.parent_revision_id,
            draft: ContentDraft::V1(r.draft),
            content_sha256: r.content_sha256,
            author_id: r.author_id,
            reason: r.reason,
            created_at: r.created_at,
        }
    }
}
impl TryFrom<ContentRevision> for crate::Revision {
    type Error = ContentError;
    fn try_from(r: ContentRevision) -> Result<Self, Self::Error> {
        let ContentDraft::V1(draft) = r.draft else {
            return invalid("unsupported_content_version");
        };
        Ok(Self {
            block_id: r.block_id,
            revision_id: r.revision_id,
            parent_revision_id: r.parent_revision_id,
            draft,
            content_sha256: r.content_sha256,
            author_id: r.author_id,
            reason: r.reason,
            created_at: r.created_at,
        })
    }
}

#[derive(Debug, Clone)]
pub struct CreateContent {
    pub request_id: Uuid,
    pub draft: ContentDraft,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct ReviseContent {
    pub request_id: Uuid,
    pub base_revision_id: Uuid,
    pub draft: ContentDraft,
    pub reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCreateContent {
    request_id: Uuid,
    contract_version: u32,
    draft: serde_json::Value,
    reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawReviseContent {
    request_id: Uuid,
    base_revision_id: Uuid,
    contract_version: u32,
    draft: serde_json::Value,
    reason: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawContentRevision {
    block_id: Uuid,
    revision_id: Uuid,
    parent_revision_id: Option<Uuid>,
    contract_version: u32,
    draft: serde_json::Value,
    content_sha256: String,
    author_id: Uuid,
    reason: String,
    created_at: DateTime<Utc>,
}

impl CreateContent {
    pub fn validate(&self) -> Result<(), ContentError> {
        self.draft.validate()?;
        validate_versioned_reason(&self.reason)
    }
}

impl ReviseContent {
    pub fn validate(&self) -> Result<(), ContentError> {
        self.draft.validate()?;
        validate_versioned_reason(&self.reason)
    }
}

impl<'de> Deserialize<'de> for CreateContent {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawCreateContent::deserialize(deserializer)?;
        let value = Self {
            request_id: raw.request_id,
            draft: ContentDraft::decode(raw.contract_version, raw.draft)
                .map_err(serde::de::Error::custom)?,
            reason: raw.reason,
        };
        value.validate().map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}

impl<'de> Deserialize<'de> for ReviseContent {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawReviseContent::deserialize(deserializer)?;
        let value = Self {
            request_id: raw.request_id,
            base_revision_id: raw.base_revision_id,
            draft: ContentDraft::decode(raw.contract_version, raw.draft)
                .map_err(serde::de::Error::custom)?,
            reason: raw.reason,
        };
        value.validate().map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}

impl Serialize for CreateContent {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("CreateContent", 4)?;
        state.serialize_field("request_id", &self.request_id)?;
        state.serialize_field("contract_version", &self.draft.contract_version())?;
        match &self.draft {
            ContentDraft::V1(value) => state.serialize_field("draft", value)?,
            ContentDraft::V2(value) => state.serialize_field("draft", value)?,
        }
        state.serialize_field("reason", &self.reason)?;
        state.end()
    }
}

impl Serialize for ReviseContent {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("ReviseContent", 5)?;
        state.serialize_field("request_id", &self.request_id)?;
        state.serialize_field("base_revision_id", &self.base_revision_id)?;
        state.serialize_field("contract_version", &self.draft.contract_version())?;
        match &self.draft {
            ContentDraft::V1(value) => state.serialize_field("draft", value)?,
            ContentDraft::V2(value) => state.serialize_field("draft", value)?,
        }
        state.serialize_field("reason", &self.reason)?;
        state.end()
    }
}

impl Serialize for ContentRevision {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("ContentRevision", 10)?;
        state.serialize_field("block_id", &self.block_id)?;
        state.serialize_field("revision_id", &self.revision_id)?;
        state.serialize_field("parent_revision_id", &self.parent_revision_id)?;
        state.serialize_field("contract_version", &self.draft.contract_version())?;
        match &self.draft {
            ContentDraft::V1(value) => state.serialize_field("draft", value)?,
            ContentDraft::V2(value) => state.serialize_field("draft", value)?,
        }
        state.serialize_field("content_sha256", &self.content_sha256)?;
        state.serialize_field("author_id", &self.author_id)?;
        state.serialize_field("reason", &self.reason)?;
        state.serialize_field("created_at", &self.created_at)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for ContentRevision {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawContentRevision::deserialize(deserializer)?;
        Ok(Self {
            block_id: raw.block_id,
            revision_id: raw.revision_id,
            parent_revision_id: raw.parent_revision_id,
            draft: ContentDraft::decode(raw.contract_version, raw.draft)
                .map_err(serde::de::Error::custom)?,
            content_sha256: raw.content_sha256,
            author_id: raw.author_id,
            reason: raw.reason,
            created_at: raw.created_at,
        })
    }
}

impl Serialize for ContentDraft {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::V1(value) => value.serialize(serializer),
            Self::V2(value) => value.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for ContentDraft {
    fn deserialize<D: serde::Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
        Err(serde::de::Error::custom(
            "ContentDraft requires explicit contract_version dispatch",
        ))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ContentRevisionPage {
    pub items: Vec<ContentRevision>,
    pub next_cursor: Option<crate::PageCursor>,
}
impl TryFrom<ContentRevisionPage> for crate::RevisionPage {
    type Error = ContentError;
    fn try_from(p: ContentRevisionPage) -> Result<Self, Self::Error> {
        Ok(Self {
            items: p
                .items
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()?,
            next_cursor: p.next_cursor,
        })
    }
}

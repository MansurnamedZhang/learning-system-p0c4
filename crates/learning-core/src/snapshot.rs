//! Versioned, storage-independent contracts for exact snapshot exchange.
//! Database authorization and filesystem validation belong to later layers.
use crate::{
    AssetRef, AssetUseRef, ContentError, ReadingMode, ReadingRef, ResourceVersionRef,
    SourceSegmentRef, canonical_json, hex_digest,
};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const SNAPSHOT_FORMAT_VERSION: u32 = 1;
pub const SNAPSHOT_MAX_OBJECTS: usize = 2048;
pub const SNAPSHOT_MAX_EDGES: usize = 4096;
pub const SNAPSHOT_MAX_REFERENCE_DEPTH: usize = 32;
pub const SNAPSHOT_MAX_COMPOSITION_DEPTH: usize = 16;
pub const SNAPSHOT_MAX_OCCURRENCES: usize = 4096;
pub const SNAPSHOT_MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
pub const SNAPSHOT_MAX_FILES: usize = 2048;
pub const SNAPSHOT_MAX_JSON_FILE_BYTES: usize = 8 * 1024 * 1024;
pub const SNAPSHOT_MAX_JSON_BYTES: usize = 64 * 1024 * 1024;
pub const SNAPSHOT_MAX_ASSET_FILE_BYTES: usize = 128 * 1024 * 1024;
pub const SNAPSHOT_MAX_ASSET_BYTES: usize = 512 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotRequest {
    pub reading: ReadingRef,
    pub mode: ReadingMode,
    pub include_personal: bool,
    pub include_originals: bool,
    pub resource_versions: Vec<ResourceVersionRef>,
    pub source_segments: Vec<SourceSegmentRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "capability", rename_all = "snake_case")]
pub enum SnapshotManifest {
    ExactImportV1(ExactSnapshotManifest),
    ReadingCopyV1(ReadingCopyManifest),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawExactSnapshotManifest")]
pub struct ExactSnapshotManifest {
    pub format_version: u32,
    pub root: ReadingRef,
    pub files: Vec<SnapshotFile>,
    pub requires_destination_assets: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExactSnapshotManifest {
    format_version: u32,
    root: ReadingRef,
    files: Vec<SnapshotFile>,
    requires_destination_assets: bool,
}

impl TryFrom<RawExactSnapshotManifest> for ExactSnapshotManifest {
    type Error = String;

    fn try_from(raw: RawExactSnapshotManifest) -> Result<Self, Self::Error> {
        if raw.format_version != SNAPSHOT_FORMAT_VERSION {
            return Err("unsupported_snapshot_format_version".into());
        }
        validate_manifest_files(&raw.files)?;
        Ok(Self {
            format_version: raw.format_version,
            root: raw.root,
            files: raw.files,
            requires_destination_assets: raw.requires_destination_assets,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawSnapshotFile")]
pub struct SnapshotFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSnapshotFile {
    path: String,
    size: u64,
    sha256: String,
}

impl TryFrom<RawSnapshotFile> for SnapshotFile {
    type Error = String;

    fn try_from(raw: RawSnapshotFile) -> Result<Self, Self::Error> {
        if !valid_sha256(&raw.sha256) || !valid_package_file_path(&raw.path) {
            return Err("invalid_snapshot_file".into());
        }
        let limit = if raw.path.starts_with("assets/") {
            SNAPSHOT_MAX_ASSET_FILE_BYTES
        } else {
            SNAPSHOT_MAX_JSON_FILE_BYTES
        };
        if raw.size > limit as u64 {
            return Err("snapshot_file_limit_exceeded".into());
        }
        Ok(Self {
            path: raw.path,
            size: raw.size,
            sha256: raw.sha256,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawReadingCopyManifest")]
pub struct ReadingCopyManifest {
    pub format_version: u32,
    pub copy_id: Uuid,
    pub files: Vec<SnapshotFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawReadingCopyManifest {
    format_version: u32,
    copy_id: Uuid,
    files: Vec<SnapshotFile>,
}

impl TryFrom<RawReadingCopyManifest> for ReadingCopyManifest {
    type Error = String;

    fn try_from(raw: RawReadingCopyManifest) -> Result<Self, Self::Error> {
        if raw.format_version != SNAPSHOT_FORMAT_VERSION {
            return Err("unsupported_snapshot_format_version".into());
        }
        validate_manifest_files(&raw.files)?;
        validate_copy_manifest_files(&raw.files)?;
        Ok(Self {
            format_version: raw.format_version,
            copy_id: raw.copy_id,
            files: raw.files,
        })
    }
}

/// A fixed whitelist. Object paths can only use one of these table names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotTable {
    Asset,
    Resource,
    ResourceVersion,
    SourceSegment,
    Block,
    BlockRevision,
    BlockAssetUse,
    Composition,
    CompositionRevision,
    CompositionOccurrence,
    Overlay,
    OverlayRevision,
    OverlayGroupIdentity,
    OverlayPlacementIdentity,
    OverlayGroup,
    OverlayPlacement,
    PlacementManualDecision,
    ReferenceObject,
    ReferenceDependency,
    Relation,
    RelationRevision,
    RelationReviewHead,
    RelationReview,
    EpistemicStream,
    EpistemicReview,
    ReadingView,
    ReadingViewRevision,
    ReadingRelationSelection,
    ReadingEpistemicSelection,
}

/// Exactly the kinds admitted by `reference_object.kind` in schema 0004.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceKind {
    Block,
    Relation,
    RelationReview,
    EpistemicReview,
}

/// One primary-key component. Text and negative numbers have no representation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum SnapshotIdentityPart {
    Uuid(Uuid),
    Kind(ReferenceKind),
    Position(u32),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PartShape {
    Uuid,
    Kind,
    Position,
}

impl SnapshotIdentityPart {
    fn shape(&self) -> PartShape {
        match self {
            Self::Uuid(_) => PartShape::Uuid,
            Self::Kind(_) => PartShape::Kind,
            Self::Position(_) => PartShape::Position,
        }
    }

    fn token(&self) -> String {
        match self {
            Self::Uuid(value) => format!("u-{value}"),
            Self::Kind(value) => {
                let kind = serde_json::to_value(value).expect("closed reference kind serializes");
                format!("k-{}", kind.as_str().expect("reference kind is a string"))
            }
            Self::Position(value) => format!("p-{value}"),
        }
    }
}

impl SnapshotTable {
    fn primary_key_shape(self) -> &'static [PartShape] {
        use PartShape::{Kind as K, Position as P, Uuid as U};
        match self {
            Self::Asset | Self::Resource | Self::ResourceVersion | Self::SourceSegment => &[U, U],
            Self::Block
            | Self::BlockRevision
            | Self::Composition
            | Self::CompositionRevision
            | Self::Overlay
            | Self::OverlayRevision
            | Self::Relation
            | Self::RelationRevision
            | Self::RelationReview
            | Self::EpistemicStream
            | Self::EpistemicReview
            | Self::ReadingView
            | Self::ReadingViewRevision => &[U],
            Self::BlockAssetUse => &[U, U, U],
            Self::CompositionOccurrence
            | Self::OverlayGroupIdentity
            | Self::OverlayPlacementIdentity
            | Self::OverlayGroup
            | Self::OverlayPlacement
            | Self::PlacementManualDecision
            | Self::RelationReviewHead => &[U, U],
            Self::ReferenceObject => &[K, U, U],
            Self::ReferenceDependency => &[K, U, U, P],
            Self::ReadingRelationSelection | Self::ReadingEpistemicSelection => &[U, P],
        }
    }
}

pub fn snapshot_object_path(
    table: SnapshotTable,
    identity: &[SnapshotIdentityPart],
) -> Result<String, ContentError> {
    if identity.len() != table.primary_key_shape().len()
        || !identity
            .iter()
            .zip(table.primary_key_shape())
            .all(|(part, shape)| part.shape() == *shape)
    {
        return Err(ContentError::Invalid("invalid_snapshot_identity".into()));
    }
    let name = serde_json::to_value(table).expect("closed table serializes");
    Ok(format!(
        "objects/{}/{}.json",
        name.as_str().expect("table name is a string"),
        identity
            .iter()
            .map(SnapshotIdentityPart::token)
            .collect::<Vec<_>>()
            .join("__")
    ))
}

pub fn parse_snapshot_object_path(
    path: &str,
) -> Result<(SnapshotTable, Vec<SnapshotIdentityPart>), ContentError> {
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() != 3 || parts[0] != "objects" {
        return Err(ContentError::Invalid("invalid_snapshot_path".into()));
    }
    let table: SnapshotTable = serde_json::from_value(Value::String(parts[1].into()))
        .map_err(|_| ContentError::Invalid("invalid_snapshot_path".into()))?;
    let file = parts[2]
        .strip_suffix(".json")
        .ok_or_else(|| ContentError::Invalid("invalid_snapshot_path".into()))?;
    let mut identity = Vec::new();
    for token in file.split("__") {
        let part = if let Some(raw) = token.strip_prefix("u-") {
            let id = Uuid::parse_str(raw)
                .map_err(|_| ContentError::Invalid("invalid_snapshot_path".into()))?;
            SnapshotIdentityPart::Uuid(id)
        } else if let Some(raw) = token.strip_prefix("k-") {
            let kind = serde_json::from_value(Value::String(raw.into()))
                .map_err(|_| ContentError::Invalid("invalid_snapshot_path".into()))?;
            SnapshotIdentityPart::Kind(kind)
        } else if let Some(raw) = token.strip_prefix("p-") {
            let position = raw
                .parse::<u32>()
                .map_err(|_| ContentError::Invalid("invalid_snapshot_path".into()))?;
            SnapshotIdentityPart::Position(position)
        } else {
            return Err(ContentError::Invalid("invalid_snapshot_path".into()));
        };
        if part.token() != token {
            return Err(ContentError::Invalid("invalid_snapshot_path".into()));
        }
        identity.push(part);
    }
    if snapshot_object_path(table, &identity)? != path {
        return Err(ContentError::Invalid("invalid_snapshot_path".into()));
    }
    Ok((table, identity))
}

/// `immutable_values` contains all columns of a revision or attached row.
/// For a container it excludes mutable head/published/last_release state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawSnapshotRow")]
pub struct SnapshotRow {
    pub table: SnapshotTable,
    pub identity: Vec<SnapshotIdentityPart>,
    pub immutable_values: Value,
    pub sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSnapshotRow {
    table: SnapshotTable,
    identity: Vec<SnapshotIdentityPart>,
    immutable_values: Value,
    sha256: String,
}

impl TryFrom<RawSnapshotRow> for SnapshotRow {
    type Error = String;

    fn try_from(raw: RawSnapshotRow) -> Result<Self, Self::Error> {
        if !raw.immutable_values.is_object()
            || !valid_sha256(&raw.sha256)
            || canonical_record_hash(&raw.immutable_values) != raw.sha256
            || snapshot_object_path(raw.table, &raw.identity).is_err()
        {
            return Err("invalid_snapshot_row".into());
        }
        Ok(Self {
            table: raw.table,
            identity: raw.identity,
            immutable_values: raw.immutable_values,
            sha256: raw.sha256,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawSnapshotAssetUse")]
pub struct SnapshotAssetUse {
    pub use_ref: AssetUseRef,
    pub asset: AssetRef,
    pub sha256: String,
    pub byte_size: u64,
    pub storage_key: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSnapshotAssetUse {
    use_ref: AssetUseRef,
    asset: AssetRef,
    sha256: String,
    byte_size: u64,
    storage_key: String,
}

impl TryFrom<RawSnapshotAssetUse> for SnapshotAssetUse {
    type Error = String;

    fn try_from(raw: RawSnapshotAssetUse) -> Result<Self, Self::Error> {
        if !valid_sha256(&raw.sha256)
            || raw.storage_key != format!("sha256/{}/{}", &raw.sha256[..2], raw.sha256)
        {
            return Err("invalid_snapshot_asset_use".into());
        }
        Ok(Self {
            use_ref: raw.use_ref,
            asset: raw.asset,
            sha256: raw.sha256,
            byte_size: raw.byte_size,
            storage_key: raw.storage_key,
        })
    }
}

/// The only data permitted in a clipped reading copy. No exact identity fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadingCopy {
    pub items: Vec<CopyItem>,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CopyItem {
    Heading {
        title: String,
    },
    Content {
        intent: String,
        title: String,
        display_text: String,
    },
    Omitted,
}

/// Hash the whole immutable row, not its narrower business content digest.
pub fn canonical_record_hash(value: &Value) -> String {
    hex_digest(canonical_json(value).as_bytes())
}

/// Database timestamptz wire form for canonical record JSON.
pub fn canonical_snapshot_timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Micros, true)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotBudgetKind {
    ReferenceObject,
    ReferenceEdge,
    CompositionObject,
    CompositionOccurrence,
    JsonFile,
    AssetFile,
}

/// One instance must be shared by every root and dependency in a package.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapshotBudget {
    objects: usize,
    edges: usize,
    body_bytes: usize,
    occurrences: usize,
    files: usize,
    json_bytes: usize,
    asset_bytes: usize,
}

impl SnapshotBudget {
    /// Charge only after all limits pass, leaving state unchanged on error.
    pub fn charge(
        &mut self,
        kind: SnapshotBudgetKind,
        bytes: usize,
        depth: usize,
    ) -> Result<(), ContentError> {
        let mut next = self.clone();
        let (counter, limit) = match kind {
            SnapshotBudgetKind::ReferenceObject => {
                if depth > SNAPSHOT_MAX_REFERENCE_DEPTH {
                    return Err(limit_error());
                }
                next.body_bytes = add_limited(next.body_bytes, bytes, SNAPSHOT_MAX_BODY_BYTES)?;
                (&mut next.objects, SNAPSHOT_MAX_OBJECTS)
            }
            SnapshotBudgetKind::ReferenceEdge => {
                if depth > SNAPSHOT_MAX_REFERENCE_DEPTH {
                    return Err(limit_error());
                }
                (&mut next.edges, SNAPSHOT_MAX_EDGES)
            }
            SnapshotBudgetKind::CompositionObject => {
                if depth > SNAPSHOT_MAX_COMPOSITION_DEPTH {
                    return Err(limit_error());
                }
                next.body_bytes = add_limited(next.body_bytes, bytes, SNAPSHOT_MAX_BODY_BYTES)?;
                (&mut next.objects, SNAPSHOT_MAX_OBJECTS)
            }
            SnapshotBudgetKind::CompositionOccurrence => {
                if depth > SNAPSHOT_MAX_COMPOSITION_DEPTH {
                    return Err(limit_error());
                }
                (&mut next.occurrences, SNAPSHOT_MAX_OCCURRENCES)
            }
            SnapshotBudgetKind::JsonFile => {
                if bytes > SNAPSHOT_MAX_JSON_FILE_BYTES {
                    return Err(limit_error());
                }
                next.json_bytes = add_limited(next.json_bytes, bytes, SNAPSHOT_MAX_JSON_BYTES)?;
                (&mut next.files, SNAPSHOT_MAX_FILES)
            }
            SnapshotBudgetKind::AssetFile => {
                if bytes > SNAPSHOT_MAX_ASSET_FILE_BYTES {
                    return Err(limit_error());
                }
                next.asset_bytes = add_limited(next.asset_bytes, bytes, SNAPSHOT_MAX_ASSET_BYTES)?;
                (&mut next.files, SNAPSHOT_MAX_FILES)
            }
        };
        *counter = add_limited(*counter, 1, limit)?;
        *self = next;
        Ok(())
    }
}

fn add_limited(current: usize, increment: usize, limit: usize) -> Result<usize, ContentError> {
    current
        .checked_add(increment)
        .filter(|total| *total <= limit)
        .ok_or_else(limit_error)
}

fn limit_error() -> ContentError {
    ContentError::Invalid("snapshot_limit_exceeded".into())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_package_file_path(path: &str) -> bool {
    if matches!(path, "validation.json" | "reading.html" | "reading.md") {
        return true;
    }
    if path.starts_with("objects/") {
        return parse_snapshot_object_path(path).is_ok();
    }
    let parts: Vec<_> = path.split('/').collect();
    parts.len() == 4
        && parts[0] == "assets"
        && parts[1] == "sha256"
        && parts[2].len() == 2
        && valid_sha256(parts[3])
        && parts[2] == &parts[3][..2]
}

fn validate_manifest_files(files: &[SnapshotFile]) -> Result<(), String> {
    if files.len() >= SNAPSHOT_MAX_FILES
        || !files.windows(2).all(|pair| pair[0].path < pair[1].path)
    {
        return Err("invalid_snapshot_manifest_files".into());
    }
    let mut budget = SnapshotBudget::default();
    // manifest.json itself is a package file, but is excluded from `files`.
    budget
        .charge(SnapshotBudgetKind::JsonFile, 0, 0)
        .map_err(|_| "snapshot_limit_exceeded".to_string())?;
    for file in files {
        let kind = if file.path.starts_with("assets/") {
            SnapshotBudgetKind::AssetFile
        } else {
            SnapshotBudgetKind::JsonFile
        };
        budget
            .charge(kind, file.size as usize, 0)
            .map_err(|_| "snapshot_limit_exceeded".to_string())?;
    }
    Ok(())
}

fn validate_copy_manifest_files(files: &[SnapshotFile]) -> Result<(), String> {
    if files.iter().all(|file| {
        matches!(
            file.path.as_str(),
            "reading.html" | "reading.md" | "validation.json"
        ) || file.path.starts_with("assets/sha256/")
    }) {
        Ok(())
    } else {
        Err("invalid_reading_copy_file".into())
    }
}

mod read;
mod write;

use learning_assets::FsAssetStore;
use learning_core::AssetRef;
use sqlx::PgPool;
use uuid::Uuid;

/// Metadata supplied by the trusted upload boundary, never used as a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetMedia {
    pub media_type: String,
    pub original_file_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetRecord {
    pub reference: AssetRef,
    pub sha256: String,
    pub byte_size: i64,
    pub storage_key: String,
    pub media: AssetMedia,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceInput {
    pub space_id: Uuid,
    pub resource_id: Option<Uuid>,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSegmentRef {
    pub space_id: Uuid,
    pub resource_id: Uuid,
    pub version_id: Uuid,
    pub segment_id: Uuid,
}

#[derive(Clone)]
pub struct AssetStore {
    pool: PgPool,
    files: FsAssetStore,
}

impl AssetStore {
    /// The pool must use the non-owner runtime role. The file store must point
    /// to the same immutable assets volume that finalized the VerifiedBlob.
    pub fn new(pool: PgPool, files: FsAssetStore) -> Self {
        Self { pool, files }
    }
}

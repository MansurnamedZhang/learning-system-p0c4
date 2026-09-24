mod read;
pub(crate) use read::read_for_use_in_tx;
mod write;

use learning_assets::FsAssetStore;
pub use learning_core::SourceSegmentRef;
use learning_core::{AssetRef, ContentError};
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

    pub(crate) fn verify_record_bytes(&self, record: &AssetRecord) -> Result<(), ContentError> {
        self.files
            .open_record(&record.storage_key, &record.sha256, record.byte_size)
            .map(|_| ())
            .map_err(crate::storage)
    }
}

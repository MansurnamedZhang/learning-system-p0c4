//! PostgreSQL persistence for the trusted application boundary.
mod assets;
mod authorization;
pub use assets::{AssetMedia, AssetRecord, AssetStore, ResourceInput, SourceSegmentRef};
mod block_write;
mod jobs;
pub use jobs::{JobFailureClass, JobLease, JobRecord, JobStore};
mod job_processor;
pub use job_processor::{AssetIntegrityProcessor, AssetProcessOutcome, PreparedAssetIntegrity};
mod lineage;
pub use lineage::LineageStore;
mod composition;
mod overlay;
mod placement_migration;
mod queries;
pub use placement_migration::MigrationStore;
pub use queries::QueryStore;
mod reading;
pub use reading::ReadingStore;
mod read;
mod references;
mod relations;
pub use relations::RelationStore;
mod reviews;
pub use reviews::ReviewStore;
mod release;
mod request;
mod snapshot;
pub use snapshot::{
    PreparedSnapshotExport, PreparedSnapshotImport, SnapshotDelivery, SnapshotImportStore,
    SnapshotPlan, SnapshotStore, StagedSnapshotExport,
};
mod versioned_content;
pub use versioned_content::VersionedContentStore;
mod write;
pub use composition::CompositionStore;
pub use release::ReleaseStore;

use chrono::{DateTime, Utc};
use learning_core::{ContentDraft, ContentError, ContentRevision};
use sqlx::{PgPool, types::Json};
use uuid::Uuid;

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

#[derive(Clone)]
pub struct ContentStore {
    pool: PgPool,
}
impl ContentStore {
    /// The pool must authenticate as the non-owner runtime role, never as migrator.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

const COLUMNS: &str = "r.block_id,r.id AS revision_id,r.parent_revision_id,r.contract_version,r.content,r.content_sha256,r.author_id,r.reason,r.created_at";

#[derive(sqlx::FromRow)]
struct RevisionRow {
    block_id: Uuid,
    revision_id: Uuid,
    parent_revision_id: Option<Uuid>,
    contract_version: i32,
    content: Json<serde_json::Value>,
    content_sha256: String,
    author_id: Uuid,
    reason: String,
    created_at: DateTime<Utc>,
}
impl RevisionRow {
    fn decode(self) -> Result<ContentRevision, ContentError> {
        let r = self;
        Ok(ContentRevision {
            block_id: r.block_id,
            revision_id: r.revision_id,
            parent_revision_id: r.parent_revision_id,
            draft: ContentDraft::decode(r.contract_version as u32, r.content.0).map_err(storage)?,
            content_sha256: r.content_sha256,
            author_id: r.author_id,
            reason: r.reason,
            created_at: r.created_at,
        })
    }
}
fn storage(_: impl std::fmt::Debug) -> ContentError {
    ContentError::Storage
}

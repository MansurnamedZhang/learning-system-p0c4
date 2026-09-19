//! PostgreSQL persistence for the trusted application boundary.
mod authorization;
mod composition;
mod read;
mod release;
mod request;
mod write;
pub use composition::CompositionStore;
pub use release::ReleaseStore;

use chrono::{DateTime, Utc};
use learning_core::{ContentError, Revision, TextDraft};
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

const COLUMNS: &str = "r.block_id,r.id AS revision_id,r.parent_revision_id,r.content,r.content_sha256,r.author_id,r.reason,r.created_at";

#[derive(sqlx::FromRow)]
struct RevisionRow {
    block_id: Uuid,
    revision_id: Uuid,
    parent_revision_id: Option<Uuid>,
    content: Json<TextDraft>,
    content_sha256: String,
    author_id: Uuid,
    reason: String,
    created_at: DateTime<Utc>,
}
impl From<RevisionRow> for Revision {
    fn from(r: RevisionRow) -> Self {
        Self {
            block_id: r.block_id,
            revision_id: r.revision_id,
            parent_revision_id: r.parent_revision_id,
            draft: r.content.0,
            content_sha256: r.content_sha256,
            author_id: r.author_id,
            reason: r.reason,
            created_at: r.created_at,
        }
    }
}
fn storage(_: impl std::fmt::Debug) -> ContentError {
    ContentError::Storage
}

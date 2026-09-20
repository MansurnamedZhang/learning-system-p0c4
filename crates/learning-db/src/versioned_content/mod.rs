mod read;
mod write;
#[derive(Clone)]
pub struct VersionedContentStore {
    pool: sqlx::PgPool,
}
impl VersionedContentStore {
    /// Uses the trusted runtime pool, never the migration owner.
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

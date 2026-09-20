mod classify;
mod decide;
mod model;
mod propose;
use sqlx::PgPool;
#[derive(Clone)]
pub struct MigrationStore {
    pub(crate) pool: PgPool,
}
impl MigrationStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

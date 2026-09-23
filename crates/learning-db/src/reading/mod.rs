mod projection;
pub(crate) use projection::project as project_for_impact;
mod read;
pub(crate) mod selection;
mod write;
use sqlx::PgPool;
#[derive(Clone)]
pub struct ReadingStore {
    pub(crate) pool: PgPool,
}
impl ReadingStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

mod projection;
mod read;
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

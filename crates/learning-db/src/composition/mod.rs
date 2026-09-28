pub(crate) mod closure;
mod read;
mod write;
use sqlx::PgPool;
#[derive(Clone)]
pub struct CompositionStore {
    pub(crate) pool: PgPool,
}
impl CompositionStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

mod read;
mod write;

use sqlx::PgPool;

#[derive(Clone)]
pub struct ReviewStore {
    pool: PgPool,
}
impl ReviewStore {
    /// The pool authenticates as the non-owner runtime role.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

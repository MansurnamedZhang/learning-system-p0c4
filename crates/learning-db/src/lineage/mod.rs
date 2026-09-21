mod read;
mod write;

#[derive(Clone)]
pub struct LineageStore {
    pool: sqlx::PgPool,
}
impl LineageStore {
    /// The pool authenticates as the trusted, non-owner runtime role.
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

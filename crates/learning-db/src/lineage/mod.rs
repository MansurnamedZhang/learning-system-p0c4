mod read;
mod write;

pub(crate) use read::load as load_for_impact;

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

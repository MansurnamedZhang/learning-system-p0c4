mod read;
mod write;
use sqlx::PgPool;
#[derive(Clone)]
pub struct ReleaseStore {
    pub(crate) pool: PgPool,
}
impl ReleaseStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn publication_token(composition: uuid::Uuid, release: Option<uuid::Uuid>) -> String {
    learning_core::hex_digest(learning_core::canonical_json(&serde_json::json!({"domain":"publication-basis-v1","composition_id":composition,"last_release_id":release})).as_bytes())
}

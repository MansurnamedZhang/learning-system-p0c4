use learning_core::{ContentError, CreateCommand, Principal, ReviseCommand, Revision};
use sqlx::PgPool;
use uuid::Uuid;

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

#[derive(Clone)]
pub struct ContentStore { pub pool: PgPool }

impl ContentStore {
    pub fn new(pool: PgPool) -> Self { Self { pool } }
    pub async fn create(&self, _actor: Principal, _space: Uuid, _cmd: CreateCommand) -> Result<Revision, ContentError> { Err(ContentError::Storage) }
    pub async fn revise(&self, _actor: Principal, _block: Uuid, _cmd: ReviseCommand) -> Result<Revision, ContentError> { Err(ContentError::Storage) }
    pub async fn read(&self, _actor: Principal, _revision: Uuid) -> Result<Option<Revision>, ContentError> { Ok(None) }
    pub async fn read_many(&self, _actor: Principal, _ids: &[Uuid]) -> Result<Vec<Revision>, ContentError> { Ok(vec![]) }
    pub async fn list(&self, _actor: Principal, _space: Uuid) -> Result<Vec<Revision>, ContentError> { Ok(vec![]) }
    pub async fn history(&self, _actor: Principal, _block: Uuid) -> Result<Vec<Revision>, ContentError> { Ok(vec![]) }
}

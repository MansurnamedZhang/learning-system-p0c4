use crate::{AssetRow, BackupError, BackupPlan};
use sqlx::PgPool;
use uuid::Uuid;

/// A management-process entry point. It rejects runtime connections even
/// though the runtime role has ordinary SELECT permission on `asset`.
#[derive(Clone)]
pub struct AdminAssetCatalog {
    pool: PgPool,
}

impl AdminAssetCatalog {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Collects all ready rows in one read-only repeatable-read transaction.
    /// Task 3 must establish the durable write gate before this is paired with
    /// a database dump; this method alone does not make a consistent backup.
    pub async fn plan_assets(&self) -> Result<BackupPlan, BackupError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await?;
        let (current, session): (String, String) =
            sqlx::query_as("SELECT current_user::text, session_user::text")
                .fetch_one(&mut *tx)
                .await?;
        if current != "learning_admin" || session != "learning_admin" {
            return Err(BackupError::Invalid("management role required"));
        }
        let rows: Vec<(Uuid, Uuid, String, i64, String)> = sqlx::query_as(
            "SELECT space_id,id,sha256,byte_size,storage_key \
             FROM public.asset WHERE status='ready' ORDER BY space_id,id",
        )
        .fetch_all(&mut *tx)
        .await?;
        let plan = BackupPlan::from_rows(
            rows.into_iter()
                .map(|(space_id, id, sha256, byte_size, storage_key)| AssetRow {
                    space_id,
                    id,
                    sha256,
                    byte_size,
                    storage_key,
                })
                .collect(),
        )?;
        tx.commit().await?;
        Ok(plan)
    }
}

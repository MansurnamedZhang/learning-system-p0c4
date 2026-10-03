use super::require_admin_owner;
use crate::BackupError;
use sqlx::{PgConnection, PgPool, Postgres, pool::PoolConnection};

/// Owns the admitted session through every SQL and filesystem phase. Closing
/// on Drop releases only session resources; it never reopens a source gate.
pub(super) struct SourceAdmission {
    connection: PoolConnection<Postgres>,
    database: String,
}

impl SourceAdmission {
    pub(super) async fn try_acquire(
        pool: &PgPool,
        expected_database: &str,
    ) -> Result<Self, BackupError> {
        let mut connection = pool.acquire().await?;
        // Set before the first fallible SQL operation: errors/cancellation must
        // never return an admitted (or unverified) session to the pool.
        connection.close_on_drop();
        let database = require_admin_owner(&mut connection).await?;
        if database != expected_database {
            return Err(BackupError::Invalid("source admission database identity"));
        }
        let acquired: bool = sqlx::query_scalar("SELECT pg_catalog.pg_try_advisory_lock($1, $2)")
            .bind(0x4b57_4334_i32)
            .bind(0x5352_4345_i32)
            .fetch_one(&mut *connection)
            .await?;
        if !acquired {
            return Err(BackupError::Invalid("source maintenance admission busy"));
        }
        Ok(Self {
            connection,
            database,
        })
    }

    pub(super) fn database(&self) -> &str {
        &self.database
    }

    pub(super) fn connection(&mut self) -> &mut PgConnection {
        &mut self.connection
    }

    pub(super) async fn close(self) -> Result<(), BackupError> {
        self.connection.close().await?;
        Ok(())
    }
}

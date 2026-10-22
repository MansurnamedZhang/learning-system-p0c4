//! Optional C0 profile/public owned observation. Never installed by startup.
//!
//! The separate broker role has exactly the three canonical EXECUTE grants
//! required by its versioned profile. C0 stays observation/rollback only;
//! private P1 and C2+D0 FIRST WRITE COMPONENT have no public factory or source
//! admission. First-write staging is not publication or positive classification.
//! A1 and A2 ObservationV1 contracts are not aliases for this unsupported schema.
mod catalog;
#[cfg_attr(
    any(not(test), not(target_os = "linux")),
    expect(dead_code, reason = "C1-P1 has no production admission factory")
)]
mod history;
mod transaction;
#[cfg(target_os = "linux")]
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "C1-P1 has no production admission factory")
)]
mod transition;
#[cfg(all(test, target_os = "linux"))]
mod transition_tests;
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "C2+D0 has no production admission factory")
)]
mod writer;
#[cfg(test)]
mod writer_tests;

pub use transaction::{ObservationBudget, ObservationTransaction};

use sqlx::PgPool;
use std::time::Duration;
use tokio::time::{Instant, timeout_at};

pub const BROKER_ROLE: &str = "learning_snapshot_claim_broker";
pub const INSTALL_SQL: &str = include_str!("../../../../extensions/snapshot-claim-v1/install.sql");

pub enum ObservationError {
    Invalid(&'static str),
    Capacity,
    Timeout,
    Database(sqlx::Error),
}
impl ObservationError {
    /// Safe diagnostic code; no DSN, password or raw server message is exposed.
    pub fn sqlstate(&self) -> Option<String> {
        match self {
            Self::Database(error) => error
                .as_database_error()
                .and_then(|error| error.code())
                .map(|code| code.into_owned()),
            _ => None,
        }
    }
}
impl std::fmt::Display for ObservationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(reason) => f.write_str(reason),
            Self::Capacity => f.write_str("snapshot claim observation capacity exceeded"),
            Self::Timeout => f.write_str("snapshot claim observation deadline exceeded"),
            Self::Database(_) => {
                f.write_str("snapshot claim observation database operation failed")
            }
        }
    }
}
impl std::fmt::Debug for ObservationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self}")
    }
}
impl std::error::Error for ObservationError {}
impl From<sqlx::Error> for ObservationError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

/// Explicit DB-owner installation, not cluster-role provisioning or enabling.
/// Unknown roles/grants/definitions are rejected, never silently repaired.
/// A failed/ambiguous install is not adopted or automatically replayed.
pub async fn install(pool: &PgPool) -> Result<(), ObservationError> {
    timeout_at(Instant::now() + Duration::from_secs(30), async {
        let mut tx = pool.begin().await?;
        for sql in [
            "SET LOCAL search_path=pg_catalog",
            "SET LOCAL lock_timeout='5s'",
            "SET LOCAL statement_timeout='15s'",
            "SET LOCAL knowweave.snapshot_claim_install='v1'",
        ] {
            sqlx::query(sql).execute(&mut *tx).await?;
        }
        catalog::set_phase(&mut tx, catalog::Phase::InstallBefore).await?;
        // INSTALL_SQL includes the complete before-profile DO before any DDL.
        sqlx::raw_sql(INSTALL_SQL).execute(&mut *tx).await?;
        catalog::verify(&mut tx, catalog::Phase::InstallAfter).await?;
        tx.commit().await?;
        Ok(())
    })
    .await
    .map_err(|_| ObservationError::Timeout)?
}

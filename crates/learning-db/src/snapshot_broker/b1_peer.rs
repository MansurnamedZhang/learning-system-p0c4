//! Default-off original-process qualification, separate from SQL/source authority.
//!
//! Canonical Root DATA, a UID or a socket cookie alone cannot construct a ctx.
//! The zero-argument factory independently joins current kernel/held originals.
//! M2b sessions and M4/M5/M6 operational producers/consumers are not substitutes
//! for these gates and have no placeholder success/Err API here.

mod job;
mod process;
mod session;
pub(super) use session::{FixedWorkPools, WorkLifetime, quiesce_b1};
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Error {
    Shape,
    Identity,
    Phase,
    Capacity,
    Poisoned,
    RootLost,
    Eof,
    Io,
    Deadline,
}
type Result<T> = std::result::Result<T, Error>;

/// Actual current process only. No Deserialize/Serialize/Clone/Default/Debug,
/// caller-owned constructor or conversion from checked DATA exists.
pub struct B1InstalledSourceProcess(std::sync::Arc<process::PeerOriginal>);
impl B1InstalledSourceProcess {
    pub async fn acquire_fixed_admin(
        &self,
    ) -> std::result::Result<OwnedB1AdminSession, super::ObservationError> {
        session::acquire_fixed_admin(self.0.custody())
            .await
            .map(OwnedB1AdminSession)
            .map_err(observation_error)
    }
    /// Permanently closes new qualification and naturally awaits only this
    /// process's already-started original resource owners. Never grants authority.
    pub async fn close_original(&self) -> std::result::Result<(), super::ObservationError> {
        self.0.close_original().await.map_err(observation_error)
    }
    pub fn admit_installed() -> std::result::Result<Self, super::ObservationError> {
        process::admit_original_process()
            .map(|original| Self(std::sync::Arc::new(original)))
            .map_err(observation_error)
    }
    pub fn revalidate(&self) -> std::result::Result<(), super::ObservationError> {
        self.0.revalidate().map_err(observation_error)
    }
    /// Same cumulative backing; consuming budget grants no operation identity.
    pub fn scan_accounting_meter(&self) -> learning_assets::root_journal::SharedJournalBudget {
        self.0.scan()
    }
    /// Capture-only DATA from this original factory's internal fresh UUID.
    pub fn capture_backup_id_data(
        &self,
    ) -> std::result::Result<uuid::Uuid, super::ObservationError> {
        self.0.capture_backup_id_data().map_err(observation_error)
    }
}
pub(super) async fn install_fresh(
    original: std::sync::Arc<B1InstalledSourceProcess>,
    installed: learning_assets::InstalledSnapshotBinding,
    source: super::lifecycle::BrokerSourceDescription,
    sockets: (
        &std::os::unix::net::UnixListener,
        &std::os::unix::net::UnixListener,
    ),
) -> super::history::Result<super::lifecycle::RootSnapshotBroker> {
    let broker =
        session::install_fresh_b1(original.0.custody(), installed, source, sockets).await?;
    *broker
        .b1_original
        .lock()
        .map_err(|_| observation_error(Error::Poisoned))? = Some(original);
    Ok(broker)
}

/// The actual session's retained connection owner; no external constructor.
pub struct OwnedB1AdminSession(session::OwnedB1AdminSession);
impl OwnedB1AdminSession {
    pub fn database(&self) -> &str {
        self.0.database()
    }
    pub fn connection_in_original_owner(&mut self) -> &mut sqlx::PgConnection {
        self.0.connection_in_original_owner()
    }
    pub async fn close(self) -> std::result::Result<(), super::ObservationError> {
        self.0.close().await.map_err(observation_error)
    }
    pub fn isolation_input_data(
        &self,
    ) -> std::result::Result<B1SourceIsolationInputData, super::ObservationError> {
        self.0
            .isolation_input_data()
            .map(B1SourceIsolationInputData)
            .map_err(observation_error)
    }
}
/// Read-only DATA from a current actual session, never a session constructor.
pub struct B1SourceIsolationInputData(session::IsolationInputData);
impl B1SourceIsolationInputData {
    pub fn job_id(&self) -> uuid::Uuid {
        self.0.job_id()
    }
    pub fn backup_id(&self) -> uuid::Uuid {
        self.0.backup_id()
    }
    pub fn phase(&self) -> &'static str {
        self.0.phase()
    }
    pub fn generation_binding_sha256(&self) -> [u8; 32] {
        self.0.generation_binding_sha256()
    }
    pub fn catalog_sha256(&self) -> [u8; 32] {
        self.0.catalog_sha256()
    }
}
fn observation_error(error: Error) -> super::ObservationError {
    match error {
        Error::Capacity => super::ObservationError::Capacity,
        Error::Deadline => super::ObservationError::Timeout,
        Error::Poisoned => super::ObservationError::Invalid("B1 source process permanently held"),
        _ => super::ObservationError::Invalid("B1 installed source process rejected"),
    }
}

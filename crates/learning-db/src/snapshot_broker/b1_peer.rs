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
    /// Fixed Source action only. Returned Root DATA is not a publication S.
    pub async fn spawn_capture_and_read_data(
        &self,
    ) -> std::result::Result<B1CapturedInputData, super::ObservationError> {
        session::spawn_capture_and_read_data(
            self.0.prepay_control_custody().map_err(observation_error)?,
        )
        .await
        .map_err(observation_error)
    }
    /// Called by the original M6 Source only after its authenticated D Ready.
    /// This request does not itself establish Ready or observe kernel settlement.
    pub async fn request_capture_settlement(
        &self,
    ) -> std::result::Result<B1CaptureSettlementData, super::ObservationError> {
        session::request_capture_settlement(
            self.0.prepay_control_custody().map_err(observation_error)?,
        )
        .await
        .map_err(observation_error)
    }
    /// Capture reports only its fixed output DATA digest and waits for the
    /// original Root stop request. Notification is not cut release or exit proof.
    pub async fn report_captured_digest_and_wait_for_settle(
        &self,
        captured_data_sha256: [u8; 32],
    ) -> std::result::Result<B1CaptureStopRequestedData, super::ObservationError> {
        session::report_captured_digest_and_wait_for_settle(
            self.0.prepay_control_custody().map_err(observation_error)?,
            captured_data_sha256,
        )
        .await
        .map_err(observation_error)
    }
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
/// Read-only projections of already joined Root records. None owns a ctx,
/// connection, descriptor, cut, permission or signer; there are no constructors.
pub struct B1CapturedInputData {
    job_id: uuid::Uuid,
    backup_id: uuid::Uuid,
    capture_start_sha256: [u8; 32],
    capture_generation_sha256: [u8; 32],
    source_group_id: uuid::Uuid,
    source_registry_generation_sha256: [u8; 32],
    manifest_sha256: [u8; 32],
    captured_roster_sha256: [u8; 32],
    snapshot_effects_roster_sha256: [u8; 32],
    record_sha256: [u8; 32],
}
impl B1CapturedInputData {
    pub fn job_id(&self) -> uuid::Uuid {
        self.job_id
    }
    pub fn backup_id(&self) -> uuid::Uuid {
        self.backup_id
    }
    pub fn capture_start_sha256(&self) -> [u8; 32] {
        self.capture_start_sha256
    }
    pub fn capture_generation_sha256(&self) -> [u8; 32] {
        self.capture_generation_sha256
    }
    pub fn source_group_id(&self) -> uuid::Uuid {
        self.source_group_id
    }
    pub fn source_registry_generation_sha256(&self) -> [u8; 32] {
        self.source_registry_generation_sha256
    }
    pub fn manifest_sha256(&self) -> [u8; 32] {
        self.manifest_sha256
    }
    pub fn captured_roster_sha256(&self) -> [u8; 32] {
        self.captured_roster_sha256
    }
    pub fn snapshot_effects_roster_sha256(&self) -> [u8; 32] {
        self.snapshot_effects_roster_sha256
    }
    pub fn record_sha256(&self) -> [u8; 32] {
        self.record_sha256
    }
}
pub struct B1CaptureSettlementData {
    job_id: uuid::Uuid,
    backup_id: uuid::Uuid,
    capture_start_sha256: [u8; 32],
    captured_data_sha256: [u8; 32],
    exec_id_bytes: [u8; 32],
    child_start_ticks: u64,
    stdout_sha256: [u8; 32],
    stderr_sha256: [u8; 32],
    record_sha256: [u8; 32],
}
impl B1CaptureSettlementData {
    pub fn job_id(&self) -> uuid::Uuid {
        self.job_id
    }
    pub fn backup_id(&self) -> uuid::Uuid {
        self.backup_id
    }
    pub fn capture_start_sha256(&self) -> [u8; 32] {
        self.capture_start_sha256
    }
    pub fn captured_data_sha256(&self) -> [u8; 32] {
        self.captured_data_sha256
    }
    pub fn exec_id_bytes(&self) -> [u8; 32] {
        self.exec_id_bytes
    }
    pub fn child_start_ticks(&self) -> u64 {
        self.child_start_ticks
    }
    pub fn stdout_sha256(&self) -> [u8; 32] {
        self.stdout_sha256
    }
    pub fn stderr_sha256(&self) -> [u8; 32] {
        self.stderr_sha256
    }
    pub fn record_sha256(&self) -> [u8; 32] {
        self.record_sha256
    }
}
pub struct B1CaptureStopRequestedData {
    backup_id: uuid::Uuid,
    captured_data_sha256: [u8; 32],
}
impl B1CaptureStopRequestedData {
    pub fn backup_id(&self) -> uuid::Uuid {
        self.backup_id
    }
    pub fn captured_data_sha256(&self) -> [u8; 32] {
        self.captured_data_sha256
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

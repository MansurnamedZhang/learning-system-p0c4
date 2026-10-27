//! Default-off original-process qualification, separate from SQL/source authority.
//!
//! Canonical Root DATA, a UID or a socket cookie alone cannot construct a ctx.
//! The zero-argument factory independently joins current kernel/held originals.
//! M2b sessions and M4/M5/M6 operational producers/consumers are not substitutes
//! for these gates and have no placeholder success/Err API here.

mod job;
mod process;
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
fn observation_error(error: Error) -> super::ObservationError {
    match error {
        Error::Capacity => super::ObservationError::Capacity,
        Error::Deadline => super::ObservationError::Timeout,
        Error::Poisoned => super::ObservationError::Invalid("B1 source process permanently held"),
        _ => super::ObservationError::Invalid("B1 installed source process rejected"),
    }
}

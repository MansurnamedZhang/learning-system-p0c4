//! Private original Source front preparation, not publication S or M6.
//!
//! There is deliberately no public runner consumer. In particular the fixed
//! publication Source entry refuses before calling this factory. These internal
//! phases cannot authenticate D Ready, settle Capture normally, sign, or publish.

use crate::{BackupError, registry::ScanBudget};
use learning_assets::root_journal::SharedJournalBudget;
use learning_db::snapshot_broker::{B1CapturedInputData, B1InstalledSourceProcess};
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, AtomicU8, Ordering},
};

static ADMISSION: AdmissionOnce = AdmissionOnce::new();
static ORIGINAL: OnceLock<SourceFrontOriginal> = OnceLock::new();

fn refused() -> BackupError {
    BackupError::Invalid("B1 original Source front rejected or permanently held")
}

/// A negative local latch independent of whether Book has stored its genuine
/// peer yet. No bit, including a first entry, qualifies a process or creates S.
struct AdmissionOnce {
    entered: AtomicBool,
    rejected: AtomicBool,
}

impl AdmissionOnce {
    const fn new() -> Self {
        Self {
            entered: AtomicBool::new(false),
            rejected: AtomicBool::new(false),
        }
    }

    fn reject(&self) {
        self.rejected.store(true, Ordering::Release);
    }

    fn check(&self) -> Result<(), BackupError> {
        if self.rejected.load(Ordering::Acquire) {
            Err(refused())
        } else {
            Ok(())
        }
    }

    fn enter(&self) -> Result<(), BackupError> {
        if self.entered.swap(true, Ordering::AcqRel) {
            self.reject();
            return Err(refused());
        }
        self.check()
    }
}

/// Negative ordering only. No stage number or success bit constructs a process,
/// registry lease, Capture output, publication admission, Ready or authority.
struct FrontProgress {
    step: AtomicU8,
    failed: AtomicBool,
}

impl FrontProgress {
    const fn new() -> Self {
        Self {
            step: AtomicU8::new(0),
            failed: AtomicBool::new(false),
        }
    }

    fn reject(&self) {
        self.failed.store(true, Ordering::Release);
    }

    fn check(&self) -> Result<(), BackupError> {
        if self.failed.load(Ordering::Acquire) {
            Err(refused())
        } else {
            Ok(())
        }
    }

    fn advance(&self, before: u8, after: u8) -> Result<(), BackupError> {
        self.check()?;
        if self
            .step
            .compare_exchange(before, after, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            self.reject();
            return Err(refused());
        }
        self.check()
    }

    fn metadata_open(&self) -> Result<(), BackupError> {
        self.advance(0, 1)
    }

    fn metadata_released(&self) -> Result<(), BackupError> {
        self.advance(1, 2)
    }

    fn capture_begin(&self) -> Result<(), BackupError> {
        self.advance(2, 3)
    }

    fn captured_data_held(&self) -> Result<(), BackupError> {
        self.advance(3, 4)
    }
}

struct BorrowedPreparation<'a> {
    progress: &'a FrontProgress,
    complete: bool,
}

impl Drop for BorrowedPreparation<'_> {
    fn drop(&mut self) {
        if !self.complete {
            self.progress.reject();
        }
    }
}

type ScanWrapper = Arc<Mutex<ScanBudget>>;

/// Accounting only. This private function cannot construct SourceFrontOriginal,
/// open a registry, or grant any filesystem/process/SQL qualification.
fn preflight_scan_wrapper(scan: SharedJournalBudget) -> Result<ScanWrapper, BackupError> {
    // Reserve the fresh wrapper/Arc/tree roots before allocating them. Factory
    // and previous phases remain charged in this same cumulative backing.
    scan.charge((std::mem::size_of::<ScanBudget>() + 4096) as u64, 2)?;
    Ok(Arc::new(Mutex::new(ScanBudget::from_b1_scan(scan))))
}

/// Original storage is static and never reconstructed from returned DATA. It
/// holds the peer before subsequent fallible allocation, locks or Root IO. No
/// fields, clone, deserialization or caller-owned constructor are exposed.
pub(crate) struct SourceFrontOriginal {
    peer: B1InstalledSourceProcess,
    progress: FrontProgress,
    registry_failure: OnceLock<Arc<AtomicBool>>,
    captured: Mutex<Option<B1CapturedInputData>>,
}

/// Synchronous fixed preparation factory only. Future runtime creation must
/// follow this call, as with the existing Capture fixed entry. No runtime,
/// helper, metadata loader or registry traversal is started by a public path.
pub(super) fn admit_fixed_source_front() -> Result<&'static SourceFrontOriginal, BackupError> {
    // The local once latch is an early negative fence, not process qualification.
    if ADMISSION.enter().is_err() {
        if let Some(original) = ORIGINAL.get() {
            original.reject();
        }
        return Err(refused());
    }
    let admitted = B1InstalledSourceProcess::admit_installed().map_err(|_| {
        ADMISSION.reject();
        refused()
    })?;
    // Construction below uses fixed in-place atomics/Mutex/OnceLock only. The
    // genuine peer is retained before any subsequent fallible charge/allocation.
    let original = ORIGINAL.get_or_init(|| SourceFrontOriginal {
        peer: admitted,
        progress: FrontProgress::new(),
        registry_failure: OnceLock::new(),
        captured: Mutex::new(None),
    });
    let observed = (|| {
        original.peer.scan_accounting_meter().charge(
            (std::mem::size_of::<SourceFrontOriginal>()
                + std::mem::size_of::<AdmissionOnce>()
                + 4096) as u64,
            4,
        )?;
        // A repeat during genuine admission remains negative even if it ran
        // before ORIGINAL existed. Retention precedes this fallible check.
        ADMISSION.check()?;
        original.revalidate()?;
        Ok(original)
    })();
    if observed.is_err() {
        original.reject();
    }
    observed
}

impl SourceFrontOriginal {
    pub(crate) fn reject(&self) {
        ADMISSION.reject();
        self.progress.reject();
        if let Some(failed) = self.registry_failure.get() {
            failed.store(true, Ordering::Release);
        }
    }

    pub(crate) fn revalidate(&self) -> Result<(), BackupError> {
        let observed = (|| {
            ADMISSION.check()?;
            self.progress.check()?;
            if self
                .registry_failure
                .get()
                .is_some_and(|failed| failed.load(Ordering::Acquire))
            {
                return Err(refused());
            }
            self.peer.revalidate().map_err(|_| refused())?;
            ADMISSION.check()
        })();
        if observed.is_err() {
            self.reject();
        }
        observed
    }

    /// Only the registry's fixed metadata-preflight bridge consumes this scope.
    /// The caller cannot select a process, meter, path, mode or resettable budget.
    pub(crate) fn take_preflight_scope(
        &self,
    ) -> Result<(ScanWrapper, Arc<AtomicBool>), BackupError> {
        let observed = (|| {
            self.progress.metadata_open()?;
            self.revalidate()?;
            let scan = self.peer.scan_accounting_meter();
            scan.charge((std::mem::size_of::<AtomicBool>() + 64) as u64, 1)?;
            let failed = Arc::new(AtomicBool::new(false));
            self.registry_failure.set(failed).map_err(|_| refused())?;
            let failed = self.registry_failure.get().ok_or_else(refused)?;
            let budget = preflight_scan_wrapper(scan)?;
            Ok((budget, Arc::clone(failed)))
        })();
        if observed.is_err() {
            self.reject();
        }
        observed
    }

    /// Prepare only the original Capture's opaque input DATA. Success is not S,
    /// authenticated Ready, child settlement, CompleteBackup or publication.
    /// This borrowed future cannot move the retained peer or DATA out of Book.
    pub(super) async fn prepare_captured_input(&'static self) -> Result<(), BackupError> {
        let mut waiting = BorrowedPreparation {
            progress: &self.progress,
            complete: false,
        };
        let observed = async {
            self.revalidate()?;
            super::policy::preflight_fixed_source_metadata(self)?;
            // The facade has explicitly dropped holder, lease and old wrapper.
            self.progress.metadata_released()?;
            self.revalidate()?;
            self.progress.capture_begin()?;
            let data = self
                .peer
                .spawn_capture_and_read_data()
                .await
                .map_err(|_| refused())?;
            // Fixed Option storage was prepaid with Book. It retains the actual
            // opaque object even if revalidation fails after delivery.
            {
                let mut held = self.captured.lock().map_err(|_| refused())?;
                if held.is_some() {
                    return Err(refused());
                }
                *held = Some(data);
            }
            self.revalidate()?;
            self.progress.captured_data_held()?;
            Ok(())
        }
        .await;
        match observed {
            Ok(()) => {
                waiting.complete = true;
                Ok(())
            }
            Err(error) => {
                self.reject();
                // This is the original DB negative-close/drain path, not the
                // normal settlement request reserved for authenticated D Ready.
                // Unknown close retains this Book and every original owner.
                if self.peer.close_original().await.is_err() {
                    return Err(BackupError::Invalid(
                        "B1 original Source negative close unconfirmed",
                    ));
                }
                Err(error)
            }
        }
    }
}

/// Also available after synchronous preparation failure/cancel/unwind. This
/// does not create a replacement runtime, peer, task, normal settlement or S.
/// It never clears the original static peer or opaque DATA on any outcome.
pub(super) async fn close_fixed_source_original_negative() -> Result<(), BackupError> {
    ADMISSION.reject();
    if let Some(original) = ORIGINAL.get() {
        original.reject();
        original
            .peer
            .close_original()
            .await
            .map_err(|_| BackupError::Invalid("B1 original Source negative close unconfirmed"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{MAX_ENTRIES, MAX_SCAN_BYTES, require_fresh_b1_wrapper_same_backing};

    #[test]
    fn repeated_admission_before_book_storage_is_sticky_at_first_check() {
        // Pure gate ordering: a duplicate occurs before the original-store
        // boundary. No genuine process or Book is constructed by this probe.
        let admission = AdmissionOnce::new();
        admission.enter().unwrap();
        assert!(admission.enter().is_err());
        assert!(admission.check().is_err());
        assert!(admission.enter().is_err());
        assert!(admission.check().is_err());
    }

    #[test]
    fn repeated_admission_after_book_storage_poisons_later_checks() {
        // Pure gate ordering: a first post-store check succeeds, then a repeat
        // must poison both later checks around original revalidation.
        let admission = AdmissionOnce::new();
        admission.enter().unwrap();
        admission.check().unwrap();
        assert!(admission.enter().is_err());
        assert!(admission.check().is_err());
        admission.reject();
        assert!(admission.check().is_err());
        assert!(admission.enter().is_err());
    }

    #[test]
    fn fresh_preflight_wrapper_keeps_the_original_cumulative_meter() {
        let original = SharedJournalBudget::default();
        original.charge(113, 7).unwrap();
        let first = preflight_scan_wrapper(original.clone()).unwrap();
        first.lock().unwrap().charge(29).unwrap();
        first.lock().unwrap().entries(3).unwrap();
        let before = original.usage().unwrap();
        let fresh = preflight_scan_wrapper(original.clone()).unwrap();
        require_fresh_b1_wrapper_same_backing(&first, &fresh).unwrap();
        let after = original.usage().unwrap();
        assert!(after.charged_bytes > before.charged_bytes);
        assert!(after.charged_entries > before.charged_entries);
        assert_eq!(
            fresh.lock().unwrap().remaining_entries(),
            MAX_ENTRIES - usize::try_from(after.charged_entries).unwrap()
        );
        let before_reuse = after.charged_bytes;
        fresh.lock().unwrap().charge(19).unwrap();
        assert_eq!(original.usage().unwrap().charged_bytes, before_reuse + 19);
        let unrelated = preflight_scan_wrapper(SharedJournalBudget::default()).unwrap();
        assert!(require_fresh_b1_wrapper_same_backing(&first, &unrelated).is_err());
    }

    #[test]
    fn fresh_preflight_wrapper_cannot_recover_an_exhausted_original_meter() {
        let original = SharedJournalBudget::default();
        let first = preflight_scan_wrapper(original.clone()).unwrap();
        original.charge(MAX_SCAN_BYTES, 0).unwrap_err();
        assert!(original.usage().unwrap().exhausted);
        assert!(preflight_scan_wrapper(original.clone()).is_err());
        assert!(first.lock().unwrap().charge(1).is_err());
        assert!(original.usage().unwrap().exhausted);
    }

    #[test]
    fn progress_rejects_capture_without_completed_preflight() {
        let progress = FrontProgress::new();
        assert!(progress.capture_begin().is_err());
        assert!(progress.metadata_open().is_err());
        assert!(progress.check().is_err());
    }

    #[test]
    fn repeated_preflight_open_is_permanently_negative() {
        let progress = FrontProgress::new();
        progress.metadata_open().unwrap();
        assert!(progress.metadata_open().is_err());
        assert!(progress.metadata_released().is_err());
        assert!(progress.capture_begin().is_err());
    }

    #[test]
    fn failed_preflight_never_reaches_the_capture_phase() {
        let progress = FrontProgress::new();
        progress.metadata_open().unwrap();
        progress.reject();
        assert!(progress.metadata_released().is_err());
        assert!(progress.capture_begin().is_err());
        assert!(progress.check().is_err());
    }

    #[test]
    fn abandoned_preparation_borrow_keeps_reentry_permanently_negative() {
        let progress = FrontProgress::new();
        progress.metadata_open().unwrap();
        progress.metadata_released().unwrap();
        progress.capture_begin().unwrap();
        {
            let _waiting = BorrowedPreparation {
                progress: &progress,
                complete: false,
            };
        }
        assert!(progress.captured_data_held().is_err());
        assert!(progress.capture_begin().is_err());
        assert!(progress.metadata_open().is_err());
    }

    #[test]
    fn captured_data_phase_cannot_begin_a_second_preflight_or_capture() {
        let progress = FrontProgress::new();
        progress.metadata_open().unwrap();
        progress.metadata_released().unwrap();
        progress.capture_begin().unwrap();
        progress.captured_data_held().unwrap();
        assert!(progress.capture_begin().is_err());
        assert!(progress.metadata_open().is_err());
        assert!(progress.check().is_err());
    }
}

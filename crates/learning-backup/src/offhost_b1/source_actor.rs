//! Private original Source front and retained first-S/M6 custody.
//!
//! The candidate's fixed publication Source entry is connected to this
//! first-S/M6 implementation, with feature and installed build-pin gates kept
//! default-closed. Code and ordinary tests do not prove installed authority,
//! Ready, settlement, signing, publication or end-to-end Task9 acceptance.

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
    captured: OnceLock<B1CapturedInputData>,
    publication_once: AdmissionOnce,
    publication_failure: OnceLock<Arc<AtomicBool>>,
    first_s_clock: OnceLock<SourceFirstSClock>,
    source_operation: OnceLock<(uuid::Uuid, [u8; 32])>,
    m6: OnceLock<super::supervisor::OriginalSshOwner>,
    settlement: OnceLock<learning_db::snapshot_broker::B1CaptureSettlementData>,
    runtime: Mutex<Option<tokio::runtime::Runtime>>,
    publication: tokio::sync::Mutex<Option<crate::source::B1PublicationOwners>>,
    publication_task: tokio::sync::Mutex<Option<SourcePublicationTask>>,
    negative_cleanup_task: tokio::sync::Mutex<Option<SourcePublicationTask>>,
}

/// Synchronous original-process admission for the fixed Source entry. Runtime
/// construction follows this call, as with the Capture fixed entry. No runtime,
/// helper, metadata loader or registry traversal precedes this admission.
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
        captured: OnceLock::new(),
        publication_once: AdmissionOnce::new(),
        publication_failure: OnceLock::new(),
        first_s_clock: OnceLock::new(),
        source_operation: OnceLock::new(),
        m6: OnceLock::new(),
        settlement: OnceLock::new(),
        runtime: Mutex::new(None),
        publication: tokio::sync::Mutex::new(None),
        publication_task: tokio::sync::Mutex::new(None),
        negative_cleanup_task: tokio::sync::Mutex::new(None),
    });
    let observed = (|| {
        // Measure the unpolled concrete cleanup frame solely for the original
        // allocation reservation. It starts no work and has no runtime credit.
        let cleanup_frame = original.cleanup_negative_in_original();
        let cleanup_storage = std::mem::size_of_val(&cleanup_frame);
        drop(cleanup_frame);
        original.peer.scan_accounting_meter().charge(
            (std::mem::size_of::<SourceFrontOriginal>()
                + std::mem::size_of::<AdmissionOnce>()
                + cleanup_storage
                + 8192) as u64,
            8,
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

/// The sole fixed Source entry, reached only through default-closed feature and
/// installed build-pin gates. It admits the original process BEFORE constructing
/// its retained runtime; actual installation and end-to-end proof remain open.
pub(super) fn run_fixed_source_entry() -> Result<(), BackupError> {
    let _pins = super::BuildPins::parse(super::fixed_build_inputs(), super::Role::Source)?;
    let original = admit_fixed_source_front()?;
    original.prepay_wire_work(262144, 128)?;
    let mut runtime = original.runtime.lock().map_err(|_| refused())?;
    if runtime.is_some() {
        original.reject();
        return Err(refused());
    }
    super::supervisor::require_fixed_source_fd_room(original)?;
    *runtime = Some(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .max_blocking_threads(2)
            .enable_io()
            .enable_time()
            .build()?,
    );
    super::supervisor::recheck_source_fd_count(original)?;
    let observed = runtime.as_ref().ok_or_else(refused)?.block_on(async {
        let result = async {
            original.prepare_captured_input().await?;
            original.run_first_s_original().await
        }
        .await;
        if result.is_err() {
            original.reject();
            if original.observe_negative_cleanup().await.is_err() {
                return Err(BackupError::Invalid(
                    "original Source resources remain HOLD",
                ));
            }
        }
        result
    });
    // Runtime/actual started tasks and all Book slots remain intact on unknown.
    // Natural process loss is negative; this does not call shutdown/abort/kill.
    observed
}

impl SourceFrontOriginal {
    pub(crate) fn reject(&self) {
        ADMISSION.reject();
        self.progress.reject();
        if let Some(failed) = self.registry_failure.get() {
            failed.store(true, Ordering::Release);
        }
        if let Some(failed) = self.publication_failure.get() {
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
            if self
                .publication_failure
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
            self.captured.set(data).map_err(|_| {
                // Preserve the first actual DATA in this Book on any duplicate.
                self.reject();
                refused()
            })?;
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

/// The once-started original Source clock. No Clone/serde/caller clock or reset.
/// The independent wire clock may borrow this object's read-only deadline only.
pub(crate) struct SourceFirstSClock {
    first_deadline: std::time::Instant,
    root_deadline: OnceLock<std::time::Instant>,
    root_bound: tokio::sync::Notify,
    failed: AtomicBool,
}
impl SourceFirstSClock {
    fn start_original() -> Result<Self, BackupError> {
        let first_deadline = std::time::Instant::now()
            .checked_add(std::time::Duration::from_secs(
                super::profile::OPERATION_SECONDS,
            ))
            .ok_or(BackupError::Overflow)?;
        Ok(Self {
            first_deadline,
            root_deadline: OnceLock::new(),
            root_bound: tokio::sync::Notify::new(),
            failed: AtomicBool::new(false),
        })
    }
    fn bind_root_original(
        &self,
        input: &crate::source::B1PublicationInputs,
        budget: &ScanWrapper,
    ) -> Result<(), BackupError> {
        let deadline = input.conservative_root_deadline(budget)?;
        self.root_deadline.set(deadline).map_err(|_| refused())?;
        // One current observer; notify_one retains a permit if binding wins
        // the race before the observer registered its wait.
        self.root_bound.notify_one();
        self.check()
    }
    pub(in crate::offhost_b1) fn handshake_deadline_data(
        &self,
    ) -> Result<std::time::Instant, BackupError> {
        self.check()?;
        self.first_deadline
            .checked_sub(std::time::Duration::from_secs(
                super::profile::OPERATION_SECONDS,
            ))
            .and_then(|start| {
                start.checked_add(std::time::Duration::from_secs(
                    super::profile::HANDSHAKE_SECONDS,
                ))
            })
            .ok_or(BackupError::Overflow)
    }
    pub(crate) fn deadline(&self) -> Result<std::time::Instant, BackupError> {
        self.check()?;
        Ok(self
            .first_deadline
            .min(*self.root_deadline.get().ok_or_else(refused)?))
    }
    pub(crate) fn check(&self) -> Result<(), BackupError> {
        if self.failed.load(Ordering::Acquire)
            || std::time::Instant::now() >= self.first_deadline
            || self
                .root_deadline
                .get()
                .is_some_and(|d| std::time::Instant::now() >= *d)
        {
            self.failed.store(true, Ordering::Release);
            return Err(refused());
        }
        Ok(())
    }
}

struct SourcePublicationTask {
    task: tokio::task::JoinHandle<Result<(), BackupError>>,
    join_observed: bool,
    result_ok: Option<bool>,
}
impl SourcePublicationTask {
    async fn observe_until(&mut self, deadline: std::time::Instant) -> Result<(), BackupError> {
        if self.join_observed {
            return if self.result_ok == Some(true) {
                Ok(())
            } else {
                Err(refused())
            };
        }
        // Only borrow the actual handle. Timeout/cancel drops this observation,
        // not the actual worker or the original query/close future it owns.
        let joined =
            tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), &mut self.task)
                .await
                .map_err(|_| {
                    BackupError::Invalid("B1 original Source worker settlement unconfirmed")
                })?;
        self.join_observed = true;
        let observed = joined.map_err(|_| refused()).and_then(|outcome| outcome);
        self.result_ok = Some(observed.is_ok());
        observed
    }
}

/// Owns all scoped first-S custody except the original lease. The lease lives
/// outside this struct in the same retained spawned async frame. Field order
/// makes the borrowed Task1b holder drop before pin/protection/admission views.
/// No constructor/fields/positive bit are exposed to the fixed M6 consumer.
pub(super) struct SourceFirstS<'a> {
    metadata: Option<super::policy::SourceFirstSMetadata<'a>>,
    pin: Option<crate::source::B1PublicationPinView>,
    protection: Option<crate::ProtectionSnapshot>,
    enrolled: Option<crate::EnrolledSource>,
    admission: crate::source::B1PublicationAdmission<'a>,
    lease: &'a crate::RegistryLease,
    original: &'static SourceFrontOriginal,
}
impl<'a> SourceFirstS<'a> {
    async fn prepare(&mut self) -> Result<(), BackupError> {
        self.original.revalidate()?;
        super::supervisor::require_fixed_source_fd_room(self.original)?;
        self.enrolled = Some(
            self.lease
                .lookup_publication_source(&mut self.admission)
                .await?,
        );
        super::supervisor::recheck_source_fd_count(self.original)?;
        let enrolled = self.enrolled.as_ref().ok_or_else(refused)?;
        super::supervisor::require_fixed_source_fd_room(self.original)?;
        self.metadata = Some(super::policy::SourceFirstSMetadata::load(
            self.lease,
            &self.admission,
            enrolled,
        )?);
        super::supervisor::recheck_source_fd_count(self.original)?;
        super::supervisor::require_fixed_source_fd_room(self.original)?;
        self.lease.require_source_mutation_allowed()?;
        crate::registry::prepay_publication_protection(self.lease, enrolled)?;
        super::supervisor::require_fixed_source_fd_room(self.original)?;
        self.protection = Some(crate::discover_backup_protection(self.lease, enrolled)?);
        super::supervisor::recheck_source_fd_count(self.original)?;
        super::supervisor::require_fixed_source_fd_room(self.original)?;
        self.pin = Some(crate::source::B1PublicationPinView::open_original(
            self.admission.input()?,
            self.lease,
            enrolled,
            self.protection.as_ref().ok_or_else(refused)?,
        )?);
        super::supervisor::recheck_source_fd_count(self.original)?;
        self.recheck().await
    }
    /// The only forward path rechecks all live originals and the actual SQL
    /// connection before and after synchronous FD/protection joins.
    pub(super) async fn recheck(&mut self) -> Result<(), BackupError> {
        let observed = async {
            self.original.revalidate()?;
            super::supervisor::require_fixed_source_fd_room(self.original)?;
            self.original.first_s_clock()?.check()?;
            self.admission.revalidate_sql().await?;
            let enrolled = self.enrolled.as_ref().ok_or_else(refused)?;
            self.metadata
                .as_mut()
                .ok_or_else(refused)?
                .recheck(&self.admission, enrolled)?;
            let protection = self.protection.as_ref().ok_or_else(refused)?;
            self.pin.as_ref().ok_or_else(refused)?.recheck(
                self.admission.input()?,
                self.lease,
                enrolled,
                protection,
            )?;
            self.pin
                .as_ref()
                .ok_or_else(refused)?
                .require_postmaster(self.admission.postmaster()?)?;
            self.admission
                .require_manifest_source(self.pin.as_ref().ok_or_else(refused)?.manifest())
                .await?;
            self.admission.revalidate_sql().await?;
            self.metadata
                .as_mut()
                .ok_or_else(refused)?
                .recheck(&self.admission, enrolled)?;
            self.original.first_s_clock()?.check()?;
            super::supervisor::recheck_source_fd_count(self.original)?;
            self.original.revalidate()
        }
        .await;
        if observed.is_err() {
            self.admission.fail();
        }
        observed
    }
    pub(super) fn clock(&self) -> Result<&'static SourceFirstSClock, BackupError> {
        self.original.first_s_clock()
    }
    pub(super) fn original_owner(&self) -> &'static SourceFrontOriginal {
        self.original
    }
    pub(super) fn retain_m6_original(
        &self,
    ) -> Result<&'static super::supervisor::OriginalSshOwner, BackupError> {
        self.original.revalidate()?;
        self.original.prepay_wire_work(
            std::mem::size_of::<super::supervisor::OriginalSshOwner>() as u64,
            8,
        )?;
        Ok(self
            .original
            .m6
            .get_or_init(|| super::supervisor::OriginalSshOwner::empty(self.original)))
    }
    pub(super) async fn settle_original_capture_after_ready(
        &mut self,
        ready: &super::wire::VerifiedReadyInOriginalS,
    ) -> Result<(), BackupError> {
        ready.require_original(self)?;
        self.recheck().await?;
        if self.original.settlement.get().is_some() {
            self.reject();
            return Err(refused());
        }
        let settled = self
            .original
            .peer
            .request_capture_settlement()
            .await
            .map_err(|_| refused())?;
        let captured = self.original.captured.get().ok_or_else(refused)?;
        if settled.job_id() != captured.job_id()
            || settled.backup_id() != captured.backup_id()
            || settled.capture_start_sha256() != captured.capture_start_sha256()
            || settled.captured_data_sha256() != captured.record_sha256()
        {
            self.original
                .settlement
                .set(settled)
                .map_err(|_| refused())?;
            self.reject();
            return Err(refused());
        }
        self.original
            .settlement
            .set(settled)
            .map_err(|_| refused())?;
        self.recheck().await
    }

    pub(super) fn pin(&self) -> Result<&crate::source::B1PublicationPinView, BackupError> {
        self.original.revalidate()?;
        self.pin.as_ref().ok_or_else(refused)
    }
    pub(super) fn reject(&self) {
        self.admission.fail();
    }
    pub(super) fn lease_original(&self) -> &'a crate::RegistryLease {
        self.lease
    }
    pub(super) fn source_original(&self) -> Result<&crate::EnrolledSource, BackupError> {
        self.enrolled.as_ref().ok_or_else(refused)
    }
    pub(super) fn fence_metadata(
        &self,
    ) -> Result<super::policy::SourceFenceMetadata<'_>, BackupError> {
        self.metadata.as_ref().ok_or_else(refused)?.fence_metadata()
    }
    pub(super) fn fence_anchor_data(
        &self,
    ) -> Result<super::policy::SourceFenceAnchorData<'_>, BackupError> {
        self.metadata
            .as_ref()
            .ok_or_else(refused)?
            .fence_anchor_data()
    }
    pub(super) fn source_operation_id_data(&self) -> Result<uuid::Uuid, BackupError> {
        self.original
            .source_operation
            .get()
            .map(|v| v.0)
            .ok_or_else(refused)
    }
    pub(super) fn source_nonce_data(&self) -> Result<&[u8; 32], BackupError> {
        self.original
            .source_operation
            .get()
            .map(|v| &v.1)
            .ok_or_else(refused)
    }
    pub(super) fn current_snapshot_bytes(&self) -> Result<&[u8], BackupError> {
        Ok(self.pin()?.current_snapshot_bytes())
    }
    pub(super) fn current_snapshot_sha256_data(&self) -> Result<String, BackupError> {
        let observed = (|| {
            self.original.revalidate()?;
            self.clock()?.check()?;
            let raw = self.current_snapshot_bytes()?;
            let digest =
                prepaid_snapshot_digest_data(raw, &self.original.peer.scan_accounting_meter())?;
            self.clock()?.check()?;
            self.original.revalidate()?;
            Ok(digest)
        })();
        if observed.is_err() {
            self.reject();
        }
        observed
    }
}

/// Pure DATA hashing/accounting only. The sole product caller above passes
/// the genuine Book's same cumulative backing and cannot select/reset it.
fn prepaid_snapshot_digest_data(
    raw: &[u8],
    scan: &SharedJournalBudget,
) -> Result<String, BackupError> {
    let work = (raw.len() as u64)
        .checked_mul(2)
        .and_then(|n| n.checked_add(1024))
        .ok_or(BackupError::Overflow)?;
    scan.charge(work, 2)?;
    Ok(crate::digest(raw))
}

impl SourceFrontOriginal {
    pub(crate) fn take_publication_scope(
        &'static self,
    ) -> Result<(ScanWrapper, Arc<AtomicBool>), BackupError> {
        let observed = (|| {
            self.progress.advance(4, 5)?;
            self.revalidate()?;
            let scan = self.peer.scan_accounting_meter();
            let frame = self.run_first_s_frame();
            let frame_storage = std::mem::size_of_val(&frame);
            drop(frame); // Unpolled reservation only; no caller or work began.
            scan.charge(
                (frame_storage
                    + std::mem::size_of::<crate::source::B1PublicationOwners>()
                    + std::mem::size_of::<crate::source::B1PublicationInputs>()
                    + std::mem::size_of::<SourcePublicationTask>()
                    + std::mem::size_of::<SourceFirstSClock>()
                    + 16384) as u64,
                16,
            )?;
            self.first_s_clock
                .set(SourceFirstSClock::start_original()?)
                .map_err(|_| refused())?;
            let mut randomness = [0u8; 48];
            let mut offset = 0usize;
            while offset < randomness.len() {
                self.first_s_clock()?.check()?;
                scan.charge((randomness.len() - offset) as u64, 1)?;
                let n = unsafe {
                    libc::getrandom(
                        randomness[offset..].as_mut_ptr().cast(),
                        randomness.len() - offset,
                        0,
                    )
                };
                if n < 0 {
                    if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(refused());
                }
                if n == 0 {
                    return Err(refused());
                }
                offset = offset
                    .checked_add(usize::try_from(n).map_err(|_| refused())?)
                    .ok_or(BackupError::Overflow)?;
            }
            let mut uuid_bytes = [0u8; 16];
            uuid_bytes.copy_from_slice(&randomness[..16]);
            uuid_bytes[6] = (uuid_bytes[6] & 0x0f) | 0x40;
            uuid_bytes[8] = (uuid_bytes[8] & 0x3f) | 0x80;
            let mut nonce = [0u8; 32];
            nonce.copy_from_slice(&randomness[16..]);
            let operation = uuid::Uuid::from_bytes(uuid_bytes);
            let captured = self.captured.get().ok_or_else(refused)?;
            if operation == captured.backup_id() || operation == captured.job_id() {
                return Err(refused());
            }
            self.source_operation
                .set((operation, nonce))
                .map_err(|_| refused())?;
            let budget = preflight_scan_wrapper(scan)?;
            let failed = Arc::new(AtomicBool::new(false));
            self.publication_failure
                .set(Arc::clone(&failed))
                .map_err(|_| refused())?;
            Ok((budget, failed))
        })();
        if observed.is_err() {
            self.reject();
        }
        observed
    }
    pub(in crate::offhost_b1) fn require_negative_m6_custody(
        &self,
        owner: &super::supervisor::OriginalSshOwner,
    ) -> Result<(), BackupError> {
        if !self.m6.get().is_some_and(|held| std::ptr::eq(held, owner))
            || !self.progress.failed.load(Ordering::Acquire)
        {
            return Err(refused());
        }
        let slot = self.publication_task.try_lock().map_err(|_| refused())?;
        if slot.as_ref().is_none_or(|worker| !worker.join_observed) {
            return Err(refused());
        }
        Ok(())
    }
    pub(in crate::offhost_b1) fn require_m6_owner(
        &self,
        owner: &super::supervisor::OriginalSshOwner,
    ) -> Result<(), BackupError> {
        if !self.m6.get().is_some_and(|held| std::ptr::eq(held, owner)) {
            return Err(refused());
        }
        self.revalidate()
    }
    pub(in crate::offhost_b1) fn prepay_wire_work(
        &self,
        bytes: u64,
        entries: usize,
    ) -> Result<(), BackupError> {
        self.revalidate()?;
        let entries = match u64::try_from(entries) {
            Ok(entries) => entries,
            Err(_) => {
                self.reject();
                return Err(BackupError::Overflow);
            }
        };
        let result = self.peer.scan_accounting_meter().charge(bytes, entries);
        if result.is_err() {
            self.reject();
        }
        result.map_err(BackupError::from)
    }
    pub(crate) fn first_s_clock(&self) -> Result<&SourceFirstSClock, BackupError> {
        let clock = self.first_s_clock.get().ok_or_else(refused)?;
        if let Err(error) = clock.check() {
            self.reject();
            return Err(error);
        }
        Ok(clock)
    }
    pub(crate) fn bind_publication_root_clock(
        &self,
        input: &crate::source::B1PublicationInputs,
        budget: &ScanWrapper,
    ) -> Result<(), BackupError> {
        self.first_s_clock()?.bind_root_original(input, budget)
    }
    pub(crate) async fn acquire_publication_admin(
        &self,
    ) -> Result<learning_db::snapshot_broker::OwnedB1AdminSession, BackupError> {
        self.revalidate()?;
        self.peer.acquire_fixed_admin().await.map_err(|_| refused())
    }
    pub(crate) fn read_publication_inputs(
        &self,
        budget: &ScanWrapper,
    ) -> Result<crate::source::B1PublicationInputs, BackupError> {
        self.revalidate()?;
        crate::source::B1PublicationInputs::read_original(
            &self.peer,
            self.captured.get().ok_or_else(refused)?,
            budget,
        )
    }
    pub(crate) fn recheck_publication_inputs(
        &self,
        input: &crate::source::B1PublicationInputs,
        budget: &ScanWrapper,
    ) -> Result<(), BackupError> {
        self.revalidate()?;
        input.recheck_original(&self.peer, self.captured.get().ok_or_else(refused)?, budget)
    }
    /// Fixed private continuation of the admitted Source entry. A borrowed
    /// observer never owns or aborts the original retained spawned frame.
    pub(super) async fn run_first_s_original(&'static self) -> Result<(), BackupError> {
        if self.publication_once.enter().is_err() {
            self.reject();
            return Err(refused());
        }
        let mut waiting = BorrowedPreparation {
            progress: &self.progress,
            complete: false,
        };
        let mut owners = self.publication.lock().await;
        if owners.is_some() {
            self.reject();
            return Err(refused());
        }
        *owners = Some(crate::source::B1PublicationOwners::original_slot(self)?);
        drop(owners);
        let mut slot = self.publication_task.lock().await;
        if slot.is_some() {
            self.reject();
            return Err(refused());
        }
        let (start, started) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let mut running = BorrowedPreparation {
                progress: &self.progress,
                complete: false,
            };
            started.await.map_err(|_| refused())?;
            let observed = self.run_first_s_frame().await;
            running.complete = observed.is_ok();
            if observed.is_err() {
                self.reject();
            }
            observed
        });
        // Store the actual handle before allowing any original SQL/Root work.
        *slot = Some(SourcePublicationTask {
            task,
            join_observed: false,
            result_ok: None,
        });
        start.send(()).map_err(|_| refused())?;
        let original = slot.as_mut().ok_or_else(refused)?;
        let clock = self.first_s_clock.get().ok_or_else(refused)?;
        let observed = loop {
            let deadline = clock.first_deadline.min(
                clock
                    .root_deadline
                    .get()
                    .copied()
                    .unwrap_or(clock.first_deadline),
            );
            tokio::select! {
                joined = &mut original.task => {
                    original.join_observed = true;
                    let outcome = joined.map_err(|_| refused()).and_then(|value| value);
                    original.result_ok = Some(outcome.is_ok());
                    break outcome;
                }
                _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {
                    self.reject();
                    break Err(BackupError::Invalid("B1 original Source worker settlement unconfirmed"));
                }
                _ = clock.root_bound.notified() => continue,
            }
        };
        drop(slot);
        if observed.is_err() {
            self.reject();
            // The separate rooted cleanup task outlives this borrowed watcher.
            // It also handles a panicked worker before any peer pool audit.
            if self.observe_negative_cleanup().await.is_err() {
                return Err(BackupError::Invalid(
                    "B1 original Source cleanup unconfirmed",
                ));
            }
        }
        waiting.complete = observed.is_ok();
        observed
    }
    async fn observe_negative_cleanup(&'static self) -> Result<(), BackupError> {
        self.reject();
        let deadline = std::time::Instant::now()
            .checked_add(std::time::Duration::from_secs(
                super::profile::SETTLEMENT_SECONDS,
            ))
            .ok_or(BackupError::Overflow)?;
        let mut slot = tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            self.negative_cleanup_task.lock(),
        )
        .await
        .map_err(|_| BackupError::Invalid("B1 cleanup owner lock unconfirmed"))?;
        if slot.is_none() {
            // This one fixed cleanup frame/task/channel storage is covered by
            // the original Book allocation prepayment. Exhaustion does not
            // manufacture a new allowance or skip closing existing owners.
            let (start, started) = tokio::sync::oneshot::channel();
            let task = tokio::spawn(async move {
                started.await.map_err(|_| refused())?;
                self.cleanup_negative_in_original().await
            });
            *slot = Some(SourcePublicationTask {
                task,
                join_observed: false,
                result_ok: None,
            });
            start.send(()).map_err(|_| refused())?;
        }
        slot.as_mut()
            .ok_or_else(refused)?
            .observe_until(deadline)
            .await
    }
    async fn cleanup_negative_in_original(&'static self) -> Result<(), BackupError> {
        // This task itself is rooted in Book before starting. A live worker
        // may still hold the owners mutex/query; this same cleanup task waits
        // for its real join rather than taking/replacing/aborting that owner.
        {
            let mut slot = self.publication_task.lock().await;
            if let Some(worker) = slot.as_mut()
                && !worker.join_observed
            {
                let joined = (&mut worker.task).await;
                worker.join_observed = true;
                worker.result_ok = Some(joined.is_ok_and(|value| value.is_ok()));
            }
        }
        let publication_closed = {
            let mut slot = self.publication.lock().await;
            match slot.as_mut() {
                Some(owner) => owner.close_in_place().await,
                None => Ok(()),
            }
        };
        // Always audit the original DB owner even when local Publication close
        // is unconfirmed; no recreated session or caller cleanup connection.
        let ssh_closed = match self.m6.get() {
            Some(owner) => owner.observe_negative_after_original_worker().await,
            None => Ok(()),
        };
        let peer_closed = self.peer.close_original().await.map_err(|_| refused());
        publication_closed.and(ssh_closed).and(peer_closed)
    }

    async fn run_first_s_frame(&'static self) -> Result<(), BackupError> {
        let mut slot = self.publication.lock().await;
        let owners = slot.as_mut().ok_or_else(refused)?;
        let acquired = owners.acquire_in_place(self).await;
        if let Err(error) = acquired {
            self.reject();
            let _ = owners.close_in_place().await;
            return Err(error);
        }
        let observed = async {
            let registry;
            let observed = {
                let admission =
                    crate::source::B1PublicationAdmission::borrow_original(self, owners)?;
                registry = crate::ManagementRegistry::open_installed_publication(&admission)?;
                match registry.try_lock() {
                    Ok(lease) => {
                        let mut first = SourceFirstS {
                            metadata: None,
                            pin: None,
                            protection: None,
                            enrolled: None,
                            admission,
                            lease: &lease,
                            original: self,
                        };
                        let observed = async {
                            first.prepare().await?;
                            super::supervisor::run_source_m6_in_first_s(&mut first).await
                        }
                        .await;
                        // Holder is the first field dropped, followed by the held
                        // views and borrowed admission, before the lease/wrapper.
                        drop(first);
                        drop(lease);
                        observed
                    }
                    Err(error) => Err(error),
                }
            };
            drop(registry);
            observed
        }
        .await;
        if observed.is_err() {
            self.reject();
        }
        // Do not skip actual natural close when local qualification is false.
        let closed = owners.close_in_place().await;
        if closed.is_err() {
            self.reject();
            return Err(BackupError::Invalid("B1 Publication close unconfirmed"));
        }
        observed
    }
}

/// Also available after synchronous preparation failure/cancel/unwind. This
/// does not create a replacement runtime, peer, task, normal settlement or S.
/// It never clears the original static peer or opaque DATA on any outcome.
pub(super) async fn close_fixed_source_original_negative() -> Result<(), BackupError> {
    ADMISSION.reject();
    if let Some(original) = ORIGINAL.get() {
        original.reject();
        original.observe_negative_cleanup().await?;
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

#[cfg(test)]
mod first_s_control_tests {
    use super::*;

    #[test]
    fn original_first_s_deadline_is_unavailable_until_real_root_binding() {
        // Clock storage is not a genuine first S; only the real held Root input
        // can fill the private bound. Ordinary clock tests supply no such input.
        let clock = SourceFirstSClock::start_original().unwrap();
        assert!(clock.check().is_ok());
        assert!(clock.deadline().is_err());
        assert!(clock.root_deadline.get().is_none());
        assert!(clock.deadline().is_err());
    }

    #[test]
    fn repeat_first_s_enter_is_sticky_before_any_owner_lock() {
        let entry = AdmissionOnce::new();
        assert!(entry.enter().is_ok());
        assert!(entry.enter().is_err());
        assert!(entry.check().is_err());
        assert!(entry.enter().is_err());
    }
}

#[cfg(test)]
mod first_s_worker_owner_tests {
    use super::*;

    #[tokio::test]
    async fn bounded_join_observation_keeps_the_same_actual_worker_after_hold() {
        // An actual ordinary Tokio task/handle only, not a DB session or S.
        let (entered, entered_rx) = tokio::sync::oneshot::channel();
        let (release, release_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = entered.send(());
            release_rx.await.map_err(|_| refused())?;
            Ok(())
        });
        let id = task.id();
        let mut owner = SourcePublicationTask {
            task,
            join_observed: false,
            result_ok: None,
        };
        entered_rx.await.unwrap();
        assert!(
            owner
                .observe_until(std::time::Instant::now())
                .await
                .is_err()
        );
        assert_eq!(owner.task.id(), id);
        assert!(!owner.join_observed);
        assert!(!owner.task.is_finished());
        release.send(()).unwrap();
        owner
            .observe_until(std::time::Instant::now() + std::time::Duration::from_secs(30))
            .await
            .unwrap();
        assert_eq!(owner.task.id(), id);
        assert!(owner.join_observed);
        assert_eq!(owner.result_ok, Some(true));
    }
}

#[cfg(test)]
mod snapshot_hash_accounting_tests {
    use super::*;
    #[test]
    fn full_snapshot_hash_prepays_each_actual_raw_and_exhaustion_is_sticky() {
        let raw = vec![7u8; 300 * 1024];
        let backing = SharedJournalBudget::default();
        let expected = (raw.len() as u64) * 2 + 1024;
        let first = prepaid_snapshot_digest_data(&raw, &backing).unwrap();
        assert_eq!(backing.usage().unwrap().charged_bytes, expected);
        let second = prepaid_snapshot_digest_data(&raw, &backing).unwrap();
        assert_eq!(first, second);
        assert_eq!(backing.usage().unwrap().charged_bytes, expected * 2);
        backing
            .charge(512 * 1024 * 1024 - expected * 2 - 1, 0)
            .unwrap();
        assert!(prepaid_snapshot_digest_data(&raw, &backing).is_err());
        assert!(backing.usage().unwrap().exhausted);
        let same = backing.clone();
        assert!(prepaid_snapshot_digest_data(&[], &same).is_err());
    }
}

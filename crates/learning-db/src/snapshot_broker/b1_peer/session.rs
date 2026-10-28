//! Private original session owners. This leaf is not exported independently.
use super::{
    Error, Result,
    job::ProcessRole,
    process::{AdminSessionData, ProcessCustodyCore},
};
use sqlx::{PgConnection, PgPool, Postgres, pool::PoolConnection};
use std::sync::{
    Arc, Mutex, OnceLock, Weak,
    atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
};
use tokio::{
    runtime::Handle,
    sync::{Mutex as AsyncMutex, Notify, OwnedMutexGuard, oneshot},
    task::JoinHandle,
};

static ORIGINAL_BOOK: OnceLock<HandleBook> = OnceLock::new();

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum AdminStage {
    Install,
    Capture,
    Publication,
}
impl AdminStage {
    fn slot(self) -> SlotKey {
        match self {
            Self::Install => SlotKey::CaptureInstallAdmin,
            Self::Capture => SlotKey::CaptureFinalAdmin,
            Self::Publication => SlotKey::HostPublicationAdmin,
        }
    }
}

#[derive(Clone, Copy)]
enum SlotKey {
    CaptureInstallAdmin,
    CaptureInstallPool,
    CaptureBrokerInstallAndPools,
    CaptureQuiesce,
    CaptureFinalAdmin,
    HostPublicationAdmin,
    SourceSpawn,
    SourceSettle,
    CaptureReport,
}
impl SlotKey {
    fn index(self) -> usize {
        match self {
            Self::CaptureInstallAdmin | Self::HostPublicationAdmin => 0,
            Self::CaptureInstallPool => 1,
            Self::CaptureBrokerInstallAndPools => 2,
            Self::CaptureQuiesce => 3,
            Self::CaptureFinalAdmin => 4,
            Self::SourceSpawn | Self::CaptureReport => 5,
            Self::SourceSettle => 6,
        }
    }
    fn belongs_to(self, role: ProcessRole) -> bool {
        matches!(
            (role, self),
            (ProcessRole::Source, Self::HostPublicationAdmin)
                | (ProcessRole::Source, Self::SourceSpawn)
                | (ProcessRole::Source, Self::SourceSettle)
                | (ProcessRole::Capture, Self::CaptureReport)
                | (ProcessRole::Capture, Self::CaptureInstallAdmin)
                | (ProcessRole::Capture, Self::CaptureInstallPool)
                | (ProcessRole::Capture, Self::CaptureBrokerInstallAndPools)
                | (ProcessRole::Capture, Self::CaptureQuiesce)
                | (ProcessRole::Capture, Self::CaptureFinalAdmin)
        )
    }
    fn for_index(role: ProcessRole, index: usize) -> Option<Self> {
        match (role, index) {
            (ProcessRole::Source, 0) => Some(Self::HostPublicationAdmin),
            (ProcessRole::Source, 5) => Some(Self::SourceSpawn),
            (ProcessRole::Source, 6) => Some(Self::SourceSettle),
            (ProcessRole::Capture, 0) => Some(Self::CaptureInstallAdmin),
            (ProcessRole::Capture, 1) => Some(Self::CaptureInstallPool),
            (ProcessRole::Capture, 2) => Some(Self::CaptureBrokerInstallAndPools),
            (ProcessRole::Capture, 3) => Some(Self::CaptureQuiesce),
            (ProcessRole::Capture, 4) => Some(Self::CaptureFinalAdmin),
            (ProcessRole::Capture, 5) => Some(Self::CaptureReport),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RequestedClose {
    None,
    Normal,
    Abandon,
}
impl RequestedClose {
    fn advance(self, requested: Self) -> Self {
        match (self, requested) {
            (Self::Abandon, _) | (_, Self::Abandon) => Self::Abandon,
            (Self::Normal, _) | (_, Self::Normal) => Self::Normal,
            _ => Self::None,
        }
    }
}

struct ResourceState {
    close: RequestedClose,
    pool_close_requested: bool,
    connection_closed: bool,
    pool_closed: bool,
    delivered: bool,
}
impl ResourceState {
    fn resources_settled(&self, creation: u8, connection_created: bool, unknown: bool) -> bool {
        if unknown {
            return false;
        }
        let pools_settled = creation == 2 || creation == 3 || (creation == 1 && self.pool_closed);
        let connection_settled = if connection_created {
            self.connection_closed
        } else {
            pools_settled
        };
        connection_settled && pools_settled
    }
}

/// Contains no handle, book, full process wrapper or self-reference.
struct SessionResourceCore {
    custody: Arc<ProcessCustodyCore>,
    stage: AdminStage,
    connection: Arc<AsyncMutex<Option<PoolConnection<Postgres>>>>,
    pool: OnceLock<PgPool>,
    // 0 creator may still publish; 1 actual pool published; 2 creator proved
    // no pool was created; 3 sole partial work pool naturally closed by creator.
    // Mutex poison is never evidence for either explicit negative outcome.
    pool_creation: AtomicU8,
    state: Mutex<ResourceState>,
    close_notify: Notify,
    pool_notify: Notify,
    running: AtomicUsize,
    unknown: AtomicBool,
    binding: OnceLock<AdminSessionData>,
    identity: OnceLock<AdminIdentity>,
    connection_created: AtomicBool,
    work: OnceLock<Arc<FixedWorkPools>>,
    epoch_original: OnceLock<Arc<super::super::history::runtime::Original>>,
}
impl SessionResourceCore {
    fn new(custody: Arc<ProcessCustodyCore>, stage: AdminStage) -> Self {
        Self {
            custody,
            stage,
            connection: Arc::new(AsyncMutex::new(None)),
            pool: OnceLock::new(),
            pool_creation: AtomicU8::new(0),
            state: Mutex::new(ResourceState {
                close: RequestedClose::None,
                pool_close_requested: false,
                connection_closed: false,
                pool_closed: false,
                delivered: false,
            }),
            close_notify: Notify::new(),
            pool_notify: Notify::new(),
            running: AtomicUsize::new(0),
            unknown: AtomicBool::new(false),
            binding: OnceLock::new(),
            identity: OnceLock::new(),
            connection_created: AtomicBool::new(false),
            work: OnceLock::new(),
            epoch_original: OnceLock::new(),
        }
    }
    fn request(&self, requested: RequestedClose) {
        if requested == RequestedClose::Abandon {
            self.custody.poison();
        }
        match self.state.lock() {
            Ok(mut state) => {
                state.close = state.close.advance(requested);
                if requested == RequestedClose::Abandon {
                    state.pool_close_requested = true;
                }
            }
            Err(_) => {
                self.unknown.store(true, Ordering::Release);
                self.custody.poison();
            }
        }
        self.close_notify.notify_one();
        self.pool_notify.notify_one();
    }
    async fn wait_close_request(&self) -> Result<RequestedClose> {
        loop {
            let notified = self.close_notify.notified();
            let requested = self.state.lock().map_err(|_| Error::Poisoned)?.close;
            if requested != RequestedClose::None {
                return Ok(requested);
            }
            notified.await;
        }
    }
    fn request_pool_close(&self) {
        match self.state.lock() {
            Ok(mut state) => state.pool_close_requested = true,
            Err(_) => {
                self.unknown.store(true, Ordering::Release);
                self.custody.poison();
            }
        }
        self.pool_notify.notify_one();
    }
    fn record_pool_closed(&self) -> Result<()> {
        match self.state.lock() {
            Ok(mut state) => {
                state.pool_closed = true;
                Ok(())
            }
            Err(_) => {
                self.unknown.store(true, Ordering::Release);
                self.custody.poison();
                Err(Error::Poisoned)
            }
        }
    }
    async fn close_actual_connection(&self) -> Result<()> {
        let actual = self.connection.lock().await.take();
        match actual {
            Some(actual) => close_local_connection(self, actual).await,
            None => {
                let proved_closed = self.state.lock().is_ok_and(|state| state.connection_closed)
                    && !self.unknown.load(Ordering::Acquire);
                if proved_closed {
                    Ok(())
                } else {
                    self.unknown.store(true, Ordering::Release);
                    self.custody.poison();
                    Err(Error::Poisoned)
                }
            }
        }
    }
}

struct TaskStatus<'a> {
    resources: &'a SessionResourceCore,
    returned: bool,
}
impl<'a> TaskStatus<'a> {
    fn enter(resources: &'a SessionResourceCore) -> Self {
        resources.running.fetch_add(1, Ordering::AcqRel);
        Self {
            resources,
            returned: false,
        }
    }
    fn returned(&mut self) {
        self.returned = true;
    }
}
impl Drop for TaskStatus<'_> {
    fn drop(&mut self) {
        self.resources.running.fetch_sub(1, Ordering::AcqRel);
        if !self.returned {
            self.resources.unknown.store(true, Ordering::Release);
            self.resources.request(RequestedClose::Abandon);
        }
    }
}

struct HandleState {
    task: Option<JoinHandle<Result<()>>>,
    joined: Option<Result<()>>,
    task_returned: bool,
}
impl HandleState {
    async fn join(&mut self) -> Result<()> {
        if let Some(result) = self.joined {
            return result;
        }
        let returned = match self.task.as_mut() {
            Some(task) => task.await,
            None => return Err(Error::Poisoned),
        };
        self.task_returned = returned.is_ok();
        let result = returned.map_err(|_| Error::Io).and_then(|result| result);
        self.task.take(); // only after actual borrowed handle returned
        self.joined = Some(result);
        result
    }
}
struct SlotRecord {
    used: bool,
    resources: Option<Arc<SessionResourceCore>>,
    control: Option<Arc<ControlResourceCore>>,
    control_signal: Option<Arc<ControlSignal>>,
    settlement: Arc<AsyncMutex<HandleState>>,
}
struct ControlSignal {
    cancelled: AtomicBool,
    returned: AtomicBool,
    unknown: AtomicBool,
    published: AtomicBool,
}
impl ControlSignal {
    fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            returned: AtomicBool::new(false),
            unknown: AtomicBool::new(false),
            published: AtomicBool::new(false),
        }
    }
}
struct ControlResourceCore {
    custody: Arc<ProcessCustodyCore>,
    signal: Arc<ControlSignal>,
    registered: AtomicBool,
    deadline: std::time::Instant,
    send_deadline: std::time::Instant,
}
impl ControlResourceCore {
    fn cancel(&self) {
        // No state/qualification lock before making a blocked original poll
        // negative. Cleanup can therefore wake and join the same actual task.
        self.signal.cancelled.store(true, Ordering::Release);
        self.custody.poison();
    }
}
// Armed before channel construction or native worker registration. A worker
// that never enters still leaves the same original slot permanently negative.
struct ControlRegistrationSignal {
    signal: Arc<ControlSignal>,
    complete: bool,
}
impl Drop for ControlRegistrationSignal {
    fn drop(&mut self) {
        if !self.complete {
            self.signal.unknown.store(true, Ordering::Release);
            self.signal.cancelled.store(true, Ordering::Release);
        }
    }
}
struct ControlRegistration {
    resources: Arc<ControlResourceCore>,
    signal: ControlRegistrationSignal,
}
impl ControlRegistration {
    fn new(resources: Arc<ControlResourceCore>) -> Self {
        let signal = ControlRegistrationSignal {
            signal: resources.signal.clone(),
            complete: false,
        };
        Self { resources, signal }
    }
}
impl Drop for ControlRegistration {
    fn drop(&mut self) {
        if !self.signal.complete {
            self.resources.cancel();
        }
    }
}
struct ControlTaskStatus(Arc<ControlResourceCore>, bool);
impl ControlTaskStatus {
    fn returned(&mut self) {
        self.0.signal.returned.store(true, Ordering::Release);
        self.1 = true;
    }
}
impl Drop for ControlTaskStatus {
    fn drop(&mut self) {
        if !self.1 {
            self.0.signal.unknown.store(true, Ordering::Release);
            self.0.cancel();
        }
    }
}
struct WaitControlPublication {
    resources: Arc<ControlResourceCore>,
    complete: bool,
}
impl WaitControlPublication {
    fn complete(&mut self, book: &HandleBook) -> Result<()> {
        let slots = book.slots.lock().map_err(|_| Error::Poisoned)?;
        if slots.positive_closed
            || std::time::Instant::now() >= self.resources.deadline
            || self.resources.signal.cancelled.load(Ordering::Acquire)
            || self.resources.custody.is_failed()
            || !slots.iter().any(|s| {
                s.control
                    .as_ref()
                    .is_some_and(|r| Arc::ptr_eq(r, &self.resources))
            })
        {
            return Err(Error::Poisoned);
        }
        self.resources
            .signal
            .published
            .store(true, Ordering::Release);
        self.complete = true;
        Ok(())
    }
}
impl Drop for WaitControlPublication {
    fn drop(&mut self) {
        if !self.complete {
            self.resources.cancel();
        }
    }
}
pub(super) fn control_storage_prepayment() -> u64 {
    // 64x the largest16KiB typed record plus bounded path/record/frame staging,
    // channels, Arc/handle storage and cloning padding, before any worker spawn.
    (2 * 1024 * 1024
        + std::mem::size_of::<ControlResourceCore>()
        + std::mem::size_of::<ControlSignal>()
        + 2 * std::mem::size_of::<SlotRecord>()
        + std::mem::size_of::<JoinHandle<Result<()>>>()
        + 8192) as u64
}
// An actual short admission reservation, not session/authority DATA. The
// worker never owns this guard or the Book. It ends only after registration.
struct AdminEntryGuard {
    book: &'static HandleBook,
    custody: Arc<ProcessCustodyCore>,
    complete: bool,
}
impl AdminEntryGuard {
    fn complete(&mut self) -> Result<()> {
        let mut slots = self.book.slots.lock().map_err(|_| Error::Poisoned)?;
        if slots.positive_closed || !slots.admin_entering || self.custody.is_failed() {
            return Err(Error::Poisoned);
        }
        slots.admin_entering = false;
        self.complete = true;
        Ok(())
    }
}
impl Drop for AdminEntryGuard {
    fn drop(&mut self) {
        if !self.complete {
            // Negative close first, without attempting Core.state/revalidation.
            self.book.abandon_original(&self.custody);
            let mut slots = match self.book.slots.lock() {
                Ok(v) => v,
                Err(v) => v.into_inner(),
            };
            slots.admin_entering = false;
        }
    }
}
impl SlotRecord {
    fn unused() -> Self {
        Self {
            used: false,
            resources: None,
            control: None,
            control_signal: None,
            settlement: Arc::new(AsyncMutex::new(HandleState {
                task: None,
                joined: None,
                task_returned: false,
            })),
        }
    }
}

/// The only lifetime root for handles. Drivers never own this object.
pub(super) struct HandleBook {
    custody_identity: Weak<ProcessCustodyCore>,
    role: ProcessRole,
    scheduler: OnceLock<Handle>,
    slots: Mutex<BookState>,
}
struct BookState {
    positive_closed: bool,
    admin_entering: bool,
    slots: [SlotRecord; 7],
}
impl std::ops::Index<usize> for BookState {
    type Output = SlotRecord;
    fn index(&self, index: usize) -> &Self::Output {
        &self.slots[index]
    }
}
impl std::ops::IndexMut<usize> for BookState {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.slots[index]
    }
}
impl BookState {
    fn iter(&self) -> std::slice::Iter<'_, SlotRecord> {
        self.slots.iter()
    }
    fn control_available(&self, role: ProcessRole, key: SlotKey) -> bool {
        !self.positive_closed
            && !self.admin_entering
            && key.belongs_to(role)
            && !self[key.index()].used
            && !self.iter().any(|s| {
                s.control_signal
                    .as_ref()
                    .is_some_and(|v| !v.published.load(Ordering::Acquire))
            })
    }
    fn cancel_controls(&mut self) {
        self.positive_closed = true;
        for slot in self.iter() {
            if let Some(signal) = &slot.control_signal {
                signal.cancelled.store(true, Ordering::Release);
            }
        }
    }
    fn enter_control(&mut self, role: ProcessRole, key: SlotKey) -> Result<Arc<ControlSignal>> {
        if !self.control_available(role, key) {
            self.cancel_controls();
            return Err(Error::Phase);
        }
        let signal = Arc::new(ControlSignal::new());
        self[key.index()].used = true;
        self[key.index()].control_signal = Some(signal.clone());
        Ok(signal)
    }
}
pub(super) fn register_original_book(
    custody: &Arc<ProcessCustodyCore>,
) -> Result<&'static HandleBook> {
    // Called only after the genuine once-factory's unique finish.
    // Include all seven separately allocated Arc<AsyncMutex<HandleState>> cells,
    // with conservative per-allocation storage padding; inline book size alone
    // is not the complete reservation. This charges the original scan, no refund.
    let book_storage = std::mem::size_of::<HandleBook>()
        + 7 * (std::mem::size_of::<AsyncMutex<HandleState>>() + 1024);
    custody
        .scan()
        .charge(book_storage as u64, 7)
        .map_err(|_| Error::Capacity)?;
    let role = custody.process_role_data()?;
    let book = HandleBook {
        custody_identity: Arc::downgrade(custody),
        role,
        scheduler: OnceLock::new(),
        slots: Mutex::new(BookState {
            positive_closed: false,
            admin_entering: false,
            slots: std::array::from_fn(|_| SlotRecord::unused()),
        }),
    };
    if ORIGINAL_BOOK.set(book).is_err() {
        custody.poison();
        return Err(Error::Poisoned);
    }
    ORIGINAL_BOOK.get().ok_or(Error::Poisoned)
}
impl HandleBook {
    fn enter_admin_entry(
        &'static self,
        custody: &Arc<ProcessCustodyCore>,
    ) -> Result<AdminEntryGuard> {
        if !Weak::ptr_eq(&self.custody_identity, &Arc::downgrade(custody)) {
            return Err(Error::Identity);
        }
        custody.require_original_pid()?;
        custody
            .scan()
            .charge((std::mem::size_of::<AdminEntryGuard>() + 1024) as u64, 1)
            .map_err(|_| Error::Capacity)?;
        let mut slots = self.slots.lock().map_err(|_| Error::Poisoned)?;
        if slots.positive_closed
            || slots.admin_entering
            || custody.is_failed()
            || slots.iter().any(|s| {
                s.control_signal
                    .as_ref()
                    .is_some_and(|v| !v.published.load(Ordering::Acquire))
            })
        {
            slots.cancel_controls();
            custody.poison();
            return Err(Error::Phase);
        }
        slots.admin_entering = true;
        Ok(AdminEntryGuard {
            book: self,
            custody: custody.clone(),
            complete: false,
        })
    }
    fn reserve_control(
        &'static self,
        custody: Arc<ProcessCustodyCore>,
        key: SlotKey,
    ) -> Result<(
        Arc<ControlResourceCore>,
        OwnedMutexGuard<HandleState>,
        &'static Handle,
    )> {
        // Identity/failure checks are lock-free with respect to Core.state.
        // Full original revalidation runs inside the owned blocking worker.
        if !Weak::ptr_eq(&self.custody_identity, &Arc::downgrade(&custody))
            || !key.belongs_to(self.role)
            || !matches!(
                key,
                SlotKey::SourceSpawn | SlotKey::SourceSettle | SlotKey::CaptureReport
            )
        {
            custody.poison();
            return Err(Error::Identity);
        }
        custody.require_original_pid()?;
        if custody.is_failed() {
            return Err(Error::Poisoned);
        }
        let scheduler = self.scheduler()?;
        let mut slots = self.slots.lock().map_err(|_| Error::Poisoned)?;
        if !slots.control_available(self.role, key) {
            slots.cancel_controls();
            custody.poison();
            for slot in slots.iter() {
                if let Some(r) = &slot.control {
                    r.cancel();
                }
            }
            return Err(Error::Phase);
        }
        let admitted = (|| {
            if !matches!(key, SlotKey::SourceSettle) {
                for slot in slots.iter() {
                    if let Some(r) = &slot.resources {
                        let state = r.state.try_lock().map_err(|_| Error::Phase)?;
                        if r.running.load(Ordering::Acquire) != 0
                            || !state.resources_settled(
                                r.pool_creation.load(Ordering::Acquire),
                                r.connection_created.load(Ordering::Acquire),
                                r.unknown.load(Ordering::Acquire),
                            )
                        {
                            return Err(Error::Phase);
                        }
                        let handle = slot
                            .settlement
                            .clone()
                            .try_lock_owned()
                            .map_err(|_| Error::Phase)?;
                        if !handle.task_returned {
                            return Err(Error::Phase);
                        }
                    }
                }
            }
            // try_lock is an entry-time phase test, never a wait that permits
            // a later worker phase to excuse an early second action.
            match key {
                SlotKey::SourceSpawn => custody.try_admit_source_spawn(),
                SlotKey::SourceSettle => custody.try_admit_source_settle(),
                SlotKey::CaptureReport => custody.try_admit_capture_report(),
                _ => Err(Error::Phase),
            }
        })();
        let (deadline, send_deadline) = match admitted {
            Ok(value) => value,
            Err(error) => {
                slots.cancel_controls();
                custody.poison();
                return Err(error);
            }
        };
        let cell = slots[key.index()]
            .settlement
            .clone()
            .try_lock_owned()
            .map_err(|_| Error::Poisoned)?;
        let signal = slots.enter_control(self.role, key)?;
        let resources = Arc::new(ControlResourceCore {
            custody,
            signal,
            registered: AtomicBool::new(false),
            deadline,
            send_deadline,
        });
        slots[key.index()].control = Some(resources.clone());
        Ok((resources, cell, scheduler))
    }
    fn start_source_spawn(
        &'static self,
        custody: Arc<ProcessCustodyCore>,
    ) -> Result<(
        Arc<ControlResourceCore>,
        oneshot::Receiver<Result<super::B1CapturedInputData>>,
    )> {
        let (resources, mut cell, scheduler) =
            self.reserve_control(custody, SlotKey::SourceSpawn)?;
        let mut registration = ControlRegistration::new(resources.clone());
        let (sender, receiver) = oneshot::channel();
        let driver = resources.clone();
        cell.task = Some(scheduler.spawn_blocking(move || {
            let mut status = ControlTaskStatus(driver.clone(), false);
            let value = driver.custody.spawn_capture_and_read_data(
                &driver.signal.cancelled,
                driver.deadline,
                driver.send_deadline,
            );
            let outcome = value.as_ref().map(|_| ()).map_err(|e| *e);
            if outcome.is_err() {
                driver.cancel();
            }
            if sender.send(value).is_err() {
                driver.cancel();
            }
            status.returned();
            outcome
        }));
        resources.registered.store(true, Ordering::Release);
        registration.signal.complete = true;
        Ok((resources, receiver))
    }
    fn start_source_settle(
        &'static self,
        custody: Arc<ProcessCustodyCore>,
    ) -> Result<(
        Arc<ControlResourceCore>,
        oneshot::Receiver<Result<super::B1CaptureSettlementData>>,
    )> {
        let (resources, mut cell, scheduler) =
            self.reserve_control(custody, SlotKey::SourceSettle)?;
        let mut registration = ControlRegistration::new(resources.clone());
        let (sender, receiver) = oneshot::channel();
        let driver = resources.clone();
        cell.task = Some(scheduler.spawn_blocking(move || {
            let mut status = ControlTaskStatus(driver.clone(), false);
            let value = driver
                .custody
                .request_capture_settlement(&driver.signal.cancelled, driver.deadline);
            let outcome = value.as_ref().map(|_| ()).map_err(|e| *e);
            if outcome.is_err() {
                driver.cancel();
            }
            if sender.send(value).is_err() {
                driver.cancel();
            }
            status.returned();
            outcome
        }));
        resources.registered.store(true, Ordering::Release);
        registration.signal.complete = true;
        Ok((resources, receiver))
    }
    fn start_capture_report(
        &'static self,
        custody: Arc<ProcessCustodyCore>,
        digest: [u8; 32],
    ) -> Result<(
        Arc<ControlResourceCore>,
        oneshot::Receiver<Result<super::B1CaptureStopRequestedData>>,
    )> {
        let (resources, mut cell, scheduler) =
            self.reserve_control(custody, SlotKey::CaptureReport)?;
        let mut registration = ControlRegistration::new(resources.clone());
        let (sender, receiver) = oneshot::channel();
        let driver = resources.clone();
        cell.task = Some(scheduler.spawn_blocking(move || {
            let mut status = ControlTaskStatus(driver.clone(), false);
            let value = driver.custody.report_captured_digest_and_wait_for_settle(
                digest,
                &driver.signal.cancelled,
                driver.deadline,
                driver.send_deadline,
            );
            let outcome = value.as_ref().map(|_| ()).map_err(|e| *e);
            if outcome.is_err() {
                driver.cancel();
            }
            if sender.send(value).is_err() {
                driver.cancel();
            }
            status.returned();
            outcome
        }));
        resources.registered.store(true, Ordering::Release);
        registration.signal.complete = true;
        Ok((resources, receiver))
    }
    fn abandon_original(&self, custody: &Arc<ProcessCustodyCore>) {
        if !Weak::ptr_eq(&self.custody_identity, &Arc::downgrade(custody)) {
            return;
        }
        // Poison cannot erase the actual handles/resources. Recovering these
        // same stored references is negative cleanup only, never qualification.
        let mut slots = match self.slots.lock() {
            Ok(slots) => slots,
            Err(poisoned) => poisoned.into_inner(),
        };
        slots.positive_closed = true;
        custody.poison(); // same lock as the positive registration fence
        for slot in slots.iter().filter(|slot| slot.used) {
            if let Some(control) = &slot.control {
                control.cancel();
            }
            if let Some(resources) = &slot.resources {
                resources.request(RequestedClose::Abandon);
            }
        }
    }
    async fn drain_original(&self, custody: &Arc<ProcessCustodyCore>) -> Result<()> {
        self.abandon_original(custody);
        if !Weak::ptr_eq(&self.custody_identity, &Arc::downgrade(custody)) {
            return Err(Error::Identity);
        }
        let used = {
            let slots = match self.slots.lock() {
                Ok(slots) => slots,
                Err(poisoned) => poisoned.into_inner(),
            };
            std::array::from_fn::<_, 7, _>(|index| slots[index].used)
        };
        let mut outcome = Ok(());
        for (index, active) in used.into_iter().enumerate() {
            if active {
                let Some(key) = SlotKey::for_index(self.role, index) else {
                    outcome = Err(Error::Poisoned);
                    continue;
                };
                let joined = self.join_original(key).await;
                if outcome.is_ok() && joined.is_err() {
                    outcome = joined;
                }
            }
        }
        // Driver qualification errors remain sticky but are not themselves a
        // statement that resource cleanup failed. Inspect actual natural join
        // and resource results separately after all original handles were awaited.
        let snapshots = {
            let slots = match self.slots.lock() {
                Ok(slots) => slots,
                Err(poisoned) => poisoned.into_inner(),
            };
            std::array::from_fn::<_, 7, _>(|index| {
                (
                    slots[index].used,
                    slots[index].settlement.clone(),
                    slots[index].resources.clone(),
                    slots[index].control.clone(),
                )
            })
        };
        let mut resource_outcome = Ok(());
        for (index, (used, handle, resources, control)) in snapshots.into_iter().enumerate() {
            if !used {
                continue;
            }
            if !handle.lock().await.task_returned {
                resource_outcome = Err(Error::Poisoned);
                continue;
            }
            if let Some(control) = control {
                if SlotKey::for_index(self.role, index).is_none()
                    || !control.registered.load(Ordering::Acquire)
                    || !control.signal.returned.load(Ordering::Acquire)
                    || control.signal.unknown.load(Ordering::Acquire)
                {
                    resource_outcome = Err(Error::Poisoned);
                }
                continue;
            }
            let Some(resources) = resources else {
                resource_outcome = Err(Error::Poisoned);
                continue;
            };
            let state = resources.state.lock().map_err(|_| Error::Poisoned)?;
            let creation = resources.pool_creation.load(Ordering::Acquire);
            // Only after the creator's actual handle returned can explicit
            // NoPoolsCreated/ClosedBroker outcomes establish negative absence.
            if !state.resources_settled(
                creation,
                resources.connection_created.load(Ordering::Acquire),
                resources.unknown.load(Ordering::Acquire),
            ) {
                resource_outcome = Err(Error::Poisoned);
            }
        }
        let _qualification_outcome = outcome;
        resource_outcome
    }
    fn start_admin(
        &'static self,
        custody: Arc<ProcessCustodyCore>,
        entry: &AdminEntryGuard,
    ) -> Result<(
        Arc<SessionResourceCore>,
        oneshot::Receiver<Result<OwnedB1AdminSession>>,
    )> {
        if !std::ptr::eq(entry.book, self)
            || entry.complete
            || !Arc::ptr_eq(&entry.custody, &custody)
        {
            custody.poison();
            return Err(Error::Identity);
        }
        self.require_original(&custody)?;
        let stage = custody.next_admin_stage()?;
        let key = stage.slot();
        if !key.belongs_to(self.role) {
            return Err(Error::Phase);
        }
        custody
            .scan()
            .charge(
                (std::mem::size_of::<SessionResourceCore>()
                    + std::mem::size_of::<AsyncMutex<Option<PoolConnection<Postgres>>>>()
                    + 2 * std::mem::size_of::<SlotRecord>()
                    + 8192) as u64, // Arc headers, two oneshot channel allocations and fixed storage padding
                2,
            )
            .map_err(|_| Error::Capacity)?;
        let resources = Arc::new(SessionResourceCore::new(custody, stage));
        let (sender, receiver) = oneshot::channel();
        let (close_sender, close_result) = oneshot::channel();
        let scheduler = self.scheduler()?;
        let mut slots = self.slots.lock().map_err(|_| Error::Poisoned)?;
        if slots.positive_closed {
            resources.custody.poison();
            return Err(Error::Poisoned);
        }
        let keys = [
            Some(key),
            (stage == AdminStage::Install).then_some(SlotKey::CaptureInstallPool),
        ];
        for key in keys.into_iter().flatten() {
            if slots[key.index()].used {
                resources.custody.poison();
                return Err(Error::Phase);
            }
        }
        // All allocation and fallible locks precede spawn. These cells already
        // belong to this process-lifetime book and cannot be reused.
        let pool_cell = slots[SlotKey::CaptureInstallPool.index()]
            .settlement
            .clone();
        let admin_cell = slots[key.index()].settlement.clone();
        let mut pool_cell = if stage == AdminStage::Install {
            Some(pool_cell.try_lock().map_err(|_| Error::Poisoned)?)
        } else {
            None
        };
        let mut admin_cell = admin_cell.try_lock().map_err(|_| Error::Poisoned)?;
        for key in keys.into_iter().flatten() {
            slots[key.index()].used = true;
            slots[key.index()].resources = Some(resources.clone());
        }
        let mut spawning = SpawnRegistration {
            resources: resources.clone(),
            complete: false,
        };
        let pool_driver = drive_install_pool(resources.clone());
        let admin_driver =
            drive_install_admin(resources.clone(), sender, close_sender, close_result);
        resources
            .custody
            .scan()
            .charge(
                (std::mem::size_of_val(&pool_driver) + std::mem::size_of_val(&admin_driver) + 8192)
                    as u64,
                2,
            )
            .map_err(|_| Error::Capacity)?;
        if let Some(pool_cell) = pool_cell.as_mut() {
            pool_cell.task = Some(scheduler.spawn(pool_driver));
        }
        admin_cell.task = Some(scheduler.spawn(admin_driver));
        spawning.complete = true;
        Ok((resources, receiver))
    }
    fn require_original(&self, custody: &Arc<ProcessCustodyCore>) -> Result<()> {
        if !Weak::ptr_eq(&self.custody_identity, &Arc::downgrade(custody)) {
            return Err(Error::Identity);
        }
        {
            let slots = self.slots.lock().map_err(|_| Error::Poisoned)?;
            if slots.iter().any(|slot| {
                slot.control_signal
                    .as_ref()
                    .is_some_and(|r| !r.published.load(Ordering::Acquire))
            }) {
                custody.poison();
                for slot in slots.iter() {
                    if let Some(r) = &slot.control {
                        r.cancel();
                    }
                }
                return Err(Error::Phase);
            }
        }
        custody.revalidate()
    }
    fn scheduler(&self) -> Result<&Handle> {
        if self.scheduler.get().is_none() {
            let current = Handle::try_current().map_err(|_| Error::Phase)?;
            // No caller handle and no runtime creation. A competing first poll
            // may only keep the scheduler already stored by this original book.
            let _ = self.scheduler.set(current);
        }
        self.scheduler.get().ok_or(Error::Phase)
    }
    async fn join_original(&self, key: SlotKey) -> Result<()> {
        let (settlement, poisoned) = {
            let (slots, poisoned) = match self.slots.lock() {
                Ok(slots) => (slots, false),
                Err(poisoned) => (poisoned.into_inner(), true),
            };
            let slot = &slots[key.index()];
            if !slot.used || !key.belongs_to(self.role) {
                return Err(Error::Phase);
            }
            if poisoned && let Some(resources) = &slot.resources {
                resources.unknown.store(true, Ordering::Release);
                resources.request(RequestedClose::Abandon);
            }
            if poisoned && let Some(control) = &slot.control {
                control.signal.unknown.store(true, Ordering::Release);
                control.cancel();
            }
            (slot.settlement.clone(), poisoned)
        };
        let mut settlement = settlement.lock().await;
        let result = settlement.join().await;
        if poisoned {
            Err(Error::Poisoned)
        } else {
            result
        }
    }
}

#[cfg(test)]
mod control_resource_tests {
    use super::*;

    #[test]
    fn control_registration_signal_unwind_is_sticky_without_qualified_core() {
        // Exercise the actual signal-only member used by all three production
        // registration guards. No Core, process owner or qualified ctx exists.
        let signal = Arc::new(ControlSignal::new());
        let observed = signal.clone();
        let unwind = std::panic::catch_unwind(move || {
            let _registration = ControlRegistrationSignal {
                signal,
                complete: false,
            };
            panic!("ordinary control registration resource unwind");
        });
        assert!(unwind.is_err());
        assert!(observed.unknown.load(Ordering::Acquire));
        assert!(observed.cancelled.load(Ordering::Acquire));
        assert!(!observed.returned.load(Ordering::Acquire));
        assert!(!observed.published.load(Ordering::Acquire));
        // A later disarmed registration cannot clear an earlier negative fact.
        drop(ControlRegistrationSignal {
            signal: observed.clone(),
            complete: true,
        });
        assert!(observed.unknown.load(Ordering::Acquire));
        assert!(observed.cancelled.load(Ordering::Acquire));
        // The complete Core guard's same-original poison is statically reviewed;
        // this primitive test makes no dynamic Core/authority claim.
        let complete = Arc::new(ControlSignal::new());
        drop(ControlRegistrationSignal {
            signal: complete.clone(),
            complete: true,
        });
        assert!(!complete.unknown.load(Ordering::Acquire));
        assert!(!complete.cancelled.load(Ordering::Acquire));
    }

    #[test]
    fn admin_entry_reservation_excludes_early_control_before_worker_registration() {
        let mut slots = BookState {
            positive_closed: false,
            admin_entering: true,
            slots: std::array::from_fn(|_| SlotRecord::unused()),
        };
        assert!(!slots.control_available(ProcessRole::Source, SlotKey::SourceSpawn));
        assert!(
            slots
                .enter_control(ProcessRole::Source, SlotKey::SourceSpawn)
                .is_err()
        );
        assert!(slots.positive_closed);
        let mut capture = BookState {
            positive_closed: false,
            admin_entering: true,
            slots: std::array::from_fn(|_| SlotRecord::unused()),
        };
        assert!(
            capture
                .enter_control(ProcessRole::Capture, SlotKey::CaptureReport)
                .is_err()
        );
        assert!(capture.positive_closed);
    }

    #[test]
    fn early_second_control_slot_is_rejected_and_cancels_without_core_lock() {
        // Only real Book slot/atomic primitives. No process/core/control owner
        // or successful qualification fixture is constructed.
        let mut slots = BookState {
            positive_closed: false,
            admin_entering: false,
            slots: std::array::from_fn(|_| SlotRecord::unused()),
        };
        let first = slots
            .enter_control(ProcessRole::Source, SlotKey::SourceSpawn)
            .unwrap();
        let slots = Arc::new(Mutex::new(slots));
        let other = slots.clone();
        let early = std::thread::spawn(move || {
            other
                .lock()
                .unwrap()
                .enter_control(ProcessRole::Source, SlotKey::SourceSettle)
                .err()
        })
        .join()
        .unwrap();
        assert!(early == Some(Error::Phase));
        assert!(first.cancelled.load(Ordering::Acquire));
        let slots = slots.lock().unwrap();
        assert!(slots.positive_closed);
        assert!(!slots.control_available(ProcessRole::Source, SlotKey::SourceSettle));
    }

    #[tokio::test]
    async fn cancelled_control_borrow_keeps_actual_blocking_handle_until_natural_join() {
        let signal = Arc::new(ControlSignal::new());
        let original = signal.clone();
        let (started, started_rx) = oneshot::channel();
        let (release, release_rx) = std::sync::mpsc::sync_channel(0);
        let task = tokio::task::spawn_blocking(move || {
            let _ = started.send(());
            release_rx.recv().map_err(|_| Error::Io)?;
            if !original.cancelled.load(Ordering::Acquire) {
                return Err(Error::Shape);
            }
            original.returned.store(true, Ordering::Release);
            Err(Error::Poisoned)
        });
        let mut state = HandleState {
            task: Some(task),
            joined: None,
            task_returned: false,
        };
        {
            let joining = state.join();
            tokio::pin!(joining);
            tokio::select! {
                result = &mut joining => panic!("original task returned before barrier: {result:?}"),
                result = started_rx => assert!(result.is_ok()),
            }
            // Cancel this borrowed join only. The task still lives in the same
            // HandleState, exactly as a caller losing its result does in Book.
        }
        assert!(state.task.is_some());
        signal.cancelled.store(true, Ordering::Release);
        assert!(release.send(()).is_ok());
        assert!(state.join().await == Err(Error::Poisoned));
        assert!(state.task_returned && state.task.is_none());
        assert!(signal.returned.load(Ordering::Acquire));
        assert!(state.join().await == Err(Error::Poisoned));
    }
}

pub(super) fn abandon_original(custody: &Arc<ProcessCustodyCore>) {
    if custody.require_original_pid().is_err() {
        return;
    }
    if let Some(book) = ORIGINAL_BOOK.get() {
        book.abandon_original(custody);
    } else {
        custody.poison();
    }
}
pub(super) async fn drain_original(custody: &Arc<ProcessCustodyCore>) -> Result<()> {
    custody.require_original_pid()?;
    match ORIGINAL_BOOK.get() {
        Some(book) => book.drain_original(custody).await,
        None => {
            custody.poison();
            Err(Error::Poisoned)
        }
    }
}

struct SpawnRegistration {
    resources: Arc<SessionResourceCore>,
    complete: bool,
}
impl Drop for SpawnRegistration {
    fn drop(&mut self) {
        if !self.complete {
            // A panic during registration is unknown, not NoPoolCreated. The
            // rooted pool driver must still accept a late actual publication.
            self.resources.unknown.store(true, Ordering::Release);
            self.resources.request(RequestedClose::Abandon);
        }
    }
}

pub(super) async fn acquire_fixed_admin(
    custody: Arc<ProcessCustodyCore>,
) -> Result<OwnedB1AdminSession> {
    let book = ORIGINAL_BOOK.get().ok_or(Error::Poisoned)?;
    let mut entry = book.enter_admin_entry(&custody)?;
    book.require_original(&custody)?;
    if custody.next_admin_stage()? == AdminStage::Capture {
        // A1 cannot overlap the original Q0 publication epilogue. The actual
        // proof generation alone does not replace the stored Q0 handle join.
        book.join_original(SlotKey::CaptureQuiesce).await?;
    }
    let (resources, receiver) = book.start_admin(custody, &entry)?;
    let mut waiting = WaitPublication::new(resources);
    entry.complete()?;
    let result = receiver
        .await
        .map_err(|_| Error::Io)
        .and_then(|result| result)
        .and_then(|owned| {
            // Losing delivery closes this same wrapper/guard, never the raw
            // connection or its original retained task.
            owned.custody.revalidate()?;
            Ok(owned)
        });
    if result.is_ok() {
        waiting.complete();
    }
    result
}

pub(super) async fn spawn_capture_and_read_data(
    custody: Arc<ProcessCustodyCore>,
) -> Result<super::B1CapturedInputData> {
    let original = custody.clone();
    let outcome = async {
        let book = ORIGINAL_BOOK.get().ok_or(Error::Poisoned)?;
        let (resources, receiver) = book.start_source_spawn(custody)?;
        let mut waiting = WaitControlPublication {
            resources: resources.clone(),
            complete: false,
        };
        let received = receiver
            .await
            .map_err(|_| Error::Io)
            .and_then(|value| value);
        if received.is_err() {
            resources.cancel();
        }
        // Keep the borrowed handle inside the Book until its actual return,
        // even when result delivery/qualification failed.
        let joined = book.join_original(SlotKey::SourceSpawn).await;
        let data = received?;
        joined?;
        resources
            .custody
            .revalidate_control_delivery(resources.deadline)?;
        waiting.complete(book)?;
        Ok(data)
    }
    .await;
    if outcome.is_err() {
        original.poison();
    }
    outcome
}
pub(super) async fn request_capture_settlement(
    custody: Arc<ProcessCustodyCore>,
) -> Result<super::B1CaptureSettlementData> {
    let original = custody.clone();
    let outcome = async {
        let book = ORIGINAL_BOOK.get().ok_or(Error::Poisoned)?;
        let (resources, receiver) = book.start_source_settle(custody)?;
        let mut waiting = WaitControlPublication {
            resources: resources.clone(),
            complete: false,
        };
        let received = receiver
            .await
            .map_err(|_| Error::Io)
            .and_then(|value| value);
        if received.is_err() {
            resources.cancel();
        }
        let joined = book.join_original(SlotKey::SourceSettle).await;
        let data = received?;
        joined?;
        resources
            .custody
            .revalidate_control_delivery(resources.deadline)?;
        waiting.complete(book)?;
        Ok(data)
    }
    .await;
    if outcome.is_err() {
        original.poison();
    }
    outcome
}
pub(super) async fn report_captured_digest_and_wait_for_settle(
    custody: Arc<ProcessCustodyCore>,
    digest: [u8; 32],
) -> Result<super::B1CaptureStopRequestedData> {
    let original = custody.clone();
    let outcome = async {
        let book = ORIGINAL_BOOK.get().ok_or(Error::Poisoned)?;
        let (resources, receiver) = book.start_capture_report(custody, digest)?;
        let mut waiting = WaitControlPublication {
            resources: resources.clone(),
            complete: false,
        };
        let received = receiver
            .await
            .map_err(|_| Error::Io)
            .and_then(|value| value);
        if received.is_err() {
            resources.cancel();
        }
        let joined = book.join_original(SlotKey::CaptureReport).await;
        let data = received?;
        joined?;
        resources
            .custody
            .revalidate_control_delivery(resources.deadline)?;
        waiting.complete(book)?;
        Ok(data)
    }
    .await;
    if outcome.is_err() {
        original.poison();
    }
    outcome
}

const ADMIN_IDENTITY: &str = "SELECT session_user::text AS session_login,current_user::text AS current_login,pg_get_userbyid(d.datdba)::text AS owner,current_database()::text AS database,d.oid::bigint AS oid,(pg_catalog.pg_control_system()).system_identifier::text AS system_identifier,pg_catalog.pg_postmaster_start_time() AS postmaster,system_user AS original_login,pg_catalog.inet_server_addr()::text AS address,pg_catalog.inet_server_port() AS port FROM pg_catalog.pg_database d WHERE d.datname=current_database()";
#[derive(sqlx::FromRow)]
struct AdminIdentity {
    session_login: String,
    current_login: String,
    owner: String,
    database: String,
    oid: i64,
    system_identifier: String,
    postmaster: chrono::DateTime<chrono::Utc>,
    original_login: Option<String>,
    address: Option<String>,
    port: Option<i32>,
}
fn fixed_admin_pool(custody: &ProcessCustodyCore) -> Result<PgPool> {
    let input = custody.fixed_options_input()?;
    let options = sqlx::postgres::PgConnectOptions::new_without_pgpass()
        .host(input.directory)
        .port(5432)
        .username("learning_admin")
        .database(&input.database)
        .ssl_mode(sqlx::postgres::PgSslMode::Disable)
        .application_name(input.application_name);
    Ok(sqlx::postgres::PgPoolOptions::new()
        .min_connections(0)
        .max_connections(1)
        .connect_lazy_with(options))
}
fn fixed_broker_pool(custody: &ProcessCustodyCore) -> Result<PgPool> {
    let input = custody.fixed_options_input()?;
    let options = sqlx::postgres::PgConnectOptions::new_without_pgpass()
        .host(input.directory)
        .port(5432)
        .username(super::super::BROKER_ROLE)
        .database(&input.database)
        .ssl_mode(sqlx::postgres::PgSslMode::Disable)
        .application_name(input.application_name);
    Ok(sqlx::postgres::PgPoolOptions::new()
        .min_connections(0)
        .max_connections(4)
        .connect_lazy_with(options))
}
fn fixed_runtime_pool(custody: &ProcessCustodyCore) -> Result<PgPool> {
    let input = custody.fixed_options_input()?;
    let options = sqlx::postgres::PgConnectOptions::new_without_pgpass()
        .host(input.directory)
        .port(5432)
        .username("learning_runtime")
        .database(&input.database)
        .ssl_mode(sqlx::postgres::PgSslMode::Disable)
        .application_name(input.application_name);
    Ok(sqlx::postgres::PgPoolOptions::new()
        .min_connections(0)
        .max_connections(4)
        .connect_lazy_with(options))
}

/// Internal provenance, constructed only by the original I0 install driver
/// from its same A0/P0 resource. No caller-pool, role, budget or clock input.
pub(in crate::snapshot_broker) struct FixedWorkPools {
    custody: Arc<ProcessCustodyCore>,
    install: Arc<SessionResourceCore>,
    broker: PgPool,
    runtime: PgPool,
    owner: Weak<SessionResourceCore>,
    c11_original: OnceLock<Arc<super::process::InstalledC11Original>>,
}
enum WorkCreationError {
    NoPoolsCreated(Error),
    BrokerNaturallyClosed(Error),
}
impl FixedWorkPools {
    async fn create(
        resources: &Arc<SessionResourceCore>,
        install: Arc<SessionResourceCore>,
    ) -> std::result::Result<Self, WorkCreationError> {
        let custody = resources.custody.clone();
        custody
            .revalidate()
            .map_err(WorkCreationError::NoPoolsCreated)?;
        let broker = fixed_broker_pool(&custody).map_err(WorkCreationError::NoPoolsCreated)?;
        let runtime = match fixed_runtime_pool(&custody) {
            Ok(runtime) => runtime,
            Err(error) => {
                broker.close().await;
                return Err(WorkCreationError::BrokerNaturallyClosed(error));
            }
        };
        Ok(Self {
            custody,
            install,
            broker,
            runtime,
            owner: Arc::downgrade(resources),
            c11_original: OnceLock::new(),
        })
    }
    pub(in crate::snapshot_broker) fn retain_epoch(
        &self,
        original: Arc<super::super::history::runtime::Original>,
    ) -> super::super::history::Result<()> {
        // The same I0 task and permanent book both strongly retain this owner.
        // WorkPools stores only its Weak backlink, so no epoch/resource cycle.
        let resources = self.owner.upgrade().ok_or_else(|| {
            self.custody.poison();
            super::super::history::rejected("B1 original install owner missing")
        })?;
        resources.epoch_original.set(original).map_err(|_| {
            resources.request(RequestedClose::Abandon);
            super::super::history::rejected("B1 epoch original never replaces")
        })
    }
    pub(in crate::snapshot_broker) fn scan(
        &self,
    ) -> learning_assets::root_journal::SharedJournalBudget {
        self.custody.scan()
    }
    pub(in crate::snapshot_broker) fn admin_pool(&self) -> super::super::history::Result<PgPool> {
        self.revalidate()?;
        self.install
            .pool
            .get()
            .cloned()
            .ok_or_else(|| super::super::history::rejected("B1 original install pool missing"))
    }
    pub(in crate::snapshot_broker) async fn acquire_admin(
        &self,
    ) -> super::super::history::Result<PoolConnection<Postgres>> {
        self.revalidate()?;
        let pool = self.admin_pool()?;
        let mut connection = pool.acquire().await?;
        connection.close_on_drop();
        let checked = validate_admin_connection(&self.custody, &mut connection)
            .await
            .and_then(|actual| {
                let first = self.install.identity.get().ok_or(Error::Identity)?;
                if actual.postmaster != first.postmaster {
                    return Err(Error::Identity);
                }
                self.revalidate_inner()
            });
        if let Err(error) = checked {
            self.custody.poison();
            if connection.close().await.is_err() {
                self.mark_work_unknown();
            }
            return Err(super::observation_error(error));
        }
        Ok(connection)
    }
    pub(in crate::snapshot_broker) async fn close_install_admin_pool(
        &self,
    ) -> super::super::history::Result<()> {
        close_install_pool(&self.install)
            .await
            .map_err(super::observation_error)
    }
    pub(in crate::snapshot_broker) fn installed(
        &self,
        installed: Arc<learning_assets::InstalledSnapshotBinding>,
        descriptor: &[u8],
        observation: super::super::ObservationBudget,
    ) -> super::super::history::Result<()> {
        self.revalidate()?;
        let state = self
            .install
            .state
            .lock()
            .map_err(|_| super::observation_error(Error::Poisoned))?;
        if !state.pool_closed {
            return Err(super::observation_error(Error::Poisoned));
        }
        drop(state);
        let original = super::process::InstalledC11Original::open(
            installed,
            descriptor,
            observation,
            &self.custody,
        )
        .map_err(|error| {
            self.custody.poison();
            super::observation_error(error)
        })?;
        let original = Arc::new(original);
        self.c11_original.set(original.clone()).map_err(|_| {
            self.custody.poison();
            super::observation_error(Error::Phase)
        })?;
        self.custody
            .broker_installed(original)
            .map_err(super::observation_error)
    }
    pub(in crate::snapshot_broker) fn bind_capture_generation(
        &self,
    ) -> super::super::history::Result<()> {
        self.custody
            .bind_capture_generation()
            .map_err(super::observation_error)
    }
    async fn close_work_pools(&self) {
        // Both originals are awaited even when qualification is already HOLD.
        // This owner never waits for Q0; the only join edge is Q0 -> I0.
        self.broker.close().await;
        self.runtime.close().await;
    }
    pub(in crate::snapshot_broker) fn broker_pool(&self) -> PgPool {
        self.broker.clone()
    }
    pub(in crate::snapshot_broker) fn runtime_pool(&self) -> PgPool {
        self.runtime.clone()
    }
    pub(in crate::snapshot_broker) fn revalidate(&self) -> super::super::history::Result<()> {
        self.revalidate_inner().map_err(|error| {
            self.custody.poison();
            super::observation_error(error)
        })
    }
    pub(in crate::snapshot_broker) fn mark_work_unknown(&self) {
        self.custody.poison();
        if let Some(resources) = self.owner.upgrade() {
            resources.unknown.store(true, Ordering::Release);
            resources.request(RequestedClose::Abandon);
        }
    }
    pub(in crate::snapshot_broker) fn revalidate_process(
        &self,
    ) -> super::super::history::Result<()> {
        self.custody.revalidate().map_err(super::observation_error)
    }
    pub(in crate::snapshot_broker) fn match_source_description(
        &self,
        source: &super::super::lifecycle::BrokerSourceDescription,
    ) -> super::super::history::Result<()> {
        self.custody
            .match_source_description(source)
            .map_err(super::observation_error)
    }
    fn revalidate_inner(&self) -> Result<()> {
        self.custody.revalidate()?;
        let binding = self.install.binding.get().ok_or(Error::Identity)?;
        self.custody.current_admin_binding_matches(binding)?;
        let state = self.install.state.lock().map_err(|_| Error::Poisoned)?;
        if self.install.unknown.load(Ordering::Acquire)
            || !state.connection_closed
            || state.close != RequestedClose::Normal
        {
            return Err(Error::Poisoned);
        }
        Ok(())
    }
    pub(in crate::snapshot_broker) async fn acquire_broker(
        &self,
    ) -> super::super::history::Result<PoolConnection<Postgres>> {
        self.acquire_work(&self.broker, super::super::BROKER_ROLE)
            .await
    }
    pub(in crate::snapshot_broker) async fn acquire_runtime(
        &self,
    ) -> super::super::history::Result<PoolConnection<Postgres>> {
        self.acquire_work(&self.runtime, "learning_runtime").await
    }
    async fn acquire_work(
        &self,
        pool: &PgPool,
        fixed_role: &'static str,
    ) -> super::super::history::Result<PoolConnection<Postgres>> {
        self.revalidate()?;
        let mut connection = pool.acquire().await?;
        connection.close_on_drop();
        // From this line on, no fallible gate can release the raw connection
        // without awaiting this same owner's explicit natural close.
        let checked = async {
            self.revalidate()?;
            self.custody
                .scan()
                .charge(4096, 1)
                .map_err(|_| super::super::ObservationError::Capacity)?;
            let actual = super::super::writer::fixed_work_identity(&mut connection).await?;
            let binding =
                self.install.binding.get().ok_or_else(|| {
                    super::super::history::rejected("B1 install SQL binding missing")
                })?;
            let admin = self.install.identity.get().ok_or_else(|| {
                super::super::history::rejected("B1 original admin identity missing")
            })?;
            if actual.database != binding.database
                || u64::try_from(actual.oid).ok() != Some(binding.database_oid)
                || actual.postmaster != admin.postmaster
                || actual.session_user != fixed_role
                || actual.current_user != fixed_role
                || actual.original_login.as_deref() != Some("peer:root")
                || !actual.restricted_login
                || actual.server_address.is_some()
                || actual.server_port.is_some()
            {
                return Err(super::super::history::rejected(
                    "B1 actual fixed work session differs",
                ));
            }
            self.revalidate()
        }
        .await;
        if let Err(error) = checked {
            self.custody.poison();
            if connection.close().await.is_err() {
                self.mark_work_unknown();
            }
            return Err(error);
        }
        Ok(connection)
    }
}

/// Lives in the actual broker, then is disarmed only after I0 naturally joins.
/// Dropping it requests cleanup; it never proves cleanup or advances a phase.
pub(in crate::snapshot_broker) struct WorkLifetime {
    resources: Arc<SessionResourceCore>,
    armed: bool,
}
impl WorkLifetime {
    pub(in crate::snapshot_broker) async fn close_pools(
        &mut self,
    ) -> super::super::history::Result<()> {
        self.resources.request(RequestedClose::Normal);
        let joined = ORIGINAL_BOOK
            .get()
            .ok_or_else(|| super::observation_error(Error::Poisoned))?
            .join_original(SlotKey::CaptureBrokerInstallAndPools)
            .await
            .map_err(super::observation_error);
        if joined.is_ok() {
            self.armed = false;
        }
        joined
    }
}
impl Drop for WorkLifetime {
    fn drop(&mut self) {
        if self.armed {
            self.resources.request(RequestedClose::Abandon);
        }
    }
}

pub(in crate::snapshot_broker) async fn install_fresh_b1(
    custody: Arc<ProcessCustodyCore>,
    installed: learning_assets::InstalledSnapshotBinding,
    source: super::super::lifecycle::BrokerSourceDescription,
    sockets: (
        &std::os::unix::net::UnixListener,
        &std::os::unix::net::UnixListener,
    ),
) -> super::super::history::Result<super::super::lifecycle::RootSnapshotBroker> {
    let book = ORIGINAL_BOOK
        .get()
        .ok_or_else(|| super::observation_error(Error::Poisoned))?;
    book.require_original(&custody)
        .map_err(super::observation_error)?;
    book.join_original(SlotKey::CaptureInstallAdmin)
        .await
        .map_err(super::observation_error)?;
    custody
        .scan()
        .charge(
            (std::mem::size_of::<SessionResourceCore>() + 16_384) as u64,
            3,
        )
        .map_err(|_| super::super::ObservationError::Capacity)?;
    let owned_sockets = [sockets.0.try_clone(), sockets.1.try_clone()];
    let [left, right] = owned_sockets;
    let sockets = [
        left.map_err(|_| super::observation_error(Error::Io))?,
        right.map_err(|_| super::observation_error(Error::Io))?,
    ];
    let scheduler = book.scheduler().map_err(super::observation_error)?;
    let resources = Arc::new(SessionResourceCore::new(custody, AdminStage::Install));
    let (sender, receiver) = oneshot::channel();
    {
        let mut slots = book
            .slots
            .lock()
            .map_err(|_| super::observation_error(Error::Poisoned))?;
        let key = SlotKey::CaptureBrokerInstallAndPools;
        if slots.positive_closed || book.role != ProcessRole::Capture || slots[key.index()].used {
            return Err(super::observation_error(Error::Phase));
        }
        let install = slots[SlotKey::CaptureInstallAdmin.index()]
            .resources
            .clone()
            .ok_or_else(|| super::observation_error(Error::Phase))?;
        {
            let state = install
                .state
                .lock()
                .map_err(|_| super::observation_error(Error::Poisoned))?;
            if !state.connection_closed
                || state.close != RequestedClose::Normal
                || state.pool_closed
                || install.unknown.load(Ordering::Acquire)
            {
                return Err(super::observation_error(Error::Phase));
            }
        }
        let cell = slots[key.index()].settlement.clone();
        let mut cell = cell
            .try_lock()
            .map_err(|_| super::observation_error(Error::Poisoned))?;
        let future = drive_broker_install(
            resources.clone(),
            install,
            installed,
            source,
            sockets,
            sender,
        );
        resources
            .custody
            .scan()
            .charge((std::mem::size_of_val(&future) + 8192) as u64, 1)
            .map_err(|_| super::super::ObservationError::Capacity)?;
        slots[key.index()].used = true;
        slots[key.index()].resources = Some(resources.clone());
        let mut spawning = SpawnRegistration {
            resources: resources.clone(),
            complete: false,
        };
        cell.task = Some(scheduler.spawn(future));
        spawning.complete = true;
    }
    let mut waiting = WaitPublication::new(resources);
    let result = receiver
        .await
        .map_err(|_| super::observation_error(Error::Io))?;
    if result.is_ok() {
        waiting.complete();
    }
    result
}

async fn drive_broker_install(
    resources: Arc<SessionResourceCore>,
    install: Arc<SessionResourceCore>,
    installed: learning_assets::InstalledSnapshotBinding,
    source: super::super::lifecycle::BrokerSourceDescription,
    sockets: [std::os::unix::net::UnixListener; 2],
    sender: oneshot::Sender<
        super::super::history::Result<super::super::lifecycle::RootSnapshotBroker>,
    >,
) -> Result<()> {
    let mut status = TaskStatus::enter(&resources);
    let work = match FixedWorkPools::create(&resources, install.clone()).await {
        Ok(work) => Arc::new(work),
        Err(creation) => {
            let error = match creation {
                WorkCreationError::NoPoolsCreated(error) => {
                    fail_uncreated_pool(&resources);
                    error
                }
                WorkCreationError::BrokerNaturallyClosed(error) => {
                    // Explicit creator outcome after the only constructed
                    // work pool's actual close; not an inferred absence.
                    resources.pool_creation.store(3, Ordering::Release);
                    resources.pool_notify.notify_one();
                    error
                }
            };
            resources.request(RequestedClose::Abandon);
            let _ = close_install_pool(&install).await;
            let _ = sender.send(Err(super::observation_error(error)));
            status.returned();
            return Err(error);
        }
    };
    // Actual pools now exist in this original owner before any fallible install.
    let published = install
        .pool
        .get()
        .cloned()
        .ok_or(Error::Poisoned)
        .and_then(|pool| resources.pool.set(pool).map_err(|_| Error::Poisoned))
        .and_then(|()| {
            resources
                .work
                .set(work.clone())
                .map_err(|_| Error::Poisoned)
        });
    if published.is_ok() {
        resources.pool_creation.store(1, Ordering::Release);
    }
    resources.pool_notify.notify_one();
    let lifetime = WorkLifetime {
        resources: resources.clone(),
        armed: true,
    };
    let result = match published {
        Ok(()) => {
            super::super::lifecycle::RootSnapshotBroker::install_b1_owned(
                installed,
                work.clone(),
                source,
                sockets,
                lifetime,
            )
            .await
        }
        Err(error) => {
            drop(lifetime);
            Err(super::observation_error(error))
        }
    };
    let installed_ok = result.is_ok();
    if !installed_ok {
        let _admin_closed = close_install_pool(&install).await;
    }
    if let Err(payload) = sender.send(result) {
        drop(payload);
    }
    if !installed_ok {
        resources.request(RequestedClose::Abandon);
    }
    let requested = resources.wait_close_request().await;
    let drained = match resources.epoch_original.get() {
        Some(original) => {
            original
                .drain_b1(requested != Ok(RequestedClose::Normal))
                .await
        }
        None if !installed_ok => Ok(()), // install returned before any work task registration
        None => Err(super::super::history::rejected(
            "B1 installed epoch owner missing",
        )),
    };
    if drained.is_err() {
        resources.unknown.store(true, Ordering::Release);
        resources.custody.poison();
    }
    work.close_work_pools().await;
    // This scalar records all original pools, not an inferred missing handle.
    let pool_closed = resources.record_pool_closed();
    let qualification = requested
        .and(pool_closed)
        .and_then(|()| resources.custody.revalidate());
    status.returned();
    qualification
}

pub(in crate::snapshot_broker) async fn quiesce_b1(
    broker: super::super::lifecycle::RootSnapshotBroker,
    work: super::super::history::runtime::EpochWork,
    fixed: Arc<FixedWorkPools>,
    backup: uuid::Uuid,
    capture: uuid::Uuid,
) -> super::super::history::Result<super::super::lifecycle::QuiescedSnapshotEpochV2> {
    let book = ORIGINAL_BOOK
        .get()
        .ok_or_else(|| super::observation_error(Error::Poisoned))?;
    book.require_original(&fixed.custody)
        .map_err(super::observation_error)?;
    fixed.revalidate()?;
    fixed
        .custody
        .scan()
        .charge(
            (std::mem::size_of::<SessionResourceCore>() + 16_384) as u64,
            2,
        )
        .map_err(|_| super::super::ObservationError::Capacity)?;
    let original = work.owner();
    let slot = work.close_admission(std::time::Duration::from_secs(60))?;
    let resources = Arc::new(SessionResourceCore::new(
        fixed.custody.clone(),
        AdminStage::Capture,
    ));
    // Q0 itself constructs no pool or SQL connection. I0 remains their sole
    // owner and is joined by quiesce_owned after all original work handles.
    resources.pool_creation.store(2, Ordering::Release);
    let scheduler = book.scheduler().map_err(super::observation_error)?;
    let (sender, receiver) = oneshot::channel();
    {
        let mut slots = book
            .slots
            .lock()
            .map_err(|_| super::observation_error(Error::Poisoned))?;
        let key = SlotKey::CaptureQuiesce;
        if slots.positive_closed || book.role != ProcessRole::Capture || slots[key.index()].used {
            return Err(super::observation_error(Error::Phase));
        }
        let cell = slots[key.index()].settlement.clone();
        let mut cell = cell
            .try_lock()
            .map_err(|_| super::observation_error(Error::Poisoned))?;
        let future = drive_quiesce(
            resources.clone(),
            broker,
            work,
            original,
            slot.clone(),
            (backup, capture),
            sender,
        );
        resources
            .custody
            .scan()
            .charge((std::mem::size_of_val(&future) + 8192) as u64, 1)
            .map_err(|_| super::super::ObservationError::Capacity)?;
        slots[key.index()].used = true;
        slots[key.index()].resources = Some(resources.clone());
        let mut spawning = SpawnRegistration {
            resources: resources.clone(),
            complete: false,
        };
        cell.task = Some(scheduler.spawn(future));
        spawning.complete = true;
    }
    let mut waiting = WaitPublication::new(resources);
    let result = super::super::history::runtime::pending_original(receiver, slot).await;
    if result.is_ok() {
        waiting.complete();
    }
    result
}
async fn drive_quiesce(
    resources: Arc<SessionResourceCore>,
    broker: super::super::lifecycle::RootSnapshotBroker,
    work: super::super::history::runtime::EpochWork,
    original: Arc<super::super::history::runtime::Original>,
    slot: Arc<super::super::history::runtime::Slot>,
    identities: (uuid::Uuid, uuid::Uuid),
    sender: oneshot::Sender<
        super::super::history::Result<super::super::lifecycle::QuiescedSnapshotEpochV2>,
    >,
) -> Result<()> {
    let (backup, capture) = identities;
    let mut status = TaskStatus::enter(&resources);
    let result = broker
        .quiesce_owned(work, original, slot.clone(), backup, capture)
        .await;
    let qualified = match result {
        Ok(view) => {
            super::super::history::runtime::publish(&slot, sender, view);
            resources.custody.revalidate()
        }
        Err(error) => {
            slot.poison();
            resources.request(RequestedClose::Abandon);
            let _ = sender.send(Err(error));
            Err(Error::Poisoned)
        }
    };
    status.returned();
    qualified
}

async fn validate_admin_connection(
    custody: &ProcessCustodyCore,
    connection: &mut PoolConnection<Postgres>,
) -> Result<AdminIdentity> {
    custody.revalidate()?;
    custody
        .scan()
        .charge(ADMIN_IDENTITY.len() as u64 + 4096, 1)
        .map_err(|_| Error::Capacity)?;
    let actual: AdminIdentity = sqlx::query_as(ADMIN_IDENTITY)
        .fetch_one(&mut **connection)
        .await
        .map_err(|_| Error::Io)?;
    if actual.session_login != "learning_admin"
        || actual.current_login != "learning_admin"
        || actual.owner != "learning_admin"
        || actual.original_login.as_deref() != Some("peer:root")
        || actual.address.is_some()
        || actual.port.is_some()
    {
        return Err(Error::Identity);
    }
    custody.match_admin_database(&actual.database, actual.oid, &actual.system_identifier)?;
    // This is the first actual fixed admin's observed SQL postmaster value,
    // never caller-provided clock/identity. Later work-session comparison is
    // wired with the original install bootstrap, not fabricated from proc ticks.
    custody.revalidate()?;
    Ok(actual)
}

fn fail_uncreated_pool(resources: &SessionResourceCore) {
    // Called only when the original creator's preparation returned before
    // connect_lazy_with constructed any pool, never inferred from poison/None.
    resources.pool_creation.store(2, Ordering::Release);
    resources.request(RequestedClose::Abandon);
}
async fn close_local_connection(
    resources: &SessionResourceCore,
    connection: PoolConnection<Postgres>,
) -> Result<()> {
    match connection.close().await {
        Ok(()) => match resources.state.lock() {
            Ok(mut state) => {
                state.connection_closed = true;
                Ok(())
            }
            Err(_) => {
                resources.unknown.store(true, Ordering::Release);
                resources.custody.poison();
                Err(Error::Poisoned)
            }
        },
        Err(_) => {
            resources.unknown.store(true, Ordering::Release);
            resources.custody.poison();
            Err(Error::Io)
        }
    }
}

async fn close_install_pool(resources: &SessionResourceCore) -> Result<()> {
    resources.request_pool_close();
    if resources.stage == AdminStage::Install {
        ORIGINAL_BOOK
            .get()
            .ok_or(Error::Poisoned)?
            .join_original(SlotKey::CaptureInstallPool)
            .await
    } else {
        match resources.pool.get() {
            Some(pool) => {
                pool.close().await;
                resources.record_pool_closed()
            }
            None => Err(Error::Poisoned),
        }
    }
}

async fn drive_install_admin(
    resources: Arc<SessionResourceCore>,
    sender: oneshot::Sender<Result<OwnedB1AdminSession>>,
    close_sender: oneshot::Sender<Result<()>>,
    close_result: oneshot::Receiver<Result<()>>,
) -> Result<()> {
    let mut status = TaskStatus::enter(&resources);
    let prepared = resources
        .custody
        .prepare_admin(resources.stage)
        .and_then(|binding| fixed_admin_pool(&resources.custody).map(|pool| (binding, pool)));
    let (binding, pool) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            fail_uncreated_pool(&resources);
            let _ = close_install_pool(&resources).await;
            let _ = sender.send(Err(error));
            status.returned();
            return Err(error);
        }
    };
    if let Err(pool) = resources.pool.set(pool) {
        resources.request(RequestedClose::Abandon);
        pool.close().await;
        let _ = close_install_pool(&resources).await;
        let _ = sender.send(Err(Error::Poisoned));
        status.returned();
        return Err(Error::Poisoned);
    }
    resources.pool_creation.store(1, Ordering::Release);
    resources.pool_notify.notify_one();
    let pool = resources.pool.get().expect("original fixed pool published");
    if let Err(error) = resources.custody.revalidate() {
        resources.request(RequestedClose::Abandon);
        let _ = close_install_pool(&resources).await;
        let _ = sender.send(Err(error));
        status.returned();
        return Err(error);
    }
    let acquired = pool.acquire().await;
    let mut connection = match acquired {
        Ok(mut connection) => {
            connection.close_on_drop(); // before any recheck, SQL, return or await
            resources.connection_created.store(true, Ordering::Release);
            connection
        }
        Err(_) => {
            resources.request(RequestedClose::Abandon);
            let _ = close_install_pool(&resources).await;
            let _ = sender.send(Err(Error::Io));
            status.returned();
            return Err(Error::Io);
        }
    };
    // Keep the actual acquired value local while observing the full outcome.
    // Any fallible identity/native/generation path still reaches this close.
    let observed = validate_admin_connection(&resources.custody, &mut connection).await;
    let identity = match observed {
        Ok(identity) => identity,
        Err(error) => {
            resources.request(RequestedClose::Abandon);
            let close = close_local_connection(&resources, connection).await;
            let _ = close_install_pool(&resources).await;
            if close.is_err() {
                resources.unknown.store(true, Ordering::Release);
            }
            let _ = sender.send(Err(error));
            status.returned();
            return Err(error);
        }
    };
    let database = identity.database.clone();
    if binding.is_some_and(|binding| resources.binding.set(binding).is_err()) {
        resources.request(RequestedClose::Abandon);
        let close = close_local_connection(&resources, connection).await;
        let _ = close_install_pool(&resources).await;
        if close.is_err() {
            resources.unknown.store(true, Ordering::Release);
        }
        let _ = sender.send(Err(Error::Poisoned));
        status.returned();
        return Err(Error::Poisoned);
    }
    if resources.identity.set(identity).is_err() {
        resources.request(RequestedClose::Abandon);
        let close = close_local_connection(&resources, connection).await;
        let _ = close_install_pool(&resources).await;
        if close.is_err() {
            resources.unknown.store(true, Ordering::Release);
        }
        let _ = sender.send(Err(Error::Poisoned));
        status.returned();
        return Err(Error::Poisoned);
    }
    let mut guard = resources.connection.clone().lock_owned().await;
    *guard = Some(connection);
    if resources.custody.is_failed() {
        drop(guard);
        resources.request(RequestedClose::Abandon);
        let _ = resources.close_actual_connection().await;
        let _ = close_install_pool(&resources).await;
        let _ = sender.send(Err(Error::Poisoned));
        status.returned();
        return Err(Error::Poisoned);
    }
    let delivered = match resources.state.lock() {
        Ok(mut state) => {
            if resources.custody.is_failed() || state.close != RequestedClose::None {
                false
            } else {
                state.delivered = true;
                true
            }
        }
        Err(_) => {
            resources.unknown.store(true, Ordering::Release);
            false
        }
    };
    if !delivered {
        drop(guard);
        resources.request(RequestedClose::Abandon);
        let _ = resources.close_actual_connection().await;
        let _ = close_install_pool(&resources).await;
        let _ = sender.send(Err(Error::Poisoned));
        status.returned();
        return Err(Error::Poisoned);
    }
    let book = ORIGINAL_BOOK
        .get()
        .expect("original factory registered book");
    let owned = OwnedB1AdminSession {
        custody: resources.custody.clone(),
        resources: resources.clone(),
        book,
        slot: resources.stage.slot(),
        guard: Some(guard),
        close_result: Some(close_result),
        database,
        closing: false,
    };
    if let Err(payload) = sender.send(Ok(owned)) {
        // Drop only this wrapper/guard. The raw connection stays in resources;
        // this same original task, not payload.close(), performs the close.
        drop(payload);
    }
    let outcome = settle_admin_connection(resources.clone(), close_sender).await;
    status.returned();
    outcome
}

/// No actual connection is moved into this payload. The same original resource
/// Mutex owns it, and the original driver survives successful payload delivery.
pub(super) struct OwnedB1AdminSession {
    custody: Arc<ProcessCustodyCore>,
    resources: Arc<SessionResourceCore>,
    book: &'static HandleBook,
    slot: SlotKey,
    guard: Option<OwnedMutexGuard<Option<PoolConnection<Postgres>>>>,
    close_result: Option<oneshot::Receiver<Result<()>>>,
    database: String,
    closing: bool,
}
pub(super) struct IsolationInputData {
    job_id: uuid::Uuid,
    backup_id: uuid::Uuid,
    phase: IsolationPhase,
    generation_binding_sha256: [u8; 32],
    catalog_sha256: [u8; 32],
}
enum IsolationPhase {
    Install,
    Capture,
}
impl IsolationInputData {
    pub(super) fn job_id(&self) -> uuid::Uuid {
        self.job_id
    }
    pub(super) fn backup_id(&self) -> uuid::Uuid {
        self.backup_id
    }
    pub(super) fn phase(&self) -> &'static str {
        match self.phase {
            IsolationPhase::Install => "Install",
            IsolationPhase::Capture => "Capture",
        }
    }
    pub(super) fn generation_binding_sha256(&self) -> [u8; 32] {
        self.generation_binding_sha256
    }
    pub(super) fn catalog_sha256(&self) -> [u8; 32] {
        self.catalog_sha256
    }
}
impl OwnedB1AdminSession {
    pub(super) fn isolation_input_data(&self) -> Result<IsolationInputData> {
        let result = self.isolation_input_data_inner();
        if result.is_err() {
            self.resources.request(RequestedClose::Abandon);
        }
        result
    }
    fn isolation_input_data_inner(&self) -> Result<IsolationInputData> {
        let phase = match self.resources.stage {
            AdminStage::Install => IsolationPhase::Install,
            AdminStage::Capture => IsolationPhase::Capture,
            AdminStage::Publication => return Err(Error::Phase),
        };
        if self.closing || self.resources.unknown.load(Ordering::Acquire) {
            return Err(Error::Poisoned);
        }
        let guard = self.guard.as_ref().ok_or(Error::Phase)?;
        if guard.as_ref().is_none() {
            return Err(Error::Phase);
        }
        self.book.require_original(&self.custody)?;
        let binding = self.resources.binding.get().ok_or(Error::Phase)?;
        self.custody.current_admin_binding_matches(binding)?;
        let state = self.resources.state.lock().map_err(|_| Error::Poisoned)?;
        if !state.delivered || state.close != RequestedClose::None || state.connection_closed {
            return Err(Error::Phase);
        }
        Ok(IsolationInputData {
            job_id: binding.job_id,
            backup_id: binding.backup_id,
            phase,
            generation_binding_sha256: binding.generation_digest,
            catalog_sha256: binding.catalog_digest,
        })
    }
    pub(super) fn database(&self) -> &str {
        &self.database
    }
    pub(super) fn connection_in_original_owner(&mut self) -> &mut PgConnection {
        self.guard
            .as_mut()
            .expect("original active session guard")
            .as_mut()
            .expect("original active session connection")
    }
    pub(super) async fn close(mut self) -> Result<()> {
        let mut waiting = WaitPublication::new(self.resources.clone());
        self.closing = true;
        self.resources.request(RequestedClose::Normal);
        drop(self.guard.take());
        let observed = match self.close_result.take() {
            Some(receiver) => receiver
                .await
                .map_err(|_| Error::Io)
                .and_then(|value| value),
            None => Err(Error::Poisoned),
        };
        // Do not let an observed error skip the original handle settlement.
        let joined = self.book.join_original(self.slot).await;
        let result = observed
            .and(joined)
            .and_then(|()| self.custody.revalidate());
        if result.is_ok() {
            waiting.complete();
        }
        result
    }
}
impl Drop for OwnedB1AdminSession {
    fn drop(&mut self) {
        if !self.closing {
            self.resources.request(RequestedClose::Abandon);
        }
        // Field destruction releases only the guard; it cannot drop the actual
        // PoolConnection, which remains in the permanently rooted resource core.
    }
}

struct WaitPublication {
    resources: Arc<SessionResourceCore>,
    complete: bool,
}
impl WaitPublication {
    fn new(resources: Arc<SessionResourceCore>) -> Self {
        Self {
            resources,
            complete: false,
        }
    }
    fn complete(&mut self) {
        self.complete = true;
    }
}
impl Drop for WaitPublication {
    fn drop(&mut self) {
        if !self.complete {
            self.resources.request(RequestedClose::Abandon);
        }
    }
}

async fn drive_install_pool(resources: Arc<SessionResourceCore>) -> Result<()> {
    let mut status = TaskStatus::enter(&resources);
    loop {
        let notified = resources.pool_notify.notified();
        let (requested, poisoned) = match resources.state.lock() {
            Ok(state) => (state.pool_close_requested, false),
            Err(_) => {
                resources.unknown.store(true, Ordering::Release);
                resources.custody.poison();
                // The original creator may already be in flight. Stay rooted
                // until it actually publishes a pool or proves NoPoolCreated.
                (true, true)
            }
        };
        if requested {
            if let Some(pool) = resources.pool.get() {
                pool.close().await;
                let recorded = resources.record_pool_closed();
                status.returned();
                return if poisoned {
                    Err(Error::Poisoned)
                } else {
                    recorded
                };
            }
            if resources.pool_creation.load(Ordering::Acquire) == 2 {
                status.returned();
                return if poisoned {
                    Err(Error::Poisoned)
                } else {
                    Ok(())
                };
            }
        }
        notified.await;
    }
}

async fn settle_admin_connection(
    resources: Arc<SessionResourceCore>,
    close_sender: oneshot::Sender<Result<()>>,
) -> Result<()> {
    let requested = resources.wait_close_request().await;
    // Even a poisoned request-state read does not abandon the actual connection.
    let connection = resources.close_actual_connection().await;
    let close_pool = resources.stage != AdminStage::Install
        || requested != Ok(RequestedClose::Normal)
        || connection.is_err();
    let mut pool_outcome = if resources.pool.get().is_some() {
        Ok(())
    } else {
        resources.unknown.store(true, Ordering::Release);
        resources.custody.poison();
        Err(Error::Poisoned)
    };
    if close_pool {
        resources.request_pool_close();
        if resources.stage == AdminStage::Install {
            pool_outcome = close_install_pool(&resources).await;
        } else {
            pool_outcome = match resources.pool.get() {
                Some(pool) => {
                    pool.close().await;
                    resources.record_pool_closed()
                }
                None => Err(Error::Poisoned),
            };
        }
    }
    let outcome = requested
        .and(connection)
        .and(pool_outcome)
        .and_then(|()| resources.custody.revalidate());
    let mut state = resources.state.lock().map_err(|_| Error::Poisoned)?;
    let outcome = if state.close == RequestedClose::Abandon || resources.custody.is_failed() {
        Err(Error::Poisoned)
    } else {
        outcome
    };
    // Actual close has ended. Successful send is still only result delivery;
    // caller cancellation can permanently close later publication, never revive it.
    if close_sender.send(outcome).is_err() {
        state.close = RequestedClose::Abandon;
        state.pool_close_requested = true;
        resources.custody.poison();
        resources.pool_notify.notify_one();
        return Err(Error::Poisoned);
    }
    outcome
}

#[cfg(test)]
mod resource_tests {
    use super::*;

    #[test]
    fn abandoning_before_or_after_normal_close_never_reopens() {
        assert!(RequestedClose::None.advance(RequestedClose::Abandon) == RequestedClose::Abandon);
        assert!(RequestedClose::Normal.advance(RequestedClose::None) == RequestedClose::Normal);
        let abandoned = RequestedClose::Normal.advance(RequestedClose::Abandon);
        assert!(abandoned.advance(RequestedClose::Normal) == RequestedClose::Abandon);
        assert!(abandoned.advance(RequestedClose::None) == RequestedClose::Abandon);
    }

    #[test]
    fn absent_resource_is_not_a_natural_close_receipt() {
        let mut state = ResourceState {
            close: RequestedClose::Abandon,
            pool_close_requested: true,
            connection_closed: false,
            pool_closed: false,
            delivered: false,
        };
        // Pending/missing creation evidence is never a known empty pool.
        assert!(!state.resources_settled(0, false, false));
        assert!(!state.resources_settled(1, false, false));
        assert!(!state.resources_settled(1, true, false));
        state.pool_closed = true;
        assert!(!state.resources_settled(1, true, false));
        state.connection_closed = true;
        assert!(state.resources_settled(1, true, false));
        assert!(!state.resources_settled(1, true, true));
        // These are resource facts only; Abandon remains terminal and there is
        // no process/session/generation construction anywhere in this test.
        assert!(state.close == RequestedClose::Abandon);
    }

    #[tokio::test]
    async fn cancelling_borrowed_join_keeps_the_same_original_task() {
        let (started, started_rx) = oneshot::channel();
        let (release, release_rx) = oneshot::channel();
        let actual_returns = Arc::new(AtomicUsize::new(0));
        let observed = actual_returns.clone();
        let task = tokio::spawn(async move {
            let _ = started.send(());
            release_rx.await.map_err(|_| Error::Io)?;
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        let mut state = HandleState {
            task: Some(task),
            joined: None,
            task_returned: false,
        };
        {
            let joining = state.join();
            tokio::pin!(joining);
            tokio::select! {
                _ = &mut joining => panic!("original cannot finish before release"),
                result = started_rx => { assert!(result.is_ok()); }
            }
            // Only this borrowed await is cancelled. Its stored original task
            // remains owned; no abort, detached task or substitute is created.
        }
        assert!(state.task.is_some());
        assert!(!state.task_returned);
        assert_eq!(actual_returns.load(Ordering::SeqCst), 0);
        assert!(release.send(()).is_ok());
        assert!(state.join().await.is_ok());
        assert!(state.task.is_none());
        assert!(state.task_returned);
        assert_eq!(actual_returns.load(Ordering::SeqCst), 1);
        assert!(state.join().await.is_ok());
        assert_eq!(actual_returns.load(Ordering::SeqCst), 1);
        // Exercise the actual worker drain receipt independently of any
        // process, journal or session. Its handle is removed on first join,
        // but a second negative drain must retain the original panic/unknown.
        let failed: JoinHandle<()> =
            tokio::spawn(async { panic!("ordinary resource-test worker panic") });
        let retained = Arc::new(AsyncMutex::new(Some(failed)));
        let unknown = AtomicBool::new(false);
        assert!(
            super::super::super::history::runtime::join_worker_original(&retained, &unknown)
                .await
                .is_err()
        );
        assert!(retained.lock().await.is_none());
        assert!(unknown.load(Ordering::SeqCst));
        assert!(
            super::super::super::history::runtime::join_worker_original(&retained, &unknown)
                .await
                .is_err()
        );
    }
}

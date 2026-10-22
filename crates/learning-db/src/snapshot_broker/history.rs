//! Private P1 records and original-operation supervision. No public factory,
//! JobLease-to-authority conversion, disk adoption or effect classification.
use super::ObservationError;
use crate::snapshot_observation::{JobObservation, OutboxObservation, SnapshotObservation};
use chrono::{DateTime, Utc};
use learning_assets::root_journal::{ByteJournalHead, MAX_FRAME_BYTES, SharedJournalBudget};
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use uuid::Uuid;

pub(super) type Result<T> = std::result::Result<T, ObservationError>;
pub(super) const LEASE_MS: i64 = 300_000;
pub(super) fn rejected(reason: &'static str) -> ObservationError {
    ObservationError::Invalid(reason)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Rows {
    pub(super) job: JobObservation,
    pub(super) outbox: OutboxObservation,
    pub(super) observed_at: DateTime<Utc>,
}
impl From<SnapshotObservation> for Rows {
    fn from(value: SnapshotObservation) -> Self {
        Self {
            job: value.job,
            outbox: value.outbox,
            observed_at: value.observed_at,
        }
    }
}
#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct DatabaseIdentity {
    pub(super) database: String,
    pub(super) oid: i64,
    pub(super) postmaster: DateTime<Utc>,
    pub(super) session_user: String,
    pub(super) current_user: String,
    pub(super) backend: i32,
    pub(super) transaction: String,
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct RecordRef {
    sequence: u64,
    sha256: [u8; 32],
}
impl From<ByteJournalHead> for RecordRef {
    fn from(head: ByteJournalHead) -> Self {
        Self {
            sequence: head.sequence,
            sha256: head.sha256,
        }
    }
}
impl RecordRef {
    pub(super) fn head(self) -> ByteJournalHead {
        ByteJournalHead {
            sequence: self.sequence,
            sha256: self.sha256,
        }
    }
}
#[cfg(test)]
impl RecordRef {
    pub(super) fn sequence(&self) -> u64 {
        self.sequence
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Intent {
    pub(super) schema: IntentSchema,
    #[serde(deserialize_with = "version_one")]
    pub(super) version: u8,
    pub(super) epoch: Uuid,
    pub(super) binding: Uuid,
    pub(super) request: Uuid,
    pub(super) job: Uuid,
    pub(super) previous: RecordRef,
    pub(super) profile_sha256: String,
    pub(super) source_build: SourceProvenance,
    pub(super) identity: DatabaseIdentity,
    pub(super) branch: Branch,
    #[serde(deserialize_with = "p1_lease")]
    pub(super) lease_ms: i64,
    pub(super) before: Rows,
}
#[derive(Serialize, Deserialize, sqlx::FromRow)]
#[serde(deny_unknown_fields)]
pub(super) struct Claim {
    pub(super) job_id: Uuid,
    pub(super) token: Uuid,
    pub(super) attempts: i32,
    pub(super) expires_at: DateTime<Utc>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Outcome {
    pub(super) schema: OutcomeSchema,
    #[serde(deserialize_with = "version_one")]
    pub(super) version: u8,
    pub(super) intent: RecordRef,
    pub(super) after: Rows,
    pub(super) claim: Claim,
    pub(super) sql_before: DateTime<Utc>,
    pub(super) sql_after: DateTime<Utc>,
    pub(super) identity: DatabaseIdentity,
    pub(super) commit: CommitState,
}
#[derive(Serialize, Deserialize)]
pub(super) enum IntentSchema {
    #[serde(rename = "knowweave.snapshot-claim.p1.intent")]
    V1,
}
#[derive(Serialize, Deserialize)]
pub(super) enum OutcomeSchema {
    #[serde(rename = "knowweave.snapshot-claim.p1.committed")]
    V1,
    #[serde(rename = "knowweave.snapshot-claim.v2.committed")]
    V2,
}
#[derive(Serialize, Deserialize)]
pub(super) enum Branch {
    #[serde(rename = "FRESH_QUEUED_0_TO_SOME_1")]
    Fresh,
}
#[derive(Serialize, Deserialize)]
pub(super) enum CommitState {
    #[serde(rename = "ORIGINAL_COMMIT_KNOWN")]
    OriginalKnown,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) enum SourceMode {
    #[serde(rename = "UNENROLLED_COMPONENT_EXTERNAL_RUN_ONLY")]
    UnenrolledComponent,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SourceProvenance {
    pub(super) mode: SourceMode,
    #[serde(deserialize_with = "true_only")]
    pub(super) external_reference_not_authority: bool,
    #[serde(deserialize_with = "binding_reference")]
    pub(super) execution_binding_sha256: String,
}
fn valid_reference(v: &str) -> bool {
    v.len() == 64
        && v.as_bytes()
            .iter()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
        && v.as_bytes().iter().any(|c| *c != b'0')
}
fn true_only<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<bool, D::Error> {
    if !bool::deserialize(d)? {
        return Err(serde::de::Error::custom("P1 reference is not authority"));
    }
    Ok(true)
}
fn binding_reference<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<String, D::Error> {
    let v = String::deserialize(d)?;
    if !valid_reference(&v) {
        return Err(serde::de::Error::custom("P1 external reference"));
    }
    Ok(v)
}
fn version_one<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<u8, D::Error> {
    let v = u8::deserialize(d)?;
    if v != 1 {
        return Err(serde::de::Error::custom("P1 version"));
    }
    Ok(v)
}
fn p1_lease<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<i64, D::Error> {
    let v = i64::deserialize(d)?;
    if v != LEASE_MS {
        return Err(serde::de::Error::custom("P1 lease"));
    }
    Ok(v)
}

// Count and charge actual escaped UTF-8 before allocating an encoded record.
struct Counter<'a> {
    bytes: usize,
    budget: &'a SharedJournalBudget,
}
// The serializer is generic: a second Serialize invocation may differ. Never
// hand it the growable Vec directly, even after prepaying the counted size.
struct BoundedWriter {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self
            .bytes
            .len()
            .checked_add(bytes.len())
            .is_none_or(|n| n > self.limit)
        {
            return Err(io::Error::other(
                "P1 second serialization exceeded prepayment",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Write for Counter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let next = self
            .bytes
            .checked_add(bytes.len())
            .filter(|n| *n <= MAX_FRAME_BYTES - 120)
            .ok_or_else(|| io::Error::other("P1 record capacity"))?;
        self.budget.charge(bytes.len() as u64, 0)?;
        self.bytes = next;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub(super) fn encode<T: Serialize>(record: &T, budget: &SharedJournalBudget) -> Result<Vec<u8>> {
    let mut count = Counter { bytes: 0, budget };
    serde_json::to_writer(&mut count, record).map_err(|_| ObservationError::Capacity)?;
    budget
        .charge(count.bytes as u64, 0)
        .map_err(|_| ObservationError::Capacity)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(count.bytes)
        .map_err(|_| ObservationError::Capacity)?;
    let mut writer = BoundedWriter {
        bytes,
        limit: count.bytes,
    };
    serde_json::to_writer(&mut writer, record).map_err(|_| ObservationError::Capacity)?;
    if writer.bytes.len() != count.bytes {
        return Err(rejected("P1 encoding length changed"));
    }
    Ok(writer.bytes)
}
pub(super) fn digest(bytes: &[u8], budget: &SharedJournalBudget) -> Result<String> {
    budget
        .charge(bytes.len() as u64, 0)
        .map_err(|_| ObservationError::Capacity)?;
    Ok(learning_core::hex_digest(bytes))
}

#[cfg(target_os = "linux")]
pub(super) mod runtime {
    use super::super::transaction::ObservationBudget;
    use super::super::transition::DurableTransition;
    use super::*;
    use learning_assets::root_journal::{OpaqueJournalBody, RootJournalBytes};
    use sqlx::PgPool;
    use std::{
        collections::BTreeMap,
        fs::File,
        future::Future,
        pin::Pin,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        },
        task::{Context, Poll},
        time::Duration,
    };
    use tokio::{
        sync::{Mutex as AsyncMutex, oneshot},
        task::JoinHandle,
        time::{Instant, Sleep, sleep_until},
    };

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(in super::super) enum Stage {
        BeforeIntent,
        AfterIntent,
        BeforeClaim,
        AfterClaim,
        BeforeCommit,
        AfterCommit,
        BeforeOutcome,
        AfterOutcome,
        BeforeIssue,
        AfterIssue,
        BeforeStageSpawn,
        AfterStageCopy,
        AfterStageError,
        BeforePublish,
        BeforeBusinessCommit,
        AfterBusinessCommit,
        LoseBusinessCommitKnowledge,
        BeforeBusinessEvent,
        AfterBusinessEvent,
        BeforeCloseEvent,
        AfterCloseEvent,
        AfterQuiesceAppend,
    }
    #[cfg(test)]
    pub(in super::super) struct Hooks {
        pub(in super::super) stage: Stage,
        pub(in super::super) arrived: tokio::sync::Notify,
        pub(in super::super) resume: tokio::sync::Notify,
        pub(in super::super) lose_commit_knowledge: bool,
        pub(in super::super) blocking_return: bool,
        pub(in super::super) blocking_release: (Mutex<bool>, std::sync::Condvar),
        pub(in super::super) post_spawn_join: bool,
        pub(in super::super) spawned: tokio::sync::Notify,
        pub(in super::super) join_resume: tokio::sync::Notify,
    }
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum State {
        Live,
        Published,
        Hold,
    }
    pub(in super::super) struct Slot {
        state: Mutex<State>,
        pub(in super::super) deadline: Instant,
        epoch_hold: Arc<AtomicBool>,
        issued: AtomicBool,
        revocation_requested: AtomicBool,
    }
    impl Slot {
        // The same short state gate used by cancellation/publication. This bit
        // is permanent, including if the original append never becomes known.
        pub(in super::super) fn issue_sticky(&self) -> Result<()> {
            let mut state = self
                .state
                .lock()
                .map_err(|_| rejected("C2 gate poisoned"))?;
            if *state == State::Live && self.revocation_requested.load(Ordering::SeqCst) {
                return Err(ObservationError::RevokedBeforeWrite);
            }
            if *state != State::Live
                || self.epoch_hold.load(Ordering::SeqCst)
                || Instant::now() >= self.deadline
                || self.issued.swap(true, Ordering::SeqCst)
            {
                *state = State::Hold;
                self.epoch_hold.store(true, Ordering::SeqCst);
                return Err(rejected("C2 issue fence closed"));
            }
            Ok(())
        }
        pub(in super::super) fn poison(&self) {
            self.epoch_hold.store(true, Ordering::SeqCst);
            if let Ok(mut state) = self.state.lock() {
                *state = State::Hold;
            }
        }
        pub(in super::super) fn live(&self) -> Result<()> {
            if Instant::now() >= self.deadline || self.epoch_hold.load(Ordering::SeqCst) {
                self.poison();
                return Err(rejected("P1 original operation HOLD"));
            }
            match self.state.lock() {
                Ok(state) if *state == State::Live => Ok(()),
                _ => Err(rejected("P1 original operation not live")),
            }
        }
        pub(in super::super) fn reserve_revocation(&self) -> Result<()> {
            let state = self
                .state
                .lock()
                .map_err(|_| rejected("revocation gate poisoned"))?;
            if *state != State::Live || self.issued.load(Ordering::SeqCst) {
                return Err(rejected("issued claim cannot be revoked"));
            }
            self.revocation_requested.store(true, Ordering::SeqCst);
            Ok(())
        }
        pub(in super::super) fn issued(&self) -> bool {
            self.issued.load(Ordering::SeqCst)
        }
    }
    struct Registry {
        jobs: BTreeMap<Uuid, Arc<Slot>>,
        requests: BTreeMap<Uuid, Uuid>,
        active: bool,
        closing: bool,
    }
    type SettlementHandle = Arc<AsyncMutex<Option<JoinHandle<()>>>>;
    pub(in super::super) struct Original {
        pub(in super::super) pool: PgPool,
        pub(in super::super) budget: SharedJournalBudget,
        pub(in super::super) observation: ObservationBudget,
        journal: Arc<AsyncMutex<RootJournalBytes>>,
        registry: Mutex<Registry>,
        handles: Mutex<Vec<SettlementHandle>>,
        control_handle: Mutex<Option<SettlementHandle>>,
        hold: Arc<AtomicBool>,
        pub(in super::super) epoch: Uuid,
        pub(in super::super) binding: Uuid,
        pub(in super::super) source_build: SourceProvenance,
        pub(in super::super) sql_calls: AtomicUsize,
        append_calls: AtomicUsize,
        stored_bytes: AtomicU64,
        // This original FD remains owned until the original operations settle.
        _broker_lock: Option<File>,
        _installation: Option<Arc<learning_assets::InstalledSnapshotBinding>>,
        #[cfg(test)]
        pub(in super::super) hooks: Option<Arc<Hooks>>,
    }
    pub(in super::super) struct EpochWork {
        original: Arc<Original>,
    }
    // No production constructor. The only constructor is a private real Root
    // fixture below; storage UUIDs never constitute enrolled authority.
    impl EpochWork {
        pub(in super::super) fn installed(
            pool: PgPool,
            installation: Arc<learning_assets::InstalledSnapshotBinding>,
            journal: RootJournalBytes,
            budget: SharedJournalBudget,
            build_sha256: String,
        ) -> Result<Self> {
            let observation = ObservationBudget::with_shared(64 * 1024 * 1024, budget.clone())?;
            Ok(Self {
                original: Arc::new(Original {
                    pool,
                    budget,
                    observation,
                    journal: Arc::new(AsyncMutex::new(journal)),
                    registry: Mutex::new(Registry {
                        jobs: BTreeMap::new(),
                        requests: BTreeMap::new(),
                        active: false,
                        closing: false,
                    }),
                    handles: Mutex::new(Vec::new()),
                    control_handle: Mutex::new(None),
                    hold: Arc::new(AtomicBool::new(false)),
                    epoch: installation.epoch_id(),
                    binding: installation.binding_id(),
                    // Descriptive build linkage is not source registry enrollment.
                    source_build: SourceProvenance {
                        mode: SourceMode::UnenrolledComponent,
                        external_reference_not_authority: true,
                        execution_binding_sha256: build_sha256,
                    },
                    sql_calls: AtomicUsize::new(0),
                    append_calls: AtomicUsize::new(0),
                    stored_bytes: AtomicU64::new(40),
                    _broker_lock: None,
                    _installation: Some(installation),
                    #[cfg(test)]
                    hooks: None,
                }),
            })
        }

        pub(in super::super) fn owner(&self) -> Arc<Original> {
            self.original.clone()
        }
        pub(in super::super) fn close_admission(&self, duration: Duration) -> Result<Arc<Slot>> {
            if duration.is_zero() || duration > Duration::from_secs(60) || self.original.is_held() {
                return Err(rejected("epoch capture admission closed"));
            }
            self.original.request_quiesce()?;
            Ok(Arc::new(Slot {
                state: Mutex::new(State::Live),
                deadline: Instant::now() + duration,
                epoch_hold: self.original.hold.clone(),
                issued: AtomicBool::new(false),
                revocation_requested: AtomicBool::new(false),
            }))
        }

        /// Consume the epoch in an independently retained original task. Its
        /// control handle is separate from the worker drain, so it can never
        /// wait for itself. Losing the outer receiver poisons publication only.
        pub(in super::super) fn start_quiesce<T, F, Fut>(self, drive: F) -> Result<Pending<T>>
        where
            T: Send + 'static,
            F: FnOnce(EpochWork, Arc<Original>, Arc<Slot>) -> Fut + Send + 'static,
            Fut: Future<Output = Result<T>> + Send + 'static,
        {
            let slot = self.close_admission(Duration::from_secs(60))?;
            let owner = self.original.clone();
            let mut retained = owner
                .control_handle
                .lock()
                .map_err(|_| rejected("epoch control handle poisoned"))?;
            if retained.is_some() {
                return Err(rejected("epoch control never replays"));
            }
            owner
                .budget
                .charge(512, 1)
                .map_err(|_| ObservationError::Capacity)?;
            let (sender, receiver) = oneshot::channel();
            let original = owner.clone();
            let task_slot = slot.clone();
            let task = tokio::spawn(async move {
                let mut guard = OwnerGuard {
                    original: original.clone(),
                    slot: task_slot.clone(),
                    completed: false,
                    _custody: None,
                };
                match drive(self, original, task_slot.clone()).await {
                    Ok(value) => publish(&task_slot, sender, value),
                    Err(error) => {
                        task_slot.poison();
                        let _ = sender.send(Err(error));
                    }
                }
                guard.completed = true;
            });
            *retained = Some(Arc::new(AsyncMutex::new(Some(task))));
            Ok(Pending {
                receiver,
                timer: Box::pin(sleep_until(slot.deadline)),
                slot,
                armed: true,
            })
        }

        pub(in super::super) fn start_continuation<T, F, Fut>(
            &mut self,
            job: Uuid,
            request: Uuid,
            duration: Duration,
            retention: Option<learning_assets::CustodyWriter>,
            drive: F,
        ) -> Result<Pending<T>>
        where
            T: Send + 'static,
            F: FnOnce(Arc<Original>, Arc<Slot>) -> Fut + Send + 'static,
            Fut: Future<Output = Result<T>> + Send + 'static,
        {
            // Only the installed lifecycle may request another genuine SQL
            // attempt. The old slot remains permanently non-live; no old cap
            // is reopened. Its complete known history is checked by lifecycle.
            {
                let mut registry = self
                    .original
                    .registry
                    .lock()
                    .map_err(|_| rejected("epoch registry poisoned"))?;
                if let Some(previous) = registry.jobs.get(&job) {
                    let state = previous
                        .state
                        .lock()
                        .map_err(|_| rejected("epoch old slot poisoned"))?;
                    if *state != State::Published || previous.issued.load(Ordering::SeqCst) {
                        return Err(rejected("epoch old attempt not revoked before write"));
                    }
                    drop(state);
                    registry.jobs.remove(&job);
                }
            }
            self.start_owned(job, request, duration, retention, drive)
        }
        pub(in super::super) fn start(
            &mut self,
            job: Uuid,
            request: Uuid,
            duration: Duration,
        ) -> Result<Pending> {
            self.start_owned(
                job,
                request,
                duration,
                None,
                move |owner, slot| async move {
                    super::super::transition::drive(owner, slot, job, request).await
                },
            )
        }
        pub(in super::super) fn start_copy(
            &mut self,
            job: Uuid,
            request: Uuid,
            duration: Duration,
            runtime: PgPool,
            custody: &learning_assets::RootSnapshotCustody,
        ) -> Result<Pending<super::super::writer::StagedComponent>> {
            // A distinct OS open+SH precedes registry insertion, queueing and
            // spawn. It moves through the original owner into its writer.
            let writer = custody
                .writer()
                .map_err(|_| rejected("C2 fresh custody unavailable"))?;
            // A consumed writer may fail and drop its own guard before the
            // JoinHandle returns. The original supervisor retains a separate
            // independent SH until its entire error/result settlement ends.
            let retention = custody
                .writer()
                .map_err(|_| rejected("C2 owner custody unavailable"))?;
            self.start_owned(
                job,
                request,
                duration,
                Some(retention),
                move |owner, slot| async move {
                    super::super::writer::drive(owner, slot, job, request, runtime, writer).await
                },
            )
        }
        // A new original operation, never adoption of a returned staged value.
        pub(in super::super) fn start_publish(
            &mut self,
            job: Uuid,
            request: Uuid,
            duration: Duration,
            runtime: PgPool,
            custody: &learning_assets::RootSnapshotCustody,
        ) -> Result<Pending<super::super::writer::PublishedComponent>> {
            let writer = custody
                .writer()
                .map_err(|_| rejected("D1 fresh custody unavailable"))?;
            let retention = custody
                .writer()
                .map_err(|_| rejected("D1 owner custody unavailable"))?;
            self.start_owned(
                job,
                request,
                duration,
                Some(retention),
                move |owner, slot| async move {
                    super::super::writer::publish(owner, slot, job, request, runtime, writer).await
                },
            )
        }
        fn start_owned<T, F, Fut>(
            &mut self,
            job: Uuid,
            request: Uuid,
            duration: Duration,
            retention: Option<learning_assets::CustodyWriter>,
            drive: F,
        ) -> Result<Pending<T>>
        where
            T: Send + 'static,
            F: FnOnce(Arc<Original>, Arc<Slot>) -> Fut + Send + 'static,
            Fut: Future<Output = Result<T>> + Send + 'static,
        {
            if job.is_nil()
                || request.is_nil()
                || duration.is_zero()
                || duration > Duration::from_secs(60)
                || self.original.hold.load(Ordering::SeqCst)
            {
                return Err(rejected("P1 unsupported admission"));
            }
            let mut registry = self
                .original
                .registry
                .lock()
                .map_err(|_| rejected("P1 registry poisoned"))?;
            if registry.closing
                || registry.active
                || registry.jobs.contains_key(&job)
                || registry.requests.contains_key(&request)
            {
                return Err(rejected("P1 single-flight never replays"));
            }
            self.original
                .budget
                .charge(256, 1)
                .map_err(|_| ObservationError::Capacity)?;
            let slot = Arc::new(Slot {
                state: Mutex::new(State::Live),
                deadline: Instant::now() + duration,
                epoch_hold: self.original.hold.clone(),
                issued: AtomicBool::new(false),
                revocation_requested: AtomicBool::new(false),
            });
            registry.jobs.insert(job, slot.clone());
            registry.requests.insert(request, job);
            registry.active = true;
            // Registration and original deadline exist before spawn/first await.
            let (sender, receiver) = oneshot::channel();
            let owner = self.original.clone();
            let task_slot = slot.clone();
            let mut handles = self
                .original
                .handles
                .lock()
                .map_err(|_| rejected("P1 supervisor poisoned"))?;
            handles
                .try_reserve(1)
                .map_err(|_| ObservationError::Capacity)?;
            let task = tokio::spawn(async move {
                let mut guard = OwnerGuard {
                    original: owner.clone(),
                    slot: task_slot.clone(),
                    completed: false,
                    _custody: retention,
                };
                let result = drive(owner.clone(), task_slot.clone()).await;
                match result {
                    Ok(record) => publish(&task_slot, sender, record),
                    Err(error) => {
                        task_slot.poison();
                        let _ = sender.send(Err(error));
                    }
                }
                if let Ok(mut registry) = owner.registry.lock() {
                    registry.active = false;
                }
                guard.completed = true;
            });
            // The task itself and every blocking closure retain this Original;
            // dropping EpochWork cannot drop an active settlement JoinHandle.
            handles.push(Arc::new(AsyncMutex::new(Some(task))));
            Ok(Pending {
                receiver,
                timer: Box::pin(sleep_until(slot.deadline)),
                slot,
                armed: true,
            })
        }
        pub(in super::super) async fn drain(&mut self) -> Result<()> {
            self.original
                .registry
                .lock()
                .map_err(|_| rejected("P1 registry poisoned"))?
                .closing = true;
            let handles = self
                .original
                .handles
                .lock()
                .map_err(|_| rejected("P1 supervisor poisoned"))?
                .clone();
            let mut failed = false;
            for handle in handles {
                // Await a borrowed handle in its permanent original slot. If
                // this drain future is cancelled, the handle remains retained.
                let mut handle = handle.lock().await;
                if let Some(task) = handle.as_mut() {
                    if task.await.is_err() {
                        failed = true;
                    }
                    handle.take(); // only after actual task settlement
                }
            }
            if failed {
                self.original.hold.store(true, Ordering::SeqCst);
                return Err(rejected("P1 owner panicked"));
            }
            Ok(())
        }
        #[cfg(test)]
        pub(in super::super) fn original(&self) -> &Arc<Original> {
            &self.original
        }
    }
    impl Drop for EpochWork {
        fn drop(&mut self) {
            self.original.hold.store(true, Ordering::SeqCst);
            if let Ok(registry) = self.original.registry.lock() {
                for slot in registry.jobs.values() {
                    slot.poison();
                }
            }
        }
    }
    struct OwnerGuard {
        original: Arc<Original>,
        slot: Arc<Slot>,
        completed: bool,
        _custody: Option<learning_assets::CustodyWriter>,
    }
    impl Drop for OwnerGuard {
        fn drop(&mut self) {
            if !self.completed {
                self.slot.poison();
                self.original.hold.store(true, Ordering::SeqCst);
            }
        }
    }
    pub(in super::super) struct Pending<T = DurableTransition> {
        receiver: oneshot::Receiver<Result<T>>,
        timer: Pin<Box<Sleep>>,
        slot: Arc<Slot>,
        armed: bool,
    }
    impl<T> Future for Pending<T> {
        type Output = Result<T>;
        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            if self.timer.as_mut().poll(cx).is_ready() {
                self.slot.poison();
                self.armed = false;
                return Poll::Ready(Err(ObservationError::Timeout));
            }
            match Pin::new(&mut self.receiver).poll(cx) {
                Poll::Ready(Ok(result)) => {
                    self.armed = false;
                    let receiving = self
                        .slot
                        .state
                        .lock()
                        .is_ok_and(|state| *state == State::Published)
                        && !self.slot.epoch_hold.load(Ordering::SeqCst)
                        && Instant::now() < self.slot.deadline;
                    if result.is_ok() && !receiving {
                        self.slot.poison();
                        Poll::Ready(Err(rejected("P1 result fence closed")))
                    } else {
                        Poll::Ready(result)
                    }
                }
                Poll::Ready(Err(_)) => {
                    self.slot.poison();
                    self.armed = false;
                    Poll::Ready(Err(rejected("P1 original owner unavailable")))
                }
                Poll::Pending => Poll::Pending,
            }
        }
    }
    impl<T> Drop for Pending<T> {
        fn drop(&mut self) {
            if self.armed {
                self.slot.poison();
            }
        }
    }
    fn publish<T>(slot: &Slot, sender: oneshot::Sender<Result<T>>, result: T) {
        if slot.live().is_err() {
            slot.poison();
            let _ = sender.send(Err(rejected("P1 late outcome HOLD")));
            return;
        }
        let Ok(mut state) = slot.state.lock() else {
            slot.poison();
            return;
        };
        if *state != State::Live
            || slot.epoch_hold.load(Ordering::SeqCst)
            || Instant::now() >= slot.deadline
            || sender.is_closed()
        {
            *state = State::Hold;
            slot.epoch_hold.store(true, Ordering::SeqCst);
            return;
        }
        *state = State::Published;
        if sender.send(Ok(result)).is_err() {
            *state = State::Hold;
            slot.epoch_hold.store(true, Ordering::SeqCst);
        }
    }

    /// Never drop an already-started SQL/commit/append future at a timeout.
    /// Deadline closes publication, while its original owner retains settlement.
    pub(in super::super) async fn settle<T>(
        slot: &Slot,
        future: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        slot.live()?;
        settle_started(slot, future).await
    }
    pub(in super::super) async fn settle_started<T>(
        slot: &Slot,
        future: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        tokio::pin!(future);
        let result = tokio::select! {
            value = &mut future => value,
            _ = sleep_until(slot.deadline) => { slot.poison(); future.await }
        };
        if Instant::now() >= slot.deadline {
            slot.poison();
        }
        // Return ownership even when late (not authority). In particular a
        // late begin must reach the original owner's explicit rollback path.
        result
    }
    impl Original {
        #[cfg(test)]
        pub(in super::super) async fn settle_control(&self) -> Result<()> {
            let retained = self
                .control_handle
                .lock()
                .map_err(|_| rejected("epoch control handle poisoned"))?
                .clone()
                .ok_or_else(|| rejected("epoch control missing"))?;
            let mut handle = retained.lock().await;
            if let Some(task) = handle.as_mut() {
                task.await.map_err(|_| rejected("epoch control panicked"))?;
                handle.take();
            }
            Ok(())
        }
        pub(in super::super) fn bind_selected_job(
            &self,
            slot: &Arc<Slot>,
            placeholder: Uuid,
            job: Uuid,
        ) -> Result<bool> {
            let mut registry = self
                .registry
                .lock()
                .map_err(|_| rejected("epoch registry poisoned"))?;
            if registry.closing {
                return Ok(false);
            }
            if !registry
                .jobs
                .get(&placeholder)
                .is_some_and(|value| Arc::ptr_eq(value, slot))
            {
                return Err(rejected("epoch original selector slot differs"));
            }
            if job != placeholder
                && let Some(previous) = registry.jobs.get(&job)
            {
                let state = previous
                    .state
                    .lock()
                    .map_err(|_| rejected("epoch old attempt gate poisoned"))?;
                if *state != State::Published || previous.issued.load(Ordering::SeqCst) {
                    return Err(rejected("epoch prior attempt is not closed before write"));
                }
            }
            registry.jobs.remove(&placeholder);
            registry.jobs.insert(job, slot.clone());
            registry.requests.insert(placeholder, job);
            Ok(true)
        }
        pub(in super::super) fn is_installed(&self) -> bool {
            self._installation.is_some()
        }
        pub(in super::super) fn request_quiesce(&self) -> Result<()> {
            let mut registry = self
                .registry
                .lock()
                .map_err(|_| rejected("epoch registry poisoned"))?;
            registry.closing = true;
            for slot in registry.jobs.values() {
                let state = slot
                    .state
                    .lock()
                    .map_err(|_| rejected("epoch slot gate poisoned"))?;
                if *state == State::Live && !slot.issued.load(Ordering::SeqCst) {
                    slot.revocation_requested.store(true, Ordering::SeqCst);
                }
            }
            Ok(())
        }
        pub(in super::super) fn is_held(&self) -> bool {
            self.hold.load(Ordering::SeqCst)
        }
        pub(in super::super) async fn visit_prefix(
            &self,
            visitor: &mut (
                     dyn FnMut(&str, u64, &mut dyn std::io::Read) -> std::io::Result<()> + Send
                 ),
        ) -> Result<RecordRef> {
            if self.is_held() {
                return Err(rejected("epoch HOLD cannot capture"));
            }
            self.journal
                .lock()
                .await
                .visit_known_files(visitor)
                .map(Into::into)
                .map_err(|_| rejected("epoch original prefix capture failed"))
        }
        #[cfg(test)]
        pub(in super::super) fn issued_for(&self, job: Uuid) -> bool {
            self.registry
                .lock()
                .unwrap()
                .jobs
                .get(&job)
                .is_some_and(|slot| slot.issued.load(Ordering::SeqCst))
        }
        #[cfg(test)]
        pub(in super::super) fn held(&self) -> bool {
            self.hold.load(Ordering::SeqCst)
        }
        #[cfg(test)]
        pub(in super::super) fn active(&self) -> bool {
            self.registry.lock().unwrap().active
        }
        pub(in super::super) fn preflight(&self) -> Result<()> {
            let used = self
                .budget
                .usage()
                .map_err(|_| ObservationError::Capacity)?;
            // Non-charging upper bound: four complete old-prefix scans plus
            // two maximum frames, typed encoding, after-row and metadata work.
            // Actual encoding and IO remain separately charged, never invented
            // as speculative byte usage or refunded/reset after this check.
            let required = self
                .stored_bytes
                .load(Ordering::SeqCst)
                .checked_mul(4)
                .and_then(|v| v.checked_add(64 * 1024 * 1024))
                .ok_or(ObservationError::Capacity)?;
            if used.exhausted
                || used
                    .charged_bytes
                    .checked_add(required)
                    .is_none_or(|v| v > 512 * 1024 * 1024)
                || used
                    .charged_entries
                    .checked_add(self.append_calls.load(Ordering::SeqCst) as u64 * 8 + 64)
                    .is_none_or(|v| v > 100_000)
            {
                return Err(ObservationError::Capacity);
            }
            Ok(())
        }
        pub(in super::super) async fn stage(&self, slot: &Slot, stage: Stage) -> Result<()> {
            #[cfg(test)]
            if let Some(hooks) = &self.hooks
                && hooks.stage == stage
                && !hooks.blocking_return
            {
                hooks.arrived.notify_one();
                hooks.resume.notified().await;
            }
            #[cfg(not(test))]
            let _ = stage;
            slot.live()
        }
        pub(in super::super) async fn head(&self, slot: &Slot) -> Result<RecordRef> {
            slot.live()?;
            settle(slot, async {
                self.journal
                    .lock()
                    .await
                    .known_byte_head()
                    .map(Into::into)
                    .map_err(|_| rejected("P1 journal prefix unavailable"))
            })
            .await
        }
        pub(in super::super) async fn append(
            self: &Arc<Self>,
            slot: &Slot,
            bytes: Vec<u8>,
        ) -> Result<RecordRef> {
            slot.live()?;
            let journal =
                settle(slot, async { Ok(self.journal.clone().lock_owned().await) }).await?;
            slot.live()?;
            let owner = self.clone();
            #[cfg(test)]
            let outcome_spawn = self.append_calls.load(Ordering::SeqCst) == 1;
            let task = tokio::task::spawn_blocking(move || {
                let mut journal = journal;
                let result = journal
                    .append_uninterpreted(
                        OpaqueJournalBody::from_uninterpreted_bytes(&bytes)
                            .map_err(|_| rejected("P1 record capacity"))?,
                    )
                    .map(Into::into)
                    .map_err(|_| rejected("P1 durable append rejected"));
                if result.is_ok() {
                    owner
                        .stored_bytes
                        .fetch_add(bytes.len() as u64 + 120, Ordering::SeqCst);
                }
                let ordinal = owner.append_calls.fetch_add(1, Ordering::SeqCst) + 1;
                #[cfg(test)]
                if let Some(hooks) = &owner.hooks
                    && hooks.blocking_return
                    && ((ordinal == 2 && hooks.stage == Stage::AfterOutcome)
                        || (ordinal == 5 && hooks.stage == Stage::AfterBusinessEvent)
                        || (ordinal == 6 && hooks.stage == Stage::AfterCloseEvent)
                        || (ordinal == 1 && hooks.stage == Stage::AfterQuiesceAppend))
                {
                    hooks.arrived.notify_one();
                    let (lock, wake) = &hooks.blocking_release;
                    let mut released = lock
                        .lock()
                        .map_err(|_| rejected("P1 fixture gate poisoned"))?;
                    while !*released {
                        released = wake
                            .wait(released)
                            .map_err(|_| rejected("P1 fixture gate poisoned"))?;
                    }
                }
                #[cfg(not(test))]
                let _ = ordinal;
                // The closure retains the original supervisor until its actual
                // append and any fixture return barrier have both settled.
                result
            });
            #[cfg(test)]
            if let Some(hooks) = &self.hooks
                && hooks.post_spawn_join
                && outcome_spawn
            {
                // Deliberately no live check here: this barrier is after spawn.
                // Even a now-HOLD owner must await the original blocking task.
                hooks.spawned.notify_one();
                hooks.join_resume.notified().await;
            }
            settle_started(slot, async {
                task.await
                    .map_err(|_| rejected("P1 original append panicked"))?
            })
            .await
        }
    }

    #[cfg(test)]
    pub(in super::super) fn real_fixture(
        pool: PgPool,
        root: &std::path::Path,
        hooks: Option<Arc<Hooks>>,
    ) -> Result<EpochWork> {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        // Untrusted descriptive reference, never an enrolled build or factory
        // input. Root separately joins this head to actual source/ELF/grants.
        let execution_binding_sha256 =
            std::env::var("TEST_SNAPSHOT_CLAIM_P1_EXECUTION_BINDING_SHA256")
                .map_err(|_| rejected("P1 explicit external execution reference required"))?;
        if !valid_reference(&execution_binding_sha256) {
            return Err(rejected("P1 external reference format"));
        }
        let source_build = SourceProvenance {
            mode: SourceMode::UnenrolledComponent,
            external_reference_not_authority: true,
            execution_binding_sha256,
        };
        // Existing Root-private parents are checked again by the real backend.
        std::fs::create_dir(root).map_err(|_| rejected("P1 fresh fixture root"))?;
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| rejected("P1 fixture permissions"))?;
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(root.join("broker.lock"))
            .map_err(|_| rejected("P1 fixture lock"))?;
        lock.try_lock()
            .map_err(|_| rejected("P1 fixture single instance"))?;
        let dir = root.join("journal");
        std::fs::create_dir(&dir).map_err(|_| rejected("P1 fresh journal"))?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| rejected("P1 fixture permissions"))?;
        let epoch = Uuid::new_v4();
        let binding = Uuid::new_v4();
        let budget = SharedJournalBudget::default();
        let observation = ObservationBudget::with_shared(64 * 1024 * 1024, budget.clone())?;
        let journal = RootJournalBytes::create_fresh(
            &dir,
            learning_assets::root_journal::ByteJournalIdentity::new(epoch, binding)
                .map_err(|_| rejected("P1 fixture identity"))?,
            budget.clone(),
        )
        .map_err(|_| rejected("P1 actual Root journal required"))?;
        Ok(EpochWork {
            original: Arc::new(Original {
                pool,
                budget,
                observation,
                journal: Arc::new(AsyncMutex::new(journal)),
                registry: Mutex::new(Registry {
                    jobs: BTreeMap::new(),
                    requests: BTreeMap::new(),
                    active: false,
                    closing: false,
                }),
                handles: Mutex::new(Vec::new()),
                control_handle: Mutex::new(None),
                hold: Arc::new(AtomicBool::new(false)),
                epoch,
                binding,
                source_build,
                sql_calls: AtomicUsize::new(0),
                append_calls: AtomicUsize::new(0),
                stored_bytes: AtomicU64::new(40),
                _broker_lock: Some(lock),
                _installation: None,
                hooks,
            }),
        })
    }
    #[cfg(test)]
    mod gate_tests {
        use super::*;
        fn fresh() -> Arc<Slot> {
            Arc::new(Slot {
                state: Mutex::new(State::Live),
                deadline: Instant::now() + Duration::from_secs(30),
                epoch_hold: Arc::new(AtomicBool::new(false)),
                issued: AtomicBool::new(false),
                revocation_requested: AtomicBool::new(false),
            })
        }
        #[test]
        fn formal_revoke_and_issue_share_one_gate_in_both_orders() {
            let revoked = fresh();
            revoked.reserve_revocation().unwrap();
            assert!(matches!(
                revoked.issue_sticky(),
                Err(ObservationError::RevokedBeforeWrite)
            ));
            assert!(!revoked.issued());
            revoked.live().unwrap();
            let issued = fresh();
            issued.issue_sticky().unwrap();
            assert!(issued.reserve_revocation().is_err());
            assert!(issued.issued());
        }
        #[test]
        fn formal_concurrent_revoke_cannot_undo_an_issued_permit() {
            for _ in 0..64 {
                let slot = fresh();
                let other = slot.clone();
                let issue = std::thread::spawn(move || other.issue_sticky().is_ok());
                let revoked = slot.reserve_revocation().is_ok();
                let issued = issue.join().unwrap();
                assert_ne!(issued, revoked);
                assert_eq!(slot.issued(), issued);
                assert_eq!(slot.revocation_requested.load(Ordering::SeqCst), revoked);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn p1_encoding_charges_actual_escaped_bytes_twice_before_allocation() {
        let budget = SharedJournalBudget::default();
        let bytes = encode(&"\u{0000}汉字\"\\", &budget).unwrap();
        assert_eq!(bytes, serde_json::to_vec(&"\u{0000}汉字\"\\").unwrap());
        assert_eq!(
            budget.usage().unwrap().charged_bytes,
            bytes.len() as u64 * 2
        );
        let _ = digest(&bytes, &budget).unwrap();
        assert_eq!(
            budget.usage().unwrap().charged_bytes,
            bytes.len() as u64 * 3
        );
    }
    #[test]
    fn p1_record_limit_and_shared_work_exhaustion_reject_without_reset() {
        let budget = SharedJournalBudget::default();
        let boundary = "x".repeat(MAX_FRAME_BYTES - 120 - 2);
        assert_eq!(
            encode(&boundary, &budget).unwrap().len(),
            MAX_FRAME_BYTES - 120
        );
        assert!(encode(&(boundary + "x"), &budget).is_err());
        assert!(encode(&"x".repeat(MAX_FRAME_BYTES), &budget).is_err());
        budget
            .charge(512 * 1024 * 1024 - budget.usage().unwrap().charged_bytes, 0)
            .unwrap();
        assert!(encode(&"x", &budget).is_err());
        assert!(encode(&"", &budget.clone()).is_err());
    }
    #[test]
    fn p1_typed_claim_rejects_duplicate_unknown_and_missing_fields_without_authority() {
        let raw = format!(
            "{{\"job_id\":\"{}\",\"token\":\"{}\",\"attempts\":1,\"expires_at\":\"2026-10-21T00:00:00Z\"}}",
            Uuid::new_v4(),
            Uuid::new_v4()
        );
        let _: Claim = serde_json::from_str(&raw).unwrap();
        assert!(
            serde_json::from_str::<Claim>(
                &raw.replace("\"attempts\":1", "\"attempts\":1,\"attempts\":1")
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<Claim>(
                &raw.replace("\"attempts\":1", "\"attempts\":1,\"authority\":true")
            )
            .is_err()
        );
        assert!(serde_json::from_str::<Claim>(&raw.replace("\"attempts\":1,", "")).is_err());
    }

    #[test]
    fn p1_external_source_reference_cannot_parse_as_enrolled_authority() {
        let reference = "a".repeat(64);
        let raw = format!(
            "{{\"mode\":\"UNENROLLED_COMPONENT_EXTERNAL_RUN_ONLY\",\"external_reference_not_authority\":true,\"execution_binding_sha256\":\"{reference}\"}}"
        );
        let _: SourceProvenance = serde_json::from_str(&raw).unwrap();
        for wrong in [
            raw.replace("true", "false"),
            raw.replace("UNENROLLED_COMPONENT_EXTERNAL_RUN_ONLY", "ENROLLED"),
            raw.replace(&reference, "NONE"),
        ] {
            assert!(serde_json::from_str::<SourceProvenance>(&wrong).is_err());
        }
    }
    #[test]
    fn p1_second_serialize_growth_is_rejected_before_unpaid_vec_growth() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Changing {
            calls: AtomicUsize,
            second: String,
        }
        impl Serialize for Changing {
            fn serialize<S: serde::Serializer>(
                &self,
                serializer: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    serializer.serialize_str("x")
                } else {
                    serializer.serialize_str(&self.second)
                }
            }
        }
        let record = Changing {
            calls: AtomicUsize::new(0),
            second: "x".repeat(MAX_FRAME_BYTES),
        };
        let budget = SharedJournalBudget::default();
        assert!(encode(&record, &budget).is_err());
        assert_eq!(record.calls.load(Ordering::SeqCst), 2);
        assert_eq!(budget.usage().unwrap().charged_bytes, 6);
    }
}

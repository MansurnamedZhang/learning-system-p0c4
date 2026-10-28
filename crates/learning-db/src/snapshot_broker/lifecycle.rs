//! Root original-epoch lifecycle. The view exported here is not a verified
//! backup capture or a NoEffect/AlreadyCommitted finding. Backup must join its
//! own original registry, source admission, six tables and sealed local pin.
use super::history::{
    self, RecordRef, Result,
    runtime::{EpochWork, Original},
};
use chrono::{DateTime, Utc};
use learning_assets::{
    CustodyCaptureGuard, InstalledSnapshotBinding, SnapshotDirectory, SnapshotIoError,
    root_journal::{JournalBudgetUsage, SharedJournalBudget},
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::{
    collections::BTreeMap,
    io::{self, Read},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use std::{
    fs::File,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{FileTypeExt, MetadataExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::PathBuf,
};
use uuid::Uuid;

pub const STORAGE_ENCODING: &str = "root_frame_v1";

async fn close_bootstrap_connection<T>(
    connection: Option<sqlx::pool::PoolConnection<sqlx::Postgres>>,
    outcome: Result<T>,
    #[cfg(feature = "b1-source-peer")] fixed: Option<&super::b1_peer::FixedWorkPools>,
) -> Result<T> {
    let closed = match connection {
        Some(connection) => connection
            .close()
            .await
            .map_err(super::ObservationError::from),
        None => Ok(()),
    };
    #[cfg(feature = "b1-source-peer")]
    if closed.is_err()
        && let Some(fixed) = fixed
    {
        fixed.mark_work_unknown();
    }
    let value = outcome?;
    closed?;
    Ok(value)
}

/// Descriptions are never registry enrollment or proof of deployment cutover.
/// The source capture independently matches these against its retained inputs.
pub struct BrokerSourceDescription {
    pub deployment_id: Uuid,
    pub source_group_id: Uuid,
    pub source_application_commit: String,
    pub source_build_sha256: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrokerSignalResult {
    NoWork,
    Completed,
    RevokedBeforeWrite,
    NotAcquired,
}

/// Trusted management requests an actual canonical operation. These variants
/// neither supply a finding nor assert that the requested operation succeeded.
#[derive(Clone, Copy)]
pub enum BeforeWriteTransition {
    None,
    Renew,
    TransientFailure,
    TerminalFailure,
}

#[derive(Serialize)]
struct Reference {
    sequence: u64,
    sha256: String,
}
impl Reference {
    fn from_record(record: RecordRef) -> Self {
        let h = record.head();
        Self {
            sequence: h.sequence,
            sha256: h.sha256.iter().map(|b| format!("{b:02x}")).collect(),
        }
    }
}

#[derive(Serialize)]
struct EpochDescriptor {
    format_version: u8,
    capability: &'static str,
    storage_encoding: &'static str,
    deployment_id: Uuid,
    epoch_id: Uuid,
    binding_id: Uuid,
    source_group_id: Uuid,
    source_application_commit: String,
    source_build_sha256: String,
    broker_binary_sha256: String,
    broker_pid: u32,
    broker_uid: u32,
    broker_gid: u32,
    broker_binary_dev: u64,
    broker_binary_ino: u64,
    broker_binary_bytes: u64,
    source_database: String,
    source_database_oid: i64,
    source_system_identifier: String,
    source_postmaster_start: DateTime<Utc>,
    companion_root_dev: u64,
    companion_root_ino: u64,
    export_root_dev: u64,
    export_root_ino: u64,
    journal_root_dev: u64,
    journal_root_ino: u64,
    effect_lock_dev: u64,
    effect_lock_ino: u64,
    initial_namespace_sha256: String,
    writer_contract_sha256: String,
    work_socket_dev: u64,
    work_socket_ino: u64,
    management_socket_dev: u64,
    management_socket_ino: u64,
}

#[derive(Serialize)]
struct AttemptSummary {
    attempt: i32,
    claim_token: Uuid,
    lease_expires_at: DateTime<Utc>,
    claim_record: Reference,
    revoked_before_write: Option<Reference>,
    write_issued: Option<Reference>,
    business_commit: Option<Reference>,
    write_closed: Option<Reference>,
}
#[derive(Serialize)]
struct JobSummary {
    job_id: Uuid,
    source_job_binding_sha256: String,
    current_rows_sha256: String,
    attempts: Vec<AttemptSummary>,
    last_event: Reference,
}
struct KnownJob {
    summary: JobSummary,
    _proof: ClosedProof,
}
enum ClosedProof {
    Revoked(RecordRef),
    NotAcquired(RecordRef),
    Committed(super::writer::PublishedComponent),
}
#[derive(Serialize)]
struct LedgerView<'a> {
    format_version: u8,
    capability: &'static str,
    storage_encoding: &'static str,
    epoch_id: Uuid,
    binding_id: Uuid,
    jobs: Vec<&'a JobSummary>,
}
#[derive(Serialize)]
struct CutView {
    format_version: u8,
    capability: &'static str,
    storage_encoding: &'static str,
    backup_id: Uuid,
    capture_id: Uuid,
    epoch_id: Uuid,
    binding_id: Uuid,
    previous_head: Reference,
    cut_record: Reference,
    descriptor_sha256: String,
    ledger_sha256: String,
    active_writers: u64,
    active_operations: u64,
    pools_closed: bool,
    observed_work_peers: Vec<PeerObservation>,
}
#[derive(Serialize)]
struct PeerObservation {
    request_id: Uuid,
    process_id: i32,
    uid: u32,
    gid: u32,
    work_socket_dev: u64,
    work_socket_ino: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Signal {
    format_version: u8,
    command: String,
    request_id: String,
    lease_ms: u64,
}
fn parse_signal(raw: &[u8]) -> Result<(Uuid, u64)> {
    let signal: Signal = serde_json::from_slice(raw)
        .map_err(|_| history::rejected("strict work signal rejected"))?;
    let request = Uuid::parse_str(&signal.request_id)
        .map_err(|_| history::rejected("work request ID rejected"))?;
    if signal.format_version != 2
        || signal.command != "SIGNAL_NEXT"
        || request.to_string() != signal.request_id
        || request.get_version_num() != 4
        || request.get_variant() != uuid::Variant::RFC4122
        || !(30_000..=300_000).contains(&signal.lease_ms)
    {
        return Err(history::rejected("work signal policy rejected"));
    }
    Ok((request, signal.lease_ms))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InstalledRootMarker {
    format_version: u8,
    capability: String,
    binding_id: Uuid,
    deployment_id: Uuid,
    export_root_dev: u64,
    export_root_ino: u64,
    journal_root_dev: u64,
    journal_root_ino: u64,
    effect_lock_dev: u64,
    effect_lock_ino: u64,
    writer_contract_sha256: String,
}

/// Owns the installed fresh OS binding and all original operation supervision.
/// No constructor accepts arbitrary SQL, caller JSON, a JobLease or old bytes.
pub struct RootSnapshotBroker {
    #[cfg(feature = "b1-source-peer")]
    b1_lifetime: Mutex<Option<super::b1_peer::WorkLifetime>>,
    #[cfg(feature = "b1-source-peer")]
    pub(super) b1_original: Mutex<Option<Arc<super::B1InstalledSourceProcess>>>,
    work: Mutex<Option<EpochWork>>,
    installed: Arc<InstalledSnapshotBinding>,
    runtime: PgPool,
    descriptor: Vec<u8>,
    jobs: Arc<Mutex<BTreeMap<Uuid, KnownJob>>>,
    settled: Arc<Mutex<BTreeMap<Uuid, (u64, BrokerSignalResult)>>>,
    quiescing: Arc<AtomicBool>,
    sockets: Vec<SocketBinding>,
    peers: Mutex<Vec<PeerObservation>>,
}

struct SocketBinding {
    listener: UnixListener,
    listener_identity: (u64, u64),
    file: File,
    parent: File,
    path: PathBuf,
    device: u64,
    inode: u64,
    group: u32,
    mode: u32,
}
impl SocketBinding {
    fn open_parent(path: &std::path::Path) -> Result<File> {
        let raw = unsafe {
            libc::open(
                c"/".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if raw < 0 {
            return Err(history::rejected("socket root unavailable"));
        }
        let mut held = unsafe { File::from_raw_fd(raw) };
        for part in path.components() {
            match part {
                std::path::Component::RootDir => {}
                std::path::Component::Normal(part) => {
                    let part = std::ffi::CString::new(part.as_bytes())
                        .map_err(|_| history::rejected("socket parent component"))?;
                    let raw = unsafe {
                        libc::openat(
                            held.as_raw_fd(),
                            part.as_ptr(),
                            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                        )
                    };
                    if raw < 0 {
                        return Err(history::rejected("socket parent unavailable"));
                    }
                    held = unsafe { File::from_raw_fd(raw) };
                }
                _ => return Err(history::rejected("socket parent must be canonical")),
            }
            let m = held
                .metadata()
                .map_err(|_| history::rejected("socket parent metadata"))?;
            if !m.is_dir() || m.uid() != 0 || m.mode() & 0o022 != 0 {
                return Err(history::rejected("socket parent is not Root controlled"));
            }
        }
        Ok(held)
    }
    fn observe(listener: &UnixListener, group: u32, mode: u32) -> Result<Self> {
        let path = listener
            .local_addr()
            .map_err(|_| history::rejected("broker socket address"))?
            .as_pathname()
            .ok_or_else(|| history::rejected("broker socket must be named"))?
            .to_path_buf();
        if !path.is_absolute() {
            return Err(history::rejected("broker socket must be absolute"));
        }
        let listener = listener
            .try_clone()
            .map_err(|_| history::rejected("broker socket original FD"))?;
        let listener_metadata = File::from(std::os::fd::OwnedFd::from(
            listener
                .try_clone()
                .map_err(|_| history::rejected("broker socket original FD"))?,
        ))
        .metadata()
        .map_err(|_| history::rejected("broker socket kernel metadata"))?;
        let parent = Self::open_parent(
            path.parent()
                .ok_or_else(|| history::rejected("broker socket parent"))?,
        )?;
        let leaf = std::ffi::CString::new(
            path.file_name()
                .ok_or_else(|| history::rejected("broker socket leaf"))?
                .as_bytes(),
        )
        .map_err(|_| history::rejected("broker socket leaf"))?;
        let raw = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                leaf.as_ptr(),
                libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if raw < 0 {
            return Err(history::rejected("broker socket leaf FD"));
        }
        let file = unsafe { File::from_raw_fd(raw) };
        let m = file
            .metadata()
            .map_err(|_| history::rejected("broker socket metadata"))?;
        let this = Self {
            listener,
            listener_identity: (listener_metadata.dev(), listener_metadata.ino()),
            file,
            parent,
            path,
            device: m.dev(),
            inode: m.ino(),
            group,
            mode,
        };
        this.recheck()?;
        Ok(this)
    }
    fn recheck(&self) -> Result<()> {
        if self
            .listener
            .local_addr()
            .map_err(|_| history::rejected("broker listener address"))?
            .as_pathname()
            != Some(self.path.as_path())
        {
            return Err(history::rejected("broker original listener changed"));
        }
        let kernel = File::from(std::os::fd::OwnedFd::from(
            self.listener
                .try_clone()
                .map_err(|_| history::rejected("broker listener original FD"))?,
        ))
        .metadata()
        .map_err(|_| history::rejected("broker listener kernel metadata"))?;
        if !kernel.file_type().is_socket() || (kernel.dev(), kernel.ino()) != self.listener_identity
        {
            return Err(history::rejected("broker listener kernel identity changed"));
        }
        let current = Self::open_parent(
            self.path
                .parent()
                .ok_or_else(|| history::rejected("broker socket parent"))?,
        )?;
        let before = self
            .parent
            .metadata()
            .map_err(|_| history::rejected("broker socket parent metadata"))?;
        let after = current
            .metadata()
            .map_err(|_| history::rejected("broker socket parent metadata"))?;
        if (before.dev(), before.ino()) != (after.dev(), after.ino()) {
            return Err(history::rejected("broker socket parent changed"));
        }
        for m in [self.file.metadata(), std::fs::symlink_metadata(&self.path)] {
            let m = m.map_err(|_| history::rejected("broker socket disappeared"))?;
            if !m.file_type().is_socket()
                || m.uid() != 0
                || m.gid() != self.group
                || m.mode() & 0o7777 != self.mode
                || m.nlink() != 1
                || (m.dev(), m.ino()) != (self.device, self.inode)
            {
                return Err(history::rejected("broker socket identity changed"));
            }
        }
        Ok(())
    }
}

impl RootSnapshotBroker {
    pub fn epoch_id(&self) -> Uuid {
        self.installed.epoch_id()
    }
    pub fn binding_id(&self) -> Uuid {
        self.installed.binding_id()
    }
    pub fn descriptor_bytes(&self) -> &[u8] {
        &self.descriptor
    }
    pub fn installation_descriptor_bytes(&self) -> &[u8] {
        &self.descriptor
    }
    pub fn root_marker_bytes(&self) -> &[u8] {
        self.installed.marker_bytes()
    }
    pub fn shared_budget(&self) -> &SharedJournalBudget {
        self.installed.shared_budget()
    }
    pub fn revalidate(&self) -> Result<()> {
        self.installed
            .revalidate()
            .map_err(|_| history::rejected("installed broker binding changed"))?;
        for socket in &self.sockets {
            socket.recheck()?;
        }
        let work = self
            .work
            .lock()
            .map_err(|_| history::rejected("broker lifecycle poisoned"))?;
        if work.as_ref().is_none_or(|work| work.owner().is_held()) {
            return Err(history::rejected("broker original lifecycle HOLD"));
        }
        #[cfg(feature = "b1-source-peer")]
        if let Some(fixed) = &work
            .as_ref()
            .expect("checked original work")
            .owner()
            .b1_work
        {
            fixed.revalidate()?;
        }
        Ok(())
    }
    pub async fn accept_signal(&self, stream: UnixStream) -> Result<BrokerSignalResult> {
        self.accept_inner(stream, None).await
    }
    pub async fn accept_signal_for_capture(
        &self,
        stream: UnixStream,
        transition: BeforeWriteTransition,
    ) -> Result<BrokerSignalResult> {
        self.accept_inner(stream, Some(transition)).await
    }
    async fn accept_inner(
        &self,
        stream: UnixStream,
        transition: Option<BeforeWriteTransition>,
    ) -> Result<BrokerSignalResult> {
        self.revalidate()?;
        let work = &self.sockets[0];
        if stream
            .local_addr()
            .map_err(|_| history::rejected("work stream local address"))?
            .as_pathname()
            != Some(work.path.as_path())
        {
            return Err(history::rejected(
                "signal does not belong to original work socket",
            ));
        }
        let (uid, gid, process_id) = super::wire::peer(&stream)
            .map_err(|_| history::rejected("work peer credentials unavailable"))?;
        if (uid, gid) != (65532, 65532) || process_id <= 0 {
            return Err(history::rejected("work peer is not restricted client"));
        }
        self.shared_budget()
            .charge(32 * 1024, 8)
            .map_err(|_| super::ObservationError::Capacity)?;
        // The bounded synchronous framing call cannot be detached by dropping
        // an async future: every recvmsg completes before the first await.
        let raw = super::wire::read_frame(&stream)
            .map_err(|_| history::rejected("work frame rejected"))?;
        let (request, lease_ms) = parse_signal(&raw)?;
        self.shared_budget()
            .charge(1024, 1)
            .map_err(|_| super::ObservationError::Capacity)?;
        {
            let mut peers = self
                .peers
                .lock()
                .map_err(|_| history::rejected("work peer observations poisoned"))?;
            if peers.len() == peers.capacity() {
                self.shared_budget()
                    .charge(
                        (peers.len() * std::mem::size_of::<PeerObservation>()) as u64,
                        0,
                    )
                    .map_err(|_| super::ObservationError::Capacity)?;
                let reserve = peers.len().max(8);
                peers
                    .try_reserve_exact(reserve)
                    .map_err(|_| super::ObservationError::Capacity)?;
            }
            peers.push(PeerObservation {
                request_id: request,
                process_id,
                uid,
                gid,
                work_socket_dev: work.device,
                work_socket_ino: work.inode,
            });
        }
        let result = self.run_signal(request, lease_ms, transition).await?;
        self.shared_budget()
            .charge(4096, 1)
            .map_err(|_| super::ObservationError::Capacity)?;
        let disposition = match result {
            BrokerSignalResult::NoWork => "NO_WORK",
            BrokerSignalResult::Completed => "COMPLETED",
            BrokerSignalResult::RevokedBeforeWrite => "REVOKED_BEFORE_WRITE",
            BrokerSignalResult::NotAcquired => "NOT_ACQUIRED",
        };
        let reply=serde_json::to_vec(&serde_json::json!({"format_version":2,"epoch":self.epoch_id(),"request_id":request,"disposition":disposition}))
            .map_err(|_|super::ObservationError::Capacity)?;
        // A failed redacted client reply is not a lost database COMMIT. The
        // original known ledger is retained and no business operation replays.
        let _ = super::wire::write_frame(&stream, &reply);
        Ok(result)
    }
    pub fn request_quiesce(&self) -> Result<()> {
        self.quiescing.store(true, Ordering::SeqCst);
        let work = self
            .work
            .lock()
            .map_err(|_| history::rejected("broker lifecycle poisoned"))?;
        work.as_ref()
            .ok_or_else(|| history::rejected("broker already consumed"))?
            .owner()
            .request_quiesce()
    }

    /// Management-only actual claim/transition/revoke. The worker protocol
    /// cannot select this path or pass an actor, job, token or effect finding.
    pub async fn revoke_next_before_write(
        &self,
        request_id: Uuid,
        lease_ms: u64,
        transition: BeforeWriteTransition,
    ) -> Result<BrokerSignalResult> {
        self.run_signal(request_id, lease_ms, Some(transition))
            .await
    }

    async fn run_signal(
        &self,
        request_id: Uuid,
        lease_ms: u64,
        before_write: Option<BeforeWriteTransition>,
    ) -> Result<BrokerSignalResult> {
        if request_id.get_version_num() != 4
            || !(1..=300_000).contains(&lease_ms)
            || self.quiescing.load(Ordering::SeqCst)
        {
            return Err(history::rejected("broker signal unavailable"));
        }
        for socket in &self.sockets {
            socket.recheck()?;
        }
        self.installed
            .revalidate()
            .map_err(|_| history::rejected("broker installed binding changed"))?;
        let owner = self
            .work
            .lock()
            .map_err(|_| history::rejected("broker lifecycle poisoned"))?
            .as_ref()
            .ok_or_else(|| history::rejected("broker already consumed"))?
            .owner();
        if owner.is_held() {
            return Err(history::rejected("broker original lifecycle HOLD"));
        }
        owner.preflight()?;
        if let Some((previous_lease, result)) = self
            .settled
            .lock()
            .map_err(|_| history::rejected("broker settled requests poisoned"))?
            .get(&request_id)
        {
            if *previous_lease != lease_ms {
                return Err(history::rejected("broker request policy changed"));
            }
            return Ok(*result);
        }
        owner
            .budget
            .charge(4096, 4)
            .map_err(|_| super::ObservationError::Capacity)?;
        let writer = self
            .installed
            .custody()
            .writer()
            .map_err(|_| history::rejected("broker writer custody busy"))?;
        let retention = self
            .installed
            .custody()
            .writer()
            .map_err(|_| history::rejected("broker owner custody busy"))?;
        let runtime_pool = self.runtime.clone();
        let jobs = self.jobs.clone();
        let quiescing = self.quiescing.clone();
        let settled = self.settled.clone();
        let pending = self
            .work
            .lock()
            .map_err(|_| history::rejected("broker lifecycle poisoned"))?
            .as_mut()
            .ok_or_else(|| history::rejected("broker already consumed"))?
            .start_continuation(
                request_id,
                request_id,
                Duration::from_secs(60),
                Some(retention),
                move |owner, slot| async move {
                    slot.live()?;
                    let result = async {
                        use super::attempts::{self, ClaimResult};
                        // Selection is itself supervised by the original owner,
                        // registered before its first SQL/await and under the same
                        // absolute deadline as claim, append and publication.
                        let mut connection = owner.acquire_runtime(&slot).await?;
                        let ids = super::history::runtime::settle(&slot, async {
                            match connection.as_mut() {
                                Some(connection) => {
                                    crate::JobStore::runnable_snapshot_ids_on(connection, 1).await
                                }
                                None => {
                                    crate::JobStore::new(runtime_pool.clone())
                                        .runnable_snapshot_ids(1)
                                        .await
                                }
                            }
                            .map_err(|_| history::rejected("broker actual runnable scan failed"))
                        })
                        .await;
                        let ids = owner.close_connection(&slot, connection, ids).await?;
                        let Some(job) = ids.into_iter().next() else {
                            return Ok(BrokerSignalResult::NoWork);
                        };
                        if !owner.bind_selected_job(&slot, request_id, job)? {
                            return Ok(BrokerSignalResult::NoWork);
                        }
                        owner
                            .budget
                            .charge(4096, 1)
                            .map_err(|_| super::ObservationError::Capacity)?;
                        let mut connection = owner.acquire_runtime(&slot).await?;
                        let selected = super::history::runtime::settle(&slot, async {
                            let selected: Option<attempts::BusinessRequest> =
                                match connection.as_mut() {
                                    Some(connection) => {
                                        sqlx::query_as(attempts::BUSINESS_REQUEST)
                                            .bind(job)
                                            .fetch_optional(&mut **connection)
                                            .await?
                                    }
                                    None => {
                                        sqlx::query_as(attempts::BUSINESS_REQUEST)
                                            .bind(job)
                                            .fetch_optional(&runtime_pool)
                                            .await?
                                    }
                                };
                            selected
                                .ok_or_else(|| history::rejected("actual business request missing"))
                        })
                        .await;
                        let selected = owner.close_connection(&slot, connection, selected).await?;
                        let expected = {
                            let ledger = jobs
                                .lock()
                                .map_err(|_| history::rejected("broker known ledger poisoned"))?;
                            ledger
                                .get(&job)
                                .map(|known| known.summary.current_rows_sha256.clone())
                        };
                        let result = attempts::claim(
                            owner.clone(),
                            slot.clone(),
                            selected,
                            request_id,
                            lease_ms as i64,
                            expected,
                        )
                        .await?;
                        let mut claimed = match result {
                            ClaimResult::NotAcquired { after, reference } => {
                                let digest = attempts::rows_digest(&after, &owner)?;
                                let mut ledger = jobs.lock().map_err(|_| {
                                    history::rejected("broker known ledger poisoned")
                                })?;
                                let known = ledger.get_mut(&job).ok_or_else(|| {
                                    history::rejected("unacquired row has no original history")
                                })?;
                                // Preserve all prior revoked attempts; the real ceiling
                                // mutation adds no invented fourth claim/token.
                                known.summary.current_rows_sha256 = digest;
                                known.summary.last_event = Reference::from_record(reference);
                                known._proof = ClosedProof::NotAcquired(reference);
                                return Ok(BrokerSignalResult::NotAcquired);
                            }
                            ClaimResult::Acquired(claimed) => *claimed,
                        };
                        if let Some(transition) = before_write {
                            let operation = match transition {
                                BeforeWriteTransition::None => None,
                                BeforeWriteTransition::Renew => {
                                    Some(attempts::Transition::Renew(lease_ms as i64))
                                }
                                BeforeWriteTransition::TransientFailure => {
                                    Some(attempts::Transition::TransientFailure)
                                }
                                BeforeWriteTransition::TerminalFailure => {
                                    Some(attempts::Transition::TerminalFailure)
                                }
                            };
                            if let Some(operation) = operation {
                                attempts::transition(&owner, &slot, &mut claimed, operation)
                                    .await?;
                            }
                        }
                        let lease = claimed.lease();
                        let claim_reference = claimed.records().1;
                        let encoded =
                            history::encode(&claimed.output().after.outbox, &owner.budget)?;
                        let binding = history::digest(&encoded, &owner.budget)?;
                        let current_digest =
                            attempts::rows_digest(&claimed.output().after, &owner)?;
                        let mut attempt = AttemptSummary {
                            attempt: lease.attempt_count,
                            claim_token: lease.token,
                            lease_expires_at: lease.lease_expires_at,
                            claim_record: Reference::from_record(claim_reference),
                            revoked_before_write: None,
                            write_issued: None,
                            business_commit: None,
                            write_closed: None,
                        };
                        let mut published = None;
                        let revoked = if before_write.is_some() || quiescing.load(Ordering::SeqCst)
                        {
                            slot.reserve_revocation()?;
                            drop(claimed);
                            true
                        } else {
                            match super::writer::publish_claimed(
                                owner.clone(),
                                slot.clone(),
                                claimed,
                                runtime_pool,
                                writer,
                            )
                            .await
                            {
                                Ok(component) => {
                                    attempt.lease_expires_at = component.final_lease_expiry();
                                    let refs = component.references();
                                    attempt.write_issued = Some(Reference::from_record(refs.0));
                                    attempt.business_commit = Some(Reference::from_record(refs.2));
                                    attempt.write_closed = Some(Reference::from_record(refs.3));
                                    published = Some(component);
                                    false
                                }
                                Err(super::ObservationError::RevokedBeforeWrite) => {
                                    slot.reserve_revocation()?;
                                    true
                                }
                                Err(error) => return Err(error),
                            }
                        };
                        let (digest, proof, disposition) = if revoked {
                            if slot.issued() {
                                return Err(history::rejected(
                                    "issued attempt can never become no-write",
                                ));
                            }
                            #[derive(Serialize)]
                            struct Revoke {
                                job_id: Uuid,
                                attempt: i32,
                                claim_record_sha256: String,
                                slot_generation: Uuid,
                                reason: &'static str,
                            }
                            let encoded = attempts::event(
                                &owner,
                                &slot,
                                "REVOKED_BEFORE_WRITE",
                                &Revoke {
                                    job_id: job,
                                    attempt: lease.attempt_count,
                                    claim_record_sha256: Reference::from_record(claim_reference)
                                        .sha256,
                                    slot_generation: request_id,
                                    reason: "management_quiesce",
                                },
                            )
                            .await?;
                            let reference = owner.append(&slot, encoded).await?;
                            slot.live()?;
                            attempt.revoked_before_write = Some(Reference::from_record(reference));
                            (
                                current_digest,
                                ClosedProof::Revoked(reference),
                                BrokerSignalResult::RevokedBeforeWrite,
                            )
                        } else {
                            let mut connection = owner.acquire_broker(&slot).await?;
                            let observed = async {
                                let mut tx =
                                    attempts::begin(&owner, &slot, &mut connection).await?;
                                let observed = super::history::runtime::settle(
                                    &slot,
                                    super::transaction::read_locked(
                                        &mut tx,
                                        &owner.observation,
                                        job,
                                    ),
                                )
                                .await;
                                let rollback =
                                    super::history::runtime::settle_started(&slot, async {
                                        Ok(tx.rollback().await?)
                                    })
                                    .await;
                                let observed = observed?.ok_or_else(|| {
                                    history::rejected("published source job missing")
                                })?;
                                rollback?;
                                Ok(observed)
                            }
                            .await;
                            let observed =
                                owner.close_connection(&slot, connection, observed).await?;
                            slot.live()?;
                            let rows: history::Rows = observed.into();
                            let digest = attempts::rows_digest(&rows, &owner)?;
                            (
                                digest,
                                ClosedProof::Committed(
                                    published.expect("known original publication"),
                                ),
                                BrokerSignalResult::Completed,
                            )
                        };
                        owner
                            .budget
                            .charge(8192, 4)
                            .map_err(|_| super::ObservationError::Capacity)?;
                        let mut ledger = jobs
                            .lock()
                            .map_err(|_| history::rejected("broker known ledger poisoned"))?;
                        let last_reference = match &proof {
                            ClosedProof::Revoked(reference)
                            | ClosedProof::NotAcquired(reference) => *reference,
                            ClosedProof::Committed(component) => component.references().3,
                        };
                        if let Some(known) = ledger.get_mut(&job) {
                            if known.summary.attempts.len() + 1 != attempt.attempt as usize
                                || known.summary.source_job_binding_sha256 != binding
                                || known.summary.attempts.iter().any(|a| {
                                    a.revoked_before_write.is_none() || a.write_issued.is_some()
                                })
                            {
                                return Err(history::rejected("broker full claim history gap"));
                            }
                            known.summary.attempts.push(attempt);
                            known.summary.current_rows_sha256 = digest;
                            known.summary.last_event = Reference::from_record(last_reference);
                            known._proof = proof;
                        } else {
                            if attempt.attempt != 1 {
                                return Err(history::rejected(
                                    "broker first known claim is not one",
                                ));
                            }
                            ledger.insert(
                                job,
                                KnownJob {
                                    summary: JobSummary {
                                        job_id: job,
                                        source_job_binding_sha256: binding,
                                        current_rows_sha256: digest,
                                        attempts: vec![attempt],
                                        last_event: Reference::from_record(last_reference),
                                    },
                                    _proof: proof,
                                },
                            );
                        }
                        Ok(disposition)
                    }
                    .await;
                    if let Ok(disposition) = result {
                        slot.live()?;
                        owner
                            .budget
                            .charge(256, 1)
                            .map_err(|_| super::ObservationError::Capacity)?;
                        settled
                            .lock()
                            .map_err(|_| history::rejected("broker settled requests poisoned"))?
                            .insert(request_id, (lease_ms, disposition));
                    }
                    result
                },
            )?;
        pending.await
    }

    pub async fn quiesce_for_capture(
        self,
        backup_id: Uuid,
        capture_id: Uuid,
    ) -> Result<QuiescedSnapshotEpochV2> {
        if backup_id.get_version_num() != 4 || capture_id.get_version_num() != 4 {
            return Err(history::rejected("capture identity"));
        }
        self.request_quiesce()?;
        let work = self
            .work
            .lock()
            .map_err(|_| history::rejected("broker lifecycle poisoned"))?
            .take()
            .ok_or_else(|| history::rejected("broker already consumed"))?;
        #[cfg(feature = "b1-source-peer")]
        if let Some(fixed) = &work.owner().b1_work {
            let fixed = fixed.clone();
            let original = self
                .b1_original
                .lock()
                .map_err(|_| history::rejected("B1 original process owner poisoned"))?
                .take()
                .ok_or_else(|| history::rejected("B1 original process owner missing"))?;
            let mut view =
                super::b1_peer::quiesce_b1(self, work, fixed, backup_id, capture_id).await?;
            view.b1_original = Some(original);
            return Ok(view);
        }
        let pending = work.start_quiesce(move |work, owner, slot| {
            self.quiesce_owned(work, owner, slot, backup_id, capture_id)
        })?;
        pending.await
    }
    pub(super) async fn quiesce_owned(
        self,
        mut work: EpochWork,
        owner: Arc<Original>,
        slot: Arc<super::history::runtime::Slot>,
        backup_id: Uuid,
        capture_id: Uuid,
    ) -> Result<QuiescedSnapshotEpochV2> {
        work.drain().await?;
        if owner.is_held() {
            return Err(history::rejected(
                "broker unknown original operation cannot capture",
            ));
        }
        #[cfg(feature = "b1-source-peer")]
        let lifetime = self
            .b1_lifetime
            .lock()
            .map_err(|_| history::rejected("B1 pool owner poisoned"))?
            .take();
        #[cfg(feature = "b1-source-peer")]
        let fixed_pools = lifetime.is_some();
        #[cfg(not(feature = "b1-source-peer"))]
        let fixed_pools = false;
        #[cfg(feature = "b1-source-peer")]
        if let Some(mut lifetime) = lifetime {
            super::history::runtime::settle_started(&slot, lifetime.close_pools()).await?;
        }
        if !fixed_pools {
            super::history::runtime::settle_started(&slot, async {
                owner.pool.close().await;
                Ok(())
            })
            .await?;
            super::history::runtime::settle_started(&slot, async {
                self.runtime.close().await;
                Ok(())
            })
            .await?;
        }
        slot.live()?;
        let ledger = {
            let jobs = self
                .jobs
                .lock()
                .map_err(|_| history::rejected("broker known ledger poisoned"))?;
            owner
                .budget
                .charge((jobs.len() * std::mem::size_of::<&JobSummary>()) as u64, 0)
                .map_err(|_| super::ObservationError::Capacity)?;
            for known in jobs.values() {
                let original = match &known._proof {
                    ClosedProof::Revoked(reference) | ClosedProof::NotAcquired(reference) => {
                        *reference
                    }
                    ClosedProof::Committed(component) => component.references().3,
                };
                if known.summary.last_event.sequence != original.head().sequence {
                    return Err(history::rejected("known ledger proof differs"));
                }
            }
            history::encode(
                &LedgerView {
                    format_version: 2,
                    capability: "snapshot_effect_known_ledger_v2",
                    storage_encoding: STORAGE_ENCODING,
                    epoch_id: owner.epoch,
                    binding_id: owner.binding,
                    jobs: jobs.values().map(|known| &known.summary).collect(),
                },
                &owner.budget,
            )?
        };
        #[derive(Serialize)]
        struct CutRecord {
            capture_id: Uuid,
            source_backup_id: Uuid,
            previous_head_sequence: u64,
            previous_head_sha256: String,
            descriptor_sha256: String,
            ledger_sha256: String,
            active_writers: u64,
            active_operations: u64,
            pools_closed: bool,
            observed_work_peers_sha256: String,
        }
        let previous = owner.head(&slot).await?;
        let descriptor_sha256 = history::digest(&self.descriptor, &owner.budget)?;
        let ledger_sha256 = history::digest(&ledger, &owner.budget)?;
        let peers = self
            .peers
            .into_inner()
            .map_err(|_| history::rejected("work peers poisoned"))?;
        let encoded_peers = history::encode(&peers, &owner.budget)?;
        let peers_sha256 = history::digest(&encoded_peers, &owner.budget)?;
        let encoded = super::attempts::event(
            &owner,
            &slot,
            "CAPTURE_CUT",
            &CutRecord {
                capture_id,
                source_backup_id: backup_id,
                previous_head_sequence: previous.head().sequence,
                previous_head_sha256: Reference::from_record(previous).sha256,
                descriptor_sha256: descriptor_sha256.clone(),
                ledger_sha256: ledger_sha256.clone(),
                active_writers: 0,
                active_operations: 0,
                pools_closed: true,
                observed_work_peers_sha256: peers_sha256,
            },
        )
        .await?;
        let head = owner.append(&slot, encoded).await?;
        slot.live()?;
        let custody = self
            .installed
            .custody()
            .try_capture()
            .map_err(|_| history::rejected("capture same-inode EX busy"))?;
        let cut = history::encode(
            &CutView {
                format_version: 2,
                capability: "snapshot_effect_capture_cut_v2",
                storage_encoding: STORAGE_ENCODING,
                backup_id,
                capture_id,
                epoch_id: owner.epoch,
                binding_id: owner.binding,
                previous_head: Reference::from_record(previous),
                cut_record: Reference::from_record(head),
                descriptor_sha256,
                ledger_sha256,
                active_writers: 0,
                active_operations: 0,
                pools_closed: true,
                observed_work_peers: peers,
            },
            &owner.budget,
        )?;
        for socket in &self.sockets {
            socket.recheck()?;
        }
        let view = QuiescedSnapshotEpochV2 {
            #[cfg(feature = "b1-source-peer")]
            b1_original: self
                .b1_original
                .into_inner()
                .map_err(|_| history::rejected("B1 original process owner poisoned"))?,
            _work: work,
            owner,
            installed: self.installed,
            custody,
            descriptor: self.descriptor,
            ledger,
            cut,
            head,
            _sockets: self.sockets,
        };
        view.revalidate()?;
        #[cfg(feature = "b1-source-peer")]
        if let Some(fixed) = &view.owner.b1_work {
            fixed.bind_capture_generation()?;
        }
        Ok(view)
    }
    /// Fixed writer implementation description, not registry enrollment.
    pub fn writer_contract_sha256() -> String {
        learning_core::hex_digest(
            concat!(
                include_str!("writer.rs"),
                include_str!("writer/publication.rs"),
                include_str!("attempts.rs"),
                include_str!("transition.rs")
            )
            .as_bytes(),
        )
    }

    /// Explicit trusted management install. The dedicated admin pool is
    /// consumed and closed before this object can perform any business work.
    /// The running broker retains only its original C0/runtime pools.
    pub async fn install_fresh(
        installed: InstalledSnapshotBinding,
        admin_install: PgPool,
        broker: PgPool,
        runtime: PgPool,
        source: BrokerSourceDescription,
        sockets: (&UnixListener, &UnixListener),
    ) -> Result<Self> {
        Self::install_with_originals(
            installed,
            admin_install,
            broker,
            runtime,
            source,
            sockets,
            #[cfg(feature = "b1-source-peer")]
            (None, None),
        )
        .await
    }
    #[cfg(feature = "b1-source-peer")]
    pub async fn install_fresh_b1_peer(
        original: Arc<super::B1InstalledSourceProcess>,
        installed: InstalledSnapshotBinding,
        source: BrokerSourceDescription,
        sockets: (&UnixListener, &UnixListener),
    ) -> Result<Self> {
        super::b1_peer::install_fresh(original, installed, source, sockets).await
    }
    #[cfg(feature = "b1-source-peer")]
    pub(super) async fn install_b1_owned(
        installed: InstalledSnapshotBinding,
        fixed: Arc<super::b1_peer::FixedWorkPools>,
        source: BrokerSourceDescription,
        sockets: [UnixListener; 2],
        lifetime: super::b1_peer::WorkLifetime,
    ) -> Result<Self> {
        let admin = fixed.admin_pool()?;
        Self::install_with_originals(
            installed,
            admin,
            fixed.broker_pool(),
            fixed.runtime_pool(),
            source,
            (&sockets[0], &sockets[1]),
            (Some(fixed), Some(lifetime)),
        )
        .await
    }
    async fn install_with_originals(
        mut installed: InstalledSnapshotBinding,
        admin_install: PgPool,
        broker: PgPool,
        runtime: PgPool,
        source: BrokerSourceDescription,
        sockets: (&UnixListener, &UnixListener),
        #[cfg(feature = "b1-source-peer")] b1_originals: (
            Option<Arc<super::b1_peer::FixedWorkPools>>,
            Option<super::b1_peer::WorkLifetime>,
        ),
    ) -> Result<Self> {
        #[cfg(feature = "b1-source-peer")]
        let (fixed, b1_lifetime) = b1_originals;
        let result = Self::install_inner(
            &mut installed,
            &admin_install,
            &broker,
            &runtime,
            &source,
            sockets,
            #[cfg(feature = "b1-source-peer")]
            fixed.as_deref(),
        )
        .await;
        // No success or error adopts an in-flight install session. Closing the
        // original dedicated pool waits for every returned install connection.
        #[cfg(feature = "b1-source-peer")]
        let admin_closed = match &fixed {
            Some(fixed) => fixed.close_install_admin_pool().await,
            None => {
                admin_install.close().await;
                Ok(())
            }
        };
        #[cfg(not(feature = "b1-source-peer"))]
        let admin_closed: Result<()> = {
            admin_install.close().await;
            Ok(())
        };
        let (descriptor, budget, journal) = result?;
        admin_closed?;
        let socket_bindings = vec![
            SocketBinding::observe(sockets.0, 65532, 0o660)?,
            SocketBinding::observe(sockets.1, 0, 0o600)?,
        ];
        let installed = Arc::new(installed);
        let mut work = EpochWork::installed(
            broker,
            installed.clone(),
            journal,
            budget,
            source.source_build_sha256,
        )?;
        #[cfg(feature = "b1-source-peer")]
        if let Some(fixed) = &fixed {
            work.attach_b1(fixed.clone())?;
            fixed.retain_epoch(work.owner())?;
        }
        let owner = work.owner();
        let descriptor_bytes = history::encode(&descriptor, &owner.budget)?;
        let initial_bytes = history::encode(&descriptor, &owner.budget)?;
        let epoch = owner.epoch;
        let pending = work.start_continuation(
            epoch,
            epoch,
            Duration::from_secs(60),
            None,
            move |owner, slot| async move {
                // Fresh original binary header is the genesis; the complete typed
                // Epoch is frame1. All later references name whole-frame hashes.
                let previous = owner.head(&slot).await?;
                if previous.head().sequence != 0 {
                    return Err(history::rejected("installed epoch already started"));
                }
                owner.append(&slot, initial_bytes).await?;
                slot.live()
            },
        )?;
        pending.await?;
        #[cfg(feature = "b1-source-peer")]
        if let Some(fixed) = &fixed {
            // This exact lifecycle owns the naturally closed install pool and
            // the durable frame1 bytes. I0 keeps these originals while the
            // fixed control owner waits for C11's authenticated ACK/record.
            fixed.installed(
                installed.clone(),
                &descriptor_bytes,
                owner.observation.clone(),
            )?;
        }
        Ok(Self {
            #[cfg(feature = "b1-source-peer")]
            b1_lifetime: Mutex::new(b1_lifetime),
            #[cfg(feature = "b1-source-peer")]
            b1_original: Mutex::new(None),
            work: Mutex::new(Some(work)),
            installed,
            runtime,
            descriptor: descriptor_bytes,
            jobs: Arc::new(Mutex::new(BTreeMap::new())),
            settled: Arc::new(Mutex::new(BTreeMap::new())),
            quiescing: Arc::new(AtomicBool::new(false)),
            sockets: socket_bindings,
            peers: Mutex::new(Vec::new()),
        })
    }

    async fn install_inner(
        installed: &mut InstalledSnapshotBinding,
        admin: &PgPool,
        broker: &PgPool,
        runtime: &PgPool,
        source: &BrokerSourceDescription,
        sockets: (&UnixListener, &UnixListener),
        #[cfg(feature = "b1-source-peer")] fixed: Option<&super::b1_peer::FixedWorkPools>,
    ) -> Result<(
        EpochDescriptor,
        SharedJournalBudget,
        learning_assets::root_journal::RootJournalBytes,
    )> {
        #[cfg(feature = "b1-source-peer")]
        if let Some(fixed) = fixed {
            fixed.revalidate()?;
            fixed.match_source_description(source)?;
            if installed.shared_budget().shares_meter_with(&fixed.scan()) {
                return Err(history::rejected(
                    "B1 installed journal shares scan accounting",
                ));
            }
        }
        installed
            .revalidate()
            .map_err(|_| history::rejected("fresh installed binding rejected"))?;
        if source.deployment_id.get_version_num() != 4
            || source.source_group_id.get_version_num() != 4
            || source.source_application_commit.len() != 40
            || !source
                .source_application_commit
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            || source.source_build_sha256.len() != 64
            || !source
                .source_build_sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            || option_env!("KNOWWEAVE_BUILD_ID_SHA256")
                .is_some_and(|value| value != source.source_build_sha256)
        {
            return Err(history::rejected("broker source description differs"));
        }
        let a = broker.connect_options();
        let b = runtime.connect_options();
        let c = admin.connect_options();
        if a.get_username() != super::BROKER_ROLE
            || b.get_username() != "learning_runtime"
            || c.get_username() != "learning_admin"
            || a.get_host() != b.get_host()
            || a.get_host() != c.get_host()
            || a.get_port() != b.get_port()
            || a.get_port() != c.get_port()
            || a.get_socket() != b.get_socket()
            || a.get_socket() != c.get_socket()
            || a.get_database() != b.get_database()
            || a.get_database() != c.get_database()
        {
            return Err(history::rejected(
                "broker actual endpoint configuration differs",
            ));
        }
        let budget = installed.shared_budget().clone();
        budget
            .charge(64 * 1024, 16)
            .map_err(|_| super::ObservationError::Capacity)?;
        if installed.marker_bytes().len() > 4096 {
            return Err(super::ObservationError::Capacity);
        }
        let marker: InstalledRootMarker = serde_json::from_slice(installed.marker_bytes())
            .map_err(|_| history::rejected("installed export marker shape"))?;
        let actual = installed
            .identities()
            .map_err(|_| history::rejected("installed export marker identity"))?;
        if marker.format_version != 2
            || marker.capability != "snapshot_export_root_v2"
            || marker.binding_id != installed.binding_id()
            || marker.deployment_id != source.deployment_id
            || (marker.export_root_dev, marker.export_root_ino) != actual[1]
            || (marker.journal_root_dev, marker.journal_root_ino) != actual[2]
            || (marker.effect_lock_dev, marker.effect_lock_ino) != actual[3]
            || marker.writer_contract_sha256 != Self::writer_contract_sha256()
        {
            return Err(history::rejected("installed export marker binding differs"));
        }
        let mut connection: Option<sqlx::pool::PoolConnection<sqlx::Postgres>> = None;
        #[cfg(feature = "b1-source-peer")]
        if let Some(fixed) = fixed {
            connection = Some(fixed.acquire_admin().await?);
        }
        let observed = async {
        use sqlx::Acquire;
        let mut tx = match connection.as_mut() {
            Some(connection) => (&mut *connection).begin().await?,
            None => admin.begin().await?,
        };
        let observed:Result<(String,i64,String,DateTime<Utc>)>=async {
            let row:(String,String,bool,String,i64,String,DateTime<Utc>)=sqlx::query_as(
                "SELECT session_user::text,current_user::text,d.datdba=(SELECT oid FROM pg_catalog.pg_roles WHERE rolname=session_user),current_database()::text,d.oid::bigint,(pg_catalog.pg_control_system()).system_identifier::text,pg_catalog.pg_postmaster_start_time() FROM pg_catalog.pg_database d WHERE d.datname=current_database()")
                .fetch_one(&mut *tx).await?;
            if row.0!="learning_admin" || row.1!="learning_admin" || !row.2 {return Err(history::rejected("broker bootstrap requires actual database owner LOGIN"));}
            super::writer::validate_publication_observation(&mut tx).await?;
            Ok((row.3,row.4,row.5,row.6))
        }.await;
        let rollback = tx.rollback().await;
        let database = observed?;
        rollback?;
        Ok(database)
        }.await;
        let database = close_bootstrap_connection(
            connection,
            observed,
            #[cfg(feature = "b1-source-peer")]
            fixed,
        )
        .await?;
        let mut connection: Option<sqlx::pool::PoolConnection<sqlx::Postgres>> = None;
        #[cfg(feature = "b1-source-peer")]
        if let Some(fixed) = fixed {
            connection = Some(fixed.acquire_broker().await?);
        }
        let observed = async {
            let mut tx = match connection.as_mut() {
                Some(connection) => super::transaction::begin_borrowed(connection).await?,
                None => super::transaction::begin_owned(broker).await?,
            };
            let observed = super::transition::identity(&mut tx).await;
            let rollback = tx.rollback().await;
            let actual = observed?;
            rollback?;
            Ok(actual)
        }
        .await;
        let actual = close_bootstrap_connection(
            connection,
            observed,
            #[cfg(feature = "b1-source-peer")]
            fixed,
        )
        .await?;
        if actual.database != database.0
            || actual.oid != database.1
            || actual.postmaster != database.3
        {
            return Err(history::rejected("broker bootstrap database changed"));
        }
        let mut connection: Option<sqlx::pool::PoolConnection<sqlx::Postgres>> = None;
        #[cfg(feature = "b1-source-peer")]
        if let Some(fixed) = fixed {
            connection = Some(fixed.acquire_runtime().await?);
        }
        let observed = async {
        use sqlx::Acquire;
        let mut tx = match connection.as_mut() {
            Some(connection) => (&mut *connection).begin().await?,
            None => runtime.begin().await?,
        };
        let observed:Result<()>=async {
            let r:(String,String,String,i64,DateTime<Utc>,Option<String>)=sqlx::query_as("SELECT session_user::text,current_user::text,current_database()::text,(SELECT oid::bigint FROM pg_catalog.pg_database WHERE datname=current_database()),pg_catalog.pg_postmaster_start_time(),system_user")
                .fetch_one(&mut *tx).await?;
            let legacy_authenticated = r.5.as_deref().and_then(|s|s.split_once(':'))
                .is_some_and(|(method,user)|!method.is_empty() && user=="learning_runtime");
            #[cfg(feature = "b1-source-peer")]
            let authenticated = if fixed.is_some() { r.5.as_deref() == Some("peer:root") }
                else { legacy_authenticated };
            #[cfg(not(feature = "b1-source-peer"))]
            let authenticated = legacy_authenticated;
            if r.0!="learning_runtime" || r.1!="learning_runtime" || r.2!=database.0 || r.3!=database.1 || r.4!=database.3
                || !authenticated
            {return Err(history::rejected("broker actual runtime LOGIN differs"));}
            super::writer::validate_publication_observation(&mut tx).await
        }.await;
        let rollback = tx.rollback().await;
        observed?;
        rollback?;
        Ok(())
        }.await;
        close_bootstrap_connection(
            connection,
            observed,
            #[cfg(feature = "b1-source-peer")]
            fixed,
        )
        .await?;
        let work = SocketBinding::observe(sockets.0, 65532, 0o660)?;
        let management = SocketBinding::observe(sockets.1, 0, 0o600)?;
        let identities = installed
            .identities()
            .map_err(|_| history::rejected("broker installed identity"))?;
        let guard = installed
            .custody()
            .try_capture()
            .map_err(|_| history::rejected("fresh export root already busy"))?;
        let mut namespace = Vec::new();
        budget
            .charge((2 * std::mem::size_of::<(String, bool, u64)>()) as u64, 0)
            .map_err(|_| super::ObservationError::Capacity)?;
        namespace
            .try_reserve_exact(2)
            .map_err(|_| super::ObservationError::Capacity)?;
        guard
            .visit_namespace(&budget, &mut |name, directory, size| {
                if namespace.len() == 2 {
                    return Err(io::Error::other("fresh namespace entry limit"));
                }
                budget.charge((name.len() + 64) as u64, 1)?;
                namespace.push((name.to_owned(), directory, size));
                Ok(())
            })
            .map_err(|_| history::rejected("fresh namespace observation failed"))?;
        if namespace.len() != 2
            || namespace[0].0 != "effect.lock"
            || namespace[0].1
            || namespace[0].2 != 0
            || namespace[1].0 != "export-root-v2.json"
            || namespace[1].1
        {
            return Err(history::rejected("installed export root is not fresh"));
        }
        let encoded = history::encode(&namespace, &budget)?;
        let initial_namespace_sha256 = history::digest(&encoded, &budget)?;
        drop(guard);
        // Hash the actual running image, never a caller-supplied binary hash.
        let mut executable = File::open("/proc/self/exe")
            .map_err(|_| history::rejected("broker original executable"))?;
        let size = executable
            .metadata()
            .map_err(|_| history::rejected("broker executable metadata"))?
            .len();
        let executable_metadata = executable
            .metadata()
            .map_err(|_| history::rejected("broker executable metadata"))?;
        if size > 128 * 1024 * 1024 {
            return Err(super::ObservationError::Capacity);
        }
        budget
            .charge(
                size.checked_mul(2)
                    .ok_or(super::ObservationError::Capacity)?,
                1,
            )
            .map_err(|_| super::ObservationError::Capacity)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(size as usize)
            .map_err(|_| super::ObservationError::Capacity)?;
        bytes.resize(size as usize, 0);
        executable
            .read_exact(&mut bytes)
            .map_err(|_| history::rejected("broker executable read"))?;
        let mut extra = [0u8; 1];
        let after = executable
            .metadata()
            .map_err(|_| history::rejected("broker executable metadata"))?;
        if executable
            .read(&mut extra)
            .map_err(|_| history::rejected("broker executable read"))?
            != 0
            || (
                after.dev(),
                after.ino(),
                after.len(),
                after.mtime(),
                after.mtime_nsec(),
                after.ctime(),
                after.ctime_nsec(),
            ) != (
                executable_metadata.dev(),
                executable_metadata.ino(),
                size,
                executable_metadata.mtime(),
                executable_metadata.mtime_nsec(),
                executable_metadata.ctime(),
                executable_metadata.ctime_nsec(),
            )
        {
            return Err(history::rejected("broker executable changed"));
        }
        let broker_binary_sha256 = learning_core::hex_digest(&bytes);
        let descriptor = EpochDescriptor {
            format_version: 2,
            capability: "snapshot_effect_epoch_v2",
            storage_encoding: STORAGE_ENCODING,
            deployment_id: source.deployment_id,
            epoch_id: installed.epoch_id(),
            binding_id: installed.binding_id(),
            source_group_id: source.source_group_id,
            source_application_commit: source.source_application_commit.clone(),
            source_build_sha256: source.source_build_sha256.clone(),
            broker_binary_sha256,
            broker_pid: std::process::id(),
            broker_uid: unsafe { libc::geteuid() },
            broker_gid: unsafe { libc::getegid() },
            broker_binary_dev: executable_metadata.dev(),
            broker_binary_ino: executable_metadata.ino(),
            broker_binary_bytes: size,
            source_database: database.0,
            source_database_oid: database.1,
            source_system_identifier: database.2,
            source_postmaster_start: database.3,
            companion_root_dev: identities[0].0,
            companion_root_ino: identities[0].1,
            export_root_dev: identities[1].0,
            export_root_ino: identities[1].1,
            journal_root_dev: identities[2].0,
            journal_root_ino: identities[2].1,
            effect_lock_dev: identities[3].0,
            effect_lock_ino: identities[3].1,
            initial_namespace_sha256,
            writer_contract_sha256: Self::writer_contract_sha256(),
            work_socket_dev: work.device,
            work_socket_ino: work.inode,
            management_socket_dev: management.device,
            management_socket_ino: management.inode,
        };
        let journal = installed
            .take_original_journal()
            .map_err(|_| history::rejected("broker original journal already transferred"))?;
        Ok((descriptor, budget, journal))
    }
}

/// Original live quiesced view only. Retains EX on the original effect inode,
/// known byte owner, settled owner registry and installed Root binding. No
/// public constructor, Clone, serde, From or new writer method is provided.
pub struct QuiescedSnapshotEpochV2 {
    #[cfg(feature = "b1-source-peer")]
    b1_original: Option<Arc<super::B1InstalledSourceProcess>>,
    _work: EpochWork,
    owner: Arc<Original>,
    installed: Arc<InstalledSnapshotBinding>,
    custody: CustodyCaptureGuard,
    descriptor: Vec<u8>,
    ledger: Vec<u8>,
    cut: Vec<u8>,
    head: RecordRef,
    _sockets: Vec<SocketBinding>,
}
impl QuiescedSnapshotEpochV2 {
    pub fn epoch_id(&self) -> Uuid {
        self.owner.epoch
    }
    pub fn binding_id(&self) -> Uuid {
        self.owner.binding
    }
    pub fn descriptor_bytes(&self) -> &[u8] {
        &self.descriptor
    }
    pub fn ledger_bytes(&self) -> &[u8] {
        &self.ledger
    }
    pub fn cut_bytes(&self) -> &[u8] {
        &self.cut
    }
    pub fn root_marker_bytes(&self) -> &[u8] {
        self.installed.marker_bytes()
    }
    pub fn head(&self) -> (u64, [u8; 32]) {
        let h = self.head.head();
        (h.sequence, h.sha256)
    }
    pub fn shared_budget(&self) -> &SharedJournalBudget {
        &self.owner.budget
    }
    pub fn work_usage(&self) -> Result<JournalBudgetUsage> {
        self.owner
            .budget
            .usage()
            .map_err(|_| super::ObservationError::Capacity)
    }
    pub fn revalidate(&self) -> Result<()> {
        if self.owner.is_held() {
            return Err(history::rejected("quiesced original epoch HOLD"));
        }
        #[cfg(feature = "b1-source-peer")]
        if let Some(fixed) = &self.owner.b1_work {
            fixed.revalidate_process()?;
        }
        self.installed
            .revalidate()
            .map_err(|_| history::rejected("quiesced installed binding changed"))?;
        for socket in &self._sockets {
            socket.recheck()?;
        }
        self.custody
            .revalidate()
            .map_err(|_| history::rejected("quiesced effect lock changed"))
    }
    pub async fn visit_journal(
        &self,
        visitor: &mut (dyn FnMut(&str, u64, &mut dyn Read) -> io::Result<()> + Send),
    ) -> Result<()> {
        self.revalidate()?;
        if self.owner.visit_prefix(visitor).await? != self.head {
            return Err(history::rejected("quiesced original head changed"));
        }
        self.revalidate()
    }
    pub fn visit_namespace(
        &self,
        visitor: &mut dyn FnMut(&str, bool, u64) -> io::Result<()>,
    ) -> Result<()> {
        self.revalidate()?;
        self.custody
            .visit_namespace(&self.owner.budget, visitor)
            .map_err(|_| history::rejected("quiesced namespace unstable"))?;
        self.revalidate()
    }
    pub fn with_verified_package<T>(
        &self,
        job: Uuid,
        token: Uuid,
        digest: &str,
        visitor: impl FnOnce(&SnapshotDirectory) -> std::result::Result<T, SnapshotIoError>,
    ) -> Result<T> {
        self.revalidate()?;
        let result = self
            .custody
            .with_verified_package_with_budget(&self.owner.budget, job, token, digest, visitor)
            .map_err(|_| history::rejected("quiesced actual package rejected"))?;
        self.revalidate()?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires Linux UID0 and private /run; explicit Root OS gate"]
    fn formal_socket_retains_pathname_inode_separately_from_listener_and_refuses_replacement() {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            unsafe { libc::geteuid() },
            0,
            "Root0 native socket component required"
        );
        let root = std::path::Path::new("/run").join(format!("task6-socket-{}", Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = root.join("work.sock");
        let listener = UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let held = SocketBinding::observe(&listener, 0, 0o600).unwrap();
        let named = std::fs::symlink_metadata(&path).unwrap();
        assert_eq!((held.device, held.inode), (named.dev(), named.ino()));
        assert_ne!(
            held.listener_identity,
            (held.device, held.inode),
            "sockfs and filesystem leaf are distinct objects"
        );
        held.recheck().unwrap();
        let moved = root.join("old.sock");
        std::fs::rename(&path, &moved).unwrap();
        let replacement = UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(held.recheck().is_err());
        drop(held);
        drop(listener);
        drop(replacement);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(moved).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn formal_signal_is_exactly_four_fields_and_never_admits_worker_authority() {
        let id = Uuid::new_v4();
        let raw = format!(
            "{{\"format_version\":2,\"command\":\"SIGNAL_NEXT\",\"request_id\":\"{id}\",\"lease_ms\":30000}}"
        );
        assert_eq!(parse_signal(raw.as_bytes()).unwrap(), (id, 30000));
        for field in [
            "actor_id",
            "job_id",
            "token",
            "path",
            "fd",
            "already_committed",
        ] {
            assert!(
                parse_signal(
                    raw.replace("30000}", &format!("30000,\"{field}\":true}}"))
                        .as_bytes()
                )
                .is_err()
            );
        }
        for wrong in [
            raw.replace("\"format_version\":2", "\"format_version\":1"),
            raw.replace("SIGNAL_NEXT", "PUBLISH"),
            raw.replace("30000", "29999"),
            raw.replace("30000", "300001"),
            raw.replace(
                "\"lease_ms\":30000",
                "\"lease_ms\":30000,\"lease_ms\":30000",
            ),
            raw.replace(&id.to_string(), &id.to_string().to_uppercase()),
            raw.replace(&id.to_string(), &Uuid::nil().to_string()),
        ] {
            assert!(parse_signal(wrong.as_bytes()).is_err());
        }
    }
}

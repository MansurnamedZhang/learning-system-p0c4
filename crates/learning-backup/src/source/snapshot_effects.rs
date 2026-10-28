//! Original-epoch snapshot effects capture. None of the public descriptions,
//! JSON records or filesystem paths can construct these capabilities.
mod cutover;
pub(crate) mod files;
pub(crate) mod verify;
pub(crate) mod wire;

use crate::{
    BackupError, FileRecord, ManagementRegistry, SourceBackupConfig, SourceLocalPin,
    full_restore::jobs::{self, JobInventory},
    registry::{EnrolledSource, RegistryLease, RootRecord, SourceGroup, canonical},
};
use files::{Budget, charge, encode};
use learning_assets::{
    FsAssetStore, InstalledSnapshotBinding, backup_fs::BackupDir, root_journal::SharedJournalBudget,
};
use learning_db::snapshot_broker::{
    BeforeWriteTransition, BrokerSignalResult, BrokerSourceDescription, QuiescedSnapshotEpochV2,
    RootSnapshotBroker,
};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};
use std::{
    collections::BTreeMap,
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
};
use uuid::Uuid;
use wire::{Capture, Cut, Descriptor, ENCODING, Ledger, NamespaceEntry, Prefix, hash, invalid};

fn exact_record_names(mut actual: Vec<String>, expected: &[String]) -> bool {
    if actual.len() != expected.len() {
        return false;
    }
    actual.sort_unstable();
    actual == expected
}

struct Enrollment {
    backup_id: Uuid,
    group: SourceGroup,
    roots: BTreeMap<String, RootRecord>,
    registry_authority: Vec<u8>,
    registry_generation: Vec<u8>,
    registry_roots: Vec<(String, Vec<u8>)>,
    companion: BackupDir,
    records: BTreeMap<String, Vec<u8>>,
    budget: Budget,
    journal_budget: SharedJournalBudget,
    binding_id: Uuid,
    epoch_id: Uuid,
    // The normal Root isolation verifier owns this exact original lock FD and
    // installed source inspection. Captured JSON is never substituted for it.
    _isolation: super::endpoint::VerifiedSourceIsolation,
    #[cfg(feature = "offhost-b1")]
    b1: Option<B1Enrollment>,
}

#[cfg(feature = "offhost-b1")]
struct B1Enrollment {
    retained: super::admission::B1RetainedInstallInputs,
    observation: super::binding::B1InstallCutoverObservation,
    local: cutover::B1LocalMounts,
}

#[cfg(feature = "offhost-b1")]
impl B1Enrollment {
    fn recheck(&self) -> Result<(), BackupError> {
        let observed = (|| {
            self.retained.recheck()?;
            self.observation.recheck(self.retained.budget())?;
            self.local.recheck(self.retained.budget())?;
            self.retained.recheck()
        })();
        self.retained.record_result(observed)
    }
}

/// A normal management install under the original registry. There is no
/// `from_broker`, deserializer, public field, Clone, or test-only constructor.
///
/// ```compile_fail
/// let _: learning_backup::EnrolledSnapshotBroker = serde_json::from_str("{}").unwrap();
/// ```
pub struct EnrolledSnapshotBroker {
    broker: RootSnapshotBroker,
    enrollment: Enrollment,
    work_listener: UnixListener,
    _management_listener: UnixListener,
    work_path: PathBuf,
    management_path: PathBuf,
    _socket_root: BackupDir,
}

impl EnrolledSnapshotBroker {
    /// Only the fixed crate-private M3 actor calls this original-operation
    /// branch. It supplies its actual process Arc and already held asset store,
    /// never pools, config, paths, DATA or a caller-selected auth mode.
    #[cfg(feature = "offhost-b1")]
    #[allow(
        dead_code,
        reason = "S5 original B1 install awaits its pending original M3 consumer; no runtime wiring or settlement credit"
    )]
    pub(crate) async fn install_fresh_b1(
        peer: std::sync::Arc<learning_db::snapshot_broker::B1InstalledSourceProcess>,
        assets: &FsAssetStore,
    ) -> Result<Self, BackupError> {
        let observed = Self::install_fresh_b1_original(std::sync::Arc::clone(&peer), assets).await;
        if observed.is_err() {
            // This is outside every DB owner Book slot. The actual M3 entry
            // also retains/joins this future on cancellation before ending its
            // runtime; an Err is never a drop-to-reinstall acknowledgement.
            let _ = peer.close_original().await;
        }
        observed
    }

    #[cfg(feature = "offhost-b1")]
    async fn install_fresh_b1_original(
        peer: std::sync::Arc<learning_db::snapshot_broker::B1InstalledSourceProcess>,
        assets: &FsAssetStore,
    ) -> Result<Self, BackupError> {
        let mut admission =
            super::admission::SourceAdmission::try_acquire_b1(std::sync::Arc::clone(&peer)).await?;
        let registry = ManagementRegistry::open_installed_b1(&admission)?;
        let lease = registry.try_lock()?;
        let enrolled = lease.lookup_source_group_b1(&mut admission, assets).await?;
        let config = admission.b1_configuration(&enrolled)?;
        let isolation = admission.b1_isolation(&config, true)?;
        admission.revalidate()?;
        lease.recheck()?;
        let deployment = Uuid::parse_str(&enrolled.group.deployment_id).map_err(|_| invalid())?;
        let group = Uuid::parse_str(&enrolled.group.group_id).map_err(|_| invalid())?;
        let budget = std::sync::Arc::clone(&lease.budget);
        let journal_budget = SharedJournalBudget::default();
        if journal_budget.shares_meter_with(&peer.scan_accounting_meter()) {
            return Err(invalid());
        }
        let authority = encode(&lease.authority, &budget, &journal_budget)?;
        let generation = encode(lease.generation(), &budget, &journal_budget)?;
        let mut roots = Vec::new();
        for root in enrolled.roots.values() {
            roots.push((
                format!("source-roots/{}.json", root.enrollment_id),
                encode(root, &budget, &journal_budget)?,
            ));
        }
        let source_group_bytes = lease
            .metadata
            .get(&format!("groups/{}.json", enrolled.group.group_id))
            .ok_or_else(invalid)?
            .0
            .len();
        let clone_work = roots
            .iter()
            .try_fold(source_group_bytes, |n, (name, raw)| {
                n.checked_add(name.len())
                    .and_then(|n| n.checked_add(raw.len()))
                    .ok_or_else(invalid)
            })?;
        budget
            .lock()
            .map_err(|_| invalid())?
            .charge(clone_work as u64)?;
        budget.lock().map_err(|_| invalid())?.entries(10)?;
        let source_group = enrolled.group.clone();
        let source_roots = enrolled.roots.clone();
        // The actual admin closes before install and its exact Install inputs
        // move into this retained owner instead of becoming copied permission.
        let retained = admission.close_b1_install().await?;
        retained.recheck_enrollment(&lease, &enrolled)?;
        drop(lease);
        let binding_id = Uuid::new_v4();
        let epoch_id = Uuid::new_v4();
        budget.lock().map_err(|_| invalid())?.charge(4096)?;
        budget.lock().map_err(|_| invalid())?.entries(64)?;
        let (socket_root, work_listener, management_listener, work_path, management_path) =
            create_sockets(binding_id, epoch_id)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        retained.recheck()?;
        let (local, companion_fd, export_fd) =
            cutover::B1LocalMounts::open(&source_roots, &budget, deadline)?;
        retained.recheck()?;
        let installed = InstalledSnapshotBinding::install_fresh_empty_mounts(
            companion_fd,
            export_fd,
            deployment,
            epoch_id,
            binding_id,
            &RootSnapshotBroker::writer_contract_sha256(),
            journal_budget.clone(),
        )
        .map_err(|_| invalid())?;
        let broker = RootSnapshotBroker::install_fresh_b1_peer(
            std::sync::Arc::clone(&peer),
            installed,
            BrokerSourceDescription {
                deployment_id: deployment,
                source_group_id: group,
                source_application_commit: source_group.application_commit.clone(),
                source_build_sha256: source_group.application_build_sha256.clone(),
            },
            (&work_listener, &management_listener),
        )
        .await
        .map_err(|_| invalid())?;
        // The approved C11 DB I0 must have received/authenticated the explicit
        // ACK and held full observation record before this actual return.
        // Independently re-read that same fixed record; no new getter mints it.
        retained.recheck()?;
        super::binding::prepay_canonical_data(broker.descriptor_bytes(), &budget)?;
        let descriptor: Descriptor = serde_json::from_slice(broker.descriptor_bytes())?;
        descriptor_matches(&descriptor, &source_group)?;
        if descriptor.binding_id != binding_id || descriptor.epoch_id != epoch_id {
            return Err(invalid());
        }
        let observation = retained.install_observation(&broker)?;
        local.recheck(&budget)?;
        let mut actual = observation.host_data(&budget)?;
        budget
            .lock()
            .map_err(|_| invalid())?
            .charge((broker.descriptor_bytes().len() as u64) * 3 + 4096)?;
        let object = actual.as_object_mut().ok_or_else(invalid)?;
        object.insert("process_id".into(), json!(descriptor.broker_pid));
        object.insert("uid".into(), json!(descriptor.broker_uid));
        object.insert(
            "binary_sha256".into(),
            json!(descriptor.broker_binary_sha256),
        );
        object.insert(
            "application_commit".into(),
            json!(descriptor.source_application_commit),
        );
        object.insert("build_sha256".into(), json!(descriptor.source_build_sha256));
        let companion = BackupDir::open_trusted_private_root(Path::new(
            InstalledSnapshotBinding::COMPANION_PATH,
        ))?;
        let mut enrollment = Enrollment {
            backup_id: config.backup_id,
            group: source_group,
            roots: source_roots,
            registry_authority: authority,
            registry_generation: generation,
            registry_roots: roots,
            companion,
            records: BTreeMap::new(),
            budget,
            journal_budget,
            binding_id,
            epoch_id,
            _isolation: isolation,
            b1: Some(B1Enrollment {
                retained,
                observation,
                local,
            }),
        };
        enrollment.install_records(&descriptor, broker.root_marker_bytes(), &actual)?;
        // The original120s cutover deadline is checked through record sync;
        // later rechecks retain originals without starting a new TTL.
        enrollment
            .b1
            .as_mut()
            .expect("actual B1 enrollment")
            .local
            .finish_install(&enrollment.budget)?;
        enrollment.recheck_records()?;
        broker.revalidate().map_err(|_| invalid())?;
        Ok(Self {
            broker,
            enrollment,
            work_listener,
            _management_listener: management_listener,
            work_path,
            management_path,
            _socket_root: socket_root,
        })
    }
    /// Fresh installation is deliberately explicit. The original install pool
    /// is consumed/closed by the DB owner before this wrapper can accept work.
    pub async fn install_fresh(
        admin_install: PgPool,
        broker_pool: PgPool,
        runtime_pool: PgPool,
        assets: &FsAssetStore,
        config: &SourceBackupConfig,
    ) -> Result<Self, BackupError> {
        if unsafe { libc::geteuid() } != 0 || unsafe { libc::getegid() } != 0 {
            return Err(invalid());
        }
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=INSTALL_ADMISSION_ACQUIRE_BEFORE");
        let mut admission = super::admission::SourceAdmission::try_acquire(
            &admin_install,
            &config.expected_database,
        )
        .await?;
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=INSTALL_ADMISSION_ACQUIRE_AFTER");
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=INSTALL_ADMIT_CONTROL_BEFORE");
        let control =
            super::binding::admit_control_root(&mut admission, &config.control_root).await?;
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=INSTALL_ADMIT_CONTROL_AFTER");
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=INSTALL_REGISTRY_ADMIT_BEFORE");
        let registry = ManagementRegistry::open_installed()?;
        let lease = registry.try_lock()?;
        let enrolled = lease.admit_source(&control, assets, config)?;
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=INSTALL_REGISTRY_ADMIT_AFTER");
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=INSTALL_ISOLATION_BEFORE");
        let isolation =
            super::verify_isolation_attestation_metered(config, true, Some(&lease.budget))?;
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=INSTALL_ISOLATION_AFTER");
        lease.recheck()?;
        let deployment = Uuid::parse_str(&enrolled.group.deployment_id).map_err(|_| invalid())?;
        let group = Uuid::parse_str(&enrolled.group.group_id).map_err(|_| invalid())?;
        let budget = lease.budget.clone();
        let journal_budget = SharedJournalBudget::default();
        let authority = encode(&lease.authority, &budget, &journal_budget)?;
        let generation = encode(lease.generation(), &budget, &journal_budget)?;
        let mut roots = Vec::new();
        for root in enrolled.roots.values() {
            roots.push((
                format!("source-roots/{}.json", root.enrollment_id),
                encode(root, &budget, &journal_budget)?,
            ));
        }
        let source_group = enrolled.group.clone();
        let source_roots = enrolled.roots.clone();
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=INSTALL_ADMISSION_CLOSE_BEFORE");
        admission.close().await?;
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=INSTALL_ADMISSION_CLOSE_AFTER");
        lease.recheck()?;
        // A captured lease clone would hold the global registry lock for the
        // entire broker lifetime. Store the immutable original record bytes
        // instead; capture must reacquire and revalidate their real handles.
        drop(lease);
        let binding_id = Uuid::new_v4();
        let epoch_id = Uuid::new_v4();
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=INSTALL_CREATE_SOCKETS_BEFORE");
        let (socket_root, work_listener, management_listener, work_path, management_path) =
            create_sockets(binding_id, epoch_id)?;
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=INSTALL_CREATE_SOCKETS_AFTER");
        let cutover_deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=INSTALL_FRESH_MOUNTS_OPEN_BEFORE");
        let (mounts, companion_fd, export_fd) = cutover::FreshMounts::open(
            config,
            &source_roots,
            &budget,
            &journal_budget,
            cutover_deadline,
        )
        .await?;
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=INSTALL_FRESH_MOUNTS_OPEN_AFTER");
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=INSTALL_BINDING_BEFORE");
        let installed = InstalledSnapshotBinding::install_fresh_empty_mounts(
            companion_fd,
            export_fd,
            deployment,
            epoch_id,
            binding_id,
            &RootSnapshotBroker::writer_contract_sha256(),
            journal_budget.clone(),
        )
        .map_err(|_| invalid())?;
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=INSTALL_BINDING_AFTER");
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=INSTALL_ROOT_BROKER_BEFORE");
        let broker = RootSnapshotBroker::install_fresh(
            installed,
            admin_install,
            broker_pool,
            runtime_pool,
            BrokerSourceDescription {
                deployment_id: deployment,
                source_group_id: group,
                source_application_commit: source_group.application_commit.clone(),
                source_build_sha256: source_group.application_build_sha256.clone(),
            },
            (&work_listener, &management_listener),
        )
        .await
        .map_err(|_| invalid())?;
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=INSTALL_ROOT_BROKER_AFTER");
        let descriptor: Descriptor = serde_json::from_slice(broker.descriptor_bytes())?;
        descriptor_matches(&descriptor, &source_group)?;
        let companion = BackupDir::open_trusted_private_root(Path::new(
            InstalledSnapshotBinding::COMPANION_PATH,
        ))?;
        let mut enrollment = Enrollment {
            backup_id: config.backup_id,
            group: source_group,
            roots: source_roots,
            registry_authority: authority,
            registry_generation: generation,
            registry_roots: roots,
            companion,
            records: BTreeMap::new(),
            budget,
            journal_budget,
            _isolation: isolation,
            #[cfg(feature = "offhost-b1")]
            b1: None,
            binding_id: descriptor.binding_id,
            epoch_id: descriptor.epoch_id,
        };
        // These durable records are written before this wrapper escapes and
        // therefore before its first actual Unix peer can request a claim.
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=INSTALL_MOUNTS_FINISH_BEFORE");
        let actual = mounts
            .finish(config, &descriptor, &enrollment.roots)
            .await?;
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=INSTALL_MOUNTS_FINISH_AFTER");
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=INSTALL_RECORDS_BEFORE");
        enrollment.install_records(&descriptor, broker.root_marker_bytes(), &actual)?;
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=INSTALL_RECORDS_AFTER");
        if std::time::Instant::now() >= cutover_deadline {
            return Err(invalid());
        }
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=INSTALL_REVALIDATE_BEFORE");
        broker.revalidate().map_err(|_| invalid())?;
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=INSTALL_REVALIDATE_AFTER");
        Ok(Self {
            broker,
            enrollment,
            work_listener,
            _management_listener: management_listener,
            work_path,
            management_path,
            _socket_root: socket_root,
        })
    }

    pub fn work_socket_path(&self) -> &Path {
        &self.work_path
    }
    pub fn management_socket_path(&self) -> &Path {
        &self.management_path
    }
    pub fn epoch_id(&self) -> Uuid {
        self.enrollment.epoch_id
    }
    pub fn binding_id(&self) -> Uuid {
        self.enrollment.binding_id
    }
    pub async fn accept_next_signal(&self) -> Result<BrokerSignalResult, BackupError> {
        self.accept_until(None).await
    }
    pub async fn accept_next_signal_for_capture(
        &self,
        transition: BeforeWriteTransition,
    ) -> Result<BrokerSignalResult, BackupError> {
        self.accept_until(Some(transition)).await
    }
    async fn accept_until(
        &self,
        transition: Option<BeforeWriteTransition>,
    ) -> Result<BrokerSignalResult, BackupError> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
        let original = self.work_listener.try_clone()?;
        original.set_nonblocking(true)?;
        let listener = tokio::net::UnixListener::from_std(original)?;
        let result = tokio::time::timeout_at(deadline, async {
            let (stream, _) = listener.accept().await?;
            let stream = stream.into_std()?;
            stream.set_nonblocking(false)?;
            if tokio::time::Instant::now() >= deadline {
                return Err(invalid());
            }
            match transition {
                Some(transition) => self
                    .broker
                    .accept_signal_for_capture(stream, transition)
                    .await
                    .map_err(|_| invalid()),
                None => self
                    .broker
                    .accept_signal(stream)
                    .await
                    .map_err(|_| invalid()),
            }
        })
        .await
        .map_err(|_| invalid())??;
        if tokio::time::Instant::now() >= deadline {
            return Err(invalid());
        }
        Ok(result)
    }
    pub async fn accept_signal(
        &self,
        stream: UnixStream,
    ) -> Result<BrokerSignalResult, BackupError> {
        self.accept_stream_until(stream, None).await
    }
    pub async fn accept_signal_for_capture(
        &self,
        stream: UnixStream,
        transition: BeforeWriteTransition,
    ) -> Result<BrokerSignalResult, BackupError> {
        self.accept_stream_until(stream, Some(transition)).await
    }
    async fn accept_stream_until(
        &self,
        stream: UnixStream,
        transition: Option<BeforeWriteTransition>,
    ) -> Result<BrokerSignalResult, BackupError> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
        let result = tokio::time::timeout_at(deadline, async {
            match transition {
                Some(t) => self.broker.accept_signal_for_capture(stream, t).await,
                None => self.broker.accept_signal(stream).await,
            }
        })
        .await
        .map_err(|_| invalid())?
        .map_err(|_| invalid())?;
        if tokio::time::Instant::now() >= deadline {
            return Err(invalid());
        }
        Ok(result)
    }
    pub fn request_quiesce(&self) -> Result<(), BackupError> {
        self.broker.request_quiesce().map_err(|_| invalid())
    }
    pub async fn quiesce_for_capture(self, backup_id: Uuid) -> Result<CaptureInput, BackupError> {
        #[cfg(feature = "offhost-b1")]
        if let Some(original) = &self.enrollment.b1 {
            let peer = std::sync::Arc::clone(original.retained.peer());
            let observed = self.quiesce_original(backup_id).await;
            if observed.is_err() {
                let _ = peer.close_original().await;
            }
            return observed;
        }
        self.quiesce_original(backup_id).await
    }
    async fn quiesce_original(self, backup_id: Uuid) -> Result<CaptureInput, BackupError> {
        #[cfg(feature = "offhost-b1")]
        if let Some(original) = &self.enrollment.b1 {
            original.recheck()?;
        }
        if backup_id != self.enrollment.backup_id {
            return Err(invalid());
        }
        let cut = self
            .broker
            .quiesce_for_capture(backup_id, Uuid::new_v4())
            .await
            .map_err(|_| invalid())?;
        #[cfg(feature = "offhost-b1")]
        if let Some(original) = &self.enrollment.b1 {
            original.recheck()?;
        }
        Ok(CaptureInput {
            cut,
            enrollment: self.enrollment,
            pending: None,
            started: false,
        })
    }
}

fn create_sockets(
    binding: Uuid,
    epoch: Uuid,
) -> Result<(BackupDir, UnixListener, UnixListener, PathBuf, PathBuf), BackupError> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    let run = std::fs::symlink_metadata("/run")?;
    if !run.is_dir() || run.uid() != 0 || run.mode() & 0o022 != 0 {
        return Err(invalid());
    }
    let base = Path::new("/run/kwc4");
    match std::fs::DirBuilder::new().mode(0o700).create(base) {
        Ok(()) => (),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error.into()),
    }
    let root = BackupDir::open_trusted_private_root(base)?;
    let sockets = match root.kind("sfx") {
        Ok(learning_assets::backup_fs::BackupEntryKind::Directory) => root.open_dir("sfx")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => root.create_dir("sfx")?,
        _ => return Err(invalid()),
    };
    sockets.require_private_directory()?;
    let binding_root = sockets.create_dir(&binding.to_string())?;
    let epoch_root = binding_root.create_dir(&epoch.to_string())?;
    let path = base
        .join("sfx")
        .join(binding.to_string())
        .join(epoch.to_string());
    let work_path = path.join("work.sock");
    let management_path = path.join("management.sock");
    let work = UnixListener::bind(&work_path)?;
    let management = UnixListener::bind(&management_path)?;
    // chmod/chown apply to the filesystem leaf. Listener sockfs metadata is a
    // different inode and is retained independently by the DB owner.
    let work_c =
        std::ffi::CString::new(work_path.as_os_str().as_encoded_bytes()).map_err(|_| invalid())?;
    if unsafe { libc::chown(work_c.as_ptr(), 0, 65532) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    std::fs::set_permissions(&work_path, std::fs::Permissions::from_mode(0o660))?;
    std::fs::set_permissions(&management_path, std::fs::Permissions::from_mode(0o600))?;
    epoch_root.sync()?;
    binding_root.sync()?;
    sockets.sync()?;
    root.sync()?;
    if BackupDir::open_trusted_private_root(&path)?.identity()? != epoch_root.identity()? {
        return Err(invalid());
    }
    Ok((epoch_root, work, management, work_path, management_path))
}

impl Enrollment {
    fn b1_fence(&self) -> Result<(), BackupError> {
        #[cfg(feature = "offhost-b1")]
        if let Some(original) = &self.b1 {
            original.recheck()?;
        }
        Ok(())
    }
    fn install_records(
        &mut self,
        descriptor: &Descriptor,
        marker: &[u8],
        actual: &Value,
    ) -> Result<(), BackupError> {
        self.b1_fence()?;
        let mut directories = BTreeMap::new();
        for name in ["bindings", "captures", "generations", "journals", "staging"] {
            let (dev, ino) = self.companion.open_dir(name)?.identity()?;
            directories.insert(name, json!({"dev":dev,"ino":ino}));
        }
        let (registry_dev, registry_ino) = {
            let value: Value = serde_json::from_slice(&self.registry_authority)?;
            (
                value["registry_dev"].as_u64().ok_or_else(invalid)?,
                value["registry_ino"].as_u64().ok_or_else(invalid)?,
            )
        };
        let inspection = json!({"source_isolation":serde_json::from_slice::<Value>(&self._isolation.inspection_bytes)?,"actual_root_cutover":actual});
        let inspection_raw = encode(&inspection, &self.budget, &self.journal_budget)?;
        self.b1_fence()?;
        let cutover = json!({
            "format_version":2,"capability":"snapshot_export_runtime_cutover_v2",
            "binding_id":descriptor.binding_id,"deployment_id":descriptor.deployment_id,
            "inspection_sha256":hash(&inspection_raw),
            "broker_identity":{"container_id":actual["container_id"],"process_id":descriptor.broker_pid,"uid":descriptor.broker_uid,
                "image_sha256":actual["image_sha256"],"binary_sha256":descriptor.broker_binary_sha256,
                "application_commit":descriptor.source_application_commit,"build_sha256":descriptor.source_build_sha256,
                "export_mount":actual["export_mount"],"journal_mount":actual["journal_mount"],"assets_mount":actual["assets_mount"]},
            "legacy_producers":actual["legacy_producers"],"worker_clients":[],
            "old_direct_producers_stopped":true,"old_export_rw_mounts_removed":true
        });
        let cutover_raw = encode(&cutover, &self.budget, &self.journal_budget)?;
        self.b1_fence()?;
        let binding = json!({
            "format_version":2,"capability":"snapshot_export_binding_v2",
            "binding_id":descriptor.binding_id,"deployment_id":descriptor.deployment_id,
            "source_group_id":descriptor.source_group_id,"source_group_sha256":hash(&encode(&self.group,&self.budget,&self.journal_budget)?),
            "source_registry_enrollment_generation_sha256":hash(&self.registry_generation),
            "source_database":descriptor.source_database,"source_database_oid":descriptor.source_database_oid,
            "source_system_identifier":descriptor.source_system_identifier,
            "source_application_commit":descriptor.source_application_commit,"source_build_sha256":descriptor.source_build_sha256,
            "broker_binary_sha256":descriptor.broker_binary_sha256,"writer_contract_sha256":descriptor.writer_contract_sha256,
            "export_root_path":InstalledSnapshotBinding::EXPORT_PATH,"export_root_dev":descriptor.export_root_dev,"export_root_ino":descriptor.export_root_ino,
            "journal_root_path":format!("{}/journals/{}",InstalledSnapshotBinding::COMPANION_PATH,descriptor.binding_id),
            "journal_root_dev":descriptor.journal_root_dev,"journal_root_ino":descriptor.journal_root_ino,
            "effect_lock_dev":descriptor.effect_lock_dev,"effect_lock_ino":descriptor.effect_lock_ino,
            "marker_sha256":hash(marker),"initial_empty_inventory_sha256":descriptor.initial_namespace_sha256,
            "runtime_cutover_sha256":hash(&cutover_raw),"enrolled_generation":1
        });
        let binding_raw = encode(&binding, &self.budget, &self.journal_budget)?;
        self.b1_fence()?;
        let generation = json!({"format_version":2,"capability":"snapshot_effect_registry_generation_v2",
            "deployment_id":descriptor.deployment_id,"generation":1,"previous_generation_sha256":null,
            "bindings":[{"id":descriptor.binding_id,"sha256":hash(&binding_raw)}]});
        let generation_raw = encode(&generation, &self.budget, &self.journal_budget)?;
        self.b1_fence()?;
        let authority = json!({"format_version":2,"capability":"snapshot_effect_registry_v2",
            "deployment_id":descriptor.deployment_id,"registry_path":crate::registry::REGISTRY_PATH,
            "registry_dev":registry_dev,"registry_ino":registry_ino,
            "backup_registry_authority_sha256":hash(&self.registry_authority),"directories":directories,
            "initial_generation":1,"initial_generation_sha256":hash(&generation_raw)});
        let authority_raw = encode(&authority, &self.budget, &self.journal_budget)?;
        for (path, bytes) in [
            ("authority.json".to_owned(), authority_raw),
            (
                format!("bindings/{}.json", descriptor.binding_id),
                binding_raw,
            ),
            (
                "generations/00000000000000000001.json".to_owned(),
                generation_raw,
            ),
            (
                format!("staging/{}/runtime-cutover.json", descriptor.binding_id),
                cutover_raw,
            ),
            (
                format!("staging/{}/inspection.json", descriptor.binding_id),
                inspection_raw,
            ),
        ] {
            self.b1_fence()?;
            files::put(
                &self.companion,
                &path,
                &bytes,
                &self.budget,
                &self.journal_budget,
            )?;
            self.records.insert(path, bytes);
        }
        self.companion.sync()?;
        files::seal_tree(
            &self
                .companion
                .open_dir("staging")?
                .open_dir(&descriptor.binding_id.to_string())?,
        )?;
        self.recheck_records()
    }

    fn recheck_records(&self) -> Result<(), BackupError> {
        self.b1_fence()?;
        if BackupDir::open_trusted_private_root(Path::new(
            InstalledSnapshotBinding::COMPANION_PATH,
        ))?
        .identity()?
            != self.companion.identity()?
        {
            return Err(invalid());
        }
        for (path, bytes) in &self.records {
            let record = FileRecord {
                path: path.clone(),
                size: bytes.len() as u64,
                sha256: hash(bytes),
            };
            if files::read(&self.companion, &record, &self.budget, &self.journal_budget)? != *bytes
            {
                return Err(invalid());
            }
        }
        let exact = |dir: &BackupDir, mut names: Vec<String>| -> Result<(), BackupError> {
            names.sort();
            if !exact_record_names(dir.list_bounded(wire::MAX_FILES)?, &names) {
                return Err(invalid());
            }
            Ok(())
        };
        exact(
            &self.companion,
            [
                "authority.json",
                "bindings",
                "captures",
                "generations",
                "journals",
                "staging",
            ]
            .map(str::to_owned)
            .to_vec(),
        )?;
        exact(
            &self.companion.open_dir("bindings")?,
            vec![format!("{}.json", self.binding_id)],
        )?;
        exact(
            &self.companion.open_dir("generations")?,
            vec!["00000000000000000001.json".into()],
        )?;
        exact(
            &self.companion.open_dir("staging")?,
            vec![self.binding_id.to_string()],
        )?;
        exact(
            &self
                .companion
                .open_dir("staging")?
                .open_dir(&self.binding_id.to_string())?,
            vec!["inspection.json".into(), "runtime-cutover.json".into()],
        )?;
        exact(
            &self.companion.open_dir("journals")?,
            vec![self.binding_id.to_string()],
        )?;
        let binding = self
            .companion
            .open_dir("journals")?
            .open_dir(&self.binding_id.to_string())?;
        exact(&binding, vec!["broker.lock".into(), "epochs".into()])?;
        exact(
            &binding.open_dir("epochs")?,
            vec![self.epoch_id.to_string()],
        )?;
        Ok(())
    }
}

fn descriptor_matches(descriptor: &Descriptor, group: &SourceGroup) -> Result<(), BackupError> {
    if descriptor.format_version != 2
        || descriptor.capability != "snapshot_effect_epoch_v2"
        || descriptor.storage_encoding != ENCODING
        || descriptor.deployment_id.to_string() != group.deployment_id
        || descriptor.source_group_id.to_string() != group.group_id
        || descriptor.source_database != group.database
        || descriptor.source_database_oid != i64::from(group.database_oid)
        || descriptor.source_system_identifier != group.system_identifier
        || descriptor.source_application_commit != group.application_commit
        || descriptor.source_build_sha256 != group.application_build_sha256
        || descriptor.broker_uid != 0
        || descriptor.broker_gid != 0
        || descriptor.broker_pid == 0
        || descriptor.writer_contract_sha256 != RootSnapshotBroker::writer_contract_sha256()
    {
        return Err(invalid());
    }
    Ok(())
}

struct Pending {
    parent: BackupDir,
    directory: BackupDir,
    staging: String,
    files: Vec<FileRecord>,
    inventory: JobInventory,
    namespace: Vec<NamespaceEntry>,
    descriptor: Descriptor,
    ledger: Ledger,
    cut: Cut,
    prefix: Prefix,
}

/// The source hook accepts this owned original cut, never caller-supplied
/// records. Failed capture attempts are not reusable because their staging
/// identity remains and because capture_closed may run only once.
pub struct CaptureInput {
    cut: QuiescedSnapshotEpochV2,
    enrollment: Enrollment,
    pending: Option<Pending>,
    started: bool,
}

/// A genuine source adjunct tied to the same sealed local pin. It retains the
/// original quiesced owner and EX guard through destination copying.
///
/// ```compile_fail
/// let _: learning_backup::CapturedSnapshotEffectsV2 = serde_json::from_str("{}").unwrap();
/// ```
pub struct CapturedSnapshotEffectsV2 {
    pub(crate) directory: BackupDir,
    pub(crate) record: Capture,
    pub(crate) inventory: JobInventory,
    pub(crate) namespace: Vec<NamespaceEntry>,
    pub(crate) prefix: Prefix,
    pub(crate) ledger: Ledger,
    pub(crate) source_identity: Value,
    pub(crate) budget: Budget,
    pub(crate) journal_budget: SharedJournalBudget,
    _cut: QuiescedSnapshotEpochV2,
    _enrollment: Enrollment,
}

impl CaptureInput {
    /// Keep the actual original peer across the entire consuming prepare
    /// future. Negative cleanup never first asks a poisoned peer to qualify.
    /// M3 still must retain this future through caller cancellation and await
    /// its own original finally before ending the runtime.
    #[cfg(feature = "offhost-b1")]
    pub(in crate::source) async fn prepare_b1_with_original_close(
        &mut self,
        assets: &FsAssetStore,
    ) -> Result<SourceLocalPin, BackupError> {
        let original = self.enrollment.b1.as_ref().ok_or_else(invalid)?;
        let peer = std::sync::Arc::clone(original.retained.peer());
        let observed = super::prepare_b1_original(assets, self).await;
        if observed.is_err() {
            // Preserve the primary operation error. close_original's actual
            // Book keeps unknown settlement permanently negative; an ignored
            // close result here cannot turn that error into business success
            // or a retry permission, and Drop is not a settlement proof.
            let _ = peer.close_original().await;
        }
        observed
    }

    #[cfg(feature = "offhost-b1")]
    pub(in crate::source) fn b1_peer(
        &self,
    ) -> Result<std::sync::Arc<learning_db::snapshot_broker::B1InstalledSourceProcess>, BackupError>
    {
        self.cut.revalidate().map_err(|_| invalid())?;
        let original = self.enrollment.b1.as_ref().ok_or_else(invalid)?;
        original.recheck()?;
        Ok(std::sync::Arc::clone(original.retained.peer()))
    }
    #[cfg(feature = "offhost-b1")]
    pub(in crate::source) fn require_b1_admission(
        &self,
        admission: &super::SourceAdmission,
        lease: &RegistryLease,
        source: &EnrolledSource,
        config: &SourceBackupConfig,
    ) -> Result<(), BackupError> {
        let peer = self.b1_peer()?;
        admission.require_same_b1_capture_peer(&peer)?;
        admission.require_b1_enrollment(lease, source)?;
        if config.backup_id != self.enrollment.backup_id
            || source.group != self.enrollment.group
            || source.roots != self.enrollment.roots
        {
            return Err(invalid());
        }
        let original = self
            .enrollment
            .b1
            .as_ref()
            .expect("actual original B1 enrollment");
        original
            .retained
            .recheck_enrollment_for_capture(lease, source)?;
        admission.revalidate()?;
        self.cut.revalidate().map_err(|_| invalid())
    }
    pub(crate) async fn capture_closed(
        &mut self,
        conn: &mut PgConnection,
        config: &SourceBackupConfig,
        source: &EnrolledSource,
        lease: &RegistryLease,
    ) -> Result<(), BackupError> {
        if self.started || self.pending.is_some() || config.backup_id != self.enrollment.backup_id {
            return Err(invalid());
        }
        self.started = true;
        self.enrollment.budget = lease.budget.clone();
        self.cut.revalidate().map_err(|_| invalid())?;
        self.enrollment.recheck_records()?;
        lease.recheck()?;
        if source.group != self.enrollment.group
            || source.roots != self.enrollment.roots
            || canonical(&lease.authority)? != self.enrollment.registry_authority
        {
            return Err(invalid());
        }
        // Enrollment generations may append other entries, but the original
        // source generation and all four exact root identities must survive.
        let enrolled_generation: crate::registry::Generation =
            serde_json::from_slice(&self.enrollment.registry_generation)?;
        crate::registry::validate_capture_generation(
            &lease.generations,
            &source.group,
            &source.roots,
            enrolled_generation.generation,
            &hash(&self.enrollment.registry_generation),
            None,
        )?;
        for (kind, root) in &source.roots {
            crate::registry::validate_root_identity(
                root,
                Path::new(&root.path),
                source.handle(kind).identity()?,
            )?;
        }
        let descriptor: Descriptor = serde_json::from_slice(self.cut.descriptor_bytes())?;
        descriptor_matches(&descriptor, &source.group)?;
        let ledger: Ledger = serde_json::from_slice(self.cut.ledger_bytes())?;
        let cut: Cut = serde_json::from_slice(self.cut.cut_bytes())?;
        verify::cut(
            &descriptor,
            &ledger,
            &cut,
            self.cut.head(),
            self.cut.descriptor_bytes(),
            self.cut.ledger_bytes(),
            config.backup_id,
        )?;
        let inventory = jobs::inventory(conn, &lease.budget).await?;
        let parent = self.enrollment.companion.open_dir("captures")?;
        let final_name = config.backup_id.to_string();
        // This fresh enrolled epoch has exactly one consuming capture. No
        // previous or orphan capture, even for another backup, is adopted.
        if !parent.list_bounded(wire::MAX_FILES)?.is_empty() {
            return Err(invalid());
        }
        let staging = format!("{final_name}.staging-{}", Uuid::new_v4());
        let directory = parent.create_dir(&staging)?;
        parent.sync()?;
        let budget = &lease.budget;
        let journal = self.cut.shared_budget();
        let mut records = Vec::new();
        let mut put = |path: &str, bytes: &[u8]| -> Result<(), BackupError> {
            records.push(files::put(&directory, path, bytes, budget, journal)?);
            Ok(())
        };
        put("source-authority.json", &self.enrollment.registry_authority)?;
        put(
            "source-generation.json",
            &encode(lease.generation(), budget, journal)?,
        )?;
        put(
            "source-enrollment-generation.json",
            &self.enrollment.registry_generation,
        )?;
        put(
            "source-group.json",
            &encode(&source.group, budget, journal)?,
        )?;
        for (path, bytes) in &self.enrollment.registry_roots {
            put(path, bytes)?;
        }
        for (path, bytes) in &self.enrollment.records {
            put(&format!("companion/{path}"), bytes)?;
        }
        put("epoch.json", self.cut.descriptor_bytes())?;
        put("ledger.json", self.cut.ledger_bytes())?;
        put("cut.json", self.cut.cut_bytes())?;
        put("root-marker.json", self.cut.root_marker_bytes())?;
        put("source-rows.json", &encode(&inventory, budget, journal)?)?;
        let mut journal_files = BTreeMap::new();
        self.cut
            .visit_journal(&mut |name, size, reader| {
                let path = format!("journal/{name}");
                let record = files::from_reader(&directory, &path, size, reader, budget, journal)
                    .map_err(std::io::Error::other)?;
                let bytes = files::read(&directory, &record, budget, journal)
                    .map_err(std::io::Error::other)?;
                journal_files.insert(path, bytes);
                records.push(record);
                Ok(())
            })
            .await
            .map_err(|_| invalid())?;
        let prefix = Prefix::check(
            &journal_files,
            self.cut.descriptor_bytes(),
            descriptor.epoch_id,
            descriptor.binding_id,
            &cut.cut_record,
        )?;
        verify::prefix_cut(
            &prefix,
            &cut,
            self.cut.descriptor_bytes(),
            self.cut.ledger_bytes(),
            budget,
            journal,
        )?;
        let mut namespace = Vec::new();
        self.cut
            .visit_namespace(&mut |name, directory, size| {
                charge(budget, journal, name.len() as u64 + 32, 1)
                    .map_err(std::io::Error::other)?;
                namespace.push(NamespaceEntry {
                    path: name.to_owned(),
                    directory,
                    size,
                });
                Ok(())
            })
            .map_err(|_| invalid())?;
        namespace.sort_by(|a, b| a.path.cmp(&b.path));
        records.push(files::put(
            &directory,
            "namespace.json",
            &encode(&namespace, budget, journal)?,
            budget,
            journal,
        )?);
        verify::inventory(&inventory, &ledger, &prefix, &namespace, budget, journal)?;
        // Package copying is driven by real committed receipts only. The
        // original verifier checks the complete four-file reading-copy package
        // before it yields any sealed member handle.
        for known in &ledger.jobs {
            if let Some(attempt) = known
                .attempts
                .last()
                .filter(|a| a.business_commit.is_some() && a.write_closed.is_some())
            {
                let receipt = verify::receipt(&inventory, known.job_id)?;
                let digest = receipt["manifest_sha256"].as_str().ok_or_else(invalid)?;
                let result = self.cut.with_verified_package(
                    known.job_id,
                    attempt.claim_token,
                    digest,
                    |package| {
                        for member in package.open_verified_file_iter()? {
                            let (name, mut file) = member?;
                            let size = file.metadata()?.len();
                            let path = format!(
                                "packages/{}/{}.ready/{name}",
                                known.job_id, attempt.claim_token
                            );
                            records.push(
                                files::from_reader(
                                    &directory, &path, size, &mut file, budget, journal,
                                )
                                .map_err(|error| {
                                    learning_assets::SnapshotIoError::Io(std::io::Error::other(
                                        error,
                                    ))
                                })?,
                            );
                        }
                        Ok(())
                    },
                );
                result.map_err(|_| invalid())?;
            }
        }
        records.sort_by(|a, b| a.path.cmp(&b.path));
        files::exact_tree(&directory, &records, budget, journal)?;
        self.cut.revalidate().map_err(|_| invalid())?;
        lease.recheck()?;
        self.pending = Some(Pending {
            parent,
            directory,
            staging,
            files: records,
            inventory,
            namespace,
            descriptor,
            ledger,
            cut,
            prefix,
        });
        Ok(())
    }

    pub(crate) async fn bind_pin(&mut self, pin: &SourceLocalPin) -> Result<(), BackupError> {
        let pending = self.pending.as_mut().ok_or_else(invalid)?;
        if pin.manifest().backup_id != self.enrollment.backup_id
            || pin.sealed().backup_id() != self.enrollment.backup_id
            || pin.manifest().canonical_sha256()? != pin.sealed().manifest_sha256()
            || pin.manifest().source.application_commit != self.enrollment.group.application_commit
            || pin.manifest().source.application_build_sha256
                != self.enrollment.group.application_build_sha256
        {
            return Err(invalid());
        }
        let budget = &self.enrollment.budget;
        let journal = self.cut.shared_budget();
        self.cut.revalidate().map_err(|_| invalid())?;
        let record_for = |path: &str| {
            pending
                .files
                .iter()
                .find(|r| r.path == path)
                .ok_or_else(invalid)
        };
        let record = Capture {
            format_version: 2,
            capability: "snapshot_effect_capture_v2".into(),
            storage_encoding: ENCODING.into(),
            backup_id: pin.manifest().backup_id,
            capture_id: pending.cut.capture_id,
            manifest_sha256: pin.sealed().manifest_sha256().into(),
            source_group_id: pending.descriptor.source_group_id,
            source_registry_generation_sha256: record_for("source-generation.json")?.sha256.clone(),
            companion_authority_sha256: record_for("companion/authority.json")?.sha256.clone(),
            companion_generation_sha256: record_for(
                "companion/generations/00000000000000000001.json",
            )?
            .sha256
            .clone(),
            binding_sha256: record_for(&format!(
                "companion/bindings/{}.json",
                pending.descriptor.binding_id
            ))?
            .sha256
            .clone(),
            epoch_sha256: record_for("epoch.json")?.sha256.clone(),
            journal_head_sequence: pending.cut.cut_record.sequence,
            journal_head_sha256: pending.cut.cut_record.sha256.clone(),
            cut_event_sha256: pending.cut.cut_record.sha256.clone(),
            rows_sha256: record_for("source-rows.json")?.sha256.clone(),
            namespace_sha256: record_for("namespace.json")?.sha256.clone(),
            files: pending.files.clone(),
        };
        let raw = encode(&record, budget, journal)?;
        pending.files.push(files::put(
            &pending.directory,
            "capture.json",
            &raw,
            budget,
            journal,
        )?);
        pending.files.sort_by(|a, b| a.path.cmp(&b.path));
        files::exact_tree(&pending.directory, &pending.files, budget, journal)?;
        files::seal_tree(&pending.directory)?;
        pending.parent.rename_noreplace_without_sync(
            &pending.staging,
            &pin.manifest().backup_id.to_string(),
        )?;
        pending.parent.sync()?;
        self.enrollment.companion.sync()?;
        if pending
            .parent
            .open_dir(&pin.manifest().backup_id.to_string())?
            .identity()?
            != pending.directory.identity()?
        {
            return Err(invalid());
        }
        Ok(())
    }

    fn finish(self, pin: &SourceLocalPin) -> Result<CapturedSnapshotEffectsV2, BackupError> {
        self.cut.revalidate().map_err(|_| invalid())?;
        self.enrollment.recheck_records()?;
        let pending = self.pending.ok_or_else(invalid)?;
        if pending.parent.list_bounded(wire::MAX_FILES)?
            != vec![pin.manifest().backup_id.to_string()]
        {
            return Err(invalid());
        }
        files::exact_tree(
            &pending.directory,
            &pending.files,
            &self.enrollment.budget,
            self.cut.shared_budget(),
        )?;
        let capture_record = FileRecord {
            path: "capture.json".into(),
            size: 0,
            sha256: String::new(),
        };
        let record = pending
            .files
            .iter()
            .find(|r| r.path == capture_record.path)
            .ok_or_else(invalid)?;
        let raw = files::read(
            &pending.directory,
            record,
            &self.enrollment.budget,
            self.cut.shared_budget(),
        )?;
        let capture: Capture = crate::registry::canonical_record(&raw, wire::MAX_FILE)?;
        if capture.manifest_sha256 != pin.sealed().manifest_sha256() {
            return Err(invalid());
        }
        Ok(CapturedSnapshotEffectsV2 {
            directory: pending.directory,
            record: capture,
            inventory: pending.inventory,
            namespace: pending.namespace,
            prefix: pending.prefix,
            ledger: pending.ledger,
            source_identity: serde_json::to_value(&pin.manifest().source)?,
            budget: self.enrollment.budget.clone(),
            journal_budget: self.cut.shared_budget().clone(),
            _cut: self.cut,
            _enrollment: self.enrollment,
        })
    }
}

/// Same source pipeline and original Pin; the ordinary path continues to pass
/// None and creates no adjunct or positive effect authority.
pub async fn prepare_source_backup_with_snapshot_effects(
    admin: &PgPool,
    assets: &FsAssetStore,
    config: &SourceBackupConfig,
    mut input: CaptureInput,
) -> Result<(SourceLocalPin, CapturedSnapshotEffectsV2), BackupError> {
    let pin = super::prepare_linux_with_effects(admin, assets, config, Some(&mut input)).await?;
    let captured = input.finish(&pin)?;
    Ok((pin, captured))
}

#[cfg(feature = "offhost-b1")]
#[allow(
    dead_code,
    reason = "S5 original B1 capture awaits the pending actual M3/SourceActor entry; no runtime integration credit"
)]
pub(crate) async fn prepare_source_backup_with_snapshot_effects_b1(
    assets: &FsAssetStore,
    mut input: CaptureInput,
) -> Result<(SourceLocalPin, CapturedSnapshotEffectsV2), BackupError> {
    // Negative ownership is available from the actual retained input even
    // when the first positive cut/ctx recheck fails. It issues no permission.
    let peer = std::sync::Arc::clone(
        input
            .enrollment
            .b1
            .as_ref()
            .ok_or_else(invalid)?
            .retained
            .peer(),
    );
    let observed = async {
        input.b1_peer()?;
        let pin = super::prepare_b1_with_effects(assets, &mut input).await?;
        let captured = input.finish(&pin)?;
        Ok((pin, captured))
    }
    .await;
    if observed.is_err() {
        let _ = peer.close_original().await;
    }
    observed
}

#[cfg(test)]
mod roster_tests {
    use super::exact_record_names;

    fn companion_names() -> Vec<String> {
        [
            "authority.json",
            "bindings",
            "captures",
            "generations",
            "journals",
            "staging",
        ]
        .map(str::to_owned)
        .to_vec()
    }

    fn permutations(names: &mut [String], offset: usize, visit: &mut impl FnMut(&[String])) {
        if offset == names.len() {
            visit(names);
            return;
        }
        for index in offset..names.len() {
            names.swap(offset, index);
            permutations(names, offset + 1, visit);
            names.swap(offset, index);
        }
    }

    #[test]
    fn all_six_companion_name_permutations_are_accepted() {
        let expected = companion_names();
        let mut actual = expected.clone();
        let mut checked = 0;
        permutations(&mut actual, 0, &mut |names| {
            assert!(exact_record_names(names.to_vec(), &expected), "{names:?}");
            checked += 1;
        });
        assert_eq!(checked, 720);
        let observed = [
            "captures",
            "bindings",
            "staging",
            "journals",
            "generations",
            "authority.json",
        ]
        .map(str::to_owned)
        .to_vec();
        assert!(exact_record_names(observed, &expected));
    }

    #[test]
    fn two_member_record_rosters_accept_both_orders() {
        for names in [
            ["inspection.json", "runtime-cutover.json"],
            ["broker.lock", "epochs"],
        ] {
            let expected = names.map(str::to_owned).to_vec();
            let mut actual = expected.clone();
            let mut checked = 0;
            permutations(&mut actual, 0, &mut |names| {
                assert!(exact_record_names(names.to_vec(), &expected), "{names:?}");
                checked += 1;
            });
            assert_eq!(checked, 2);
        }
    }

    #[test]
    fn missing_record_members_are_rejected() {
        let expected = companion_names();
        for index in 0..expected.len() {
            let mut actual = expected.clone();
            actual.remove(index);
            assert!(!exact_record_names(actual, &expected));
        }
        assert!(!exact_record_names(Vec::new(), &expected));
    }

    #[test]
    fn extra_record_members_are_rejected() {
        let expected = companion_names();
        for index in 0..=expected.len() {
            let mut actual = expected.clone();
            actual.insert(index, "unexpected.json".into());
            assert!(!exact_record_names(actual, &expected));
        }
    }

    #[test]
    fn duplicate_record_members_are_rejected() {
        let expected = companion_names();
        for index in 0..expected.len() {
            let mut actual = expected.clone();
            actual[index] = expected[(index + 1) % expected.len()].clone();
            assert!(!exact_record_names(actual, &expected));
        }
        let mut actual = expected.clone();
        actual.push(expected[0].clone());
        assert!(!exact_record_names(actual, &expected));
    }

    #[test]
    fn spoofed_record_names_are_rejected() {
        let expected = companion_names();
        for spoof in [
            "Authority.json",
            "authority.json ",
            "authority.json/",
            "authority.json.bak",
            "../authority.json",
        ] {
            let mut actual = expected.clone();
            actual[0] = spoof.into();
            assert!(!exact_record_names(actual, &expected));
        }
    }
}

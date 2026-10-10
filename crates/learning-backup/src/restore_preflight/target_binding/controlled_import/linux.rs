//! Linux-only adapter for the private controlled fixture supervisor.
//! No live caller exists until Task 6's producer/TOC issuer is reviewed.
use super::super as binding;
use super::super::super as preflight;
use super::*;
use binding::{
    BoundTargetGuard, LockChallenge,
    child_attestation::{ChildFailure, Isolation, linux_child},
};
use candidate_attempt::{CandidateAttemptContext, persist_attempt, persist_commit_intent};
use commands::FixedImportCommand;
use diagnostics::{Case, Failure, Phase};
use fixture_sql::{FrozenDump, verify_fixture_sql};
use learning_assets::backup_fs::BackupDir;
use live_tests::{Evidence, Observation};
use sha2::{Digest, Sha256};
use sqlx::{Connection, PgConnection, PgPool, Postgres, Transaction};
use std::{
    fs::File,
    time::{Duration, Instant},
};
use stream::{StreamBudget, StreamOwner, spawn_stream};

struct CloneWriter {
    claim: binding::DockerClaim,
    started: String,
}
struct LiveHooks {
    case: Case,
    evidence: Evidence,
    cancel: watch::Sender<bool>,
    clone: Option<CloneWriter>,
    ready: bool,
    restarted: bool,
}

// Actual files/no-follow operations; the one injected failure is at the real
// file synchronization boundary, so no durable permit is returned on failure.
struct SyncFailure<'a>(&'a BackupDir);
impl candidate_attempt::JournalIo for SyncFailure<'_> {
    type File = File;
    type Reader = File;
    fn identity(&self) -> std::io::Result<(u64, u64)> {
        self.0.identity()
    }
    fn kind(&self, name: &str) -> std::io::Result<learning_assets::backup_fs::BackupEntryKind> {
        self.0.kind(name)
    }
    fn create_file(&self, name: &str) -> std::io::Result<File> {
        self.0.create_file(name)
    }
    fn open_file(&self, name: &str) -> std::io::Result<File> {
        self.0.open_file(name)
    }
    fn sync_file(&self, _: &File) -> std::io::Result<()> {
        Err(std::io::Error::other("fixed sync fault"))
    }
    fn sync_dir(&self) -> std::io::Result<()> {
        self.0.sync()
    }
}

fn clone_writer(admission: &OwnedCandidateAdmission) -> Result<CloneWriter, ImportFailure> {
    let get = live_tests::required;
    let mut claim = admission.guard.claim.clone();
    claim.container_id = get("KNOWWEAVE_C4_CLONE_CONTAINER_ID")?;
    claim.network_id = get("KNOWWEAVE_C4_CLONE_NETWORK_ID")?;
    claim.network_name = get("KNOWWEAVE_C4_CLONE_NETWORK_NAME")?;
    claim.project = get("KNOWWEAVE_C4_CLONE_PROJECT")?;
    claim.volume_name = get("KNOWWEAVE_C4_CLONE_VOLUME_NAME")?;
    claim.subnet = get("KNOWWEAVE_C4_CLONE_SUBNET")?;
    let suffix = claim
        .project
        .strip_prefix("learning-system-p0c4-restore-")
        .ok_or(ImportFailure::Identity)?;
    let batch = uuid::Uuid::parse_str(suffix).map_err(|_| ImportFailure::Identity)?;
    if batch.get_version_num() != 4
        || batch.to_string() != suffix
        || batch == admission.batch
        || claim.network_name != format!("{}_test", claim.project)
        || claim.volume_name != format!("{}_pg", claim.project)
        || !binding::exact_id(&claim.container_id)
        || !binding::exact_id(&claim.network_id)
    {
        return Err(ImportFailure::Identity);
    }
    for (kind, expected) in [
        ("container", &claim.container_id),
        ("network", &claim.network_id),
        ("volume", &claim.volume_name),
    ] {
        let label = format!("label=com.docker.compose.project={}", claim.project);
        let args = if kind == "volume" {
            vec![kind, "ls", "-q", "--filter", &label]
        } else if kind == "network" {
            vec![kind, "ls", "-q", "--no-trunc", "--filter", &label]
        } else {
            vec![kind, "ls", "-aq", "--no-trunc", "--filter", &label]
        };
        binding::only_claimed_resource(
            &binding::linux::docker(&args.into_iter().map(str::to_owned).collect::<Vec<_>>())
                .map_err(|_| ImportFailure::Identity)?,
            expected,
        )
        .map_err(|_| ImportFailure::Identity)?;
    }
    let pg = binding::linux::inspect("container", &claim.container_id)
        .map_err(|_| ImportFailure::Identity)?;
    let net = binding::linux::inspect("network", &claim.network_id)
        .map_err(|_| ImportFailure::Identity)?;
    let vol = binding::linux::inspect("volume", &claim.volume_name)
        .map_err(|_| ImportFailure::Identity)?;
    claim.mountpoint = vol["Mountpoint"]
        .as_str()
        .ok_or(ImportFailure::Identity)?
        .into();
    let ip = binding::validate_copy_docker(&admission.guard.claim, &claim, &pg, &net, &vol)
        .map_err(|_| ImportFailure::Identity)?;
    binding::validate_same_id_wrong_endpoint(
        &admission.guard.claim,
        &claim,
        admission
            .guard
            .target_ip()
            .map_err(|_| ImportFailure::Identity)?,
        ip,
    )
    .map_err(|_| ImportFailure::Identity)?;
    for endpoint in [&admission.guard.claim, &claim] {
        binding::validate_pg_line(
            endpoint,
            &binding::linux::docker(
                &binding::pg_exec_args(endpoint).map_err(|_| ImportFailure::Identity)?,
            )
            .map_err(|_| ImportFailure::Identity)?,
        )
        .map_err(|_| ImportFailure::Identity)?;
    }
    if pg["RestartCount"].as_u64() != Some(0) {
        return Err(ImportFailure::Identity);
    }
    Ok(CloneWriter {
        claim,
        started: pg["State"]["StartedAt"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or(ImportFailure::Identity)?
            .into(),
    })
}

pub(super) async fn run_live_case(
    mut admission: OwnedCandidateAdmission,
    dump: FrozenDump,
    case: Case,
) -> Result<Observation, Failure> {
    let toc_sha256 = match dump.toc_sha256() {
        Ok(hash) => hex::encode(hash),
        Err(error) => {
            admission.quarantine().await;
            return Err(Failure::at(Phase::LiveToc)(error));
        }
    };
    let clone = if case == Case::WrongEndpoint {
        match clone_writer(&admission) {
            Ok(clone) => Some(clone),
            Err(error) => {
                admission.quarantine().await;
                return Err(Failure::at(Phase::LiveClone)(error));
            }
        }
    } else {
        None
    };
    let evidence = Arc::new(std::sync::Mutex::new(Observation {
        schema: 1,
        case: case.name(),
        dump_sha256: hex::encode(dump.sha256()),
        toc_sha256,
        ..Observation::default()
    }));
    if clone.is_some() {
        evidence
            .lock()
            .unwrap()
            .checkpoints
            .insert("CLONE_SAME_SYSTEM_ID_DATABASE_OID");
    }
    let (cancel, rx) = watch::channel(false);
    admission.guard.child_usable.set(false);
    let report = supervise(
        LinuxCandidate {
            admission,
            dump,
            stream: None,
            deadline: Instant::now() + Duration::from_secs(15),
            live: Some(LiveHooks {
                case,
                evidence: evidence.clone(),
                cancel,
                clone,
                ready: false,
                restarted: false,
            }),
        },
        rx,
        Arc::new(AtomicBool::new(false)),
    )
    .await;
    let expected = match case {
        Case::Success => None,
        Case::ReadyEof | Case::PrecommitEof => Some(ImportFailure::Protocol),
        Case::Cancel => Some(ImportFailure::Cancelled),
        Case::Restart => report
            .failure
            .filter(|f| matches!(f, ImportFailure::Identity | ImportFailure::Session)),
        Case::SqlError | Case::CopyTruncated | Case::WrongEndpoint => Some(ImportFailure::Exit),
        Case::AttemptSync | Case::IntentSync => Some(ImportFailure::Journal),
        Case::CommitUnknown => Some(ImportFailure::CommitUnknown),
    };
    if report.failure != expected
        || !report.stop_confirmed
        || (case == Case::Restart && expected.is_none())
        || (case == Case::Success && !report.content_verified)
        || (matches!(case, Case::ReadyEof | Case::PrecommitEof | Case::Cancel)
            && !report.content_verified)
        || report.commit_attempted != matches!(case, Case::Success | Case::CommitUnknown)
    {
        return Err(Failure::report(&report));
    }
    let mut record = Arc::try_unwrap(evidence)
        .map_err(|_| Failure::evidence_shared(&report))?
        .into_inner()
        .map_err(|_| Failure::evidence_poisoned(&report))?;
    if case == Case::Restart {
        diagnostics::require_restart_transition(&report, &record.checkpoints)?;
    }
    record.failure = match case {
        Case::Success => None,
        Case::Restart => Some("Identity"),
        Case::Cancel => Some("Cancelled"),
        Case::AttemptSync | Case::IntentSync => Some("Journal"),
        Case::CommitUnknown => Some("CommitUnknown"),
        Case::SqlError | Case::CopyTruncated | Case::WrongEndpoint => Some("Exit"),
        _ => Some("Protocol"),
    };
    record.commit_attempted = report.commit_attempted;
    record.rollback_verified = matches!(case, Case::ReadyEof | Case::PrecommitEof | Case::Cancel)
        && report.content_verified;
    Ok(record)
}

pub(super) struct OwnedCandidateAdmission {
    guard: BoundTargetGuard<File, Option<File>>,
    challenge: LockChallenge<Transaction<'static, Postgres>>,
    config: preflight::RestorePreflightConfig,
    admin: PgPool,
    control: Option<BackupDir>,
    _assets: BackupDir,
    _destination: BackupDir,
    expected: WriterExpected,
    batch: uuid::Uuid,
    birth_sha256: [u8; 32],
    inspection_sha256: [u8; 32],
}

impl OwnedCandidateAdmission {
    pub(super) async fn into_full_preflight(
        mut self,
        manifest: crate::BackupManifestV1,
        plan: crate::BackupPlan,
    ) -> Result<
        (
            preflight::RestorePreflight,
            preflight::RestorePreflightConfig,
        ),
        crate::BackupError,
    > {
        preflight::observed_build_and_pg(self.challenge.lease_mut())
            .await?
            .validate(&manifest.source)?;
        Ok((
            preflight::RestorePreflight {
                manifest,
                plan,
                bound_target: self.guard,
                sql_session: self.challenge,
            },
            self.config,
        ))
    }
}

impl AdmissionQuarantine for OwnedCandidateAdmission {
    async fn quarantine(&mut self) {
        self.guard.child_usable.set(false);
        let _ = linux_child::candidate_quarantine(&self.guard.claim).await;
    }
}

// The existing root guard/birth/catalog checks remain the authority. A candidate
// never creates a CompleteBackup or substitutes a weaker clean-target check.
pub(super) async fn admit_candidate_target(
    config: preflight::RestorePreflightConfig,
    admin: PgPool,
) -> Result<OwnedCandidateAdmission, ImportFailure> {
    let (send, receive) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let result = admit_owned(config, admin).await;
        let _ = handoff_admission(send, result);
    });
    receive
        .await
        .map_err(|_| ImportFailure::UnconfirmedIsolation)?
        .map(AdmissionHandoff::take)
}

pub(super) async fn admit_full_candidate_target(
    config: preflight::RestorePreflightConfig,
    admin: PgPool,
    audit: Option<full::AdmissionAudit>,
) -> Result<OwnedCandidateAdmission, ImportFailure> {
    let (send, receive) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let result = admit_owned_with_audit(config, admin, audit).await;
        let _ = handoff_admission(send, result);
    });
    receive
        .await
        .map_err(|_| ImportFailure::UnconfirmedIsolation)?
        .map(AdmissionHandoff::take)
}

async fn admit_owned(
    config: preflight::RestorePreflightConfig,
    admin: PgPool,
) -> Result<OwnedCandidateAdmission, ImportFailure> {
    admit_owned_with_audit(config, admin, None).await
}

async fn admit_owned_with_audit(
    config: preflight::RestorePreflightConfig,
    admin: PgPool,
    audit: Option<full::AdmissionAudit>,
) -> Result<OwnedCandidateAdmission, ImportFailure> {
    config.validate().map_err(|_| ImportFailure::Identity)?;
    let (mut guard, config) = tokio::task::spawn_blocking(move || {
        binding::acquire_for_restore(&config).map(|guard| (guard, config))
    })
    .await
    .map_err(|_| ImportFailure::Io)?
    .map_err(|_| ImportFailure::Identity)?;
    let deadline = Instant::now() + Duration::from_secs(45);
    // Keep the acquired guard even on any error after this point, until the
    // exact target's bounded quarantine has settled.
    let mut challenge = None;
    let result = async {
        let control = BackupDir::open_trusted_private_root(&config.control_root)
            .map_err(|_| ImportFailure::Identity)?;
        let assets = BackupDir::open_trusted_private_root(&config.asset_root)
            .map_err(|_| ImportFailure::Identity)?;
        let destination = BackupDir::open_trusted_private_root(&config.destination_root)
            .map_err(|_| ImportFailure::Identity)?;
        for name in [
            preflight::restore_attempt_name(&config.expected_database)
                .map_err(|_| ImportFailure::Identity)?,
            format!("{}.restore.commit-attempt", config.expected_database),
        ] {
            preflight::reject_existing_attempt(control.kind(&name))
                .map_err(|_| ImportFailure::Journal)?;
        }
        let options = admin.connect_options();
        if options.get_username() != "learning_admin"
            || options.get_database() != Some(config.expected_database.as_str())
            || options.get_port() != 5432
        {
            return Err(ImportFailure::Identity);
        }
        let (original, pid, oid) =
            tokio::time::timeout_at(deadline.into(), preflight::begin_sql_session(&admin))
                .await
                .map_err(|_| ImportFailure::Deadline)?
                .map_err(|_| ImportFailure::Session)?;
        challenge = Some(original);
        let lease = challenge.as_mut().ok_or(ImportFailure::Session)?;
        linux_child::candidate_recheck(&mut guard, lease, pid, oid, deadline)
            .await
            .map_err(failure)?;
        if let Some(audit) = &audit {
            audit
                .before_admission(lease.lease_mut(), &config, &control, &assets)
                .await
                .map_err(|_| ImportFailure::Fixture)?;
        }
        let facts = tokio::time::timeout_at(
            deadline.into(),
            preflight::target_facts(
                lease.lease_mut(),
                assets.list().map_err(|_| ImportFailure::Identity)?.len(),
            ),
        )
        .await
        .map_err(|_| ImportFailure::Deadline)?
        .map_err(|_| ImportFailure::Identity)?;
        if let Err(error) = facts.validate() {
            if matches!(
                error,
                crate::BackupError::Invalid("dirty or available restore target")
            ) {
                if let Some(audit) = &audit {
                    if audit
                        .dirty_refusal(&facts, lease.lease_mut(), &config, &control, &assets)
                        .await
                        .is_err()
                    {
                        audit.failed();
                    }
                }
            }
            return Err(ImportFailure::Identity);
        }
        tokio::time::timeout_at(
            deadline.into(),
            preflight::verify_target_birth(lease.lease_mut(), &config, &control, &assets),
        )
        .await
        .map_err(|_| ImportFailure::Deadline)?
        .map_err(|_| ImportFailure::Identity)?;
        linux_child::candidate_recheck(&mut guard, lease, pid, oid, deadline)
            .await
            .map_err(failure)?;
        let birth = preflight::read_private_target_file(
            &control,
            &format!("{}.birth.json", config.expected_database),
            4096,
        )
        .map_err(|_| ImportFailure::Identity)?;
        let pinned =
            option_env!("KNOWWEAVE_C4_TARGET_BIRTH_SHA256").ok_or(ImportFailure::Identity)?;
        preflight::parse_pinned_birth(&birth, pinned).map_err(|_| ImportFailure::Identity)?;
        let batch = uuid::Uuid::parse_str(
            config
                .expected_database
                .strip_prefix("learning_restore_c4_")
                .ok_or(ImportFailure::Identity)?,
        )
        .map_err(|_| ImportFailure::Identity)?;
        let expected = WriterExpected::new(
            config.expected_database.clone(),
            oid,
            &guard.claim.system_identifier,
            pid,
            lease.keys(),
            protocol::Nonce::random()?,
        )?;
        let inspection_sha256 = Sha256::digest(
            serde_json::to_vec(&guard.observation).map_err(|_| ImportFailure::Identity)?,
        )
        .into();
        Ok((
            control,
            assets,
            destination,
            expected,
            batch,
            Sha256::digest(birth).into(),
            inspection_sha256,
        ))
    }
    .await;
    match result {
        Ok((control, assets, destination, expected, batch, birth_sha256, inspection_sha256)) => {
            Ok(OwnedCandidateAdmission {
                guard,
                challenge: challenge.ok_or(ImportFailure::Session)?,
                config,
                admin,
                control: Some(control),
                _assets: assets,
                _destination: destination,
                expected,
                batch,
                birth_sha256,
                inspection_sha256,
            })
        }
        Err(error) => {
            guard.child_usable.set(false);
            let isolated = linux_child::candidate_quarantine(&guard.claim).await;
            // Both original authorities are still owned here.
            drop(challenge);
            drop(guard);
            Err(if isolated == Isolation::Stopped {
                error
            } else {
                ImportFailure::UnconfirmedIsolation
            })
        }
    }
}

pub(super) fn start_candidate_import(
    admission: OwnedCandidateAdmission,
    dump: FrozenDump,
) -> CandidateImportHandle {
    let (cancel, rx) = watch::channel(false);
    let commit_started = Arc::new(AtomicBool::new(false));
    let task = tokio::spawn(run_import_with_cancel(
        admission,
        dump,
        rx,
        commit_started.clone(),
    ));
    CandidateImportHandle {
        cancel,
        task: Some(task),
        commit_started,
    }
}

async fn run_import(admission: OwnedCandidateAdmission, dump: FrozenDump) -> CandidateImportReport {
    // The convenience await is cancellation-safe too: dropping this future
    // drops only the handle, whose Drop requests cancellation of its owner.
    start_candidate_import(admission, dump).wait().await
}

async fn run_import_with_cancel(
    admission: OwnedCandidateAdmission,
    dump: FrozenDump,
    cancel: watch::Receiver<bool>,
    commit_started: Arc<AtomicBool>,
) -> CandidateImportReport {
    // This task owns guard/challenge and every pipe until quarantine completes.
    admission.guard.child_usable.set(false);
    supervise(
        LinuxCandidate {
            admission,
            dump,
            stream: None,
            deadline: Instant::now() + Duration::from_secs(15),
            live: None,
        },
        cancel,
        commit_started,
    )
    .await
}

struct LinuxCandidate {
    admission: OwnedCandidateAdmission,
    dump: FrozenDump,
    stream: Option<StreamOwner>,
    deadline: Instant,
    live: Option<LiveHooks>,
}

pub(super) fn failure(error: ChildFailure) -> ImportFailure {
    match error {
        ChildFailure::Session => ImportFailure::Session,
        ChildFailure::Identity | ChildFailure::Unusable => ImportFailure::Identity,
        ChildFailure::Protocol => ImportFailure::Protocol,
        ChildFailure::Version => ImportFailure::Version,
        ChildFailure::Deadline => ImportFailure::Deadline,
        ChildFailure::StdoutLimit => ImportFailure::StdoutLimit,
        ChildFailure::StderrLimit => ImportFailure::StderrLimit,
        ChildFailure::Exit => ImportFailure::Exit,
        ChildFailure::Stderr => ImportFailure::Stderr,
        ChildFailure::Io => ImportFailure::Io,
    }
}

fn command(fixed: &FixedImportCommand) -> Result<tokio::process::Command, ImportFailure> {
    binding::linux::trusted_docker_path().map_err(|_| ImportFailure::Identity)?;
    let mut command = tokio::process::Command::new("/usr/bin/docker");
    command
        .args(&fixed.argv()[1..])
        .env_clear()
        .env("DOCKER_HOST", "unix:///var/run/docker.sock");
    Ok(command)
}

impl CandidateIo for LinuxCandidate {
    type Payload = VerifiedFixtureSql;
    fn expected(&self) -> &WriterExpected {
        &self.admission.expected
    }
    async fn prepare(&mut self) -> Result<VerifiedFixtureSql, ImportFailure> {
        // Must precede all decoder/writer creation, even for a matching golden.
        self.dump.require_provenance(
            self.admission.batch,
            &self.admission.config.expected_database,
            self.live
                .as_ref()
                .map_or("success", |live| live.case.name()),
        )?;
        let fixed = FixedImportCommand::decoder(&self.admission.guard.claim.container_id)?;
        let mut version_args = fixed.argv()[1..12].to_vec();
        version_args.push("--version".into());
        linux_child::candidate_version(&version_args, self.deadline)
            .await
            .map_err(failure)?;
        self.stream = Some(spawn_stream(command(&fixed)?, StreamBudget::decoder())?);
        let stream = self.stream.as_mut().ok_or(ImportFailure::Io)?;
        stream.send(self.dump.bytes()).await?;
        stream.close_input().await?;
        let mut bytes = Vec::new();
        while let Some(line) = stream.next_line(self.deadline).await? {
            bytes.extend_from_slice(&line);
        }
        stream.finish().await?;
        self.stream.take();
        let sql = verify_fixture_sql(&bytes)?;
        if let Some(live) = &self.live {
            let mut record = live.evidence.lock().unwrap();
            record.raw_sql_sha256 = hex::encode(sql.raw_sha256());
            record.transformed_sql_sha256 = hex::encode(sql.transformed_sha256());
            record.checkpoints.extend([
                "FRESH_SOURCE_FIXED_PRODUCER",
                "SNAPSHOT_NOFOLLOW_FROZEN",
                "REAL_TOC_VERIFIED",
                "FULL_GOLDEN_VERIFIED",
            ]);
        }
        Ok(sql)
    }
    async fn start(&mut self, header: Vec<u8>) -> Result<(), ImportFailure> {
        let fixed = FixedImportCommand::writer(
            self.live
                .as_ref()
                .and_then(|live| live.clone.as_ref())
                .map_or(self.admission.guard.claim.container_id.as_str(), |clone| {
                    clone.claim.container_id.as_str()
                }),
            &self.admission.config.expected_database,
        )?;
        self.deadline = Instant::now() + Duration::from_secs(45);
        let clone = self
            .live
            .as_ref()
            .and_then(|live| live.clone.as_ref())
            .map(|clone| clone.claim.clone());
        if let Some(claim) = clone
            && !linux_child::candidate_locks_absent(
                &claim,
                self.admission.challenge.keys(),
                self.deadline,
            )
            .await
            .map_err(failure)?
        {
            return Err(ImportFailure::Identity);
        }
        let budget = if self.is_case(Case::WrongEndpoint) {
            StreamBudget::identity_diagnostic()
        } else {
            StreamBudget::writer()
        };
        self.stream = Some(spawn_stream(command(&fixed)?, budget)?);
        self.send(&header).await
    }
    async fn line(&mut self) -> Result<Vec<u8>, ImportFailure> {
        let line = self
            .stream
            .as_mut()
            .ok_or(ImportFailure::Io)?
            .next_line(self.deadline)
            .await?;
        if line.is_none() && self.is_case(Case::WrongEndpoint) {
            self.expected_client_error().await?;
            self.mark("WRITER_DOUBLE_LOCK_REJECTED");
            return Err(ImportFailure::Exit);
        }
        let line = line.ok_or(ImportFailure::Protocol)?;
        if let Some(live) = &mut self.live {
            match protocol::parse_writer_line(&line, &self.admission.expected.parts().5) {
                Ok(protocol::WriterEvent::Ready(_)) => {
                    live.ready = true;
                    live.evidence.lock().unwrap().checkpoints.insert("READY");
                }
                Ok(protocol::WriterEvent::Precommit(_)) => {
                    live.evidence
                        .lock()
                        .unwrap()
                        .checkpoints
                        .insert("PRECOMMIT");
                }
                Ok(protocol::WriterEvent::Committed) => {
                    live.evidence
                        .lock()
                        .unwrap()
                        .checkpoints
                        .insert("COMMITTED");
                }
                _ => {}
            }
        }
        Ok(line)
    }
    async fn recheck(&mut self) -> Result<(), ImportFailure> {
        let restart = self
            .live
            .as_ref()
            .is_some_and(|live| live.case == Case::Restart && live.ready && !live.restarted);
        if restart {
            let claim = self.admission.guard.claim.clone();
            linux_child::candidate_restart(&claim, self.deadline)
                .await
                .map_err(failure)?;
            self.live.as_mut().unwrap().restarted = true;
            self.mark("RESTART_BEFORE_DDL");
        }
        let (_, oid, _, pid, _, _) = self.admission.expected.parts();
        let result = linux_child::candidate_recheck(
            &mut self.admission.guard,
            &mut self.admission.challenge,
            pid,
            oid,
            self.deadline,
        )
        .await
        .map_err(failure);
        if restart && result.is_err() {
            self.mark("OLD_GUARD_REJECTED");
            self.mark("DDL_NOT_SENT");
        }
        result
    }
    async fn attempt(
        &mut self,
        sql: &VerifiedFixtureSql,
        writer: WriterIdentity,
    ) -> Result<DurableAttempt, ImportFailure> {
        if self.is_case(Case::ReadyEof) {
            self.true_eof().await?;
            return Err(ImportFailure::Protocol);
        }
        let context = CandidateAttemptContext::new(
            self.admission.batch,
            self.admission.config.expected_database.clone(),
            self.admission.birth_sha256,
            self.admission.inspection_sha256,
            self.dump.sha256(),
            sql.raw_sha256(),
            sql.transformed_sha256(),
            writer,
        )?;
        let dir = self
            .admission
            .control
            .take()
            .ok_or(ImportFailure::Journal)?;
        let inject = self.is_case(Case::AttemptSync);
        let (dir, result) = tokio::task::spawn_blocking(move || {
            let result = if inject {
                candidate_attempt::persist_attempt_with_io(&SyncFailure(&dir), &context)
            } else {
                persist_attempt(&dir, &context)
            };
            (dir, result)
        })
        .await
        .map_err(|_| ImportFailure::Journal)?;
        self.admission.control = Some(dir);
        if result.is_ok() {
            self.mark("ATTEMPT_SYNCED");
        }
        if inject && matches!(result, Err(ImportFailure::Journal)) {
            self.mark("ATTEMPT_SYNC_FAILED");
            self.mark("DDL_NOT_SENT");
            self.mark("COMMIT_NOT_SENT");
        }
        result
    }
    async fn send(&mut self, bytes: &[u8]) -> Result<(), ImportFailure> {
        let payload = bytes.starts_with(b"--\n-- Name: c4_import_probe;");
        if payload && self.is_case(Case::SqlError) {
            self.stream
                .as_mut()
                .ok_or(ImportFailure::Io)?
                .send(b"SELECT 1 / 0;\n")
                .await?;
            self.expected_client_error().await?;
            self.mark("FIXED_SQL_ERROR_OBSERVED");
            self.mark("COMMIT_NOT_SENT");
            return Err(ImportFailure::Exit);
        }
        if payload && self.is_case(Case::CopyTruncated) {
            let boundary = bytes
                .windows(b"\n1\talpha\n".len())
                .position(|part| part == b"\n1\talpha\n")
                .ok_or(ImportFailure::Fixture)?;
            self.stream
                .as_mut()
                .ok_or(ImportFailure::Io)?
                .send(&bytes[..boundary + 2])
                .await?;
            self.expected_client_error().await?;
            self.mark("FIXED_COPY_TRUNCATED");
            self.mark("NO_BLIND_ROLLBACK");
            self.mark("COMMIT_NOT_SENT");
            return Err(ImportFailure::Exit);
        }
        if bytes.starts_with(b"SELECT 'KW_C4|") && self.is_case(Case::CommitUnknown) {
            self.stream
                .as_mut()
                .ok_or(ImportFailure::Io)?
                .close_input()
                .await?;
            self.stream
                .as_mut()
                .ok_or(ImportFailure::Io)?
                .finish()
                .await?;
            self.stream.take();
            self.mark("CONFIRMATION_LOST");
            self.mark("OUTCOME_UNKNOWN_NO_RETRY");
            return Err(ImportFailure::Protocol);
        }
        self.stream
            .as_mut()
            .ok_or(ImportFailure::Io)?
            .send(bytes)
            .await?;
        if payload {
            self.mark("PAYLOAD_SENT");
        }
        if bytes == b"COMMIT;\n" {
            self.mark("COMMIT_ONCE");
        }
        Ok(())
    }
    async fn intent(
        &mut self,
        attempt: DurableAttempt,
        writer: WriterIdentity,
    ) -> Result<DurableCommitIntent, ImportFailure> {
        if self.is_case(Case::PrecommitEof) {
            self.true_eof().await?;
            return Err(ImportFailure::Protocol);
        }
        if self.is_case(Case::Cancel) {
            self.recheck().await?;
            self.mark("GUARDS_RETAINED");
            self.live.as_ref().unwrap().cancel.send_replace(true);
            self.mark("CANCEL_ACCEPTED_BEFORE_COMMIT");
            return Err(ImportFailure::Cancelled);
        }
        let dir = self
            .admission
            .control
            .take()
            .ok_or(ImportFailure::Journal)?;
        let inject = self.is_case(Case::IntentSync);
        let (dir, result) = tokio::task::spawn_blocking(move || {
            let result = if inject {
                candidate_attempt::persist_commit_intent_with_io(
                    &SyncFailure(&dir),
                    &attempt,
                    &writer,
                )
            } else {
                persist_commit_intent(&dir, &attempt, &writer)
            };
            (dir, result)
        })
        .await
        .map_err(|_| ImportFailure::Journal)?;
        self.admission.control = Some(dir);
        if result.is_ok() {
            self.mark("INTENT_SYNCED");
        }
        if inject && matches!(result, Err(ImportFailure::Journal)) {
            self.mark("INTENT_SYNC_FAILED");
            self.mark("COMMIT_NOT_SENT");
        }
        result
    }
    async fn finish(&mut self) -> Result<(), ImportFailure> {
        self.stream
            .as_mut()
            .ok_or(ImportFailure::Io)?
            .finish()
            .await?;
        self.stream.take();
        Ok(())
    }
    async fn cleanup(&mut self) -> Result<(), ImportFailure> {
        if let Some(mut stream) = self.stream.take() {
            stream.kill_and_wait().await?;
        }
        if self.is_case(Case::WrongEndpoint) {
            self.observe_wrong_endpoint_zero().await?;
        }
        Ok(())
    }
    async fn readback(&mut self, writer: WriterIdentity, committed: bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        // Every poll gets a new physical read-only connection and its own stats
        // snapshot. No rollback/zero-content claim from a stopped or new target.
        loop {
            if Instant::now() >= deadline {
                return false;
            }
            match self.observe_content(writer, committed, deadline).await {
                Ok(Some(value)) => {
                    if value {
                        if self.live.as_ref().is_some_and(|live| {
                            matches!(
                                live.case,
                                Case::Success | Case::ReadyEof | Case::PrecommitEof | Case::Cancel
                            )
                        }) {
                            self.mark("WRITER_GONE");
                        }
                        if committed {
                            self.mark("CONTENT_ROWS_PK_DIGEST");
                        } else if self.live.as_ref().is_some_and(|live| {
                            matches!(
                                live.case,
                                Case::ReadyEof | Case::PrecommitEof | Case::Cancel
                            )
                        }) {
                            self.mark("ZERO_OBJECTS_BEFORE_STOP");
                        }
                    }
                    return value;
                }
                Ok(None) => tokio::time::sleep(Duration::from_millis(50)).await,
                Err(_) => return false,
            }
        }
    }
    async fn quarantine(&mut self) -> bool {
        let markers = self.observe_markers();
        let clone = self
            .live
            .as_ref()
            .and_then(|live| live.clone.as_ref())
            .map(|clone| clone.claim.clone());
        let stopped = if let Some(claim) = clone {
            linux_child::candidate_quarantine_pair(&self.admission.guard.claim, &claim).await
                == Isolation::Stopped
        } else {
            linux_child::candidate_quarantine(&self.admission.guard.claim).await
                == Isolation::Stopped
        };
        if stopped {
            self.mark("EXACT_TARGET_STOPPED");
            self.mark("TARGET_UNUSABLE");
            if self.is_case(Case::WrongEndpoint) {
                self.mark("BOTH_EXACT_CONTAINERS_STOPPED");
            }
        }
        markers && stopped
    }
}

impl LinuxCandidate {
    fn is_case(&self, case: Case) -> bool {
        self.live.as_ref().is_some_and(|live| live.case == case)
    }
    fn mark(&self, point: &'static str) {
        if let Some(live) = &self.live {
            live.evidence.lock().unwrap().checkpoints.insert(point);
        }
    }
    async fn true_eof(&mut self) -> Result<(), ImportFailure> {
        let stream = self.stream.as_mut().ok_or(ImportFailure::Io)?;
        stream.close_input().await?;
        stream.finish().await?;
        self.stream.take();
        self.mark("TRUE_EOF_NO_TRANSACTION_COMMAND");
        Ok(())
    }
    async fn expected_client_error(&mut self) -> Result<(), ImportFailure> {
        let require_identity = self.is_case(Case::WrongEndpoint);
        let stream = self.stream.as_mut().ok_or(ImportFailure::Io)?;
        stream.close_input().await?;
        let result = stream.finish().await;
        let identity = stream.observed_identity_error();
        self.stream.take();
        if matches!(result, Err(ImportFailure::Exit | ImportFailure::Stderr))
            && (!require_identity || identity)
        {
            Ok(())
        } else {
            Err(ImportFailure::Protocol)
        }
    }
    fn observe_markers(&self) -> bool {
        let Some(live) = &self.live else { return true };
        let Some(dir) = &self.admission.control else {
            return false;
        };
        let db = &self.admission.config.expected_database;
        let exists = |name: &str| -> Option<bool> {
            match dir.kind(name) {
                Ok(learning_assets::backup_fs::BackupEntryKind::File) => Some(true),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(false),
                _ => None,
            }
        };
        let attempt = exists(&format!("{db}.restore.attempt"));
        let intent = exists(&format!("{db}.restore.commit-attempt"));
        let wants_attempt = !matches!(
            live.case,
            Case::ReadyEof | Case::Restart | Case::WrongEndpoint
        );
        let wants_intent = matches!(
            live.case,
            Case::Success | Case::IntentSync | Case::CommitUnknown
        );
        if attempt != Some(wants_attempt) || intent != Some(wants_intent) {
            return false;
        }
        if !wants_attempt {
            self.mark("ATTEMPT_ABSENT");
        }
        if !wants_intent {
            self.mark("INTENT_ABSENT");
        }
        if live.case == Case::AttemptSync {
            self.mark("ATTEMPT_PARTIAL_RETAINED");
        }
        if live.case == Case::IntentSync {
            self.mark("INTENT_PARTIAL_RETAINED");
        }
        if live.case == Case::WrongEndpoint {
            self.mark("DDL_NOT_SENT");
        }
        true
    }
    async fn observe_wrong_endpoint_zero(&mut self) -> Result<(), ImportFailure> {
        self.recheck().await?;
        let copy = self
            .live
            .as_ref()
            .and_then(|live| live.clone.as_ref())
            .ok_or(ImportFailure::Identity)?;
        let claim = copy.claim.clone();
        let started = copy.started.clone();
        let primary = self.admission.guard.claim.clone();
        let check = || -> Result<(), ImportFailure> {
            let pg = binding::linux::inspect("container", &claim.container_id)
                .map_err(|_| ImportFailure::Identity)?;
            let net = binding::linux::inspect("network", &claim.network_id)
                .map_err(|_| ImportFailure::Identity)?;
            let vol = binding::linux::inspect("volume", &claim.volume_name)
                .map_err(|_| ImportFailure::Identity)?;
            binding::validate_copy_docker(&primary, &claim, &pg, &net, &vol)
                .map_err(|_| ImportFailure::Identity)?;
            if pg["State"]["StartedAt"].as_str() != Some(started.as_str())
                || pg["RestartCount"].as_u64() != Some(0)
            {
                return Err(ImportFailure::Identity);
            }
            Ok(())
        };
        check()?;
        let ids = [
            self.admission.guard.claim.container_id.clone(),
            claim.container_id.clone(),
        ];
        for cid in ids {
            let output=fixture_sql::fixed_bytes(FixedImportCommand::writer(&cid,&self.admission.config.expected_database)?,
                b"BEGIN READ ONLY; SET LOCAL statement_timeout='5000ms'; SELECT pg_catalog.to_regclass('public.c4_import_probe') IS NULL; ROLLBACK;\n",true).await?;
            if output != b"t\n" {
                return Err(ImportFailure::Fixture);
            }
        }
        check()?;
        self.mark("ZERO_OBJECTS_BOTH_BEFORE_STOP");
        Ok(())
    }
    async fn observe_content(
        &mut self,
        writer: WriterIdentity,
        committed: bool,
        deadline: Instant,
    ) -> Result<Option<bool>, ImportFailure> {
        let (_, oid, _, pid, _, _) = self.admission.expected.parts();
        linux_child::candidate_recheck(
            &mut self.admission.guard,
            &mut self.admission.challenge,
            pid,
            oid,
            deadline,
        )
        .await
        .map_err(failure)?;
        let claim = self.admission.guard.claim.clone();
        let before = linux_child::candidate_observe(&claim, deadline)
            .await
            .map_err(failure)?;
        binding::same_observation(&self.admission.guard.observation, &before)
            .map_err(|_| ImportFailure::Identity)?;
        let options = self.admission.admin.connect_options();
        let predicate = writer_sql::identity_predicate(&self.admission.expected);
        let (writer_pid, writer_start, _) = writer.parts();
        let live_evidence = self.live.as_ref().map(|live| live.evidence.clone());
        let value=tokio::time::timeout_at(deadline.into(),async {
            let mut connection=PgConnection::connect_with(&options).await.map_err(|_|ImportFailure::Session)?;
            sqlx::query("BEGIN READ ONLY").execute(&mut connection).await.map_err(|_|ImportFailure::Session)?;
            sqlx::query("SET LOCAL statement_timeout = '5000ms'").execute(&mut connection).await.map_err(|_|ImportFailure::Session)?;
            let bound: bool=sqlx::query_scalar(&format!("SELECT ({predicate})")).fetch_one(&mut connection).await.map_err(|_|ImportFailure::Session)?;
            if !bound { return Err(ImportFailure::Identity); }
            let live: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_stat_activity a WHERE a.pid=$1 AND (EXTRACT(EPOCH FROM a.backend_start)*1000000)::bigint=$2)")
                .bind(writer_pid).bind(writer_start).fetch_one(&mut connection).await.map_err(|_|ImportFailure::Session)?;
            let value=if live { None } else {
                let sql=if committed { format!("SELECT ({})",writer_sql::content_predicate()) } else { "SELECT pg_catalog.to_regclass('public.c4_import_probe') IS NULL".into() };
                let content=sqlx::query_scalar::<_,bool>(&sql).fetch_one(&mut connection).await.map_err(|_|ImportFailure::Fixture)?;
                let constraints=if committed && content {
                    let rows=sqlx::query_as::<_,writer_sql::ConstraintRow>("SELECT c.contype::text,c.conname::text,c.conkey,c.convalidated FROM pg_catalog.pg_constraint c WHERE c.conrelid='public.c4_import_probe'::pg_catalog.regclass")
                        .fetch_all(&mut connection).await.map_err(|_|ImportFailure::Fixture)?;
                    writer_sql::constraints_match(&rows)
                } else { true };
                let exact_rows=if committed && content && constraints && let Some(evidence) = live_evidence.as_ref() {
                    let rows=sqlx::query_as::<_,(i32,String)>("SELECT id,label FROM public.c4_import_probe ORDER BY id")
                        .fetch_all(&mut connection).await.map_err(|_|ImportFailure::Fixture)?;
                    if rows != vec![(1,"alpha".into()),(2,"beta".into())] {false} else {
                        let canonical=rows.iter().map(|(id,label)|format!("{id}|{label}\n")).collect::<String>();
                        evidence.lock().unwrap().content_sha256=Some(hex::encode(Sha256::digest(canonical.as_bytes())));
                        true
                    }
                } else {true};
                Some(content && constraints && exact_rows)
            };
            sqlx::query("ROLLBACK").execute(&mut connection).await.map_err(|_|ImportFailure::Session)?;
            connection.close().await.map_err(|_|ImportFailure::Session)?;
            Ok(value)
        }).await.map_err(|_|ImportFailure::Deadline)??;
        let after = linux_child::candidate_observe(&claim, deadline)
            .await
            .map_err(failure)?;
        binding::same_observation(&self.admission.guard.observation, &after)
            .map_err(|_| ImportFailure::Identity)?;
        Ok(value)
    }
}

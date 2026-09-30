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
use fixture_sql::{FrozenDump, verify_fixture_sql};
use learning_assets::backup_fs::BackupDir;
use sha2::{Digest, Sha256};
use sqlx::{Connection, PgConnection, PgPool, Postgres, Transaction};
use std::{
    fs::File,
    time::{Duration, Instant},
};
use stream::{StreamBudget, StreamOwner, spawn_stream};

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

async fn admit_owned(
    config: preflight::RestorePreflightConfig,
    admin: PgPool,
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
        facts.validate().map_err(|_| ImportFailure::Identity)?;
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
}

fn failure(error: ChildFailure) -> ImportFailure {
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
    fn expected(&self) -> &WriterExpected {
        &self.admission.expected
    }
    async fn prepare(&mut self) -> Result<VerifiedFixtureSql, ImportFailure> {
        // Must precede all decoder/writer creation, even for a matching golden.
        self.dump.require_provenance(
            self.admission.batch,
            &self.admission.config.expected_database,
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
        verify_fixture_sql(&bytes)
    }
    async fn start(&mut self, header: Vec<u8>) -> Result<(), ImportFailure> {
        let fixed = FixedImportCommand::writer(
            &self.admission.guard.claim.container_id,
            &self.admission.config.expected_database,
        )?;
        self.deadline = Instant::now() + Duration::from_secs(45);
        self.stream = Some(spawn_stream(command(&fixed)?, StreamBudget::writer())?);
        self.send(&header).await
    }
    async fn line(&mut self) -> Result<Vec<u8>, ImportFailure> {
        self.stream
            .as_mut()
            .ok_or(ImportFailure::Io)?
            .next_line(self.deadline)
            .await?
            .ok_or(ImportFailure::Protocol)
    }
    async fn recheck(&mut self) -> Result<(), ImportFailure> {
        let (_, oid, _, pid, _, _) = self.admission.expected.parts();
        linux_child::candidate_recheck(
            &mut self.admission.guard,
            &mut self.admission.challenge,
            pid,
            oid,
            self.deadline,
        )
        .await
        .map_err(failure)
    }
    async fn attempt(
        &mut self,
        sql: &VerifiedFixtureSql,
        writer: WriterIdentity,
    ) -> Result<DurableAttempt, ImportFailure> {
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
        let (dir, result) = tokio::task::spawn_blocking(move || {
            let result = persist_attempt(&dir, &context);
            (dir, result)
        })
        .await
        .map_err(|_| ImportFailure::Journal)?;
        self.admission.control = Some(dir);
        result
    }
    async fn send(&mut self, bytes: &[u8]) -> Result<(), ImportFailure> {
        self.stream
            .as_mut()
            .ok_or(ImportFailure::Io)?
            .send(bytes)
            .await
    }
    async fn intent(
        &mut self,
        attempt: DurableAttempt,
        writer: WriterIdentity,
    ) -> Result<DurableCommitIntent, ImportFailure> {
        let dir = self
            .admission
            .control
            .take()
            .ok_or(ImportFailure::Journal)?;
        let (dir, result) = tokio::task::spawn_blocking(move || {
            let result = persist_commit_intent(&dir, &attempt, &writer);
            (dir, result)
        })
        .await
        .map_err(|_| ImportFailure::Journal)?;
        self.admission.control = Some(dir);
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
                Ok(Some(value)) => return value,
                Ok(None) => tokio::time::sleep(Duration::from_millis(50)).await,
                Err(_) => return false,
            }
        }
    }
    async fn quarantine(&mut self) -> bool {
        linux_child::candidate_quarantine(&self.admission.guard.claim).await == Isolation::Stopped
    }
}

impl LinuxCandidate {
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
                Some(content && constraints)
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

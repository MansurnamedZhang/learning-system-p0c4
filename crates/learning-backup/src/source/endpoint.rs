//! Source-local executor. Opaque authority is issued only from installed,
//! private root-driver evidence and the original admitted SQLx session.
use crate::BackupError;
use crate::restore_preflight::target_binding::{ChallengeKeys, validate_challenge_rows};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct NativeIdentity {
    pub(super) pid_namespace: String,
    pub(super) mount_namespace: String,
    pub(super) postmaster_start_ticks: u64,
    pub(super) data_dev: u64,
    pub(super) data_ino: u64,
    pub(super) socket_dev: u64,
    pub(super) socket_ino: u64,
}
impl NativeIdentity {
    pub(super) fn require_same(&self, actual: &Self) -> Result<(), BackupError> {
        if self != actual {
            return Err(BackupError::Invalid(
                "source executor namespace/postmaster/socket changed",
            ));
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use crate::registry::{EnrolledSource, RegistryLease};
    use crate::restore_preflight::target_binding::child_attestation::{
        self, ChildFailure,
        bounded_process::{self, CaptureGuard},
    };
    use crate::source::{SourceAdmission, SourceBackupConfig};
    use learning_assets::backup_fs::BackupDir;
    use sha2::{Digest, Sha256};
    use sqlx::PgConnection;
    use std::{
        fs::File,
        io::{Read, Seek},
        os::unix::fs::{FileTypeExt, MetadataExt},
        path::PathBuf,
        time::{Duration, Instant},
    };

    const PG_DUMP: &str = "/usr/lib/postgresql/18/bin/pg_dump";
    const PSQL: &str = "/usr/lib/postgresql/18/bin/psql";
    const PG_IMAGE: &str =
        "sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d";
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct SourceClaim {
        format_version: u32,
        capability: String,
        epoch: uuid::Uuid,
        project: String,
        container_id: String,
        image_id: String,
        image_digest: String,
        daemon_id: String,
        network_id: String,
        mounts_sha256: String,
        pg_dump_sha256: String,
        pg_dump_version: String,
        native: NativeIdentity,
    }

    fn source_capability(claim: &SourceClaim) -> Result<bool, BackupError> {
        if claim.capability == "source_native_endpoint_v1" {
            return Ok(true);
        }
        #[cfg(test)]
        if claim.capability == "source_native_full_rehearsal_v1" {
            crate::full_restore::profile::FullRehearsalProfile::read_installed()?.source_claim(
                &claim.container_id,
                &claim.project,
                &claim.daemon_id,
                &claim.network_id,
                &claim.mounts_sha256,
            )?;
            return Ok(true);
        }
        Ok(false)
    }
    /// Constructed only by the existing root/private-proof verifier. This is
    /// never deserializable authority and never derives a route from config.
    pub(in crate::source) struct VerifiedSourceIsolation {
        pub(in crate::source) root: BackupDir,
        pub(in crate::source) proof_name: String,
        pub(in crate::source) proof_bytes: Vec<u8>,
        pub(in crate::source) inspection_name: String,
        pub(in crate::source) inspection_bytes: Vec<u8>,
        pub(in crate::source) lock_name: String,
        pub(in crate::source) lock: File,
    }
    impl VerifiedSourceIsolation {
        fn read_claim(&self, config: &SourceBackupConfig) -> Result<SourceClaim, BackupError> {
            let value: serde_json::Value = serde_json::from_slice(&self.inspection_bytes)?;
            let claim: SourceClaim =
                serde_json::from_value(value.get("source_endpoint").cloned().ok_or(
                    BackupError::Invalid("source same-namespace executor proof required"),
                )?)?;
            let ns = |s: &str, prefix: &str| {
                s.strip_prefix(prefix)
                    .and_then(|s| s.strip_suffix(']'))
                    .is_some_and(|s| s.parse::<u64>().is_ok_and(|n| n > 0 && n.to_string() == s))
            };
            if claim.format_version != 1
                || !source_capability(&claim)?
                || claim.epoch.get_version_num() != 4
                || claim.project != config.expected_compose_project
                || !crate::valid_digest(&claim.container_id)
                || !crate::valid_digest(&claim.network_id)
                || !claim
                    .image_id
                    .strip_prefix("sha256:")
                    .is_some_and(crate::valid_digest)
                || claim.image_digest != PG_IMAGE
                || claim.daemon_id.is_empty()
                || claim.daemon_id.len() > 128
                || !claim.daemon_id.is_ascii()
                || !crate::valid_digest(&claim.mounts_sha256)
                || !crate::valid_digest(&claim.pg_dump_sha256)
                || !claim
                    .pg_dump_version
                    .starts_with("pg_dump (PostgreSQL) 18.6 (Debian 18.6-")
                || claim.pg_dump_version.len() > 128
                || !ns(&claim.native.pid_namespace, "pid:[")
                || !ns(&claim.native.mount_namespace, "mnt:[")
                || [
                    claim.native.postmaster_start_ticks,
                    claim.native.data_dev,
                    claim.native.data_ino,
                    claim.native.socket_dev,
                    claim.native.socket_ino,
                ]
                .contains(&0)
            {
                return Err(BackupError::Invalid("source endpoint claim"));
            }
            Ok(claim)
        }
        fn recheck(
            &self,
            budget: &std::sync::Arc<std::sync::Mutex<crate::registry::ScanBudget>>,
        ) -> Result<(), BackupError> {
            use std::os::fd::AsRawFd;
            self.root.require_private_directory()?;
            let held = self.lock.metadata()?;
            let current = self.root.open_file(&self.lock_name)?.metadata()?;
            if (held.dev(), held.ino()) != (current.dev(), current.ino())
                || current.uid() != 0
                || current.mode() & 0o7777 != 0o600
                || current.nlink() != 1
            {
                return Err(BackupError::Invalid("source isolation lock changed"));
            }
            if unsafe { libc::flock(self.lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                unsafe { libc::flock(self.lock.as_raw_fd(), libc::LOCK_UN) };
                return Err(BackupError::Invalid("source isolation driver lock lost"));
            }
            if std::io::Error::last_os_error().raw_os_error() != Some(libc::EWOULDBLOCK) {
                return Err(BackupError::Invalid("source isolation lock unconfirmed"));
            }
            for (name, expected) in [
                (&self.proof_name, &self.proof_bytes),
                (&self.inspection_name, &self.inspection_bytes),
            ] {
                let mut file = self.root.open_file(name)?;
                let metadata = file.metadata()?;
                if metadata.len() != expected.len() as u64
                    || metadata.uid() != 0
                    || metadata.mode() & 0o7777 != 0o600
                    || metadata.nlink() != 1
                {
                    return Err(BackupError::Invalid("source isolation epoch changed"));
                }
                let raw = crate::registry::read_metadata(&mut file, metadata.len(), budget)?;
                let after = file.metadata()?;
                if &raw != expected
                    || after.len() != metadata.len()
                    || after.uid() != metadata.uid()
                    || after.mode() != metadata.mode()
                    || after.nlink() != 1
                {
                    return Err(BackupError::Invalid("source isolation epoch changed"));
                }
            }
            Ok(())
        }
    }

    fn native_identity() -> Result<NativeIdentity, BackupError> {
        let pid_ns = std::fs::read_link("/proc/self/ns/pid")?;
        let mount_ns = std::fs::read_link("/proc/self/ns/mnt")?;
        // Root inside the unprivileged PG container need not have ptrace
        // access to UID999's /proc/1/exe or namespace symlinks. The independent
        // host producer samples those as999; locally compare OUR live kernel
        // namespaces to that claim and re-read PID1's non-ptrace metadata.
        if std::fs::metadata("/proc/1")?.uid() != 999
            || std::fs::read_to_string("/proc/1/comm")? != "postgres\n"
        {
            return Err(BackupError::Invalid(
                "source executor is outside attested PG namespace",
            ));
        }
        let raw = std::fs::read_to_string("/proc/1/stat")?;
        let tail = raw
            .strip_prefix("1 (postgres) ")
            .ok_or(BackupError::Invalid("source postmaster identity"))?;
        let fields: Vec<_> = tail.split_whitespace().collect();
        if fields.len() < 20 || matches!(fields[0], "Z" | "X" | "x") {
            return Err(BackupError::Invalid("source postmaster state"));
        }
        let start = fields[19]
            .parse()
            .map_err(|_| BackupError::Invalid("source postmaster epoch"))?;
        let data = std::fs::symlink_metadata("/var/lib/postgresql")?;
        let socket = std::fs::symlink_metadata("/var/run/postgresql/.s.PGSQL.5432")?;
        if !data.is_dir() || !socket.file_type().is_socket() || socket.uid() != 999 {
            return Err(BackupError::Invalid("source local socket identity"));
        }
        Ok(NativeIdentity {
            pid_namespace: pid_ns.to_string_lossy().into(),
            mount_namespace: mount_ns.to_string_lossy().into(),
            postmaster_start_ticks: start,
            data_dev: data.dev(),
            data_ino: data.ino(),
            socket_dev: socket.dev(),
            socket_ino: socket.ino(),
        })
    }
    fn tool_hash(path: &str) -> Result<String, BackupError> {
        let meta = std::fs::symlink_metadata(path)?;
        if !meta.is_file()
            || meta.uid() != 0
            || meta.mode() & 0o022 != 0
            || meta.len() > 64 * 1024 * 1024
        {
            return Err(BackupError::Invalid("source fixed PG tool"));
        }
        let mut file = File::open(path)?;
        let mut hash = Sha256::new();
        let mut buffer = [0; 65536];
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
        }
        Ok(format!("{:x}", hash.finalize()))
    }

    pub(in crate::source) struct BoundSourceEndpoint {
        admission: SourceAdmission,
        lease: RegistryLease,
        roots: Vec<(PathBuf, BackupDir, (u64, u64))>,
        isolation: VerifiedSourceIsolation,
        claim: SourceClaim,
        keys: ChallengeKeys,
        backend: i32,
        oid: u64,
        system: String,
        group: String,
        backup_id: uuid::Uuid,
        capture: Option<crate::CaptureProtection>,
        #[cfg(test)]
        test_control: Option<crate::source::endpoint_tests::faults::TestControl>,
    }
    pub(in crate::source) struct DumpObservation {
        pub(in crate::source) bytes: u64,
        pub(in crate::source) sha256: String,
        pub(in crate::source) client_sha256: String,
        pub(in crate::source) source_container_id: String,
        pub(in crate::source) source_backend: i32,
        pub(in crate::source) source_database: String,
        pub(in crate::source) source_database_oid: u64,
        pub(in crate::source) source_system_identifier: String,
        pub(in crate::source) source_epoch: uuid::Uuid,
        pub(in crate::source) exit_code: i32,
    }
    /// A disjoint borrow minted only by the already-bound endpoint. The final
    /// release fence checks the same admission after any release checkpoint.
    pub(in crate::source) struct ReleaseWitness<'a> {
        lease: &'a RegistryLease,
        roots: &'a [(PathBuf, BackupDir, (u64, u64))],
        isolation: &'a VerifiedSourceIsolation,
        claim: &'a SourceClaim,
        keys: ChallengeKeys,
        backend: i32,
        oid: u64,
        system: &'a str,
        capture: Option<&'a crate::CaptureProtection>,
        group: &'a str,
        backup_id: uuid::Uuid,
    }
    impl ReleaseWitness<'_> {
        fn recheck_local(&self) -> Result<(), BackupError> {
            self.isolation.recheck(&self.lease.budget)?;
            self.claim.native.require_same(&native_identity()?)?;
            self.lease.recheck()?;
            for (path, held, expected) in self.roots {
                if held.identity()? != *expected
                    || BackupDir::open_trusted_private_root(path)?.identity()? != *expected
                {
                    return Err(BackupError::Invalid("source enrolled root changed"));
                }
            }
            if let Some(capture) = self.capture {
                capture.recheck_capture(self.group, self.backup_id)?;
            }
            Ok(())
        }
        pub(in crate::source) async fn recheck(
            &self,
            admission: &mut SourceAdmission,
        ) -> Result<(), BackupError> {
            self.recheck_local()?;
            check_session(admission, self.backend, self.oid, self.system, self.keys).await?;
            self.recheck_local()
        }
        pub(in crate::source) async fn recheck_release(
            &self,
            admission: &mut SourceAdmission,
        ) -> Result<(), BackupError> {
            self.recheck(admission).await?;
            self.capture
                .ok_or(BackupError::Invalid(
                    "source release capture protection missing",
                ))?
                .recheck_before_release()
        }
    }
    async fn check_session(
        admission: &mut SourceAdmission,
        backend: i32,
        expected_oid: u64,
        expected_system: &str,
        keys: ChallengeKeys,
    ) -> Result<(), BackupError> {
        let (pid,oid,system):(i32,i64,String)=sqlx::query_as("SELECT pg_catalog.pg_backend_pid(),d.oid::bigint,pcs.system_identifier::text FROM pg_catalog.pg_database d CROSS JOIN pg_catalog.pg_control_system() pcs WHERE d.datname=pg_catalog.current_database()")
            .fetch_one(admission.connection()).await?;
        if pid != backend || oid as u64 != expected_oid || system != expected_system {
            return Err(BackupError::Invalid("source admitted session changed"));
        }
        let [first, second] = keys.values();
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM pg_catalog.pg_locks WHERE locktype='advisory' AND pid=pg_catalog.pg_backend_pid() AND database=(SELECT oid FROM pg_catalog.pg_database WHERE datname=pg_catalog.current_database()) AND objsubid=1 AND mode='ExclusiveLock' AND granted AND ((classid::bigint,objid::bigint)=(($1::bigint >> 32) & 4294967295,$1::bigint & 4294967295) OR (classid::bigint,objid::bigint)=(($2::bigint >> 32) & 4294967295,$2::bigint & 4294967295))")
            .bind(first).bind(second).fetch_one(admission.connection()).await?;
        if count != 2 {
            return Err(BackupError::Invalid("source admitted challenge lost"));
        }
        Ok(())
    }
    impl BoundSourceEndpoint {
        pub(in crate::source) fn budget(
            &self,
        ) -> &std::sync::Arc<std::sync::Mutex<crate::registry::ScanBudget>> {
            &self.lease.budget
        }
        pub(in crate::source) fn database(&self) -> &str {
            self.admission.database()
        }
        pub(in crate::source) fn attach_capture(
            mut self,
            capture: crate::CaptureProtection,
        ) -> Result<Self, BackupError> {
            if self.capture.is_some() {
                return Err(BackupError::Invalid(
                    "source capture protection already attached",
                ));
            }
            capture.recheck_capture(&self.group, self.backup_id)?;
            self.capture = Some(capture);
            Ok(self)
        }
        pub(in crate::source) fn protection(
            &mut self,
        ) -> Result<&mut crate::CaptureProtection, BackupError> {
            self.capture
                .as_mut()
                .ok_or(BackupError::Invalid("source capture protection missing"))
        }
        pub(in crate::source) async fn recheck_release(&mut self) -> Result<(), BackupError> {
            let (admission, witness) = self.release_parts();
            witness.recheck_release(admission).await
        }
        pub(in crate::source) fn connection(&mut self) -> &mut PgConnection {
            self.admission.connection()
        }
        pub(in crate::source) fn release_parts(
            &mut self,
        ) -> (&mut SourceAdmission, ReleaseWitness<'_>) {
            (
                &mut self.admission,
                ReleaseWitness {
                    lease: &self.lease,
                    roots: &self.roots,
                    isolation: &self.isolation,
                    claim: &self.claim,
                    keys: self.keys,
                    backend: self.backend,
                    oid: self.oid,
                    system: &self.system,
                    capture: self.capture.as_ref(),
                    group: &self.group,
                    backup_id: self.backup_id,
                },
            )
        }
        async fn recheck_session(&mut self) -> Result<(), BackupError> {
            let (admission, witness) = self.release_parts();
            witness.recheck(admission).await
        }
        pub(in crate::source) async fn recheck(mut self) -> Result<Self, BackupError> {
            self.recheck_session().await?;
            let args = [
                "-XAt".into(),
                "--no-password".into(),
                "--host=/var/run/postgresql".into(),
                "--port=5432".into(),
                "--username=learning_admin".into(),
                format!("--dbname={}", self.admission.database()),
                "-q".into(),
                "-v".into(),
                "ON_ERROR_STOP=1".into(),
                "-c".into(),
                format!(
                    "SELECT pg_catalog.pg_backend_pid()::text||'|'||backend_start::text FROM pg_catalog.pg_stat_activity WHERE pid=pg_catalog.pg_backend_pid();{}",
                    child_attestation::socket_sql(self.keys)
                ),
            ];
            let _ = tool_hash(PSQL)?;
            let mut command = tokio::process::Command::new(PSQL);
            command
                .args(args)
                .env_clear()
                .env("LC_ALL", "C")
                .env("PGCONNECT_TIMEOUT", "5");
            let (mut endpoint, output) = bounded_process::execute_guarded_text(
                &mut command,
                self,
                Instant::now() + Duration::from_secs(10),
                8192,
                4096,
            )
            .await
            .map_err(|_| BackupError::Invalid("source socket observer failed"))?;
            let (observer, proof) = output
                .split_once('\n')
                .ok_or(BackupError::Invalid("source observer backend receipt"))?;
            let (observer_pid, observer_start) = observer
                .split_once('|')
                .ok_or(BackupError::Invalid("source observer backend receipt"))?;
            let observer_pid: i32 = observer_pid
                .parse()
                .map_err(|_| BackupError::Invalid("source observer backend receipt"))?;
            if observer_pid <= 0
                || observer_pid == endpoint.backend
                || observer_start.is_empty()
                || observer_start.len() > 64
            {
                return Err(BackupError::Invalid("source observer backend receipt"));
            }
            validate_socket_observation(
                endpoint.admission.database(),
                endpoint.oid,
                &endpoint.system,
                endpoint.backend,
                endpoint.keys,
                proof,
            )?;
            // Waiting for the exact read-only observer's backend disappearance
            // avoids granting a transient other-session exception to the gate.
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let remaining: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE pid=$1 AND backend_start::text=$2",
                )
                .bind(observer_pid)
                .bind(observer_start)
                .fetch_one(endpoint.admission.connection())
                .await?;
                if remaining == 0 {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err(BackupError::Invalid(
                        "source observer backend did not drain",
                    ));
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            endpoint.recheck_session().await?;
            Ok(endpoint)
        }
        pub(in crate::source) async fn dump_to(
            mut self,
            output: &mut File,
        ) -> Result<(Self, DumpObservation), BackupError> {
            self.capture
                .as_ref()
                .ok_or(BackupError::Invalid(
                    "source dump capture protection missing",
                ))?
                .recheck_capture(&self.group, self.backup_id)?;
            self = self.recheck().await?;
            if tool_hash(PG_DUMP)? != self.claim.pg_dump_sha256 {
                return Err(BackupError::Invalid("source PG dump tool changed"));
            }
            let mut version = tokio::process::Command::new(PG_DUMP);
            version.arg("--version").env_clear().env("LC_ALL", "C");
            let (checked, line) = bounded_process::execute_guarded_text(
                &mut version,
                self,
                Instant::now() + Duration::from_secs(10),
                128,
                4096,
            )
            .await
            .map_err(|_| BackupError::Invalid("source dump version failed"))?;
            self = checked;
            if line != format!("{}\n", self.claim.pg_dump_version) {
                return Err(BackupError::Invalid("source dump version differs"));
            }
            if output.metadata()?.len() != 0 || output.stream_position()? != 0 {
                return Err(BackupError::Invalid("source dump output must be empty"));
            }
            let mut command = tokio::process::Command::new(PG_DUMP);
            command
                .args(dump_args(self.admission.database())?)
                .env_clear()
                .env("LC_ALL", "C")
                .env("PGCONNECT_TIMEOUT", "5");
            let (duration, cap) = (Duration::from_secs(900), 1024 * 1024 * 1024);
            #[cfg(test)]
            let (duration, cap) = if let Some(control) = self.test_control.as_mut() {
                control
                    .before_dump(
                        &self.isolation.root,
                        self.backup_id,
                        self.claim.epoch,
                        self.claim.native.postmaster_start_ticks,
                        self.backend,
                        self.oid,
                        self.keys.values(),
                    )
                    .await?
            } else {
                (duration, cap)
            };
            #[cfg(test)]
            let diagnostic = self
                .test_control
                .as_ref()
                .map(|control| control.diagnostic_handle());
            let streamed = bounded_process::execute_stream(
                &mut command,
                output.try_clone()?,
                self,
                Instant::now() + duration,
                cap,
                8192,
            )
            .await;
            #[cfg(test)]
            if let Some(diagnostic) = diagnostic {
                diagnostic
                    .lock()
                    .unwrap()
                    .stream_result(streamed.as_ref().map(|_| ()).map_err(|reason| *reason));
            }
            let (mut endpoint, observation) = streamed
                .map_err(|_| BackupError::Invalid("source dump failed; gate remains closed"))?;
            #[cfg(test)]
            if let Some(control) = endpoint.test_control.as_mut() {
                control.after_dump();
            }
            endpoint = endpoint.recheck().await?;
            if observation.prefix != b"PGDMP" {
                return Err(BackupError::Invalid("source dump custom format magic"));
            }
            let record = DumpObservation {
                bytes: observation.bytes,
                sha256: observation.sha256,
                client_sha256: endpoint.claim.pg_dump_sha256.clone(),
                source_container_id: endpoint.claim.container_id.clone(),
                source_backend: endpoint.backend,
                source_database: endpoint.admission.database().to_owned(),
                source_database_oid: endpoint.oid,
                source_system_identifier: endpoint.system.clone(),
                source_epoch: endpoint.claim.epoch,
                exit_code: 0,
            };
            Ok((endpoint, record))
        }
        pub(in crate::source) async fn close(self) -> Result<(), BackupError> {
            self.admission.close().await
        }
    }
    impl CaptureGuard for BoundSourceEndpoint {
        async fn recheck(&mut self) -> Result<(), ChildFailure> {
            #[cfg(test)]
            if self
                .test_control
                .as_mut()
                .is_some_and(|control| control.lose_lock())
            {
                let _: bool =
                    sqlx::query_scalar("SELECT pg_catalog.pg_advisory_unlock($1::bigint)")
                        .bind(self.keys.values()[0])
                        .fetch_one(self.admission.connection())
                        .await
                        .map_err(|_| ChildFailure::Session)?;
            }
            self.recheck_session()
                .await
                .map_err(|_| ChildFailure::Session)
        }
        #[cfg(test)]
        fn spawned(&mut self, child: &mut tokio::process::Child) -> Result<(), ChildFailure> {
            if let Some(control) = self.test_control.as_mut() {
                control.spawned(
                    child,
                    self.backend,
                    self.oid,
                    self.keys.values(),
                    self.admission.database(),
                )
            } else {
                Ok(())
            }
        }
        #[cfg(test)]
        async fn cleanup(&mut self, reason: ChildFailure) {
            if let Some(control) = self.test_control.as_mut() {
                control.cleanup(reason).await;
            }
        }
        #[cfg(test)]
        async fn settled(&mut self) -> Result<(), ChildFailure> {
            if let Some(control) = self.test_control.as_mut() {
                control.resume().await.map_err(|_| ChildFailure::Identity)?;
            }
            Ok(())
        }
    }
    async fn bind_identity(
        mut admission: SourceAdmission,
        enrolled: &EnrolledSource,
        lease: &RegistryLease,
        isolation: VerifiedSourceIsolation,
        config: &SourceBackupConfig,
    ) -> Result<BoundSourceEndpoint, BackupError> {
        let claim = isolation.read_claim(config)?;
        claim.native.require_same(&native_identity()?)?;
        let (backend,oid,system):(i32,i64,String)=sqlx::query_as("SELECT pg_catalog.pg_backend_pid(),d.oid::bigint,pcs.system_identifier::text FROM pg_catalog.pg_database d CROSS JOIN pg_catalog.pg_control_system() pcs WHERE d.datname=pg_catalog.current_database()")
            .fetch_one(admission.connection()).await?;
        if admission.database() != enrolled.group.database
            || config.expected_database != enrolled.group.database
            || oid <= 0
            || oid as u32 != enrolled.group.database_oid
            || system != enrolled.group.system_identifier
        {
            return Err(BackupError::Invalid("source endpoint enrollment identity"));
        }
        let keys = ChallengeKeys::random()?;
        for key in keys.values() {
            let acquired: bool =
                sqlx::query_scalar("SELECT pg_catalog.pg_try_advisory_lock($1::bigint)")
                    .bind(key)
                    .fetch_one(admission.connection())
                    .await?;
            if !acquired {
                return Err(BackupError::Invalid("source endpoint challenge busy"));
            }
        }
        let mut roots = Vec::new();
        for (name, handle) in &enrolled.handles {
            roots.push((
                PathBuf::from(&enrolled.roots[name].path),
                handle.try_clone()?,
                handle.identity()?,
            ));
        }
        let endpoint = BoundSourceEndpoint {
            admission,
            lease: lease.clone_held()?,
            roots,
            isolation,
            claim,
            keys,
            backend,
            oid: oid as u64,
            system,
            group: enrolled.group_id().to_owned(),
            backup_id: config.backup_id,
            capture: None,
            #[cfg(test)]
            test_control: crate::source::endpoint_tests::faults::current_control(),
        };
        Ok(endpoint)
    }
    pub(in crate::source) async fn admit_source_endpoint(
        admission: SourceAdmission,
        enrolled: &EnrolledSource,
        lease: &RegistryLease,
        isolation: VerifiedSourceIsolation,
        config: &SourceBackupConfig,
    ) -> Result<BoundSourceEndpoint, BackupError> {
        bind_identity(admission, enrolled, lease, isolation, config)
            .await?
            .recheck()
            .await
    }
    pub(in crate::source) async fn admit_recovery_endpoint(
        admission: SourceAdmission,
        enrolled: &EnrolledSource,
        lease: &RegistryLease,
        isolation: VerifiedSourceIsolation,
        config: &SourceBackupConfig,
        capture: crate::CaptureProtection,
    ) -> Result<BoundSourceEndpoint, BackupError> {
        bind_identity(admission, enrolled, lease, isolation, config)
            .await?
            .attach_capture(capture)?
            .recheck()
            .await
    }
}
#[cfg(target_os = "linux")]
pub(super) use linux::{
    BoundSourceEndpoint, ReleaseWitness, VerifiedSourceIsolation, admit_recovery_endpoint,
    admit_source_endpoint,
};

pub(super) fn validate_socket_observation(
    database: &str,
    oid: u64,
    system_id: &str,
    backend: i32,
    keys: ChallengeKeys,
    output: &str,
) -> Result<(), BackupError> {
    let (identity, rows) = output
        .split_once('\n')
        .ok_or(BackupError::Invalid("source socket identity"))?;
    if identity != format!("learning_admin|{database}|{oid}|{system_id}") {
        return Err(BackupError::Invalid("source socket identity"));
    }
    validate_challenge_rows(keys, backend, oid, rows)
}

pub(super) fn dump_args(database: &str) -> Result<Vec<String>, BackupError> {
    if !crate::maintenance::valid_c4_database(database) {
        return Err(BackupError::Invalid("source dump database"));
    }
    Ok([
        "--format=custom".to_owned(),
        "--no-password".into(),
        "--lock-wait-timeout=5000".into(),
        "--host=/var/run/postgresql".into(),
        "--port=5432".into(),
        "--username=learning_admin".into(),
        format!("--dbname={database}"),
    ]
    .into())
}

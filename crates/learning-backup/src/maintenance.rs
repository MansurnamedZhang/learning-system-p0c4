//! Source-side management protocol. This module deliberately has no API that
//! turns a local source record or a `SealedBackup` into `complete`.
use crate::{BackupError, valid_digest};
use learning_assets::backup_fs::BackupDir;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    path::Path,
};
#[cfg(target_os = "linux")]
use std::{
    fs::File,
    process::{Command, Stdio},
};
use uuid::Uuid;

/// A snapshot of the facts the live database gate must establish. This value
/// is diagnostic only; constructing it does not grant authority to run dump.
#[derive(Debug, Clone, Copy)]
pub struct GateInspection {
    pub admin_is_database_owner: bool,
    pub runtime_can_connect: bool,
    pub public_can_connect: bool,
    pub runtime_can_inherit_admin: bool,
    pub runtime_is_privileged: bool,
    pub other_sessions: i64,
    pub other_login_writers: i64,
}

impl GateInspection {
    pub fn validate(&self) -> Result<(), BackupError> {
        if !self.admin_is_database_owner
            || self.runtime_can_connect
            || self.public_can_connect
            || self.runtime_can_inherit_admin
            || self.runtime_is_privileged
            || self.other_sessions != 0
            || self.other_login_writers != 0
        {
            return Err(BackupError::Invalid(
                "database write gate cannot prove isolation",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatePhase {
    Intent,
    Closed,
    Drained,
    DumpAndIndexDurable,
    PinsDurable,
    ReleaseReady,
    Released,
}

impl GatePhase {
    fn file_name(self) -> &'static str {
        match self {
            Self::Intent => "intent.json",
            Self::Closed => "closed.json",
            Self::Drained => "drained.json",
            Self::DumpAndIndexDurable => "dump-and-index-durable.json",
            Self::PinsDurable => "pins-durable.json",
            Self::ReleaseReady => "release-ready.json",
            Self::Released => "released.json",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceGateRecord {
    backup_id: Uuid,
    phase: GatePhase,
    dump_and_index_sha256: Option<String>,
    pins_sha256: Option<String>,
}

impl SourceGateRecord {
    pub fn new(backup_id: Uuid) -> Self {
        Self {
            backup_id,
            phase: GatePhase::Intent,
            dump_and_index_sha256: None,
            pins_sha256: None,
        }
    }
    pub fn phase(&self) -> GatePhase {
        self.phase
    }
    pub fn pinned_manifest_sha256(&self) -> Option<&str> {
        self.pins_sha256.as_deref()
    }
    pub fn validate_for(&self, backup_id: Uuid) -> Result<(), BackupError> {
        if backup_id.is_nil() || self.backup_id != backup_id {
            return Err(BackupError::Invalid("gate record backup id"));
        }
        let source_needed = matches!(
            self.phase,
            GatePhase::DumpAndIndexDurable
                | GatePhase::PinsDurable
                | GatePhase::ReleaseReady
                | GatePhase::Released
        );
        let pins_needed = matches!(
            self.phase,
            GatePhase::PinsDurable | GatePhase::ReleaseReady | GatePhase::Released
        );
        if self.dump_and_index_sha256.is_some() != source_needed
            || self.pins_sha256.is_some() != pins_needed
            || self
                .dump_and_index_sha256
                .as_deref()
                .is_some_and(|v| !valid_digest(v))
            || self
                .pins_sha256
                .as_deref()
                .is_some_and(|v| !valid_digest(v))
        {
            return Err(BackupError::Invalid("gate proof state"));
        }
        Ok(())
    }
    pub fn advance(
        &mut self,
        next: GatePhase,
        evidence_sha256: Option<&str>,
    ) -> Result<(), BackupError> {
        let valid = matches!(
            (self.phase, next),
            (GatePhase::Intent, GatePhase::Closed)
                | (GatePhase::Closed, GatePhase::Drained)
                | (GatePhase::Drained, GatePhase::DumpAndIndexDurable)
                | (GatePhase::DumpAndIndexDurable, GatePhase::PinsDurable)
                | (GatePhase::PinsDurable, GatePhase::ReleaseReady)
                | (GatePhase::ReleaseReady, GatePhase::Released)
        );
        if !valid {
            return Err(BackupError::Invalid("gate phase order"));
        }
        let needs_evidence = matches!(
            next,
            GatePhase::DumpAndIndexDurable | GatePhase::PinsDurable
        );
        if evidence_sha256.is_some() != needs_evidence
            || evidence_sha256.is_some_and(|v| !valid_digest(v))
        {
            return Err(BackupError::Invalid("gate phase evidence"));
        }
        match next {
            GatePhase::DumpAndIndexDurable => {
                self.dump_and_index_sha256 = evidence_sha256.map(str::to_owned)
            }
            GatePhase::PinsDurable => self.pins_sha256 = evidence_sha256.map(str::to_owned),
            _ => {}
        }
        self.phase = next;
        self.validate_for(self.backup_id)?;
        Ok(())
    }
}

/// Append-only private source control record, outside the database being
/// backed up. A failed or missing phase is never inferred as successful.
/// Management recovery must inspect this record and the live DB before any
/// re-grant. This journal is not a backup completion receipt.
#[derive(Debug)]
pub struct SourceGateJournal {
    directory: BackupDir,
    record: SourceGateRecord,
}

impl SourceGateJournal {
    pub fn start(root_path: &Path, backup_id: Uuid) -> Result<Self, BackupError> {
        if backup_id.is_nil() {
            return Err(BackupError::Invalid("gate journal backup id"));
        }
        let root = BackupDir::open_private_root(root_path)?;
        let directory = root.create_dir(&format!("{backup_id}.control"))?;
        let record = SourceGateRecord::new(backup_id);
        write_phase(&directory, &record)?;
        Ok(Self { directory, record })
    }

    pub fn recover(root_path: &Path, backup_id: Uuid) -> Result<Self, BackupError> {
        if backup_id.is_nil() {
            return Err(BackupError::Invalid("gate journal backup id"));
        }
        let root = BackupDir::open_private_root(root_path)?;
        let directory = root.open_dir(&format!("{backup_id}.control"))?;
        let found = directory.list()?.into_iter().collect::<BTreeSet<_>>();
        let phases = [
            GatePhase::Intent,
            GatePhase::Closed,
            GatePhase::Drained,
            GatePhase::DumpAndIndexDurable,
            GatePhase::PinsDurable,
            GatePhase::ReleaseReady,
            GatePhase::Released,
        ];
        let mut record: Option<SourceGateRecord> = None;
        let mut expected = BTreeSet::new();
        let mut missing = false;
        for phase in phases {
            let name = phase.file_name();
            if !found.contains(name) {
                missing = true;
                continue;
            }
            if missing {
                return Err(BackupError::Invalid("gate journal phase gap"));
            }
            expected.insert(name.to_owned());
            let mut file = directory.open_file(name)?;
            if file.metadata()?.len() > 4096 {
                return Err(BackupError::Invalid("gate journal record length"));
            }
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            let next: SourceGateRecord = serde_json::from_slice(&bytes)?;
            if serde_json::to_vec(&next)? != bytes || next.phase != phase {
                return Err(BackupError::Invalid("gate journal record bytes"));
            }
            next.validate_for(backup_id)?;
            if let Some(previous) = &record {
                let mut predicted = previous.clone();
                let evidence = match phase {
                    GatePhase::DumpAndIndexDurable => next.dump_and_index_sha256.as_deref(),
                    GatePhase::PinsDurable => next.pins_sha256.as_deref(),
                    _ => None,
                };
                predicted.advance(phase, evidence)?;
                if predicted != next {
                    return Err(BackupError::Invalid("gate journal transition changed"));
                }
            }
            record = Some(next);
        }
        if expected != found {
            return Err(BackupError::Invalid("gate journal extra entries"));
        }
        Ok(Self {
            directory,
            record: record.ok_or(BackupError::Invalid("gate journal missing intent"))?,
        })
    }

    pub fn record(&self) -> &SourceGateRecord {
        &self.record
    }

    pub fn advance(
        &mut self,
        phase: GatePhase,
        evidence_sha256: Option<&str>,
    ) -> Result<(), BackupError> {
        let mut next = self.record.clone();
        next.advance(phase, evidence_sha256)?;
        write_phase(&self.directory, &next)?;
        self.record = next;
        Ok(())
    }
}

fn write_phase(directory: &BackupDir, record: &SourceGateRecord) -> Result<(), BackupError> {
    let bytes = serde_json::to_vec(record)?;
    let mut file = directory.create_file(record.phase.file_name())?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    directory.sync()?;
    Ok(())
}

/// Trusted, fixed-option `pg_dump` invocation. Passwords are supplied only via
/// the process environment's private PGPASSFILE by the caller, never as argv.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgDumpSpec {
    database: String,
    host: String,
    port: u16,
}

impl PgDumpSpec {
    pub fn new(database: &str, host: &str, port: u16) -> Result<Self, BackupError> {
        fn hostname(v: &str) -> bool {
            !v.is_empty()
                && !v.starts_with('-')
                && v.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        }
        if !valid_c4_database(database) || !hostname(host) || port == 0 {
            return Err(BackupError::Invalid("pg_dump target"));
        }
        Ok(Self {
            database: database.into(),
            host: host.into(),
            port,
        })
    }
    pub fn args(&self) -> Vec<String> {
        vec![
            "--format=custom".into(),
            "--no-password".into(),
            "--lock-wait-timeout=5000".into(),
            format!("--host={}", self.host),
            format!("--port={}", self.port),
            "--username=learning_admin".into(),
            format!("--dbname={}", self.database),
        ]
    }

    /// Stream custom-format output into an already-open private file handle.
    /// No shell, password argv, arbitrary pg_dump options, or raw stderr log.
    #[cfg(target_os = "linux")]
    pub(crate) fn run_to_file(
        &self,
        executable: &Path,
        pgpassfile: &Path,
        output: &mut File,
    ) -> Result<(), BackupError> {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let meta = std::fs::symlink_metadata(executable)?;
        let passmeta = std::fs::symlink_metadata(pgpassfile)?;
        if !executable.is_absolute()
            || !meta.file_type().is_file()
            || meta.uid() != 0
            || meta.permissions().mode() & 0o022 != 0
            || !pgpassfile.is_absolute()
            || !passmeta.file_type().is_file()
            || passmeta.uid() != unsafe { libc::geteuid() }
            || passmeta.permissions().mode() & 0o077 != 0
        {
            return Err(BackupError::Invalid(
                "trusted pg_dump executable or private password file",
            ));
        }
        let version = Command::new(executable)
            .arg("--version")
            .env_clear()
            .env("LC_ALL", "C")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()?;
        if !version.status.success()
            || !String::from_utf8_lossy(&version.stdout).contains("(PostgreSQL) 18.")
        {
            return Err(BackupError::Invalid("pg_dump 18 required"));
        }
        let status = Command::new(executable)
            .args(self.args())
            .env_clear()
            .env("LC_ALL", "C")
            .env("PGPASSFILE", pgpassfile)
            .env("PGAPPNAME", "knowweave_c4_pg_dump")
            .env("PGCONNECT_TIMEOUT", "10")
            .stdout(Stdio::from(output.try_clone()?))
            .stderr(Stdio::null())
            .status()?;
        if !status.success() {
            return Err(BackupError::Invalid("pg_dump failed"));
        }
        output.sync_all()?;
        Ok(())
    }
}

/// The only source database namespace accepted by the isolated Task 3 gate.
/// The canonical UUID may contain hyphens; SQL callers quote the identifier,
/// while pg_dump receives it as one fixed argv element, never via a shell.
pub(crate) fn valid_c4_database(value: &str) -> bool {
    let Some(marker) = value.strip_prefix("learning_backup_c4_task3_") else {
        return false;
    };
    Uuid::parse_str(marker).is_ok_and(|id| id.to_string() == marker)
}

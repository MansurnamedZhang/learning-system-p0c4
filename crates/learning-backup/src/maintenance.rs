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
    pub(crate) fn file_name(self) -> &'static str {
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
    #[cfg(target_os = "linux")]
    pub(crate) fn source_manifest_sha256(&self) -> Option<&str> {
        self.dump_and_index_sha256.as_deref()
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
    root: BackupDir,
    directory: BackupDir,
    record: SourceGateRecord,
}

impl SourceGateJournal {
    pub fn start(root_path: &Path, backup_id: Uuid) -> Result<Self, BackupError> {
        if backup_id.is_nil() {
            return Err(BackupError::Invalid("gate journal backup id"));
        }
        let root = BackupDir::open_private_root(root_path)?;
        Self::start_in(&root, backup_id)
    }

    pub(crate) fn start_in(root: &BackupDir, backup_id: Uuid) -> Result<Self, BackupError> {
        if backup_id.is_nil() {
            return Err(BackupError::Invalid("gate journal backup id"));
        }
        let staging = root.create_dir(&format!("{backup_id}.journal-staging"))?;
        let initial_name = format!("initial-{}", Uuid::new_v4());
        let directory = staging.create_dir(&initial_name)?;
        let record = SourceGateRecord::new(backup_id);
        let bytes = serde_json::to_vec(&record)?;
        let mut file = directory.create_file(record.phase.file_name())?;
        #[cfg(all(test, target_os = "linux"))]
        crate::source::lifecycle_tests::hook("initial_before_write")?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        directory.sync()?;
        staging.sync()?;
        #[cfg(all(test, target_os = "linux"))]
        crate::source::lifecycle_tests::hook("initial_before_rename")?;
        staging.rename_entry_to_noreplace_without_sync(
            &initial_name,
            root,
            &format!("{backup_id}.control"),
        )?;
        #[cfg(all(test, target_os = "linux"))]
        crate::source::lifecycle_tests::hook("initial_after_rename")?;
        root.sync()?;
        staging.sync()?;
        #[cfg(all(test, target_os = "linux"))]
        crate::source::lifecycle_tests::hook("initial_before_readback")?;
        readback_phase(&directory, &record)?;
        Ok(Self {
            root: root.try_clone()?,
            directory,
            record,
        })
    }

    pub fn recover(root_path: &Path, backup_id: Uuid) -> Result<Self, BackupError> {
        if backup_id.is_nil() {
            return Err(BackupError::Invalid("gate journal backup id"));
        }
        let root = BackupDir::open_private_root(root_path)?;
        Self::recover_in(&root, backup_id)
    }

    pub(crate) fn recover_in(root: &BackupDir, backup_id: Uuid) -> Result<Self, BackupError> {
        Self::recover_in_observed(root, backup_id, None)
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn recover_in_metered(
        root: &BackupDir,
        backup_id: Uuid,
        budget: &std::sync::Arc<std::sync::Mutex<crate::registry::ScanBudget>>,
    ) -> Result<Self, BackupError> {
        Self::recover_in_observed(root, backup_id, Some(budget))
    }

    fn recover_in_observed(
        root: &BackupDir,
        backup_id: Uuid,
        budget: Option<&std::sync::Arc<std::sync::Mutex<crate::registry::ScanBudget>>>,
    ) -> Result<Self, BackupError> {
        if backup_id.is_nil() {
            return Err(BackupError::Invalid("gate journal backup id"));
        }
        let directory = root.open_dir(&format!("{backup_id}.control"))?;
        let entries = if let Some(budget) = budget {
            directory
                .list_bounded(7.min(budget.lock().expect("scan budget").remaining_entries()))?
        } else {
            directory.list()?
        };
        if let Some(budget) = budget {
            budget.lock().expect("scan budget").entries(entries.len())?;
        }
        let found = entries.into_iter().collect::<BTreeSet<_>>();
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
            let size = file.metadata()?.len();
            if size > 4096 {
                return Err(BackupError::Invalid("gate journal record length"));
            }
            let bytes = if let Some(budget) = budget {
                let raw = crate::registry::read_metadata(&mut file, size, budget)?;
                if file.metadata()?.len() != size {
                    return Err(BackupError::Invalid("gate journal record length"));
                }
                raw
            } else {
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes)?;
                bytes
            };
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
        directory.sync()?;
        root.sync()?;
        Ok(Self {
            root: root.try_clone()?,
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
        let name = format!("{}.journal-staging", next.backup_id);
        let staging = match self.root.open_dir(&name) {
            Ok(dir) => dir,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.root.create_dir(&name)?
            }
            Err(error) => return Err(error.into()),
        };
        write_phase(&staging, &self.directory, &next)?;
        self.record = next;
        Ok(())
    }
}

fn write_phase(
    staging: &BackupDir,
    directory: &BackupDir,
    record: &SourceGateRecord,
) -> Result<(), BackupError> {
    let bytes = serde_json::to_vec(record)?;
    if bytes.len() > 4096 {
        return Err(BackupError::Invalid("gate journal record length"));
    }
    let temporary = format!(
        "{}-{}.tmp",
        record.phase.file_name().trim_end_matches(".json"),
        Uuid::new_v4()
    );
    let mut file = staging.create_file(&temporary)?;
    #[cfg(all(test, target_os = "linux"))]
    crate::source::lifecycle_tests::hook("phase_before_write")?;
    file.write_all(&bytes)?;
    #[cfg(all(test, target_os = "linux"))]
    crate::source::lifecycle_tests::hook("phase_before_file_sync")?;
    file.sync_all()?;
    #[cfg(all(test, target_os = "linux"))]
    crate::source::lifecycle_tests::hook("phase_before_staging_sync")?;
    staging.sync()?;
    #[cfg(all(test, target_os = "linux"))]
    crate::source::lifecycle_tests::hook("phase_before_rename")?;
    staging.rename_entry_to_noreplace_without_sync(
        &temporary,
        directory,
        record.phase.file_name(),
    )?;
    #[cfg(all(test, target_os = "linux"))]
    crate::source::lifecycle_tests::hook("phase_after_rename")?;
    directory.sync()?;
    staging.sync()?;
    #[cfg(all(test, target_os = "linux"))]
    crate::source::lifecycle_tests::hook("phase_before_readback")?;
    readback_phase(directory, record)
}
fn readback_phase(directory: &BackupDir, record: &SourceGateRecord) -> Result<(), BackupError> {
    let mut bytes = Vec::new();
    directory
        .open_file(record.phase.file_name())?
        .take(4097)
        .read_to_end(&mut bytes)?;
    if bytes != serde_json::to_vec(record)? {
        return Err(BackupError::Invalid("gate journal publication readback"));
    }
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

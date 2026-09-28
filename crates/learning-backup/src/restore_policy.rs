//! Fail-closed policy for a future clean-instance restore executor.
//!
//! This module does not authorize a restore: Task 3 currently publishes only
//! a local `sealed` source pin. A separate independent-target verifier must
//! mint an opaque `CompleteBackup` before any executor can write a target.
use crate::{AssetRow, BackupError, BackupPlan, MigrationRecord, SourceIdentity};
use learning_assets::FsAssetStore;
use serde::Deserialize;
use std::collections::BTreeSet;
#[cfg(target_os = "linux")]
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
    process::{Command, Stdio},
};
use uuid::Uuid;

/// A strict restore command for a freshly created C4 target database. The
/// future executor must use this only after complete-package and clean-target
/// preflight. No shell or caller-supplied PostgreSQL flags are accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgRestoreSpec {
    database: String,
    host: String,
    port: u16,
}

impl PgRestoreSpec {
    pub fn new(database: &str, host: &str, port: u16) -> Result<Self, BackupError> {
        let isolated = database
            .strip_prefix("learning_restore_c4_")
            .and_then(|id| {
                Uuid::parse_str(id)
                    .ok()
                    .filter(|uuid| uuid.to_string() == id)
            })
            .is_some();
        let host_ok = !host.is_empty()
            && !host.starts_with('-')
            && host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
        if !isolated || !host_ok || port == 0 {
            return Err(BackupError::Invalid("pg_restore target"));
        }
        Ok(Self {
            database: database.into(),
            host: host.into(),
            port,
        })
    }

    pub fn args(&self) -> Vec<String> {
        vec![
            "--exit-on-error".into(),
            "--single-transaction".into(),
            "--no-owner".into(),
            "--no-password".into(),
            format!("--host={}", self.host),
            format!("--port={}", self.port),
            "--username=learning_admin".into(),
            format!("--dbname={}", self.database),
        ]
    }

    /// Execute from an already verified, no-follow archive handle. Exposed
    /// only inside this crate until a `CompleteBackup` authority exists.
    #[cfg(target_os = "linux")]
    #[allow(dead_code)] // Wired only after a separately verified complete receipt is available.
    pub(crate) fn run_from_open_file(
        &self,
        executable: &Path,
        pgpassfile: &Path,
        archive: &mut File,
    ) -> Result<(), BackupError> {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let binary = std::fs::symlink_metadata(executable)?;
        let pass = std::fs::symlink_metadata(pgpassfile)?;
        if !executable.is_absolute()
            || !binary.file_type().is_file()
            || binary.uid() != 0
            || binary.permissions().mode() & 0o022 != 0
            || !pgpassfile.is_absolute()
            || !pass.file_type().is_file()
            || pass.uid() != unsafe { libc::geteuid() }
            || pass.permissions().mode() & 0o077 != 0
        {
            return Err(BackupError::Invalid(
                "trusted pg_restore or private password file",
            ));
        }
        let archive_meta = archive.metadata()?;
        if !archive_meta.is_file() || archive_meta.nlink() != 1 || archive_meta.len() <= 5 {
            return Err(BackupError::Invalid("restore archive handle"));
        }
        archive.seek(SeekFrom::Start(0))?;
        let mut magic = [0; 5];
        archive.read_exact(&mut magic)?;
        if &magic != b"PGDMP" {
            return Err(BackupError::Invalid("restore archive format"));
        }
        archive.seek(SeekFrom::Start(0))?;
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
            return Err(BackupError::Invalid("pg_restore 18 required"));
        }
        let status = Command::new(executable)
            .args(self.args())
            .env_clear()
            .env("LC_ALL", "C")
            .env("PGPASSFILE", pgpassfile)
            .env("PGAPPNAME", "knowweave_c4_pg_restore")
            .env("PGCONNECT_TIMEOUT", "10")
            .stdin(Stdio::from(archive.try_clone()?))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if !status.success() {
            return Err(BackupError::Invalid("pg_restore failed"));
        }
        Ok(())
    }
}

/// Observed environment supplied by a trusted restore binary after it checks
/// its compiled package identity, live PostgreSQL major and migration set.
/// Passing this value alone is never authorization to start a restore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreEnvironment {
    pub application_build_sha256: String,
    pub application_commit: String,
    pub postgres_major: u32,
    pub migrations: Vec<MigrationRecord>,
}

impl RestoreEnvironment {
    pub fn from_source_identity(identity: &SourceIdentity) -> Self {
        Self {
            application_build_sha256: identity.application_build_sha256.clone(),
            application_commit: identity.application_commit.clone(),
            postgres_major: identity.postgres_major,
            migrations: identity.migrations.clone(),
        }
    }

    pub fn validate(&self, source: &SourceIdentity) -> Result<(), BackupError> {
        // Reconstruct both fingerprints. Comparing only the highest migration
        // version would accept a modified or missing intermediate migration.
        let actual = SourceIdentity::from_migrations(
            self.application_build_sha256.clone(),
            self.application_commit.clone(),
            self.postgres_major,
            self.migrations.clone(),
        )?;
        if actual != *source || actual.postgres_major != 18 {
            return Err(BackupError::Invalid(
                "restore build, PG, or migration identity",
            ));
        }
        Ok(())
    }
}

/// Facts read from a newly created target while no runtime service is started.
/// The caller must gather these facts under an exclusive target-creation lock;
/// this pure check does not close a time-of-check/time-of-use race by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestoreTargetFacts {
    pub non_system_relations: u64,
    pub asset_root_entries: u64,
    pub database_owner_is_admin: bool,
    pub runtime_can_connect: bool,
    pub public_can_connect: bool,
    pub other_sessions: u64,
    pub private_asset_root: bool,
}

impl RestoreTargetFacts {
    pub fn validate(&self) -> Result<(), BackupError> {
        if self.non_system_relations != 0
            || self.asset_root_entries != 0
            || !self.database_owner_is_admin
            || self.runtime_can_connect
            || self.public_can_connect
            || self.other_sessions != 0
            || !self.private_asset_root
        {
            return Err(BackupError::Invalid("dirty or available restore target"));
        }
        Ok(())
    }
}

/// Compare *all* restored ready rows against the package index. The caller
/// must query the entire restored table rather than a visible reading subset.
pub fn validate_restored_assets(
    expected: &BackupPlan,
    actual_rows: Vec<AssetRow>,
) -> Result<(), BackupError> {
    let actual = BackupPlan::from_rows(actual_rows)?;
    if actual.assets() != expected.assets() {
        return Err(BackupError::Invalid(
            "restored asset rows differ from index",
        ));
    }
    Ok(())
}

/// Re-open and rehash every distinct original after restore, using the
/// storage adapter's no-follow checked handle. Repeated logical references do
/// not skip the corresponding byte object.
pub fn validate_restored_asset_bytes(
    store: &FsAssetStore,
    expected: &BackupPlan,
) -> Result<(), BackupError> {
    let mut checked = BTreeSet::new();
    for row in expected.assets() {
        if checked.insert(&row.sha256) {
            store.open_record(&row.storage_key, &row.sha256, row.byte_size)?;
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoleRecipe {
    format_version: u32,
    roles: Vec<Role>,
    memberships: Vec<RoleMembership>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Role {
    name: String,
    login: bool,
    inherit: bool,
    superuser: bool,
    createdb: bool,
    createrole: bool,
    bypassrls: bool,
    replication: bool,
    connection_limit: i32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoleMembership {
    role: String,
    member: String,
    inherit: bool,
    set: bool,
    admin: bool,
}

/// The role recipe contains only names and non-sensitive flags. Passwords
/// and unknown roles are not accepted from backup content.
pub fn validate_role_recipe(bytes: &[u8]) -> Result<(), BackupError> {
    if bytes.len() > 16 * 1024 || bytes.is_empty() {
        return Err(BackupError::Invalid("role recipe size"));
    }
    let recipe: RoleRecipe = serde_json::from_slice(bytes)?;
    if recipe.format_version != 1 || recipe.roles.len() != 3 || recipe.memberships.len() != 1 {
        return Err(BackupError::Invalid("role recipe shape"));
    }
    for (role, name) in
        recipe
            .roles
            .iter()
            .zip(["learning_admin", "learning_auth_lock", "learning_runtime"])
    {
        if role.name != name
            || role.login != (name != "learning_auth_lock")
            || role.superuser
            || role.createdb
            || role.createrole
            || role.bypassrls
            || role.replication
            || role.connection_limit < -1
        {
            return Err(BackupError::Invalid("unsafe or unknown restore role"));
        }
        let _ = role.inherit; // Keep the source flag; executor must apply it exactly.
    }
    let membership = &recipe.memberships[0];
    if membership.role != "learning_auth_lock"
        || membership.member != "learning_admin"
        || membership.inherit
        || !membership.set
        || membership.admin
    {
        return Err(BackupError::Invalid("unsafe restore role membership"));
    }
    Ok(())
}

/// A job row read from the restored DB before any Worker is allowed to start.
/// This is a decision input, not a mutator. The future mutator must invalidate
/// every source lease token atomically under an isolated admin transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoredJob {
    pub status: String,
    pub event_type: String,
    pub attempt_count: i32,
    pub lease_token_present: bool,
    pub checkpoint_present: bool,
    /// Set only after looking up the durable external idempotency receipt.
    pub external_effect: ExternalEffectFinding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalEffectFinding {
    Unknown,
    ConfirmedNoEffect,
    AlreadyCommitted,
    Conflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobRecoveryAction {
    LeaveQueued,
    LeaveWaiting,
    LeaveTerminal,
    RetryWait,
    FailedRetryBudget,
    AwaitExternalReconciliation,
}

pub fn classify_restored_job(job: &RestoredJob) -> Result<JobRecoveryAction, BackupError> {
    if !(0..=3).contains(&job.attempt_count) {
        return Err(BackupError::Invalid("job attempt count"));
    }
    let snapshot = match job.event_type.as_str() {
        "asset_integrity_requested" => false,
        "snapshot_export_requested" => true,
        _ => return Err(BackupError::Invalid("unknown job event type")),
    };
    let action = match job.status.as_str() {
        "running" if !snapshot && job.lease_token_present && job.attempt_count > 0 => {
            if job.attempt_count < 3 {
                JobRecoveryAction::RetryWait
            } else {
                JobRecoveryAction::FailedRetryBudget
            }
        }
        "snapshot_running" if snapshot && job.lease_token_present && job.attempt_count > 0 => {
            if job.external_effect != ExternalEffectFinding::ConfirmedNoEffect {
                JobRecoveryAction::AwaitExternalReconciliation
            } else if job.attempt_count < 3 {
                JobRecoveryAction::RetryWait
            } else {
                JobRecoveryAction::FailedRetryBudget
            }
        }
        "queued" if !snapshot && !job.lease_token_present && job.attempt_count == 0 => {
            JobRecoveryAction::LeaveQueued
        }
        "snapshot_queued" if snapshot && !job.lease_token_present && job.attempt_count == 0 => {
            JobRecoveryAction::LeaveQueued
        }
        "retry_wait" if !snapshot && !job.lease_token_present => JobRecoveryAction::LeaveWaiting,
        "snapshot_retry_wait" if snapshot && !job.lease_token_present => {
            JobRecoveryAction::LeaveWaiting
        }
        "succeeded" | "failed" | "cancelled" if !job.lease_token_present => {
            JobRecoveryAction::LeaveTerminal
        }
        _ => return Err(BackupError::Invalid("invalid restored job state or lease")),
    };
    // A snapshot checkpoint may have been written before an externally visible
    // file publication. Never equate a checkpoint with reconciliation evidence.
    let _ = job.checkpoint_present;
    Ok(action)
}

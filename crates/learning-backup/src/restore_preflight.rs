//! Read-only clean-target admission. An opaque `CompleteBackup` is mandatory;
//! neither a path nor a `SealedBackup` can call this entry point.
#[cfg(target_os = "linux")]
use crate::{
    AssetRow, MigrationRecord, RestoreEnvironment, RestoreTargetFacts, open_complete_backup,
    validate_role_recipe,
};
use crate::{BackupError, BackupManifestV1, BackupPlan, CompleteBackup};
#[cfg(target_os = "linux")]
use learning_assets::backup_fs::BackupDir;
#[cfg(target_os = "linux")]
use learning_db::MIGRATOR;
#[cfg(target_os = "linux")]
use serde::Deserialize;
#[cfg(target_os = "linux")]
use sha2::{Digest, Sha256};
use sqlx::PgPool;
#[cfg(target_os = "linux")]
use sqlx::Row;
#[cfg(target_os = "linux")]
use std::{
    fs::File,
    io::Read,
    os::fd::AsRawFd,
    os::unix::fs::{MetadataExt, PermissionsExt},
};
use std::{
    io,
    path::{Component, PathBuf},
};

#[derive(Debug, Clone)]
pub struct RestorePreflightConfig {
    pub destination_root: PathBuf,
    pub trust_path: PathBuf,
    pub control_root: PathBuf,
    pub asset_root: PathBuf,
    pub expected_database: String,
}

impl RestorePreflightConfig {
    pub fn validate(&self) -> Result<(), BackupError> {
        let roots = [
            self.destination_root.as_path(),
            self.control_root.as_path(),
            self.asset_root.as_path(),
        ];
        for path in roots.into_iter().chain([self.trust_path.as_path()]) {
            if !path.is_absolute()
                || path
                    .components()
                    .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
            {
                return Err(BackupError::Invalid(
                    "restore paths must be absolute and canonical",
                ));
            }
        }
        for (i, left) in roots.iter().enumerate() {
            for right in &roots[i + 1..] {
                if left.starts_with(right) || right.starts_with(left) {
                    return Err(BackupError::Invalid("restore roots overlap"));
                }
            }
        }
        if roots.iter().any(|root| self.trust_path.starts_with(root)) {
            return Err(BackupError::Invalid(
                "verifier trust is inside a restore root",
            ));
        }
        let marker = self
            .expected_database
            .strip_prefix("learning_restore_c4_")
            .ok_or(BackupError::Invalid("isolated restore database required"))?;
        if !uuid::Uuid::parse_str(marker).is_ok_and(|id| id.to_string() == marker) {
            return Err(BackupError::Invalid("isolated restore database required"));
        }
        Ok(())
    }
}

/// Holds the exclusive management lock until dropped. This API performs no
/// target data write; a later executor must retain the lock through
/// restore, data closure, derived rebuild and separate acceptance.
#[derive(Debug)]
pub struct RestorePreflight {
    manifest: BackupManifestV1,
    plan: BackupPlan,
    #[cfg(target_os = "linux")]
    _lock: File,
}

impl RestorePreflight {
    pub fn manifest(&self) -> &BackupManifestV1 {
        &self.manifest
    }
    pub fn plan(&self) -> &BackupPlan {
        &self.plan
    }
}

/// Re-open the complete receipt and every package byte under an exclusive
/// root-owned control lock, then query the live target before any DB/asset
/// write. This only admits preflight; it never makes the instance available.
pub async fn preflight_restore(
    complete: &CompleteBackup,
    config: &RestorePreflightConfig,
    admin: &PgPool,
) -> Result<RestorePreflight, BackupError> {
    config.validate()?;
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (complete, admin);
        Err(BackupError::Io(io::Error::new(
            io::ErrorKind::Unsupported,
            "restore preflight requires Linux",
        )))
    }
    #[cfg(target_os = "linux")]
    {
        preflight_linux(complete, config, admin).await
    }
}

#[cfg(target_os = "linux")]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AssetIndex {
    format_version: u32,
    assets: Vec<AssetRow>,
}

#[cfg(target_os = "linux")]
async fn preflight_linux(
    complete: &CompleteBackup,
    config: &RestorePreflightConfig,
    admin: &PgPool,
) -> Result<RestorePreflight, BackupError> {
    if unsafe { libc::geteuid() } != 0 {
        return Err(BackupError::Invalid(
            "root-owned restore controller required",
        ));
    }
    let lock_root = BackupDir::open_private_root(&config.control_root)?;
    let lock_name = format!("{}.restore.lock", config.expected_database);
    let lock = match lock_root.create_file(&lock_name) {
        Ok(file) => {
            file.sync_all()?;
            lock_root.sync()?;
            file
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            lock_root.open_file(&lock_name)?
        }
        Err(error) => return Err(error.into()),
    };
    let meta = lock.metadata()?;
    if meta.uid() != 0 || meta.permissions().mode() & 0o777 != 0o600 {
        return Err(BackupError::Invalid("private restore lock"));
    }
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(BackupError::Invalid("restore target is already locked"));
    }
    let checked = open_complete_backup(
        &config.destination_root,
        &config.trust_path,
        complete.backup_id(),
    )?;
    if checked.receipt_sha256() != complete.receipt_sha256()
        || checked.manifest_sha256() != complete.manifest_sha256()
    {
        return Err(BackupError::Invalid(
            "complete receipt changed after opening",
        ));
    }
    let destination = BackupDir::open_private_root(&config.destination_root)?;
    let package = destination.open_dir(&format!("{}.sealed", complete.backup_id()))?;
    let manifest_bytes = read_limited(&package, "manifest.json", 64 * 1024 * 1024)?;
    let manifest: BackupManifestV1 = serde_json::from_slice(&manifest_bytes)?;
    if manifest.backup_id != complete.backup_id()
        || manifest.canonical_bytes()? != manifest_bytes
        || format!("{:x}", Sha256::digest(&manifest_bytes)) != complete.manifest_sha256()
    {
        return Err(BackupError::Invalid(
            "restore manifest differs from complete receipt",
        ));
    }
    let index_bytes = read_limited(
        &package,
        "asset-index.json",
        crate::MAX_ASSET_INDEX_BYTES as u64,
    )?;
    manifest.validate_with_index(&index_bytes)?;
    let index: AssetIndex = serde_json::from_slice(&index_bytes)?;
    if index.format_version != crate::BACKUP_FORMAT_VERSION {
        return Err(BackupError::Invalid("restore asset index version"));
    }
    let plan = BackupPlan::from_rows(index.assets)?;
    if plan.asset_index_bytes() != index_bytes {
        return Err(BackupError::Invalid("noncanonical restore asset index"));
    }
    let roles = read_limited(&package, "roles.json", 16 * 1024)?;
    validate_role_recipe(&roles)?;
    observed_build_and_pg(admin, &config.expected_database)
        .await?
        .validate(&manifest.source)?;
    let assets = BackupDir::open_private_root(&config.asset_root)?;
    let facts = target_facts(admin, assets.list()?.len()).await?;
    facts.validate()?;
    Ok(RestorePreflight {
        manifest,
        plan,
        _lock: lock,
    })
}

#[cfg(target_os = "linux")]
fn read_limited(dir: &BackupDir, name: &str, max: u64) -> Result<Vec<u8>, BackupError> {
    let file = dir.open_file(name)?;
    if file.metadata()?.len() > max {
        return Err(BackupError::Capacity("restore package metadata"));
    }
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(BackupError::Capacity("restore package metadata"));
    }
    Ok(bytes)
}

#[cfg(target_os = "linux")]
async fn observed_build_and_pg(
    admin: &PgPool,
    database: &str,
) -> Result<RestoreEnvironment, BackupError> {
    let options = admin.connect_options();
    if options.get_username() != "learning_admin" || options.get_database() != Some(database) {
        return Err(BackupError::Invalid("restore database endpoint or role"));
    }
    let version: String = sqlx::query_scalar("SHOW server_version_num")
        .fetch_one(admin)
        .await?;
    let postgres_major = version
        .parse::<u32>()
        .map_err(|_| BackupError::Invalid("restore PostgreSQL version"))?
        / 10_000;
    let build = option_env!("KNOWWEAVE_BUILD_ID_SHA256").ok_or(BackupError::Invalid(
        "restore build lacks reviewed package identity",
    ))?;
    let commit = option_env!("KNOWWEAVE_SOURCE_COMMIT")
        .ok_or(BackupError::Invalid("restore build lacks source commit"))?;
    let migrations = MIGRATOR
        .iter()
        .map(|m| {
            Ok(MigrationRecord {
                version: u64::try_from(m.version).map_err(|_| BackupError::Overflow)?,
                checksum_hex: m.checksum.iter().map(|b| format!("{b:02x}")).collect(),
            })
        })
        .collect::<Result<Vec<_>, BackupError>>()?;
    Ok(RestoreEnvironment {
        application_build_sha256: build.into(),
        application_commit: commit.into(),
        postgres_major,
        migrations,
    })
}

#[cfg(target_os = "linux")]
async fn target_facts(
    admin: &PgPool,
    asset_entries: usize,
) -> Result<RestoreTargetFacts, BackupError> {
    let mut conn = admin.acquire().await?;
    let row = sqlx::query(
        "SELECT current_user::text AS current_role,session_user::text AS session_role, \
         (pg_get_userbyid(d.datdba)='learning_admin') AS admin_owner, \
         has_database_privilege('learning_runtime',current_database(),'CONNECT') AS runtime_connect, \
         EXISTS(SELECT 1 FROM aclexplode(COALESCE(d.datacl,acldefault('d',d.datdba))) a WHERE a.grantee=0 AND a.privilege_type='CONNECT') AS public_connect, \
         (SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid()) AS other_sessions, \
         (SELECT count(*) FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
           WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname NOT LIKE 'pg_toast%' AND n.nspname NOT LIKE 'pg_temp%') AS user_relations, \
         (SELECT count(*) FROM pg_catalog.pg_namespace n WHERE n.nspname NOT IN ('pg_catalog','information_schema','public') \
           AND n.nspname NOT LIKE 'pg_toast%' AND n.nspname NOT LIKE 'pg_temp%') AS user_schemas \
         FROM pg_catalog.pg_database d WHERE d.datname=current_database()"
    ).fetch_one(&mut *conn).await?;
    let current_role: String = row.try_get("current_role")?;
    let session_role: String = row.try_get("session_role")?;
    if current_role != "learning_admin" || session_role != "learning_admin" {
        return Err(BackupError::Invalid(
            "restore requires authenticated admin session",
        ));
    }
    let relations: i64 = row.try_get("user_relations")?;
    let schemas: i64 = row.try_get("user_schemas")?;
    let non_system_relations = u64::try_from(relations)
        .map_err(|_| BackupError::Overflow)?
        .checked_add(u64::try_from(schemas).map_err(|_| BackupError::Overflow)?)
        .ok_or(BackupError::Overflow)?;
    Ok(RestoreTargetFacts {
        non_system_relations,
        asset_root_entries: u64::try_from(asset_entries).map_err(|_| BackupError::Overflow)?,
        database_owner_is_admin: row.try_get("admin_owner")?,
        runtime_can_connect: row.try_get("runtime_connect")?,
        public_can_connect: row.try_get("public_connect")?,
        other_sessions: u64::try_from(row.try_get::<i64, _>("other_sessions")?)
            .map_err(|_| BackupError::Overflow)?,
        private_asset_root: true, // BackupDir::open_private_root enforced 0700 ownership.
    })
}

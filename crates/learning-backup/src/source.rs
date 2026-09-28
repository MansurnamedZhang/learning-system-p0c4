//! Management-side source capture. The result is a local `sealed` pin, never
//! an independent fault-domain copy or a restorable `complete` receipt.
#[cfg(target_os = "linux")]
use crate::{
    AdminAssetCatalog, FileRecord, GateInspection, GatePhase, MigrationRecord, PgDumpSpec,
    SourceGateJournal, SourceIdentity, seal_backup,
};
use crate::{BackupError, BackupManifestV1, SealedBackup};
use learning_assets::FsAssetStore;
#[cfg(target_os = "linux")]
use learning_assets::backup_fs::BackupDir;
#[cfg(target_os = "linux")]
use learning_db::MIGRATOR;
#[cfg(target_os = "linux")]
use serde::{Deserialize, Serialize};
#[cfg(target_os = "linux")]
use sha2::{Digest, Sha256};
use sqlx::PgPool;
#[cfg(target_os = "linux")]
use sqlx::Row;
#[cfg(target_os = "linux")]
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

/// Paths are pre-created private directories in an isolated deployment. The
/// process that calls this API must stop runtime/Worker containers first; this
/// function additionally revokes the database privilege and proves no old
/// database session remains. A privileged PostgreSQL superuser is part of the
/// trusted isolated deployment, not a role this crate can disable.
#[derive(Debug, Clone)]
pub struct SourceBackupConfig {
    pub backup_id: Uuid,
    pub expected_database: String,
    pub expected_compose_project: String,
    pub isolation_attestation: PathBuf,
    pub control_root: PathBuf,
    pub local_pin_root: PathBuf,
    pub pg_dump_executable: PathBuf,
    pub pgpassfile: PathBuf,
    pub pg_host: String,
    pub pg_port: u16,
    pub drain_timeout: Duration,
}

#[derive(Debug)]
pub struct SourceLocalPin {
    sealed: SealedBackup,
    manifest: BackupManifestV1,
}

impl SourceLocalPin {
    pub fn manifest(&self) -> &BackupManifestV1 {
        &self.manifest
    }
    pub fn sealed(&self) -> &SealedBackup {
        &self.sealed
    }
}

/// Keep the database gate closed on controlled errors after `REVOKE CONNECT`.
/// Release occurs only after dump/index/control files are durable and every
/// original is verified in the local sealed pin. This can make the maintenance
/// window long; a future verified hard-link pin can shorten it. This API never
/// publishes a completed off-host backup. A process crash during the final
/// GRANT window leaves `release_ready`, which requires explicit DB inspection
/// and REVOKE before any further backup attempt.
pub async fn prepare_source_backup(
    admin: &PgPool,
    assets: &FsAssetStore,
    config: &SourceBackupConfig,
) -> Result<SourceLocalPin, BackupError> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (admin, assets, config);
        Err(BackupError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "source backup requires Linux no-follow handles",
        )))
    }
    #[cfg(target_os = "linux")]
    {
        prepare_linux(admin, assets, config).await
    }
}

/// Crash recovery for a durable `release_ready` record only. It never resumes
/// service or labels a backup complete: it revokes runtime CONNECT, confirms
/// no session remains, and writes a separate private recovery observation.
/// Corrupt/missing journal records require manual DB isolation and inspection.
pub async fn force_close_release_ready(
    admin: &PgPool,
    control_root: &Path,
    backup_id: Uuid,
    expected_database: &str,
) -> Result<(), BackupError> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (admin, control_root, backup_id, expected_database);
        Err(BackupError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "source gate recovery requires Linux",
        )))
    }
    #[cfg(target_os = "linux")]
    {
        let database = require_admin_owner(admin).await?;
        if database != expected_database {
            return Err(BackupError::Invalid("recovery database identity"));
        }
        let journal = SourceGateJournal::recover(control_root, backup_id)?;
        if journal.record().phase() != GatePhase::ReleaseReady {
            return Err(BackupError::Invalid(
                "recovery requires release-ready phase",
            ));
        }
        compensate_release(admin, &database).await?;
        let root = BackupDir::open_private_root(control_root)?;
        let dir = root.create_dir(&format!("{backup_id}.release-recovery"))?;
        let bytes = serde_json::to_vec(&serde_json::json!({
            "format_version": 1,
            "backup_id": backup_id,
            "database": database,
            "result": "runtime_connect_revoked_and_sessions_drained"
        }))?;
        write_bytes(&dir, "closed.json", &bytes)?;
        root.sync()?;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
async fn prepare_linux(
    admin: &PgPool,
    assets: &FsAssetStore,
    config: &SourceBackupConfig,
) -> Result<SourceLocalPin, BackupError> {
    if config.backup_id.is_nil()
        || config.drain_timeout.is_zero()
        || config.drain_timeout > Duration::from_secs(60)
        || config.control_root == config.local_pin_root
    {
        return Err(BackupError::Invalid("source backup configuration"));
    }
    verify_isolation_attestation(config)?;
    // Both roots must exist and be owned 0700 before altering DB privileges.
    let control = BackupDir::open_private_root(&config.control_root)?;
    BackupDir::open_private_root(&config.local_pin_root)?;
    let database = require_admin_owner(admin).await?;
    if database != config.expected_database {
        return Err(BackupError::Invalid("source database identity mismatch"));
    }
    let options = admin.connect_options();
    if options.get_username() != "learning_admin"
        || options.get_host() != config.pg_host
        || options.get_port() != config.pg_port
        || options.get_database() != Some(database.as_str())
    {
        return Err(BackupError::Invalid(
            "pg_dump and SQLx source endpoints differ",
        ));
    }
    let dump_spec = PgDumpSpec::new(&database, &config.pg_host, config.pg_port)?;
    ensure_no_unfinished_journal(&control, &config.control_root)?;
    let before = inspect_gate(admin).await?;
    if !before.admin_is_database_owner
        || !before.runtime_can_connect
        || before.public_can_connect
        || before.runtime_can_inherit_admin
        || before.runtime_is_privileged
        || before.other_login_writers != 0
    {
        return Err(BackupError::Invalid("source role preflight"));
    }
    let mut journal = SourceGateJournal::start(&config.control_root, config.backup_id)?;
    let source = control.create_dir(&format!("{}.source", config.backup_id))?;
    let revoke = format!("REVOKE CONNECT ON DATABASE \"{database}\" FROM PUBLIC, learning_runtime");
    sqlx::query(&revoke).execute(admin).await?;
    journal.advance(GatePhase::Closed, None)?;
    let deadline = Instant::now() + config.drain_timeout;
    loop {
        let facts = inspect_gate(admin).await?;
        if !facts.runtime_can_connect && facts.other_sessions == 0 {
            facts.validate()?;
            break;
        }
        if facts.runtime_can_connect || facts.other_login_writers != 0 || Instant::now() >= deadline
        {
            return Err(BackupError::Invalid("database sessions did not drain"));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    journal.advance(GatePhase::Drained, None)?;

    let plan = AdminAssetCatalog::new(admin.clone()).plan_assets().await?;
    let identity = collect_source_identity(admin).await?;
    let role_bytes = role_recipe(admin).await?;
    let mut roles_write = source.create_file("roles.json")?;
    roles_write.write_all(&role_bytes)?;
    roles_write.sync_all()?;
    drop(roles_write);
    let mut roles = source.open_file("roles.json")?;
    let role_record = record_file("roles.json", &mut roles)?;
    let mut dump_write = source.create_file("database.dump")?;
    dump_spec.run_to_file(
        &config.pg_dump_executable,
        &config.pgpassfile,
        &mut dump_write,
    )?;
    drop(dump_write);
    source.sync()?;
    let mut dump = source.open_file("database.dump")?;
    let mut magic = [0_u8; 5];
    dump.read_exact(&mut magic)?;
    if &magic != b"PGDMP" {
        return Err(BackupError::Invalid("pg_dump custom format magic"));
    }
    let dump_record = record_file("database.dump", &mut dump)?;
    write_bytes(&source, "asset-index.json", plan.asset_index_bytes())?;
    let manifest =
        BackupManifestV1::from_plan(config.backup_id, identity, &plan, dump_record, role_record)?;
    let manifest_sha256 = manifest.canonical_sha256()?;
    write_bytes(&source, "manifest.json", &manifest.canonical_bytes()?)?;
    inspect_gate(admin).await?.validate()?;
    journal.advance(GatePhase::DumpAndIndexDurable, Some(&manifest_sha256))?;

    let sealed = seal_backup(
        &config.local_pin_root,
        &manifest,
        &plan,
        assets,
        &mut dump,
        &mut roles,
    )?;
    if sealed.manifest_sha256() != manifest_sha256 {
        return Err(BackupError::Invalid("local pin manifest digest"));
    }
    journal.advance(GatePhase::PinsDurable, Some(&manifest_sha256))?;
    inspect_gate(admin).await?.validate()?;
    release_runtime_connect(admin, &database, &mut journal).await?;
    Ok(SourceLocalPin { sealed, manifest })
}

#[cfg(target_os = "linux")]
async fn release_runtime_connect(
    admin: &PgPool,
    database: &str,
    journal: &mut SourceGateJournal,
) -> Result<(), BackupError> {
    journal.advance(GatePhase::ReleaseReady, None)?;
    let grant = format!("GRANT CONNECT ON DATABASE \"{database}\" TO learning_runtime");
    let granted = sqlx::query(&grant).execute(admin).await;
    if granted.is_err() {
        compensate_release(admin, database).await?;
        return Err(BackupError::Invalid("runtime CONNECT grant failed"));
    }
    match inspect_gate(admin).await {
        Ok(after) if after.runtime_can_connect && !after.public_can_connect => {}
        _ => {
            compensate_release(admin, database).await?;
            return Err(BackupError::Invalid(
                "runtime CONNECT release could not be verified",
            ));
        }
    }
    if let Err(error) = journal.advance(GatePhase::Released, None) {
        compensate_release(admin, database).await?;
        return Err(error);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
async fn compensate_release(admin: &PgPool, database: &str) -> Result<(), BackupError> {
    let revoke = format!("REVOKE CONNECT ON DATABASE \"{database}\" FROM PUBLIC, learning_runtime");
    if sqlx::query(&revoke).execute(admin).await.is_err() {
        return Err(BackupError::Invalid(
            "source gate outcome ambiguous; manual recovery required",
        ));
    }
    match inspect_gate(admin).await {
        Ok(facts) if facts.validate().is_ok() => Ok(()),
        _ => Err(BackupError::Invalid(
            "source gate outcome ambiguous; manual recovery required",
        )),
    }
}

/// Written by the root-owned isolation driver immediately after a live Docker
/// inspect of this exact project. It must keep exclusive orchestration control
/// until this call returns: transient admin sessions during pg_dump cannot be
/// ruled out by two pg_stat_activity snapshots. The driver retains its own
/// inspect output; this private attestation binds it to one backup attempt.
#[cfg(target_os = "linux")]
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct IsolationAttestation {
    format_version: u32,
    backup_id: Uuid,
    database: String,
    compose_project: String,
    observed_unix_ms: u64,
    runtime_running: u32,
    worker_running: u32,
    other_admin_processes: u32,
    postgres_published_ports: u32,
    network_internal: bool,
    manager_runtime_secret_mounts: u32,
    manager_runtime_env_keys: u32,
    docker_inspection_sha256: String,
}

#[cfg(target_os = "linux")]
fn verify_isolation_attestation(config: &SourceBackupConfig) -> Result<(), BackupError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let parent = config
        .isolation_attestation
        .parent()
        .ok_or(BackupError::Invalid("isolation attestation path"))?;
    let leaf = config
        .isolation_attestation
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or(BackupError::Invalid("isolation attestation file name"))?;
    if leaf != format!("isolation-{}.json", config.backup_id)
        || !config
            .expected_compose_project
            .starts_with("learning-system-p0c4-")
    {
        return Err(BackupError::Invalid("isolation attestation identity"));
    }
    let root = BackupDir::open_private_root(parent)?;
    let mut file = root.open_file(leaf)?;
    let metadata = file.metadata()?;
    if metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o777 != 0o600
        || metadata.len() > 4096
    {
        return Err(BackupError::Invalid("private isolation attestation"));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let proof: IsolationAttestation = serde_json::from_slice(&bytes)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| BackupError::Invalid("system clock"))?
        .as_millis();
    let age = now
        .checked_sub(u128::from(proof.observed_unix_ms))
        .ok_or(BackupError::Invalid("future isolation attestation"))?;
    if proof.format_version != 1
        || proof.backup_id != config.backup_id
        || proof.database != config.expected_database
        || proof.compose_project != config.expected_compose_project
        || age > 30_000
        || proof.runtime_running != 0
        || proof.worker_running != 0
        || proof.other_admin_processes != 0
        || proof.postgres_published_ports != 0
        || !proof.network_internal
        || proof.manager_runtime_secret_mounts != 0
        || proof.manager_runtime_env_keys != 0
        || !crate::valid_digest(&proof.docker_inspection_sha256)
    {
        return Err(BackupError::Invalid(
            "isolation attestation does not prove stopped services",
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn ensure_no_unfinished_journal(root: &BackupDir, path: &Path) -> Result<(), BackupError> {
    for name in root.list()? {
        if let Some(id) = name.strip_suffix(".control") {
            let id = Uuid::parse_str(id)
                .map_err(|_| BackupError::Invalid("unknown control directory"))?;
            let journal = SourceGateJournal::recover(path, id)?;
            if journal.record().phase() != GatePhase::Released {
                return Err(BackupError::Invalid(
                    "unfinished source maintenance journal",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn write_bytes(dir: &BackupDir, name: &str, bytes: &[u8]) -> Result<(), BackupError> {
    let mut file = dir.create_file(name)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    dir.sync()?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn record_file(path: &str, file: &mut File) -> Result<FileRecord, BackupError> {
    file.seek(SeekFrom::Start(0))?;
    let mut hash = Sha256::new();
    let mut size = 0_u64;
    let mut buf = [0_u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        size = size.checked_add(n as u64).ok_or(BackupError::Overflow)?;
        hash.update(&buf[..n]);
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(FileRecord {
        path: path.into(),
        size,
        sha256: format!("{:x}", hash.finalize()),
    })
}

#[cfg(target_os = "linux")]
async fn require_admin_owner(pool: &PgPool) -> Result<String, BackupError> {
    let (current, session, authenticated, database, owner): (String, String, Option<String>, String, String) = sqlx::query_as(
        "SELECT current_user::text, session_user::text, system_user, current_database()::text, pg_get_userbyid(d.datdba)::text FROM pg_catalog.pg_database d WHERE d.datname=current_database()",
    ).fetch_one(pool).await?;
    if current != "learning_admin"
        || session != "learning_admin"
        || owner != "learning_admin"
        || authenticated
            .as_deref()
            .and_then(|v| v.split_once(':'))
            .is_none_or(|(method, user)| method.is_empty() || user != "learning_admin")
        || database.is_empty()
        || !database
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(BackupError::Invalid(
            "authenticated management database owner required",
        ));
    }
    Ok(database)
}

#[cfg(target_os = "linux")]
async fn inspect_gate(pool: &PgPool) -> Result<GateInspection, BackupError> {
    let mut conn = pool.acquire().await?;
    let row = sqlx::query(
        "SELECT \
            (pg_get_userbyid(d.datdba)='learning_admin') AS admin_is_owner, \
            has_database_privilege('learning_runtime',current_database(),'CONNECT') AS runtime_connect, \
            EXISTS(SELECT 1 FROM aclexplode(COALESCE(d.datacl,acldefault('d',d.datdba))) a WHERE a.grantee=0 AND a.privilege_type='CONNECT') AS public_connect, \
            EXISTS(SELECT 1 FROM pg_catalog.pg_auth_members am JOIN pg_catalog.pg_roles r ON r.oid=am.member WHERE r.rolname='learning_runtime') AS runtime_inherits_admin, \
            (SELECT rolsuper OR rolbypassrls OR rolcreaterole OR rolcreatedb FROM pg_catalog.pg_roles WHERE rolname='learning_runtime') AS runtime_privileged, \
            (SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid()) AS other_sessions, \
            (SELECT count(*) FROM pg_catalog.pg_roles r WHERE r.rolcanlogin AND r.rolname NOT IN ('learning_admin','learning_runtime','postgres') AND has_database_privilege(r.rolname,current_database(),'CONNECT')) AS other_login_writers \
         FROM pg_catalog.pg_database d WHERE d.datname=current_database()",
    ).fetch_one(&mut *conn).await?;
    Ok(GateInspection {
        admin_is_database_owner: row.try_get("admin_is_owner")?,
        runtime_can_connect: row.try_get("runtime_connect")?,
        public_can_connect: row.try_get("public_connect")?,
        runtime_can_inherit_admin: row.try_get("runtime_inherits_admin")?,
        runtime_is_privileged: row.try_get("runtime_privileged")?,
        other_sessions: row.try_get("other_sessions")?,
        other_login_writers: row.try_get("other_login_writers")?,
    })
}

#[cfg(target_os = "linux")]
async fn collect_source_identity(pool: &PgPool) -> Result<SourceIdentity, BackupError> {
    let version: String = sqlx::query_scalar("SHOW server_version_num")
        .fetch_one(pool)
        .await?;
    let version = version
        .parse::<u32>()
        .map_err(|_| BackupError::Invalid("PostgreSQL version"))?;
    if version / 10_000 != 18 {
        return Err(BackupError::Invalid("PostgreSQL 18 required"));
    }
    let rows: Vec<(i64, String, bool)> = sqlx::query_as(
        "SELECT version,encode(checksum,'hex'),success FROM public._sqlx_migrations ORDER BY version",
    ).fetch_all(pool).await?;
    let expected = MIGRATOR.iter().collect::<Vec<_>>();
    if rows.len() != expected.len() {
        return Err(BackupError::Invalid(
            "migration set differs from embedded build",
        ));
    }
    let mut migrations = Vec::with_capacity(rows.len());
    for ((version, checksum, success), embedded) in rows.into_iter().zip(expected) {
        let embedded_hex = embedded
            .checksum
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        if !success || version != embedded.version || checksum != embedded_hex {
            return Err(BackupError::Invalid(
                "migration checksum or status differs from embedded build",
            ));
        }
        migrations.push(MigrationRecord {
            version: u64::try_from(version).map_err(|_| BackupError::Overflow)?,
            checksum_hex: checksum,
        });
    }
    let commit = option_env!("KNOWWEAVE_SOURCE_COMMIT")
        .ok_or(BackupError::Invalid("build lacks embedded source commit"))?;
    let mut executable = File::open(std::env::current_exe()?)?;
    let build_sha256 = record_file("running-executable", &mut executable)?.sha256;
    SourceIdentity::from_migrations(build_sha256, commit.into(), 18, migrations)
}

#[cfg(target_os = "linux")]
#[derive(Serialize)]
struct RoleRecipe {
    format_version: u32,
    roles: Vec<RoleRecord>,
    memberships: Vec<RoleMembership>,
}
#[cfg(target_os = "linux")]
#[derive(Serialize)]
struct RoleRecord {
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
#[cfg(target_os = "linux")]
#[derive(Serialize)]
struct RoleMembership {
    role: String,
    member: String,
    inherit: bool,
    set: bool,
    admin: bool,
}

#[cfg(target_os = "linux")]
async fn role_recipe(pool: &PgPool) -> Result<Vec<u8>, BackupError> {
    let rows = sqlx::query(
        "SELECT rolname::text AS name,rolcanlogin,rolinherit,rolsuper,rolcreatedb,rolcreaterole,rolbypassrls,rolreplication,rolconnlimit FROM pg_catalog.pg_roles WHERE rolname IN ('learning_admin','learning_runtime','learning_auth_lock') ORDER BY rolname",
    ).fetch_all(pool).await?;
    if rows.len() != 3 {
        return Err(BackupError::Invalid("missing role recipe role"));
    }
    let mut roles = Vec::new();
    for row in rows {
        let name: String = row.try_get("name")?;
        let login: bool = row.try_get("rolcanlogin")?;
        let inherit: bool = row.try_get("rolinherit")?;
        let superuser: bool = row.try_get("rolsuper")?;
        let createdb: bool = row.try_get("rolcreatedb")?;
        let createrole: bool = row.try_get("rolcreaterole")?;
        let bypassrls: bool = row.try_get("rolbypassrls")?;
        let replication: bool = row.try_get("rolreplication")?;
        let connection_limit: i32 = row.try_get("rolconnlimit")?;
        if superuser
            || createdb
            || createrole
            || bypassrls
            || replication
            || login != (name != "learning_auth_lock")
        {
            return Err(BackupError::Invalid("unexpected role recipe privileges"));
        }
        roles.push(RoleRecord {
            name,
            login,
            inherit,
            superuser,
            createdb,
            createrole,
            bypassrls,
            replication,
            connection_limit,
        });
    }
    let rows = sqlx::query(
        "SELECT granted.rolname::text AS role,member.rolname::text AS member,am.inherit_option,am.set_option,am.admin_option FROM pg_catalog.pg_auth_members am JOIN pg_catalog.pg_roles granted ON granted.oid=am.roleid JOIN pg_catalog.pg_roles member ON member.oid=am.member WHERE granted.rolname IN ('learning_admin','learning_runtime','learning_auth_lock') OR member.rolname IN ('learning_admin','learning_runtime','learning_auth_lock') ORDER BY granted.rolname,member.rolname",
    ).fetch_all(pool).await?;
    if rows.len() != 1 {
        return Err(BackupError::Invalid("role membership recipe differs"));
    }
    let row = &rows[0];
    let membership = RoleMembership {
        role: row.try_get("role")?,
        member: row.try_get("member")?,
        inherit: row.try_get("inherit_option")?,
        set: row.try_get("set_option")?,
        admin: row.try_get("admin_option")?,
    };
    if membership.role != "learning_auth_lock"
        || membership.member != "learning_admin"
        || membership.inherit
        || !membership.set
        || membership.admin
    {
        return Err(BackupError::Invalid("unexpected role membership recipe"));
    }
    Ok(serde_json::to_vec(&RoleRecipe {
        format_version: 1,
        roles,
        memberships: vec![membership],
    })?)
}

#[cfg(all(test, target_os = "linux"))]
mod linux_gate_tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn isolation_proof_is_private_fresh_and_bound_to_one_attempt() {
        let root = std::env::temp_dir().join(format!("c4-isolation-{}", Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let id = Uuid::new_v4();
        let file = root.join(format!("isolation-{id}.json"));
        let config = SourceBackupConfig {
            backup_id: id,
            expected_database: "learning_backup_c4_task3_a".into(),
            expected_compose_project: "learning-system-p0c4-isolation-test".into(),
            isolation_attestation: file.clone(),
            control_root: root.clone(),
            local_pin_root: root.join("pin"),
            pg_dump_executable: PathBuf::from("/usr/bin/pg_dump"),
            pgpassfile: root.join("pgpass"),
            pg_host: "pg".into(),
            pg_port: 5432,
            drain_timeout: Duration::from_secs(5),
        };
        assert!(verify_isolation_attestation(&config).is_err());
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let mut proof = serde_json::json!({
            "format_version": 1,
            "backup_id": id,
            "database": config.expected_database,
            "compose_project": config.expected_compose_project,
            "observed_unix_ms": now,
            "runtime_running": 0,
            "worker_running": 0,
            "other_admin_processes": 0,
            "postgres_published_ports": 0,
            "network_internal": true,
            "manager_runtime_secret_mounts": 0,
            "manager_runtime_env_keys": 0,
            "docker_inspection_sha256": "a".repeat(64)
        });
        std::fs::write(&file, serde_json::to_vec(&proof).unwrap()).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(verify_isolation_attestation(&config).is_ok());
        proof["worker_running"] = serde_json::json!(1);
        std::fs::write(&file, serde_json::to_vec(&proof).unwrap()).unwrap();
        assert!(verify_isolation_attestation(&config).is_err());
        proof["worker_running"] = serde_json::json!(0);
        proof["manager_runtime_secret_mounts"] = serde_json::json!(1);
        std::fs::write(&file, serde_json::to_vec(&proof).unwrap()).unwrap();
        assert!(verify_isolation_attestation(&config).is_err());
        proof["manager_runtime_secret_mounts"] = serde_json::json!(0);
        proof["observed_unix_ms"] = serde_json::json!(now - 31_000);
        std::fs::write(&file, serde_json::to_vec(&proof).unwrap()).unwrap();
        assert!(verify_isolation_attestation(&config).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Dedicated, already-gated UUID database. Deliberately collide with the
    /// final journal name after GRANT; the compensation must revoke CONNECT.
    #[tokio::test]
    #[ignore = "requires separate isolated PG18 release-fault database"]
    async fn release_journal_failure_recloses_runtime_connect() {
        let database = std::env::var("TEST_C4_RELEASE_DATABASE_NAME").unwrap();
        let suffix = database
            .strip_prefix("learning_backup_c4_task3_release_")
            .unwrap();
        assert_eq!(Uuid::parse_str(suffix).unwrap().to_string(), suffix);
        let url = std::env::var("TEST_C4_RELEASE_ADMIN_DATABASE_URL").unwrap();
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap();
        assert_eq!(require_admin_owner(&pool).await.unwrap(), database);
        let root = PathBuf::from(std::env::var("TEST_C4_RELEASE_CONTROL_ROOT").unwrap());
        let id = Uuid::new_v4();
        let mut journal = SourceGateJournal::start(&root, id).unwrap();
        let revoke =
            format!("REVOKE CONNECT ON DATABASE \"{database}\" FROM PUBLIC, learning_runtime");
        sqlx::query(&revoke).execute(&pool).await.unwrap();
        journal.advance(GatePhase::Closed, None).unwrap();
        inspect_gate(&pool).await.unwrap().validate().unwrap();
        journal.advance(GatePhase::Drained, None).unwrap();
        journal
            .advance(GatePhase::DumpAndIndexDurable, Some(&"a".repeat(64)))
            .unwrap();
        journal
            .advance(GatePhase::PinsDurable, Some(&"b".repeat(64)))
            .unwrap();
        let collision = root.join(format!("{id}.control")).join("released.json");
        std::fs::create_dir(&collision).unwrap();
        assert!(
            release_runtime_connect(&pool, &database, &mut journal)
                .await
                .is_err()
        );
        assert_eq!(journal.record().phase(), GatePhase::ReleaseReady);
        assert!(!inspect_gate(&pool).await.unwrap().runtime_can_connect);
        std::fs::remove_dir(collision).unwrap();
        force_close_release_ready(&pool, &root, id, &database)
            .await
            .unwrap();
        assert!(
            root.join(format!("{id}.release-recovery"))
                .join("closed.json")
                .is_file()
        );
    }
}

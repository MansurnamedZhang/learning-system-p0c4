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
#[cfg(any(target_os = "linux", test))]
use serde::{Deserialize, Serialize};
#[cfg(any(target_os = "linux", test))]
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

#[cfg(any(target_os = "linux", test))]
#[derive(Debug, Default)]
struct RestoreCatalogCounts {
    relations: i64,
    schemas: i64,
    routines: i64,
    types: i64,
    extensions: i64,
    event_triggers: i64,
    publications: i64,
    large_objects: i64,
    collations: i64,
    conversions: i64,
    operators: i64,
    operator_classes: i64,
    operator_families: i64,
    text_search_objects: i64,
    default_acls: i64,
    foreign_objects: i64,
    custom_languages: i64,
    custom_access_methods: i64,
    global_ddl: i64,
}

#[cfg(any(target_os = "linux", test))]
impl RestoreCatalogCounts {
    fn total(&self) -> Result<u64, BackupError> {
        [
            self.relations,
            self.schemas,
            self.routines,
            self.types,
            self.extensions,
            self.event_triggers,
            self.publications,
            self.large_objects,
            self.collations,
            self.conversions,
            self.operators,
            self.operator_classes,
            self.operator_families,
            self.text_search_objects,
            self.default_acls,
            self.foreign_objects,
            self.custom_languages,
            self.custom_access_methods,
            self.global_ddl,
        ]
        .into_iter()
        .try_fold(0_u64, |sum, count| {
            sum.checked_add(u64::try_from(count).map_err(|_| BackupError::Overflow)?)
                .ok_or(BackupError::Overflow)
        })
    }
}

/// Created by a separate, reviewed, root-only target provisioner before this
/// preflight. This crate only consumes a build-pinned copy; it never issues one.
/// Project/volume names remain issuer claims until a later driver independently
/// re-inspects the live Docker mount under its creation lock.
#[cfg(any(target_os = "linux", test))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetBirthAttestation {
    format_version: u32,
    project_name: String,
    pg_volume_name: String,
    database_name: String,
    database_oid: u64,
    pg_system_identifier: String,
    control_dev: u64,
    control_ino: u64,
    asset_dev: u64,
    asset_ino: u64,
    creation_nonce: uuid::Uuid,
    template_database: String,
    baseline_cast_count: u64,
}

#[cfg(any(target_os = "linux", test))]
#[derive(Debug, Clone)]
struct ObservedTargetBirth {
    database_oid: u64,
    pg_system_identifier: String,
    control_dev: u64,
    control_ino: u64,
    asset_dev: u64,
    asset_ino: u64,
    cast_count: u64,
}

#[cfg(any(target_os = "linux", test))]
impl TargetBirthAttestation {
    fn validate(&self, database: &str, live: &ObservedTargetBirth) -> Result<(), BackupError> {
        let suffix = database
            .strip_prefix("learning_restore_c4_")
            .ok_or(BackupError::Invalid("target birth database"))?;
        let id = uuid::Uuid::parse_str(suffix)
            .map_err(|_| BackupError::Invalid("target birth database"))?;
        let project = format!("learning-system-p0c4-restore-{id}");
        if self.format_version != 1
            || self.project_name != project
            || self.pg_volume_name != format!("{project}_pg")
            || self.database_name != database
            || self.template_database != "template0"
            || self.creation_nonce.is_nil()
            || self.database_oid == 0
            || self.control_dev == 0
            || self.control_ino == 0
            || self.asset_dev == 0
            || self.asset_ino == 0
            || self.baseline_cast_count == 0
            || self
                .pg_system_identifier
                .parse::<u64>()
                .ok()
                .filter(|value| *value > 0 && value.to_string() == self.pg_system_identifier)
                .is_none()
            || self.database_oid != live.database_oid
            || self.pg_system_identifier != live.pg_system_identifier
            || self.control_dev != live.control_dev
            || self.control_ino != live.control_ino
            || self.asset_dev != live.asset_dev
            || self.asset_ino != live.asset_ino
            || self.baseline_cast_count != live.cast_count
        {
            return Err(BackupError::Invalid("target birth identity differs"));
        }
        Ok(())
    }
}

#[cfg(any(target_os = "linux", test))]
fn parse_pinned_birth(
    bytes: &[u8],
    pinned_sha256: &str,
) -> Result<TargetBirthAttestation, BackupError> {
    if bytes.is_empty()
        || bytes.len() > 4096
        || !crate::valid_digest(pinned_sha256)
        || format!("{:x}", Sha256::digest(bytes)) != pinned_sha256
    {
        return Err(BackupError::Invalid(
            "target birth attestation is not pinned",
        ));
    }
    let birth: TargetBirthAttestation = serde_json::from_slice(bytes)?;
    if serde_json::to_vec(&birth)? != bytes {
        return Err(BackupError::Invalid(
            "noncanonical target birth attestation",
        ));
    }
    Ok(birth)
}

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
    let lock_root = BackupDir::open_trusted_private_root(&config.control_root)?;
    let _trust_parent = BackupDir::open_trusted_private_root(
        config
            .trust_path
            .parent()
            .ok_or(BackupError::Invalid("verifier trust parent"))?,
    )?;
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
    let destination = BackupDir::open_trusted_private_root(&config.destination_root)?;
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
    let assets = BackupDir::open_trusted_private_root(&config.asset_root)?;
    let facts = target_facts(admin, assets.list()?.len()).await?;
    facts.validate()?;
    verify_target_birth(admin, config, &lock_root, &assets).await?;
    Ok(RestorePreflight {
        manifest,
        plan,
        _lock: lock,
    })
}

#[cfg(target_os = "linux")]
async fn verify_target_birth(
    admin: &PgPool,
    config: &RestorePreflightConfig,
    control: &BackupDir,
    assets: &BackupDir,
) -> Result<(), BackupError> {
    // This pin is installed in the reviewed build *before* restore. A
    // same-run caller cannot provide the expected IDs or digest in config.
    let pinned = option_env!("KNOWWEAVE_C4_TARGET_BIRTH_SHA256").ok_or(BackupError::Invalid(
        "target birth digest is not build-pinned",
    ))?;
    let name = format!("{}.birth.json", config.expected_database);
    let file = control.open_file(&name)?;
    let meta = file.metadata()?;
    if meta.uid() != 0 || meta.permissions().mode() & 0o777 != 0o600 || meta.nlink() != 1 {
        return Err(BackupError::Invalid("private target birth attestation"));
    }
    let mut bytes = Vec::new();
    file.take(4097).read_to_end(&mut bytes)?;
    let birth = parse_pinned_birth(&bytes, pinned)?;
    // The isolated PG18 bootstrap must grant only EXECUTE on
    // pg_control_system() to learning_admin, or use a trusted admin observer.
    // Missing privilege fails closed; pg_monitor is not required or implied.
    let row = sqlx::query(
        "SELECT d.oid::bigint AS database_oid, pcs.system_identifier::text AS system_identifier, \
         (SELECT count(*) FROM pg_catalog.pg_cast) AS cast_count \
         FROM pg_catalog.pg_database d CROSS JOIN pg_catalog.pg_control_system() pcs \
         WHERE d.datname=pg_catalog.current_database()",
    )
    .fetch_one(admin)
    .await?;
    let database_oid =
        u64::try_from(row.try_get::<i64, _>("database_oid")?).map_err(|_| BackupError::Overflow)?;
    let (control_dev, control_ino) = control.identity()?;
    let (asset_dev, asset_ino) = assets.identity()?;
    let live = ObservedTargetBirth {
        database_oid,
        pg_system_identifier: row.try_get("system_identifier")?,
        control_dev,
        control_ino,
        asset_dev,
        asset_ino,
        cast_count: u64::try_from(row.try_get::<i64, _>("cast_count")?)
            .map_err(|_| BackupError::Overflow)?,
    };
    birth.validate(&config.expected_database, &live)
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
         (pg_catalog.pg_get_userbyid(d.datdba)='learning_admin') AS admin_owner, \
         pg_catalog.has_database_privilege('learning_runtime',pg_catalog.current_database(),'CONNECT') AS runtime_connect, \
         EXISTS(SELECT 1 FROM pg_catalog.aclexplode(COALESCE(d.datacl,pg_catalog.acldefault('d',d.datdba))) a WHERE a.grantee=0 AND a.privilege_type='CONNECT') AS public_connect, \
         (SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE datname=pg_catalog.current_database() AND pid<>pg_catalog.pg_backend_pid()) AS other_sessions, \
         (SELECT count(*) FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
           WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname NOT LIKE 'pg_toast%' AND n.nspname NOT LIKE 'pg_temp%') AS user_relations, \
         (SELECT count(*) FROM pg_catalog.pg_namespace n WHERE n.nspname NOT IN ('pg_catalog','information_schema','public') \
           AND n.nspname NOT LIKE 'pg_toast%' AND n.nspname NOT LIKE 'pg_temp%') AS user_schemas, \
         (SELECT count(*) FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
           WHERE n.nspname='public') AS user_routines, \
         (SELECT count(*) FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace \
           WHERE n.nspname='public') AS user_types, \
         (SELECT count(*) FROM pg_catalog.pg_extension WHERE extname<>'plpgsql') AS user_extensions, \
         (SELECT count(*) FROM pg_catalog.pg_event_trigger) AS user_event_triggers, \
         (SELECT count(*) FROM pg_catalog.pg_publication) AS user_publications, \
         (SELECT count(*) FROM pg_catalog.pg_largeobject_metadata) AS user_large_objects, \
         (SELECT count(*) FROM pg_catalog.pg_collation c JOIN pg_catalog.pg_namespace n ON n.oid=c.collnamespace \
           WHERE n.nspname='public') AS user_collations, \
         (SELECT count(*) FROM pg_catalog.pg_conversion c JOIN pg_catalog.pg_namespace n ON n.oid=c.connamespace \
           WHERE n.nspname='public') AS user_conversions, \
         (SELECT count(*) FROM pg_catalog.pg_operator o JOIN pg_catalog.pg_namespace n ON n.oid=o.oprnamespace \
           WHERE n.nspname='public') AS user_operators, \
         (SELECT count(*) FROM pg_catalog.pg_opclass o JOIN pg_catalog.pg_namespace n ON n.oid=o.opcnamespace \
           WHERE n.nspname='public') AS user_operator_classes, \
         (SELECT count(*) FROM pg_catalog.pg_opfamily o JOIN pg_catalog.pg_namespace n ON n.oid=o.opfnamespace \
           WHERE n.nspname='public') AS user_operator_families, \
         ((SELECT count(*) FROM pg_catalog.pg_ts_config t JOIN pg_catalog.pg_namespace n ON n.oid=t.cfgnamespace WHERE n.nspname='public') + \
          (SELECT count(*) FROM pg_catalog.pg_ts_dict t JOIN pg_catalog.pg_namespace n ON n.oid=t.dictnamespace WHERE n.nspname='public') + \
          (SELECT count(*) FROM pg_catalog.pg_ts_parser t JOIN pg_catalog.pg_namespace n ON n.oid=t.prsnamespace WHERE n.nspname='public') + \
          (SELECT count(*) FROM pg_catalog.pg_ts_template t JOIN pg_catalog.pg_namespace n ON n.oid=t.tmplnamespace WHERE n.nspname='public')) AS user_text_search_objects, \
         (SELECT count(*) FROM pg_catalog.pg_default_acl) AS user_default_acls, \
         ((SELECT count(*) FROM pg_catalog.pg_foreign_data_wrapper) + \
          (SELECT count(*) FROM pg_catalog.pg_foreign_server)) AS user_foreign_objects, \
         (SELECT count(*) FROM pg_catalog.pg_language WHERE lanname NOT IN ('internal','c','sql','plpgsql')) AS user_custom_languages, \
         (SELECT count(*) FROM pg_catalog.pg_am WHERE amname NOT IN ('heap','btree','hash','gist','gin','spgist','brin')) AS user_custom_access_methods, \
         ((SELECT count(*) FROM pg_catalog.pg_transform) + \
          (SELECT count(*) FROM pg_catalog.pg_parameter_acl) + \
          (SELECT count(*) FROM pg_catalog.pg_subscription) + \
          (SELECT count(*) FROM pg_catalog.pg_db_role_setting) + \
          (SELECT count(*) FROM pg_catalog.pg_tablespace WHERE spcname NOT IN ('pg_default','pg_global')) + \
          (SELECT count(*) FROM pg_catalog.pg_database WHERE datname NOT IN ('template0','template1','postgres',pg_catalog.current_database()))) AS user_global_ddl \
         FROM pg_catalog.pg_database d WHERE d.datname=pg_catalog.current_database()"
    ).fetch_one(&mut *conn).await?;
    let current_role: String = row.try_get("current_role")?;
    let session_role: String = row.try_get("session_role")?;
    if current_role != "learning_admin" || session_role != "learning_admin" {
        return Err(BackupError::Invalid(
            "restore requires authenticated admin session",
        ));
    }
    let non_system_relations = RestoreCatalogCounts {
        relations: row.try_get("user_relations")?,
        schemas: row.try_get("user_schemas")?,
        routines: row.try_get("user_routines")?,
        types: row.try_get("user_types")?,
        extensions: row.try_get("user_extensions")?,
        event_triggers: row.try_get("user_event_triggers")?,
        publications: row.try_get("user_publications")?,
        large_objects: row.try_get("user_large_objects")?,
        collations: row.try_get("user_collations")?,
        conversions: row.try_get("user_conversions")?,
        operators: row.try_get("user_operators")?,
        operator_classes: row.try_get("user_operator_classes")?,
        operator_families: row.try_get("user_operator_families")?,
        text_search_objects: row.try_get("user_text_search_objects")?,
        default_acls: row.try_get("user_default_acls")?,
        foreign_objects: row.try_get("user_foreign_objects")?,
        custom_languages: row.try_get("user_custom_languages")?,
        custom_access_methods: row.try_get("user_custom_access_methods")?,
        global_ddl: row.try_get("user_global_ddl")?,
    }
    .total()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_objects_outside_relations_make_a_target_dirty() {
        for kind in 0..18 {
            let mut counts = RestoreCatalogCounts::default();
            match kind {
                0 => counts.routines = 1,
                1 => counts.types = 1,
                2 => counts.extensions = 1,
                3 => counts.event_triggers = 1,
                4 => counts.publications = 1,
                5 => counts.large_objects = 1,
                6 => counts.schemas = 1,
                7 => counts.collations = 1,
                8 => counts.conversions = 1,
                9 => counts.operators = 1,
                10 => counts.operator_classes = 1,
                11 => counts.operator_families = 1,
                12 => counts.text_search_objects = 1,
                13 => counts.default_acls = 1,
                14 => counts.foreign_objects = 1,
                15 => counts.custom_languages = 1,
                16 => counts.custom_access_methods = 1,
                _ => counts.global_ddl = 1,
            }
            assert!(counts.total().unwrap() > 0, "catalog family {kind}");
        }
        assert_eq!(RestoreCatalogCounts::default().total().unwrap(), 0);
    }

    #[test]
    fn birth_attestation_must_match_independent_live_facts() {
        let id = uuid::Uuid::new_v4();
        let database = format!("learning_restore_c4_{id}");
        let project = format!("learning-system-p0c4-restore-{id}");
        let birth = TargetBirthAttestation {
            format_version: 1,
            project_name: project.clone(),
            pg_volume_name: format!("{project}_pg"),
            database_name: database.clone(),
            database_oid: 16385,
            pg_system_identifier: "7361082129910479001".into(),
            control_dev: 42,
            control_ino: 100,
            asset_dev: 43,
            asset_ino: 200,
            creation_nonce: uuid::Uuid::new_v4(),
            template_database: "template0".into(),
            baseline_cast_count: 203,
        };
        let live = ObservedTargetBirth {
            database_oid: 16385,
            pg_system_identifier: "7361082129910479001".into(),
            control_dev: 42,
            control_ino: 100,
            asset_dev: 43,
            asset_ino: 200,
            cast_count: 203,
        };
        assert!(birth.validate(&database, &live).is_ok());
        let mut wrong = live.clone();
        wrong.pg_system_identifier = "7361082129910479002".into();
        assert!(birth.validate(&database, &wrong).is_err());
        wrong = live.clone();
        wrong.database_oid += 1;
        assert!(birth.validate(&database, &wrong).is_err());
        wrong = live.clone();
        wrong.asset_ino += 1;
        assert!(birth.validate(&database, &wrong).is_err());
        let mut replayed = birth.clone();
        replayed.pg_volume_name = "old-volume_pg".into();
        assert!(replayed.validate(&database, &live).is_err());
        wrong = live.clone();
        wrong.cast_count += 1;
        assert!(birth.validate(&database, &wrong).is_err());
    }

    #[test]
    fn birth_attestation_requires_exact_canonical_pinned_bytes() {
        let id = uuid::Uuid::new_v4();
        let database = format!("learning_restore_c4_{id}");
        let project = format!("learning-system-p0c4-restore-{id}");
        let birth = TargetBirthAttestation {
            format_version: 1,
            project_name: project.clone(),
            pg_volume_name: format!("{project}_pg"),
            database_name: database,
            database_oid: 16385,
            pg_system_identifier: "7361082129910479001".into(),
            control_dev: 42,
            control_ino: 100,
            asset_dev: 43,
            asset_ino: 200,
            creation_nonce: uuid::Uuid::new_v4(),
            template_database: "template0".into(),
            baseline_cast_count: 203,
        };
        let canonical = serde_json::to_vec(&birth).unwrap();
        let pinned = format!("{:x}", sha2::Sha256::digest(&canonical));
        assert!(parse_pinned_birth(&canonical, &pinned).is_ok());
        let mut whitespace = canonical.clone();
        whitespace.push(b'\n');
        assert!(parse_pinned_birth(&whitespace, &pinned).is_err());
        assert!(parse_pinned_birth(&canonical, &"0".repeat(64)).is_err());
    }
}

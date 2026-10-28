//! Management-side source capture. The result is a local `sealed` pin, never
//! an independent fault-domain copy or a restorable `complete` receipt.
#[cfg(target_os = "linux")]
mod admission;
#[cfg(any(target_os = "linux", test))]
pub(crate) mod binding;
pub(crate) mod c0_d1;
#[cfg(any(target_os = "linux", test))]
pub(crate) mod lifecycle;
#[cfg(test)]
pub(crate) mod lifecycle_tests;
#[cfg(target_os = "linux")]
pub(crate) mod snapshot_effects;
#[cfg(target_os = "linux")]
pub(crate) use admission::SourceAdmission;
#[cfg(all(test, target_os = "linux"))]
mod admission_tests;
#[cfg(all(test, target_os = "linux"))]
mod binding_tests;
#[cfg(any(target_os = "linux", test))]
mod endpoint;
#[cfg(test)]
mod endpoint_tests;

#[cfg(target_os = "linux")]
use crate::maintenance::valid_c4_database;
use crate::{BackupError, BackupManifestV1, SealedBackup};
#[cfg(target_os = "linux")]
use crate::{
    FileRecord, GateInspection, GatePhase, MigrationRecord, SourceGateJournal, SourceIdentity,
};
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
use sqlx::{PgConnection, Row};
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

/// Explicitly finish an interrupted capture only from an already published,
/// fully verified local pin. Cancellation in a GRANT window requires explicit
/// inspection and reclosure; dropping admission never reopens the gate.
pub async fn finish_source_backup(
    admin: &PgPool,
    config: &SourceBackupConfig,
) -> Result<SourceLocalPin, BackupError> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (admin, config);
        Err(BackupError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "source lifecycle requires Linux",
        )))
    }
    #[cfg(target_os = "linux")]
    {
        lifecycle::finish(admin, config).await
    }
}
/// Persistently abandon an unresolved source attempt, retaining every original.
/// A visible terminal with closed ACL is ambiguous and requires manual handling.
pub async fn abandon_source_backup(
    admin: &PgPool,
    config: &SourceBackupConfig,
) -> Result<(), BackupError> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (admin, config);
        Err(BackupError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "source lifecycle requires Linux",
        )))
    }
    #[cfg(target_os = "linux")]
    {
        lifecycle::abandon(admin, config).await
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
        let mut admission = SourceAdmission::try_acquire(admin, expected_database).await?;
        let database = admission.database().to_owned();
        if database != expected_database {
            return Err(BackupError::Invalid("recovery database identity"));
        }
        let root = binding::admit_control_root(&mut admission, control_root).await?;
        let journal = SourceGateJournal::recover_in(&root, backup_id)?;
        if journal.record().phase() != GatePhase::ReleaseReady {
            return Err(BackupError::Invalid(
                "recovery requires release-ready phase",
            ));
        }
        let registry = crate::ManagementRegistry::open_installed()?;
        let lease = registry.try_lock()?;
        let enrolled = lease.admit_control_source(&root, expected_database)?;
        let _protection = crate::CaptureProtection::reopen(&lease, &enrolled, backup_id)?;
        lease.recheck()?;
        let profile = c0_d1::SourceProfile::from_broker_oid(journal.full_c_broker_oid());
        compensate_release_profile(&mut admission, &database, profile).await?;
        let dir = root.create_dir(&format!("{backup_id}.release-recovery"))?;
        let bytes = serde_json::to_vec(&serde_json::json!({
            "format_version": 1,
            "backup_id": backup_id,
            "database": database,
            "result": "runtime_connect_revoked_and_sessions_drained"
        }))?;
        write_bytes(&dir, "closed.json", &bytes)?;
        root.sync()?;
        admission.close().await?;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
async fn prepare_linux(
    admin: &PgPool,
    assets: &FsAssetStore,
    config: &SourceBackupConfig,
) -> Result<SourceLocalPin, BackupError> {
    prepare_linux_with_effects(admin, assets, config, None).await
}

#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
#[allow(
    dead_code,
    reason = "S5 original B1 prepare awaits its pending original M3 consumer; no runtime wiring or settlement credit"
)]
pub(super) async fn prepare_b1_with_effects(
    assets: &FsAssetStore,
    input: &mut snapshot_effects::CaptureInput,
) -> Result<SourceLocalPin, BackupError> {
    input.prepare_b1_with_original_close(assets).await
}

#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
async fn prepare_b1_original(
    assets: &FsAssetStore,
    input: &mut snapshot_effects::CaptureInput,
) -> Result<SourceLocalPin, BackupError> {
    let peer = input.b1_peer()?;
    let mut admission = SourceAdmission::try_acquire_b1(std::sync::Arc::clone(&peer)).await?;
    let registry = crate::ManagementRegistry::open_installed_b1(&admission)?;
    let lease = registry.try_lock()?;
    let enrolled = lease.lookup_source_group_b1(&mut admission, assets).await?;
    let configuration = admission.b1_configuration(&enrolled)?;
    let config = &configuration;
    input.require_b1_admission(&admission, &lease, &enrolled, config)?;
    let effects = Some(input);
    if config.backup_id.is_nil()
        || config.drain_timeout.is_zero()
        || config.drain_timeout > Duration::from_secs(60)
        || config.control_root == config.local_pin_root
    {
        return Err(BackupError::Invalid("source backup configuration"));
    }
    let database = admission.database().to_owned();
    if database != config.expected_database {
        return Err(BackupError::Invalid("source database identity mismatch"));
    }
    let control = binding::admit_control_root(&mut admission, &config.control_root).await?;
    // Binding and unfinished evidence are authoritative before an attestation
    // is needed. No ACL, attempt journal, or dump mutation has occurred.
    ensure_no_unfinished_journal(&control, &config.control_root)?;
    let profile = prepare_source_preflight(&mut admission, effects.as_deref()).await?;
    admission.require_b1_enrollment(&lease, &enrolled)?;
    let pin_root = enrolled.handle("local_pins").try_clone()?;
    if control.identity()? == pin_root.identity()? {
        return Err(BackupError::Invalid("source control and pin root alias"));
    }
    let attestation_parent = config
        .isolation_attestation
        .parent()
        .ok_or(BackupError::Invalid("isolation attestation path"))?;
    BackupDir::open_trusted_private_root(attestation_parent)?;
    #[cfg(test)]
    endpoint_tests::observe_admitted_source(&mut admission, config).await?;
    let isolation = admission.b1_isolation(config, true)?;
    // Fixed options/provenance were created and checked by the actual same
    // DB OwnedAdmin owner. No caller pool/options/DSN or post-close admin is
    // accepted; endpoint rechecks use this exact Capture connection.
    admission.revalidate()?;
    prepare_admitted_source(
        AdmittedSourcePipeline {
            admission,
            control,
            enrolled,
            lease,
            isolation,
            profile,
            database,
            pin_root,
        },
        assets,
        config,
        effects,
    )
    .await
}

#[cfg(target_os = "linux")]
pub(super) async fn prepare_linux_with_effects(
    admin: &PgPool,
    assets: &FsAssetStore,
    config: &SourceBackupConfig,
    effects: Option<&mut snapshot_effects::CaptureInput>,
) -> Result<SourceLocalPin, BackupError> {
    if config.backup_id.is_nil()
        || config.drain_timeout.is_zero()
        || config.drain_timeout > Duration::from_secs(60)
        || config.control_root == config.local_pin_root
    {
        return Err(BackupError::Invalid("source backup configuration"));
    }
    let mut admission = SourceAdmission::try_acquire(admin, &config.expected_database).await?;
    let database = admission.database().to_owned();
    if database != config.expected_database {
        return Err(BackupError::Invalid("source database identity mismatch"));
    }
    let control = binding::admit_control_root(&mut admission, &config.control_root).await?;
    // Binding and unfinished evidence are authoritative before an attestation
    // is needed. No ACL, attempt journal, or dump mutation has occurred.
    ensure_no_unfinished_journal(&control, &config.control_root)?;
    let profile = prepare_source_preflight(&mut admission, effects.as_deref()).await?;
    let registry = crate::ManagementRegistry::open_installed()?;
    let lease = registry.try_lock()?;
    let enrolled = lease.admit_source(&control, assets, config)?;
    let pin_root = enrolled.handle("local_pins").try_clone()?;
    if control.identity()? == pin_root.identity()? {
        return Err(BackupError::Invalid("source control and pin root alias"));
    }
    let attestation_parent = config
        .isolation_attestation
        .parent()
        .ok_or(BackupError::Invalid("isolation attestation path"))?;
    BackupDir::open_trusted_private_root(attestation_parent)?;
    #[cfg(test)]
    endpoint_tests::observe_admitted_source(&mut admission, config).await?;
    let isolation = verify_isolation_attestation_metered(config, true, Some(&lease.budget))?;
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
    prepare_admitted_source(
        AdmittedSourcePipeline {
            admission,
            control,
            enrolled,
            lease,
            isolation,
            profile,
            database,
            pin_root,
        },
        assets,
        config,
        effects,
    )
    .await
}

#[cfg(target_os = "linux")]
async fn prepare_source_preflight(
    admission: &mut SourceAdmission,
    effects: Option<&snapshot_effects::CaptureInput>,
) -> Result<c0_d1::SourceProfile, BackupError> {
    // This inspection is read-only on the already admitted session. Reject
    // unsafe prepared work/roles before requiring any orchestration evidence;
    // isolation remains mandatory before every journal, ACL, and dump mutation.
    let profile = c0_d1::SourceProfile::admit(admission.connection()).await?;
    if effects.is_some() && profile == c0_d1::SourceProfile::BaseV1 {
        return Err(BackupError::Invalid(
            "snapshot effect capture requires the full-C source profile",
        ));
    }
    if profile == c0_d1::SourceProfile::BaseV1 {
        require_base_snapshot_claim_schema(admission.connection()).await?;
        require_base_snapshot_observation_schema(admission.connection()).await?;
    }
    let mut before = inspect_gate(admission.connection()).await?;
    if profile.broker_oid().is_some() {
        before.other_login_writers = before
            .other_login_writers
            .checked_sub(1)
            .ok_or(BackupError::Invalid("full-C broker writer count"))?;
    }
    if !before.admin_is_database_owner
        || !before.runtime_can_connect
        || before.public_can_connect
        || before.runtime_can_inherit_admin
        || before.runtime_is_privileged
        || before.other_login_writers != 0
    {
        return Err(BackupError::Invalid("source role preflight"));
    }
    Ok(profile)
}

#[cfg(target_os = "linux")]
struct AdmittedSourcePipeline {
    admission: SourceAdmission,
    control: BackupDir,
    enrolled: crate::registry::EnrolledSource,
    lease: crate::registry::RegistryLease,
    isolation: endpoint::VerifiedSourceIsolation,
    profile: c0_d1::SourceProfile,
    database: String,
    pin_root: BackupDir,
}

#[cfg(target_os = "linux")]
async fn prepare_admitted_source(
    original: AdmittedSourcePipeline,
    assets: &FsAssetStore,
    config: &SourceBackupConfig,
    mut effects: Option<&mut snapshot_effects::CaptureInput>,
) -> Result<SourceLocalPin, BackupError> {
    let AdmittedSourcePipeline {
        admission,
        control,
        enrolled,
        lease,
        isolation,
        profile,
        database,
        pin_root,
    } = original;
    let mut endpoint =
        endpoint::admit_source_endpoint(admission, &enrolled, &lease, isolation, config).await?;
    // Repeat the original role/prepared preflight after orchestration proof;
    // the earlier read-only rejection does not authorize a later mutation.
    endpoint.b1_pipeline_fence().await?;
    let repeated = c0_d1::SourceProfile::admit(endpoint.connection()).await?;
    endpoint.b1_pipeline_fence().await?;
    if repeated != profile {
        return Err(BackupError::Invalid(
            "source profile changed after admission",
        ));
    }
    if profile == c0_d1::SourceProfile::BaseV1 {
        require_base_snapshot_claim_schema(endpoint.connection()).await?;
        require_base_snapshot_observation_schema(endpoint.connection()).await?;
    }
    endpoint.b1_pipeline_fence().await?;
    let mut before = inspect_gate(endpoint.connection()).await?;
    endpoint.b1_pipeline_fence().await?;
    if profile.broker_oid().is_some() {
        before.other_login_writers = before
            .other_login_writers
            .checked_sub(1)
            .ok_or(BackupError::Invalid("full-C broker writer count"))?;
    }
    if !before.admin_is_database_owner
        || !before.runtime_can_connect
        || before.public_can_connect
        || before.runtime_can_inherit_admin
        || before.runtime_is_privileged
        || before.other_login_writers != 0
    {
        return Err(BackupError::Invalid("source role preflight"));
    }
    endpoint = endpoint.recheck().await?;
    let protection = crate::begin_capture_protection(&lease, &enrolled, config.backup_id)?;
    endpoint = endpoint.attach_capture(protection)?;
    let mut journal = match profile.broker_oid() {
        None => SourceGateJournal::start_in(&control, config.backup_id)?,
        Some(oid) => SourceGateJournal::start_full_c_in(&control, config.backup_id, oid)?,
    };
    let source = control.create_dir(&format!("{}.source", config.backup_id))?;
    let revoke = profile.revoke_statement(&database);
    endpoint = endpoint.recheck().await?;
    endpoint.b1_pipeline_fence().await?;
    sqlx::query(&revoke).execute(endpoint.connection()).await?;
    endpoint.b1_pipeline_fence().await?;
    journal.advance(GatePhase::Closed, None)?;
    wait_for_runtime_connect_probe_for_endpoint(config, &mut endpoint).await?;
    let deadline = Instant::now() + config.drain_timeout;
    loop {
        endpoint.b1_pipeline_fence().await?;
        let facts = inspect_gate(endpoint.connection()).await?;
        endpoint.b1_pipeline_fence().await?;
        if !facts.runtime_can_connect && facts.other_sessions == 0 {
            profile.recheck(endpoint.connection(), false).await?;
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

    if let Some(input) = effects.as_deref_mut() {
        input
            .capture_closed(endpoint.connection(), config, &enrolled, &lease)
            .await?;
    }

    endpoint.b1_pipeline_fence().await?;
    let plan = crate::catalog::plan_assets_on(endpoint.connection()).await?;
    endpoint.b1_pipeline_fence().await?;
    endpoint.protection()?.bind_catalog(&plan)?;
    let identity = collect_source_identity(endpoint.connection()).await?;
    endpoint.b1_pipeline_fence().await?;
    let role_bytes = match profile {
        c0_d1::SourceProfile::BaseV1 => role_recipe(endpoint.connection()).await?,
        c0_d1::SourceProfile::C0D1 { .. } => c0_d1::role_recipe(endpoint.connection()).await?,
    };
    #[cfg(test)]
    Box::pin(
        crate::full_restore::attention_fixture::observe_closed_source(
            endpoint.connection(),
            &role_bytes,
            config,
        ),
    )
    .await?;
    endpoint.b1_pipeline_fence().await?;
    endpoint.prepay_source_io(role_bytes.len() as u64, 5)?;
    let mut roles_write = source.create_file("roles.json")?;
    roles_write.write_all(&role_bytes)?;
    roles_write.sync_all()?;
    drop(roles_write);
    let mut roles = source.open_file("roles.json")?;
    let role_record = record_file_for_endpoint("roles.json", &mut roles, &endpoint)?;
    endpoint.prepay_source_io(0, 2)?;
    let mut dump_write = source.create_file("database.dump")?;
    let (held, dump_observation) = endpoint.dump_to(&mut dump_write).await?;
    endpoint = held;
    drop(dump_write);
    source.sync()?;
    let mut dump = source.open_file("database.dump")?;
    let mut magic = [0_u8; 5];
    endpoint.prepay_source_io(5, 1)?;
    dump.read_exact(&mut magic)?;
    if &magic != b"PGDMP" {
        return Err(BackupError::Invalid("pg_dump custom format magic"));
    }
    let dump_record = record_file_for_endpoint("database.dump", &mut dump, &endpoint)?;
    if dump_record.size != dump_observation.bytes
        || dump_record.sha256 != dump_observation.sha256
        || dump_observation.source_database != database
        || dump_observation.source_database_oid != u64::from(enrolled.group.database_oid)
        || dump_observation.source_system_identifier != enrolled.group.system_identifier
        || dump_observation.source_backend <= 0
        || !crate::valid_digest(&dump_observation.source_container_id)
        || !crate::valid_digest(&dump_observation.client_sha256)
        || dump_observation.source_epoch.get_version_num() != 4
        || dump_observation.exit_code != 0
    {
        return Err(BackupError::Invalid("source dump observation differs"));
    }
    endpoint.prepay_source_io(plan.asset_index_bytes().len() as u64, 4)?;
    write_bytes(&source, "asset-index.json", plan.asset_index_bytes())?;
    let manifest =
        BackupManifestV1::from_plan(config.backup_id, identity, &plan, dump_record, role_record)?;
    let manifest_sha256 = manifest.canonical_sha256()?;
    write_bytes(&source, "manifest.json", &manifest.canonical_bytes()?)?;
    endpoint.b1_pipeline_fence().await?;
    inspect_gate(endpoint.connection()).await?.validate()?;
    endpoint.b1_pipeline_fence().await?;
    profile.recheck(endpoint.connection(), false).await?;
    journal.advance(GatePhase::DumpAndIndexDurable, Some(&manifest_sha256))?;
    #[cfg(test)]
    lifecycle_tests::hook("capture_dump_durable")?;

    let sealed =
        crate::sealed::seal_backup_in(&pin_root, &manifest, &plan, assets, &mut dump, &mut roles)?;
    if sealed.manifest_sha256() != manifest_sha256 {
        return Err(BackupError::Invalid("local pin manifest digest"));
    }
    let pin = SourceLocalPin { sealed, manifest };
    endpoint.protection()?.retain_pin(&pin)?;
    journal.advance(GatePhase::PinsDurable, Some(&manifest_sha256))?;
    if let Some(input) = effects {
        input.bind_pin(&pin).await?;
    }
    #[cfg(test)]
    lifecycle_tests::hook("capture_pins_durable")?;
    endpoint.b1_pipeline_fence().await?;
    inspect_gate(endpoint.connection()).await?.validate()?;
    endpoint.b1_pipeline_fence().await?;
    profile.recheck(endpoint.connection(), false).await?;
    endpoint.protection()?.recheck_before_release()?;
    endpoint = endpoint.recheck().await?;
    let (admission, witness) = endpoint.release_parts();
    release_runtime_connect_inner(admission, &database, &mut journal, || Ok(()), witness).await?;
    endpoint.close().await?;
    Ok(pin)
}

/// The independent C0 profile is not an admitted export/restore schema.
/// Reject every signature before attempt journal, ACL or dump mutation.
#[cfg(target_os = "linux")]
async fn require_base_snapshot_claim_schema(conn: &mut PgConnection) -> Result<(), BackupError> {
    let present: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public' AND p.proname='p0c4_snapshot_claim_locked_observation')",
    ).fetch_one(conn).await?;
    if present {
        return Err(BackupError::Invalid(
            "snapshot claim extension requires independent source schema support",
        ));
    }
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod snapshot_claim_c0_tests {
    use super::*;
    use sqlx::Connection;

    #[tokio::test]
    #[ignore = "explicit isolated C0 PostgreSQL owner login; no silent skip"]
    async fn snapshot_claim_any_signature_blocks_source_before_mutation() {
        let url = std::env::var("TEST_SNAPSHOT_CLAIM_ADMIN_DATABASE_URL")
            .expect("isolated C0 owner DSN required");
        let mut conn = PgConnection::connect(&url)
            .await
            .expect("C0 owner connection");
        let identity: (String, String, String) =
            sqlx::query_as("SELECT session_user::text,current_user::text,current_database()")
                .fetch_one(&mut conn)
                .await
                .unwrap();
        assert_eq!(
            (&*identity.0, &*identity.1),
            ("learning_admin", "learning_admin")
        );
        let suffix = identity
            .2
            .strip_prefix("learning_task6_claim_")
            .expect("fresh dedicated C0 database required");
        assert_eq!(Uuid::parse_str(suffix).unwrap().to_string(), suffix);
        learning_db::MIGRATOR.run(&mut conn).await.unwrap();
        let empty: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM public.job),(SELECT count(*) FROM public.job_outbox)",
        )
        .fetch_one(&mut conn)
        .await
        .unwrap();
        assert_eq!(empty, (0, 0));
        require_base_snapshot_claim_schema(&mut conn)
            .await
            .expect("fresh canonical source; run after C0 fixture cleanup");
        let mut tx = conn.begin().await.unwrap();
        sqlx::query("CREATE FUNCTION public.p0c4_snapshot_claim_locked_observation(integer) RETURNS integer LANGUAGE sql AS 'SELECT 1'")
            .execute(&mut *tx).await.unwrap();
        let facts = "SELECT (SELECT count(*) FROM public.job),(SELECT count(*) FROM public.job_outbox),has_database_privilege('learning_runtime',current_database(),'CONNECT')";
        let before: (i64, i64, bool) = sqlx::query_as(facts).fetch_one(&mut *tx).await.unwrap();
        assert!(matches!(
            require_base_snapshot_claim_schema(&mut tx).await,
            Err(BackupError::Invalid(
                "snapshot claim extension requires independent source schema support"
            ))
        ));
        let after: (i64, i64, bool) = sqlx::query_as(facts).fetch_one(&mut *tx).await.unwrap();
        assert_eq!(
            before, after,
            "source refusal cannot mutate rows or runtime CONNECT"
        );
        tx.rollback().await.unwrap();
        require_base_snapshot_claim_schema(&mut conn).await.unwrap();
    }
}

/// A1 adds an optional function/ACL surface, not a supported source schema.
/// Until A2 exists, reject any overload before journal/ACL/dump mutation.
#[cfg(target_os = "linux")]
async fn require_base_snapshot_observation_schema(
    conn: &mut PgConnection,
) -> Result<(), BackupError> {
    let present: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public' AND p.proname='p0c4_snapshot_locked_observation')",
    ).fetch_one(conn).await?;
    if present {
        return Err(BackupError::Invalid(
            "snapshot observation extension requires source schema support",
        ));
    }
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod snapshot_observation_a1_tests {
    use super::*;
    use sqlx::Connection;

    #[tokio::test]
    #[ignore = "explicit isolated A1 PostgreSQL owner login; no silent skip"]
    async fn snapshot_observation_any_signature_blocks_source_before_mutation() {
        let url = std::env::var("TEST_SNAPSHOT_OBSERVATION_ADMIN_DATABASE_URL")
            .expect("isolated A1 owner DSN required");
        let mut conn = PgConnection::connect(&url)
            .await
            .expect("A1 owner connection");
        let identity: (String, String, String) =
            sqlx::query_as("SELECT session_user::text,current_user::text,current_database()")
                .fetch_one(&mut conn)
                .await
                .unwrap();
        assert_eq!(
            (&*identity.0, &*identity.1),
            ("learning_admin", "learning_admin")
        );
        assert!(identity.2.starts_with("learning_task6_observation_"));
        require_base_snapshot_observation_schema(&mut conn)
            .await
            .expect("fresh canonical source; run after A1 tests cleanup");
        let mut tx = conn.begin().await.unwrap();
        sqlx::query("CREATE FUNCTION public.p0c4_snapshot_locked_observation(integer) RETURNS integer LANGUAGE sql AS 'SELECT 1'").execute(&mut *tx).await.unwrap();
        let facts = "SELECT (SELECT count(*) FROM public.job),(SELECT count(*) FROM public.job_outbox),has_database_privilege('learning_runtime',current_database(),'CONNECT')";
        let before: (i64, i64, bool) = sqlx::query_as(facts).fetch_one(&mut *tx).await.unwrap();
        assert!(matches!(
            require_base_snapshot_observation_schema(&mut tx).await,
            Err(BackupError::Invalid(
                "snapshot observation extension requires source schema support"
            ))
        ));
        let after: (i64, i64, bool) = sqlx::query_as(facts).fetch_one(&mut *tx).await.unwrap();
        assert_eq!(
            before, after,
            "source refusal cannot mutate business rows or runtime CONNECT"
        );
        tx.rollback().await.unwrap();
    }
}

#[cfg(all(test, target_os = "linux"))]
async fn release_runtime_connect(
    admission: &mut SourceAdmission,
    database: &str,
    journal: &mut SourceGateJournal,
) -> Result<(), BackupError> {
    release_runtime_connect_checked(admission, database, journal, || Ok(())).await
}
#[cfg(all(test, target_os = "linux"))]
async fn release_runtime_connect_checked(
    admission: &mut SourceAdmission,
    database: &str,
    journal: &mut SourceGateJournal,
    before_grant: impl FnOnce() -> Result<(), BackupError>,
) -> Result<(), BackupError> {
    release_runtime_connect_inner(admission, database, journal, before_grant, TestReleaseCheck)
        .await
}

#[cfg(target_os = "linux")]
trait SourceReleaseCheck: Sync {
    fn recheck(
        &self,
        admission: &mut SourceAdmission,
    ) -> impl std::future::Future<Output = Result<(), BackupError>> + Send;
}
#[cfg(target_os = "linux")]
impl SourceReleaseCheck for endpoint::ReleaseWitness<'_> {
    async fn recheck(&self, admission: &mut SourceAdmission) -> Result<(), BackupError> {
        self.recheck_release(admission).await
    }
}
#[cfg(all(test, target_os = "linux"))]
struct TestReleaseCheck;
#[cfg(all(test, target_os = "linux"))]
impl SourceReleaseCheck for TestReleaseCheck {
    async fn recheck(&self, _admission: &mut SourceAdmission) -> Result<(), BackupError> {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
async fn release_runtime_connect_inner(
    admission: &mut SourceAdmission,
    database: &str,
    journal: &mut SourceGateJournal,
    before_grant: impl FnOnce() -> Result<(), BackupError>,
    witness: impl SourceReleaseCheck,
) -> Result<(), BackupError> {
    let profile = c0_d1::SourceProfile::from_broker_oid(journal.full_c_broker_oid());
    profile.recheck(admission.connection(), false).await?;
    journal.advance(GatePhase::ReleaseReady, None)?;
    #[cfg(test)]
    lifecycle_tests::hook("capture_release_ready")?;
    #[cfg(test)]
    lifecycle_tests::checkpoint("capture_before_grant").await;
    before_grant()?;
    witness.recheck(admission).await?;
    let grant = profile.grant_statement(database);
    let granted = sqlx::query(&grant).execute(admission.connection()).await;
    if granted.is_err() {
        compensate_release_profile(admission, database, profile).await?;
        return Err(BackupError::Invalid("runtime CONNECT grant failed"));
    }
    #[cfg(test)]
    lifecycle_tests::checkpoint("capture_after_grant").await;
    if let Err(error) = witness.recheck(admission).await {
        compensate_release_profile(admission, database, profile).await?;
        return Err(error);
    }
    if let Err(error) = profile.recheck(admission.connection(), true).await {
        compensate_release_profile(admission, database, profile).await?;
        return Err(error);
    }
    match inspect_gate(admission.connection()).await {
        Ok(after) if after.runtime_can_connect && !after.public_can_connect => {}
        _ => {
            compensate_release_profile(admission, database, profile).await?;
            return Err(BackupError::Invalid(
                "runtime CONNECT release could not be verified",
            ));
        }
    }
    if let Err(error) = journal.advance(GatePhase::Released, None) {
        compensate_release_profile(admission, database, profile).await?;
        return Err(error);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
async fn compensate_release_profile(
    admission: &mut SourceAdmission,
    database: &str,
    profile: c0_d1::SourceProfile,
) -> Result<(), BackupError> {
    let revoke = profile.revoke_statement(database);
    if sqlx::query(&revoke)
        .execute(admission.connection())
        .await
        .is_err()
    {
        return Err(BackupError::Invalid(
            "source gate outcome ambiguous; manual recovery required",
        ));
    }
    profile.recheck(admission.connection(), false).await?;
    match inspect_gate(admission.connection()).await {
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
    driver_pid: u32,
    inspection_file: String,
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
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeConnectProbe {
    backup_id: Uuid,
    database: String,
    result: String,
}

#[cfg(target_os = "linux")]
async fn wait_for_runtime_connect_probe_for_endpoint(
    config: &SourceBackupConfig,
    endpoint: &mut endpoint::BoundSourceEndpoint,
) -> Result<(), BackupError> {
    #[cfg(feature = "offhost-b1")]
    if endpoint.is_b1() {
        return wait_for_runtime_connect_probe_b1(config, endpoint).await;
    }
    wait_for_runtime_connect_probe(config, endpoint.budget()).await
}

#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
async fn wait_for_runtime_connect_probe_b1(
    config: &SourceBackupConfig,
    endpoint: &mut endpoint::BoundSourceEndpoint,
) -> Result<(), BackupError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    endpoint.recheck_original().await?;
    let budget = std::sync::Arc::clone(endpoint.budget());
    budget
        .lock()
        .map_err(|_| BackupError::Invalid("B1 scan budget"))?
        .entries(32)?;
    let parent = config
        .isolation_attestation
        .parent()
        .ok_or(BackupError::Invalid("driver proof path"))?;
    let root = BackupDir::open_private_root(parent)?;
    let deadline = Instant::now() + config.drain_timeout;
    let name = format!("runtime-connect-denied-{}.json", config.backup_id);
    loop {
        endpoint.recheck_original().await?;
        budget
            .lock()
            .map_err(|_| BackupError::Invalid("B1 scan budget"))?
            .entries(3)?;
        match root.open_file(&name) {
            Ok(mut file) => {
                let meta = file.metadata()?;
                if meta.uid() != 0
                    || meta.permissions().mode() & 0o777 != 0o600
                    || meta.len() > 1024
                {
                    return Err(BackupError::Invalid("runtime probe evidence permissions"));
                }
                let bytes = crate::registry::read_metadata(&mut file, meta.len(), &budget)?;
                let probe: RuntimeConnectProbe = serde_json::from_slice(&bytes)?;
                if probe.backup_id != config.backup_id
                    || probe.database != config.expected_database
                    || probe.result != "runtime_connect_denied"
                {
                    return Err(BackupError::Invalid("runtime probe evidence mismatch"));
                }
                endpoint.recheck_b1_isolation(config, false)?;
                endpoint.recheck_original().await?;
                return Ok(());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if Instant::now() >= deadline {
            return Err(BackupError::Invalid(
                "runtime connect denial probe timed out",
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(target_os = "linux")]
async fn wait_for_runtime_connect_probe(
    config: &SourceBackupConfig,
    budget: &std::sync::Arc<std::sync::Mutex<crate::registry::ScanBudget>>,
) -> Result<(), BackupError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let parent = config
        .isolation_attestation
        .parent()
        .ok_or(BackupError::Invalid("driver proof path"))?;
    let root = BackupDir::open_private_root(parent)?;
    let deadline = Instant::now() + config.drain_timeout;
    let name = format!("runtime-connect-denied-{}.json", config.backup_id);
    loop {
        match root.open_file(&name) {
            Ok(mut file) => {
                let meta = file.metadata()?;
                if meta.uid() != 0
                    || meta.permissions().mode() & 0o777 != 0o600
                    || meta.len() > 1024
                {
                    return Err(BackupError::Invalid("runtime probe evidence permissions"));
                }
                let bytes = crate::registry::read_metadata(&mut file, meta.len(), budget)?;
                let probe: RuntimeConnectProbe = serde_json::from_slice(&bytes)?;
                if probe.backup_id != config.backup_id
                    || probe.database != config.expected_database
                    || probe.result != "runtime_connect_denied"
                {
                    return Err(BackupError::Invalid("runtime probe evidence mismatch"));
                }
                verify_isolation_attestation_metered(config, false, Some(budget))?;
                return Ok(());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if Instant::now() >= deadline {
            return Err(BackupError::Invalid(
                "runtime connect denial probe timed out",
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(all(test, target_os = "linux"))]
fn verify_isolation_attestation(
    config: &SourceBackupConfig,
    require_fresh: bool,
) -> Result<endpoint::VerifiedSourceIsolation, BackupError> {
    verify_isolation_attestation_metered(config, require_fresh, None)
}

#[cfg(target_os = "linux")]
fn verify_isolation_attestation_metered(
    config: &SourceBackupConfig,
    require_fresh: bool,
    budget: Option<&std::sync::Arc<std::sync::Mutex<crate::registry::ScanBudget>>>,
) -> Result<endpoint::VerifiedSourceIsolation, BackupError> {
    use std::os::unix::{
        fs::{MetadataExt, PermissionsExt},
        io::AsRawFd,
    };
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
        || !(config
            .expected_compose_project
            .starts_with("learning-system-p0c4-")
            || config
                .expected_compose_project
                .strip_prefix("kwc4c-")
                .is_some_and(|s| crate::valid_hex(s, 32)))
    {
        return Err(BackupError::Invalid("isolation attestation identity"));
    }
    let root = BackupDir::open_private_root(parent)?;
    let parent_meta = std::fs::symlink_metadata(parent)?;
    if unsafe { libc::geteuid() } != 0 || parent_meta.uid() != 0 {
        return Err(BackupError::Invalid("root-owned isolation driver required"));
    }
    let mut file = root.open_file(leaf)?;
    let metadata = file.metadata()?;
    if metadata.uid() != 0
        || metadata.permissions().mode() & 0o777 != 0o600
        || metadata.len() > 4096
    {
        return Err(BackupError::Invalid("private isolation attestation"));
    }
    let bytes = if let Some(budget) = budget {
        crate::registry::read_metadata(&mut file, metadata.len(), budget)?
    } else {
        let mut raw = Vec::new();
        file.read_to_end(&mut raw)?;
        raw
    };
    let proof: IsolationAttestation = serde_json::from_slice(&bytes)?;
    let expected_inspection = format!("inspection-{}.json", config.backup_id);
    if proof.inspection_file != expected_inspection || proof.driver_pid == 0 {
        return Err(BackupError::Invalid("isolation driver evidence identity"));
    }
    let mut inspection_file = root.open_file(&expected_inspection)?;
    let inspection_meta = inspection_file.metadata()?;
    if inspection_meta.uid() != 0
        || inspection_meta.permissions().mode() & 0o777 != 0o600
        || inspection_meta.len() > 1024 * 1024
    {
        return Err(BackupError::Invalid("private Docker inspection evidence"));
    }
    let inspection_bytes = if let Some(budget) = budget {
        crate::registry::read_metadata(&mut inspection_file, inspection_meta.len(), budget)?
    } else {
        let mut raw = Vec::new();
        inspection_file.read_to_end(&mut raw)?;
        raw
    };
    if !inspection_digest_matches(&proof.docker_inspection_sha256, &inspection_bytes) {
        return Err(BackupError::Invalid("Docker inspection digest mismatch"));
    }
    let inspection: serde_json::Value = serde_json::from_slice(&inspection_bytes)?;
    if inspection["network"]["project"] != config.expected_compose_project
        || inspection["network"]["internal"] != true
        || inspection["runtime_running"] != proof.runtime_running
        || inspection["worker_running"] != proof.worker_running
        || inspection["other_admin_processes"] != proof.other_admin_processes
        || inspection["postgres_published_ports"] != proof.postgres_published_ports
        || inspection["network_internal"] != proof.network_internal
        || inspection["manager_runtime_secret_mounts"] != proof.manager_runtime_secret_mounts
        || inspection["manager_runtime_env_keys"] != proof.manager_runtime_env_keys
        || inspection["containers"].as_array().is_none_or(|items| {
            items.is_empty()
                || !items
                    .iter()
                    .any(|v| v["service"] == "pg" && v["running"] == true)
                || items
                    .iter()
                    .any(|v| v["service"] != "pg" && v["running"] == true)
        })
    {
        return Err(BackupError::Invalid(
            "Docker inspection does not prove isolation",
        ));
    }
    let lock = root.open_file(&format!("isolation-{}.lock", config.backup_id))?;
    let lock_meta = lock.metadata()?;
    if lock_meta.uid() != 0 || lock_meta.permissions().mode() & 0o777 != 0o600 {
        return Err(BackupError::Invalid("private isolation driver lock"));
    }
    let locked_elsewhere = unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if locked_elsewhere == 0 {
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_UN) };
        return Err(BackupError::Invalid(
            "isolation driver no longer holds exclusive lock",
        ));
    }
    if std::io::Error::last_os_error().raw_os_error() != Some(libc::EWOULDBLOCK) {
        return Err(BackupError::Invalid(
            "isolation driver lock could not be checked",
        ));
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| BackupError::Invalid("system clock"))?
        .as_millis();
    let age = now
        .checked_sub(u128::from(proof.observed_unix_ms))
        .ok_or(BackupError::Invalid("future isolation attestation"))?;
    if proof.format_version != 2
        || proof.backup_id != config.backup_id
        || proof.database != config.expected_database
        || proof.compose_project != config.expected_compose_project
        || (require_fresh && age > 30_000)
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
    Ok(endpoint::VerifiedSourceIsolation {
        root,
        proof_name: leaf.to_owned(),
        proof_bytes: bytes,
        inspection_name: expected_inspection,
        inspection_bytes,
        lock_name: format!("isolation-{}.lock", config.backup_id),
        lock,
    })
}

#[cfg(target_os = "linux")]
fn ensure_no_unfinished_journal(root: &BackupDir, _path: &Path) -> Result<(), BackupError> {
    lifecycle::scan(root, None)
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
fn record_file(path: &str, file: &mut (impl Read + Seek)) -> Result<FileRecord, BackupError> {
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
fn record_file_for_endpoint(
    path: &str,
    file: &mut File,
    _endpoint: &endpoint::BoundSourceEndpoint,
) -> Result<FileRecord, BackupError> {
    #[cfg(feature = "offhost-b1")]
    if _endpoint.is_b1() {
        // The original recording/hash algorithm is shared. Its reader pays
        // the requested bytes before each actual read, including EOF probes;
        // appended/replaced bytes cannot acquire an unreserved free tail.
        _endpoint.prepay_source_io(64 * 1024 + 64, 2)?;
        let mut metered = MeteredSourceFile {
            file,
            endpoint: _endpoint,
            failure: None,
        };
        let observed = record_file(path, &mut metered);
        return match metered.failure {
            Some(error) => Err(error),
            None => observed,
        };
    }
    record_file(path, file)
}

#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
struct MeteredSourceFile<'a> {
    file: &'a mut File,
    endpoint: &'a endpoint::BoundSourceEndpoint,
    failure: Option<BackupError>,
}

#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
impl MeteredSourceFile<'_> {
    fn prepay(&mut self, bytes: u64) -> std::io::Result<()> {
        if self.failure.is_some() {
            return Err(std::io::Error::other("B1 source read stopped"));
        }
        if let Err(error) = self.endpoint.prepay_source_io(bytes, 1) {
            self.failure = Some(error);
            return Err(std::io::Error::other(
                "B1 source read budget or current owner",
            ));
        }
        Ok(())
    }
}

#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
impl Read for MeteredSourceFile<'_> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        // One requested read and the hash of at most that many returned bytes.
        self.prepay((output.len() as u64).saturating_mul(2))?;
        self.file.read(output)
    }
}

#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
impl Seek for MeteredSourceFile<'_> {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.prepay(0)?;
        self.file.seek(position)
    }
}

#[cfg(target_os = "linux")]
async fn require_admin_owner(connection: &mut PgConnection) -> Result<String, BackupError> {
    let (current, session, authenticated, database, owner): (String, String, Option<String>, String, String) = sqlx::query_as(
        "SELECT current_user::text, session_user::text, system_user, current_database()::text, pg_get_userbyid(d.datdba)::text FROM pg_catalog.pg_database d WHERE d.datname=current_database()",
    ).fetch_one(&mut *connection).await?;
    if current != "learning_admin"
        || session != "learning_admin"
        || owner != "learning_admin"
        || authenticated
            .as_deref()
            .and_then(|v| v.split_once(':'))
            .is_none_or(|(method, user)| method.is_empty() || user != "learning_admin")
        || !valid_c4_database(&database)
    {
        return Err(BackupError::Invalid(
            "authenticated management database owner required",
        ));
    }
    Ok(database)
}

#[cfg(any(target_os = "linux", test))]
fn require_no_prepared_transactions(count: Result<i64, sqlx::Error>) -> Result<(), BackupError> {
    // Prepared work survives its originating session; only a successfully
    // decoded zero can prove it drained. Do not identify or resolve that work.
    if count? != 0 {
        return Err(BackupError::Invalid(
            "source database has unsafe prepared transaction count",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod prepared_count_tests {
    use super::*;

    #[test]
    fn empty_prepared_count_allows_inspection() {
        assert!(require_no_prepared_transactions(Ok(0)).is_ok());
    }

    #[test]
    fn nonzero_or_negative_prepared_count_refuses_inspection() {
        for count in [1, i64::MAX, -1, i64::MIN] {
            assert!(
                matches!(
                    require_no_prepared_transactions(Ok(count)),
                    Err(BackupError::Invalid(_))
                ),
                "unsafe prepared count {count} was accepted"
            );
        }
    }

    #[test]
    fn unavailable_or_malformed_prepared_count_refuses_inspection() {
        for error in [
            sqlx::Error::ColumnNotFound("prepared_transactions".into()),
            sqlx::Error::Decode(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "not an i64 count",
            ))),
            sqlx::Error::PoolClosed,
        ] {
            assert!(matches!(
                require_no_prepared_transactions(Err(error)),
                Err(BackupError::Database(_))
            ));
        }
    }
}

#[cfg(target_os = "linux")]
async fn inspect_gate(connection: &mut PgConnection) -> Result<GateInspection, BackupError> {
    let row = sqlx::query(
        "SELECT \
            (pg_get_userbyid(d.datdba)='learning_admin') AS admin_is_owner, \
            has_database_privilege('learning_runtime',current_database(),'CONNECT') AS runtime_connect, \
            EXISTS(SELECT 1 FROM aclexplode(COALESCE(d.datacl,acldefault('d',d.datdba))) a WHERE a.grantee=0 AND a.privilege_type='CONNECT') AS public_connect, \
            EXISTS(SELECT 1 FROM pg_catalog.pg_auth_members am JOIN pg_catalog.pg_roles r ON r.oid=am.member WHERE r.rolname='learning_runtime') AS runtime_inherits_admin, \
            (SELECT rolsuper OR rolbypassrls OR rolcreaterole OR rolcreatedb FROM pg_catalog.pg_roles WHERE rolname='learning_runtime') AS runtime_privileged, \
            (SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid()) AS other_sessions, \
            (SELECT count(*) FROM pg_catalog.pg_prepared_xacts WHERE database=current_database()) AS prepared_transactions, \
            (SELECT count(*) FROM pg_catalog.pg_roles r WHERE r.rolcanlogin AND r.rolname NOT IN ('learning_admin','learning_runtime','postgres') AND has_database_privilege(r.rolname,current_database(),'CONNECT')) AS other_login_writers \
         FROM pg_catalog.pg_database d WHERE d.datname=current_database()",
    ).fetch_one(&mut *connection).await?;
    require_no_prepared_transactions(row.try_get("prepared_transactions"))?;
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
async fn collect_source_identity(
    connection: &mut PgConnection,
) -> Result<SourceIdentity, BackupError> {
    let version: String = sqlx::query_scalar("SHOW server_version_num")
        .fetch_one(&mut *connection)
        .await?;
    let version = version
        .parse::<u32>()
        .map_err(|_| BackupError::Invalid("PostgreSQL version"))?;
    if version / 10_000 != 18 {
        return Err(BackupError::Invalid("PostgreSQL 18 required"));
    }
    let rows: Vec<(i64, String, bool)> = sqlx::query_as(
        "SELECT version,encode(checksum,'hex'),success FROM public._sqlx_migrations ORDER BY version",
    ).fetch_all(&mut *connection).await?;
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
    // This is a shared package/build-input identity, not the hash of the
    // particular management executable. The same reviewed package must be
    // embedded in the separate restore binary or Task 4 rejects it.
    let build_sha256 = compiled_build_id(option_env!("KNOWWEAVE_BUILD_ID_SHA256"))?;
    SourceIdentity::from_migrations(build_sha256, commit.into(), 18, migrations)
}

#[cfg(any(target_os = "linux", test))]
fn compiled_build_id(value: Option<&str>) -> Result<String, BackupError> {
    let value = value.ok_or(BackupError::Invalid(
        "build lacks reviewed package identity",
    ))?;
    if !crate::valid_digest(value) {
        return Err(BackupError::Invalid(
            "noncanonical reviewed package identity",
        ));
    }
    Ok(value.to_owned())
}

#[cfg(any(target_os = "linux", test))]
fn inspection_digest_matches(expected: &str, bytes: &[u8]) -> bool {
    crate::valid_digest(expected) && crate::digest(bytes) == expected
}

#[cfg(test)]
mod build_identity_tests {
    use super::*;

    #[test]
    fn source_and_restore_require_same_reviewed_package_digest() {
        assert!(compiled_build_id(None).is_err());
        assert!(compiled_build_id(Some(&"A".repeat(64))).is_err());
        let digest = "a".repeat(64);
        assert_eq!(compiled_build_id(Some(&digest)).unwrap(), digest);
    }

    #[test]
    fn fabricated_docker_digest_cannot_bind_unseen_inspection_bytes() {
        let bytes = br#"{"network":{"internal":true}}"#;
        assert!(!inspection_digest_matches(&"a".repeat(64), bytes));
        assert!(inspection_digest_matches(&crate::digest(bytes), bytes));
    }
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
async fn role_recipe(connection: &mut PgConnection) -> Result<Vec<u8>, BackupError> {
    let rows = sqlx::query(
        "SELECT rolname::text AS name,rolcanlogin,rolinherit,rolsuper,rolcreatedb,rolcreaterole,rolbypassrls,rolreplication,rolconnlimit FROM pg_catalog.pg_roles WHERE rolname IN ('learning_admin','learning_runtime','learning_auth_lock') ORDER BY rolname",
    ).fetch_all(&mut *connection).await?;
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
    ).fetch_all(&mut *connection).await?;
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
    use sqlx::{Connection, PgConnection, postgres::PgPoolOptions};
    use std::os::unix::fs::PermissionsExt;

    fn prepared_env_path(name: &str) -> PathBuf {
        PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} required")))
    }

    async fn prepared_test_pool(database_env: &str, url_env: &str) -> (PgPool, String) {
        let database = std::env::var(database_env).unwrap();
        let suffix = database.strip_prefix("learning_backup_c4_task3_").unwrap();
        assert_eq!(Uuid::parse_str(suffix).unwrap().to_string(), suffix);
        let url = std::env::var(url_env).unwrap();
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap();
        assert_eq!(
            require_admin_owner(&mut pool.acquire().await.unwrap())
                .await
                .unwrap(),
            database
        );
        let version: String = sqlx::query_scalar("SHOW server_version_num")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!((180_000..190_000).contains(&version.parse::<u32>().unwrap()));
        let enabled: String = sqlx::query_scalar("SHOW max_prepared_transactions")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(enabled.parse::<u32>().unwrap() > 0);
        (pool, url)
    }

    /// Every mutation belongs to this exact synthetic UUID. The table is
    /// created inside the prepared transaction and vanishes on its rollback.
    async fn prepare_fixture_work(
        url: &str,
        id: Uuid,
    ) -> Result<Result<(), sqlx::Error>, sqlx::Error> {
        let mut origin = PgConnection::connect(url).await?;
        sqlx::query("BEGIN").execute(&mut origin).await?;
        let table = format!("c4_prepared_{}", id.simple());
        sqlx::query(&format!(
            "CREATE TABLE public.{table}(id integer PRIMARY KEY)"
        ))
        .execute(&mut origin)
        .await?;
        sqlx::query(&format!("INSERT INTO public.{table}(id) VALUES(1)"))
            .execute(&mut origin)
            .await?;
        sqlx::query(&format!("PREPARE TRANSACTION '{id}'"))
            .execute(&mut origin)
            .await?;
        // Caller owns the ID before any mutation. Return the close outcome so
        // a close error cannot bypass cleanup of confirmed prepared work.
        Ok(origin.close().await)
    }

    async fn rollback_fixture_work(pool: &PgPool, id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query(&format!("ROLLBACK PREPARED '{id}'"))
            .execute(pool)
            .await?;
        Ok(())
    }

    async fn wait_for_no_other_sessions(pool: &PgPool) -> Result<i64, BackupError> {
        for _ in 0..100 {
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid()",
            ).fetch_one(pool).await?;
            if count == 0 {
                return Ok(count);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Err(BackupError::Invalid("fixture origin session did not close"))
    }

    /// Fresh migrated PG18 source, prepared transactions enabled, authentic
    /// root-driver isolation proof, independently preissued/compile-pinned
    /// control binding, other private empty roots, no runtime sessions.
    /// No driver CONNECT-denial receipt is needed: refusal must precede REVOKE.
    #[tokio::test]
    #[ignore = "deferred: requires updated independent issuer/build pin, fresh PG18 prepared source and root-driver proof"]
    async fn prepared_target_transaction_survives_origin_close_and_blocks_source_preflight() {
        let id = Uuid::new_v4();
        let (pool, url) = prepared_test_pool(
            "TEST_C4_PREPARED_DATABASE_NAME",
            "TEST_C4_PREPARED_ADMIN_DATABASE_URL",
        )
        .await;
        let initial = inspect_gate(&mut pool.acquire().await.unwrap())
            .await
            .unwrap();
        assert!(initial.admin_is_database_owner && initial.runtime_can_connect);
        assert!(!initial.public_can_connect);
        assert!(!initial.runtime_can_inherit_admin && !initial.runtime_is_privileged);
        assert_eq!(initial.other_login_writers, 0);
        assert_eq!(wait_for_no_other_sessions(&pool).await.unwrap(), 0);
        let control = prepared_env_path("TEST_C4_PREPARED_CONTROL_ROOT");
        let pin = prepared_env_path("TEST_C4_PREPARED_PIN_ROOT");
        let asset_root = prepared_env_path("TEST_C4_PREPARED_ASSET_ROOT");
        let stage_root = prepared_env_path("TEST_C4_PREPARED_ASSET_STAGE_ROOT");
        binding::preissued_fixture(&control);
        let control_before = binding::fixture_inventory(&control).unwrap();
        for path in [&pin, &asset_root, &stage_root] {
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert!(std::fs::read_dir(path).unwrap().next().is_none());
        }
        let assets = FsAssetStore::new(asset_root, stage_root).unwrap();
        let config = SourceBackupConfig {
            backup_id: std::env::var("TEST_C4_PREPARED_BACKUP_ID")
                .unwrap()
                .parse()
                .unwrap(),
            expected_database: std::env::var("TEST_C4_PREPARED_DATABASE_NAME").unwrap(),
            expected_compose_project: std::env::var("TEST_C4_PREPARED_COMPOSE_PROJECT").unwrap(),
            isolation_attestation: prepared_env_path("TEST_C4_PREPARED_ISOLATION_ATTESTATION"),
            control_root: control.clone(),
            local_pin_root: pin.clone(),
            pg_dump_executable: prepared_env_path("TEST_C4_PREPARED_PGDUMP_BIN"),
            pgpassfile: prepared_env_path("TEST_C4_PREPARED_PGPASSFILE"),
            pg_host: std::env::var("TEST_C4_PREPARED_PGHOST").unwrap(),
            pg_port: std::env::var("TEST_C4_PREPARED_PGPORT")
                .unwrap()
                .parse()
                .unwrap(),
            drain_timeout: Duration::from_secs(5),
        };
        // Prove prerequisite checks pass before introducing the sole unsafe
        // condition, so an unrelated early refusal cannot satisfy the test.
        verify_isolation_attestation(&config, true).unwrap();
        assert!(!config.backup_id.is_nil());
        let options = pool.connect_options();
        assert_eq!(options.get_host(), config.pg_host);
        assert_eq!(options.get_port(), config.pg_port);
        let acl_before: Option<String> = sqlx::query_scalar(
            "SELECT datacl::text FROM pg_catalog.pg_database WHERE datname=current_database()",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        let origin_closed = prepare_fixture_work(&url, id).await.unwrap_or_else(|_| {
            // Private gate failure log retains only a fixed reason and our
            // exact ID if PREPARE confirmation is lost. Do not resolve work
            // automatically when setup/confirmation has an uncertain outcome.
            panic!("prepared_fixture_setup_or_prepare_unconfirmed:{id}")
        });
        // Collect fallible observations first, then resolve only our UUID
        // before asserting. Ordinary assertion failures leave no prepared work.
        let observed: Result<_, BackupError> = async {
            let sessions = wait_for_no_other_sessions(&pool).await?;
            let inspection = inspect_gate(&mut pool.acquire().await.unwrap()).await;
            let capture = prepare_source_backup(&pool, &assets, &config).await;
            let acl_after: Option<String> = sqlx::query_scalar(
                "SELECT datacl::text FROM pg_catalog.pg_database WHERE datname=current_database()",
            ).fetch_one(&pool).await?;
            let own_prepared: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_catalog.pg_prepared_xacts WHERE database=current_database() AND gid=$1",
            ).bind(id.to_string()).fetch_one(&pool).await?;
            let control_after = binding::fixture_inventory(&control)?;
            let pin_empty = std::fs::read_dir(&pin)?.next().is_none();
            Ok((sessions, inspection, capture, acl_after, own_prepared, control_after, pin_empty))
        }.await;
        rollback_fixture_work(&pool, id).await.unwrap();
        origin_closed.unwrap();
        let (sessions, inspection, capture, acl_after, own_prepared, control_after, pin_empty) =
            observed.unwrap();
        assert_eq!(sessions, 0, "prepared work has no ordinary session");
        assert!(matches!(
            inspection,
            Err(BackupError::Invalid(
                "source database has unsafe prepared transaction count"
            ))
        ));
        assert!(matches!(
            capture,
            Err(BackupError::Invalid(
                "source database has unsafe prepared transaction count"
            ))
        ));
        assert_eq!(own_prepared, 1, "capture must not resolve prepared work");
        assert_eq!(
            acl_after, acl_before,
            "preflight must not mutate CONNECT ACL"
        );
        assert_eq!(
            control_after, control_before,
            "preissued binding inventory unchanged; no journal, source directory or dump may be created"
        );
        assert!(
            pin_empty,
            "no pin, sealed package or complete may be published"
        );
        let after_cleanup = inspect_gate(&mut pool.acquire().await.unwrap())
            .await
            .unwrap();
        assert!(after_cleanup.runtime_can_connect);
        assert_eq!(after_cleanup.other_sessions, 0);
        pool.close().await;
    }

    /// Runner provisions two fresh UUID databases on one isolated PG18 server.
    /// Only the second owns synthetic prepared work; target must remain usable
    /// even though max_prepared_transactions is enabled server-wide.
    #[tokio::test]
    #[ignore = "requires two fresh isolated PG18 databases with prepared transactions enabled"]
    async fn prepared_other_database_does_not_block_empty_target() {
        let id = Uuid::new_v4();
        let (target, _) = prepared_test_pool(
            "TEST_C4_PREPARED_DATABASE_NAME",
            "TEST_C4_PREPARED_ADMIN_DATABASE_URL",
        )
        .await;
        let (other, other_url) = prepared_test_pool(
            "TEST_C4_PREPARED_OTHER_DATABASE_NAME",
            "TEST_C4_PREPARED_OTHER_ADMIN_DATABASE_URL",
        )
        .await;
        let target_options = target.connect_options();
        let other_options = other.connect_options();
        assert_eq!(target_options.get_host(), other_options.get_host());
        assert_eq!(target_options.get_port(), other_options.get_port());
        assert_ne!(target_options.get_database(), other_options.get_database());
        inspect_gate(&mut target.acquire().await.unwrap())
            .await
            .unwrap();
        inspect_gate(&mut other.acquire().await.unwrap())
            .await
            .unwrap();
        let origin_closed = prepare_fixture_work(&other_url, id)
            .await
            .unwrap_or_else(|_| panic!("prepared_fixture_setup_or_prepare_unconfirmed:{id}"));
        let observed: Result<_, BackupError> = async {
            let other_sessions = wait_for_no_other_sessions(&other).await?;
            let target_count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_catalog.pg_prepared_xacts WHERE database=current_database()",
            ).fetch_one(&target).await?;
            let other_count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_catalog.pg_prepared_xacts WHERE database=current_database() AND gid=$1",
            ).bind(id.to_string()).fetch_one(&other).await?;
            let target_inspection = inspect_gate(&mut target.acquire().await.unwrap()).await;
            let other_inspection = inspect_gate(&mut other.acquire().await.unwrap()).await;
            Ok((other_sessions, target_count, other_count, target_inspection, other_inspection))
        }.await;
        rollback_fixture_work(&other, id).await.unwrap();
        origin_closed.unwrap();
        let (other_sessions, target_count, other_count, target_inspection, other_inspection) =
            observed.unwrap();
        assert_eq!(other_sessions, 0);
        assert_eq!(target_count, 0);
        assert_eq!(other_count, 1);
        assert_eq!(target_inspection.unwrap().other_sessions, 0);
        assert!(matches!(
            other_inspection,
            Err(BackupError::Invalid(
                "source database has unsafe prepared transaction count"
            ))
        ));
        inspect_gate(&mut other.acquire().await.unwrap())
            .await
            .unwrap();
        target.close().await;
        other.close().await;
    }

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
        assert!(verify_isolation_attestation(&config, true).is_err());
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let inspection = serde_json::to_vec(&serde_json::json!({
            "network": {"project": config.expected_compose_project, "internal": true},
            "containers": [{"service": "pg", "running": true}],
            "runtime_running": 0, "worker_running": 0, "other_admin_processes": 0,
            "postgres_published_ports": 0, "network_internal": true,
            "manager_runtime_secret_mounts": 0, "manager_runtime_env_keys": 0
        }))
        .unwrap();
        let inspection_name = format!("inspection-{id}.json");
        std::fs::write(root.join(&inspection_name), &inspection).unwrap();
        std::fs::set_permissions(
            root.join(&inspection_name),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let lock = root.join(format!("isolation-{id}.lock"));
        std::fs::write(&lock, []).unwrap();
        std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o600)).unwrap();
        let mut proof = serde_json::json!({
            "format_version": 2,
            "backup_id": id,
            "database": config.expected_database,
            "compose_project": config.expected_compose_project,
            "observed_unix_ms": now,
            "driver_pid": std::process::id(),
            "inspection_file": inspection_name,
            "runtime_running": 0,
            "worker_running": 0,
            "other_admin_processes": 0,
            "postgres_published_ports": 0,
            "network_internal": true,
            "manager_runtime_secret_mounts": 0,
            "manager_runtime_env_keys": 0,
            "docker_inspection_sha256": format!("{:x}", Sha256::digest(&inspection))
        });
        std::fs::write(&file, serde_json::to_vec(&proof).unwrap()).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(verify_isolation_attestation(&config, true).is_err());
        proof["worker_running"] = serde_json::json!(1);
        std::fs::write(&file, serde_json::to_vec(&proof).unwrap()).unwrap();
        assert!(verify_isolation_attestation(&config, true).is_err());
        proof["worker_running"] = serde_json::json!(0);
        proof["manager_runtime_secret_mounts"] = serde_json::json!(1);
        std::fs::write(&file, serde_json::to_vec(&proof).unwrap()).unwrap();
        assert!(verify_isolation_attestation(&config, true).is_err());
        proof["manager_runtime_secret_mounts"] = serde_json::json!(0);
        proof["observed_unix_ms"] = serde_json::json!(now - 31_000);
        std::fs::write(&file, serde_json::to_vec(&proof).unwrap()).unwrap();
        assert!(verify_isolation_attestation(&config, true).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Dedicated, already-gated UUID database. Deliberately collide with the
    /// final journal name after GRANT; the compensation must revoke CONNECT.
    /// Requires independently preissued source binding and matching build pin.
    #[tokio::test]
    #[ignore = "deferred: requires updated independent issuer/build pin and separate PG18 release-fault database"]
    async fn release_journal_failure_recloses_runtime_connect() {
        let database = std::env::var("TEST_C4_RELEASE_DATABASE_NAME").unwrap();
        let suffix = database.strip_prefix("learning_backup_c4_task3_").unwrap();
        assert_eq!(Uuid::parse_str(suffix).unwrap().to_string(), suffix);
        let url = std::env::var("TEST_C4_RELEASE_ADMIN_DATABASE_URL").unwrap();
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap();
        assert_eq!(
            require_admin_owner(&mut pool.acquire().await.unwrap())
                .await
                .unwrap(),
            database
        );
        let mut admission = SourceAdmission::try_acquire(&pool, &database)
            .await
            .unwrap();
        let root = PathBuf::from(std::env::var("TEST_C4_RELEASE_CONTROL_ROOT").unwrap());
        let (control, binding_before) = binding::preissued_fixture(&root);
        binding::admit_control_root(&mut admission, &root)
            .await
            .unwrap();
        let id = Uuid::new_v4();
        let mut journal = SourceGateJournal::start_in(&control, id).unwrap();
        let revoke =
            format!("REVOKE CONNECT ON DATABASE \"{database}\" FROM PUBLIC, learning_runtime");
        sqlx::query(&revoke)
            .execute(admission.connection())
            .await
            .unwrap();
        journal.advance(GatePhase::Closed, None).unwrap();
        inspect_gate(admission.connection())
            .await
            .unwrap()
            .validate()
            .unwrap();
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
            release_runtime_connect(&mut admission, &database, &mut journal)
                .await
                .is_err()
        );
        assert_eq!(journal.record().phase(), GatePhase::ReleaseReady);
        assert!(
            !inspect_gate(admission.connection())
                .await
                .unwrap()
                .runtime_can_connect
        );
        std::fs::remove_dir(collision).unwrap();
        admission.close().await.unwrap();
        force_close_release_ready(&pool, &root, id, &database)
            .await
            .unwrap();
        assert!(
            root.join(format!("{id}.release-recovery"))
                .join("closed.json")
                .is_file()
        );
        assert_eq!(
            std::fs::read(root.join("source-binding.json")).unwrap(),
            binding_before
        );
        assert_eq!(
            SourceGateJournal::recover_in(&control, id)
                .unwrap()
                .record()
                .phase(),
            GatePhase::ReleaseReady
        );
    }
}

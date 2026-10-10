//! Full importer compiled inside the private target binding module so raw
//! Docker claims, original guards and challenge sessions never escape it.
use super::super as binding;
use super::super::super as preflight;
use super::*;
use crate::full_restore::{
    SchemaContract, VerifiedFullImport, freeze_full_dump, trusted_ownership, validate_full_dump,
};
use crate::{BackupError, BackupManifestV1, BackupPlan, CompleteBackup, RestorePreflightConfig};
use binding::child_attestation::{ChildFailure, Isolation, linux_child};
use learning_assets::backup_fs::BackupDir;
use sha2::{Digest, Sha256};
use sqlx::{Connection, PgConnection, PgPool};
use std::{
    io::Read,
    time::{Duration, Instant},
};

#[path = "catalog_context.rs"]
mod catalog_context;
#[cfg(test)]
#[path = "catalog_observation.rs"]
mod catalog_observation;
#[cfg(test)]
#[path = "recovery_tests.rs"]
pub(crate) mod recovery_tests;

/// No constructor, serde, activation or usable conversion exists. Task 6 must
/// consume this held target rather than acquire an unbound replacement pool.
pub struct RecoveryPending {
    target: ImportedTarget,
    complete: CompleteBackup,
    package: VerifiedPackage,
    assets: AssetRestoreReport,
}
impl RecoveryPending {
    pub(crate) fn target_mut(&mut self) -> &mut ImportedTarget {
        &mut self.target
    }
    pub(crate) fn manifest(&self) -> &BackupManifestV1 {
        &self.package.manifest
    }
    pub(crate) fn plan(&self) -> &BackupPlan {
        &self.package.plan
    }
    pub fn backup_id(&self) -> uuid::Uuid {
        self.complete.backup_id()
    }
    pub fn assets(&self) -> &AssetRestoreReport {
        &self.assets
    }
}
#[derive(Debug)]
pub struct AssetRestoreReport {
    logical_assets: u64,
    distinct_originals: u64,
    original_bytes: u64,
}
impl AssetRestoreReport {
    pub fn logical_assets(&self) -> u64 {
        self.logical_assets
    }
    pub fn distinct_originals(&self) -> u64 {
        self.distinct_originals
    }
    pub fn original_bytes(&self) -> u64 {
        self.original_bytes
    }
}

struct VerifiedPackage {
    directory: BackupDir,
    manifest: BackupManifestV1,
    plan: BackupPlan,
    roles: Vec<u8>,
    authority: PackageAuthority,
}
enum PackageAuthority {
    Complete {
        _destination: crate::EnrolledDestination,
        _proof: crate::CapturedSourceProof,
    },
    #[cfg(test)]
    Fresh(crate::SourceLocalPin),
}
impl VerifiedPackage {
    fn open_complete(
        complete: &CompleteBackup,
        config: &RestorePreflightConfig,
    ) -> Result<Self, BackupError> {
        Self::open_at(complete, &config.destination_root, &config.trust_path)
    }
    fn open_at(
        complete: &CompleteBackup,
        destination_root: &std::path::Path,
        trust_path: &std::path::Path,
    ) -> Result<Self, BackupError> {
        let checked =
            crate::open_complete_backup(destination_root, trust_path, complete.backup_id())?;
        if checked.receipt_sha256() != complete.receipt_sha256()
            || checked.manifest_sha256() != complete.manifest_sha256()
        {
            return Err(BackupError::Invalid("complete package changed"));
        }
        let registry = crate::ManagementRegistry::open_installed()?;
        let lease = registry.try_lock()?;
        let destination = lease.admit_destination(destination_root)?;
        let proof = crate::open_captured_source_proof(&destination, complete)?;
        let directory = destination
            .destination()
            .open_dir(&format!("{}.sealed", complete.backup_id()))?;
        let manifest_raw = preflight::read_limited(&directory, "manifest.json", 64 * 1024 * 1024)?;
        let manifest: BackupManifestV1 = serde_json::from_slice(&manifest_raw)?;
        if manifest.canonical_bytes()? != manifest_raw
            || manifest.canonical_sha256()? != complete.manifest_sha256()
            || manifest.backup_id != complete.backup_id()
        {
            return Err(BackupError::Invalid("held full package manifest"));
        }
        let index = preflight::read_limited(
            &directory,
            "asset-index.json",
            crate::MAX_ASSET_INDEX_BYTES as u64,
        )?;
        manifest.validate_with_index(&index)?;
        let parsed: preflight::AssetIndex = serde_json::from_slice(&index)?;
        let plan = BackupPlan::from_rows(parsed.assets)?;
        if plan.asset_index_bytes() != index {
            return Err(BackupError::Invalid("full package index"));
        }
        let roles = preflight::read_limited(&directory, "roles.json", 16384)?;
        crate::validate_role_recipe(&roles)?;
        // Verify from these held directories too; no path re-open can swap the
        // bytes checked by the complete-receipt reader for different inputs.
        for record in &manifest.files {
            let mut file = crate::full_restore::assets::open_relative(&directory, &record.path)?;
            crate::full_restore::assets::copy_original(&mut file, &mut std::io::sink(), record)?;
        }
        Ok(Self {
            directory,
            manifest,
            plan,
            roles,
            authority: PackageAuthority::Complete {
                _destination: destination,
                _proof: proof,
            },
        })
    }
}

/// Owned representation of the exact existing preflight, not a second admission.
struct OwnedRestoreAdmission {
    preflight: preflight::RestorePreflight,
    config: RestorePreflightConfig,
    control: BackupDir,
    assets: BackupDir,
    expected: WriterExpected,
    birth_sha256: [u8; 32],
    inspection_sha256: [u8; 32],
}
impl OwnedRestoreAdmission {
    async fn from_preflight(
        mut admitted: preflight::RestorePreflight,
        config: RestorePreflightConfig,
        roles: &[u8],
    ) -> Result<Self, BackupError> {
        let result=async {
        let control = BackupDir::open_trusted_private_root(&config.control_root)?;
        let assets = BackupDir::open_trusted_private_root(&config.asset_root)?;
        for name in [
            preflight::restore_attempt_name(&config.expected_database)?,
            format!("{}.restore.commit-attempt", config.expected_database),
        ] {
            preflight::reject_existing_attempt(control.kind(&name))?;
        }
        let (pid,oid):(i32,i64)=sqlx::query_as("SELECT pg_catalog.pg_backend_pid(),d.oid::bigint FROM pg_catalog.pg_database d WHERE d.datname=pg_catalog.current_database()")
            .fetch_one(&mut **admitted.sql_session.lease_mut()).await?;
        let expected = WriterExpected::new(
            config.expected_database.clone(),
            u64::try_from(oid).map_err(|_| BackupError::Overflow)?,
            &admitted.bound_target.claim.system_identifier,
            pid,
            admitted.sql_session.keys(),
            protocol::Nonce::random().map_err(error)?,
        )
        .map_err(error)?;
        let roles_result=verify_roles(admitted.sql_session.lease_mut(), roles).await;
        #[cfg(test)]
        if matches!(&roles_result, Err(BackupError::Invalid("full restore role recipe differs"))) {
            if let Some(audit)=current_audit().filter(|a| a.stage==Boundary::Roles) {
                audit.refusal(Boundary::Roles);
                if audit.after(admitted.sql_session.lease_mut(), &config, &control, &assets).await.is_err() { audit.failed(); }
            }
        }
        roles_result?;
        let birth = preflight::read_private_target_file(
            &control,
            &format!("{}.birth.json", config.expected_database),
            4096,
        )?;
        let inspection_sha256 =
            Sha256::digest(serde_json::to_vec(&admitted.bound_target.observation)?).into();
        Ok::<_,BackupError>((control,assets,expected,Sha256::digest(birth).into(),inspection_sha256))
        }.await;
        match result {
            Ok((control, assets, expected, birth_sha256, inspection_sha256)) => Ok(Self {
                preflight: admitted,
                config,
                control,
                assets,
                expected,
                birth_sha256,
                inspection_sha256,
            }),
            Err(error) => {
                admitted.bound_target.child_usable.set(false);
                if linux_child::candidate_quarantine(&admitted.bound_target.claim).await
                    != Isolation::Stopped
                {
                    return Err(BackupError::Invalid(
                        "rejected full admission isolation unconfirmed",
                    ));
                }
                Err(error)
            }
        }
    }
    async fn recheck(&mut self, deadline: Instant) -> Result<(), ImportFailure> {
        for (path, held) in [
            (&self.config.control_root, &self.control),
            (&self.config.asset_root, &self.assets),
        ] {
            let current =
                BackupDir::open_trusted_private_root(path).map_err(|_| ImportFailure::Identity)?;
            if current.identity().map_err(|_| ImportFailure::Identity)?
                != held.identity().map_err(|_| ImportFailure::Identity)?
            {
                return Err(ImportFailure::Identity);
            }
        }
        let (_, oid, _, pid, _, _) = self.expected.parts();
        linux_child::candidate_recheck(
            &mut self.preflight.bound_target,
            &mut self.preflight.sql_session,
            pid,
            oid,
            deadline,
        )
        .await
        .map_err(child_error)
    }
}
impl AdmissionQuarantine for OwnedRestoreAdmission {
    async fn quarantine(&mut self) {
        self.preflight.bound_target.child_usable.set(false);
        let _ = linux_child::candidate_quarantine(&self.preflight.bound_target.claim).await;
    }
}

pub struct ImportedTarget {
    admission: Option<OwnedRestoreAdmission>,
    runtime: tokio::runtime::Handle,
    quarantined: bool,
}
impl ImportedTarget {
    fn admitted(&mut self) -> &mut OwnedRestoreAdmission {
        self.admission.as_mut().expect("owned target")
    }
    async fn stop(&mut self) -> Result<(), BackupError> {
        if self.quarantined {
            return Ok(());
        }
        let admission = self.admitted();
        admission.preflight.bound_target.child_usable.set(false);
        let stopped = linux_child::candidate_quarantine(&admission.preflight.bound_target.claim)
            .await
            == Isolation::Stopped;
        self.quarantined = stopped;
        if stopped {
            Ok(())
        } else {
            Err(BackupError::Invalid("full target isolation unconfirmed"))
        }
    }
    pub(crate) fn connection(&mut self) -> &mut PgConnection {
        self.admitted().preflight.sql_session.lease_mut()
    }
    pub(crate) async fn recheck(&mut self) -> Result<(), BackupError> {
        self.admitted()
            .recheck(Instant::now() + Duration::from_secs(15))
            .await
            .map_err(error)
    }
}
impl Drop for ImportedTarget {
    fn drop(&mut self) {
        if self.quarantined {
            return;
        }
        if let Some(mut admission) = self.admission.take() {
            self.runtime.spawn(async move {
                admission.quarantine().await;
            });
        }
    }
}

struct Cancellation(watch::Sender<bool>);
impl Drop for Cancellation {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}
/// Normal recovery always consumes an actual independently verified complete
/// backup. Returned state remains isolated and needs Task 6 validation.
pub async fn restore_complete_backup(
    complete: CompleteBackup,
    config: RestorePreflightConfig,
    admin: PgPool,
) -> Result<RecoveryPending, BackupError> {
    config.validate()?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let (cancel, rx) = watch::channel(false);
    let _cancel = Cancellation(cancel);
    tokio::spawn(async move {
        let result = restore_owned(complete, config, admin, rx).await;
        // If cancellation destroyed the receiver, dropping a successful result
        // hands its original target to exactly one quarantine owner.
        let _ = sender.send(result);
    });
    receiver
        .await
        .map_err(|_| BackupError::Invalid("restore owner lost; target unusable"))?
}
async fn restore_owned(
    complete: CompleteBackup,
    config: RestorePreflightConfig,
    admin: PgPool,
    cancel: watch::Receiver<bool>,
) -> Result<RecoveryPending, BackupError> {
    let package = VerifiedPackage::open_complete(&complete, &config)?;
    let preflight = crate::preflight_restore(&complete, &config, &admin).await?;
    let admission =
        OwnedRestoreAdmission::from_preflight(preflight, config, &package.roles).await?;
    let mut imported = ImportedTarget {
        admission: Some(admission),
        runtime: tokio::runtime::Handle::current(),
        quarantined: false,
    };
    let result = async {
        gate(&cancel, Instant::now() + Duration::from_secs(900)).map_err(error)?;
        let spool = imported
            .admitted()
            .control
            .create_dir(&format!("full-spool-{}", uuid::Uuid::new_v4()))?;
        let mut dump = package.directory.open_file("database.dump")?;
        let record = package
            .manifest
            .files
            .iter()
            .find(|r| r.path == "database.dump")
            .ok_or(BackupError::Invalid("missing full dump"))?;
        let frozen = freeze_full_dump(&mut dump, record, &spool)?;
        let input = validate_full_dump(frozen, &SchemaContract::embedded()?, &spool).await?;
        import_verified(
            &mut imported,
            input,
            &package,
            FullOrigin::Complete(&complete),
            cancel.clone(),
        )
        .await?;
        gate(&cancel, Instant::now() + Duration::from_secs(900)).map_err(error)?;
        let assets = restore_originals(&mut imported, &package).await?;
        imported.recheck().await?;
        gate(&cancel, Instant::now() + Duration::from_secs(1)).map_err(error)?;
        Ok::<_, BackupError>(assets)
    }
    .await;
    let assets = match result {
        Ok(assets) => assets,
        Err(error) => {
            imported.stop().await?;
            return Err(error);
        }
    };
    Ok(RecoveryPending {
        target: imported,
        complete,
        package,
        assets,
    })
}

async fn import_verified(
    target: &mut ImportedTarget,
    input: VerifiedFullImport,
    package: &VerifiedPackage,
    origin: FullOrigin<'_>,
    cancel: watch::Receiver<bool>,
) -> Result<(), BackupError> {
    let deadline = Instant::now() + Duration::from_secs(900);
    let roles = package.roles.clone();
    #[cfg(test)]
    full_stage(Stage::Payload);
    let payload = tokio::task::spawn_blocking(move || FullPayload::new(input, &roles, deadline))
        .await
        .map_err(|_| BackupError::Invalid("full payload preparation lost"))??;
    let mut io = FullIo {
        target,
        payload: Some(payload),
        stream: None,
        deadline,
        manifest: &package.manifest,
        origin,
        roles: &package.roles,
    };
    #[cfg(test)]
    full_stage(Stage::Supervisor);
    let report = supervise_held(&mut io, cancel, Arc::new(AtomicBool::new(false))).await;
    #[cfg(test)]
    full_stage(Stage::SupervisorReport);
    #[cfg(test)]
    let _ = FULL_TEST.try_with(|control| {
        *control.observation.lock().unwrap() = Some(CandidateImportReport {
            phase: report.phase,
            failure: report.failure,
            stop_confirmed: report.stop_confirmed,
            content_verified: report.content_verified,
            commit_attempted: report.commit_attempted,
        })
    });
    if let Some(failure) = report.failure {
        return Err(error(failure));
    }
    Ok(())
}
struct FullPayload {
    input: VerifiedFullImport,
    header: Vec<u8>,
    offset: u64,
    ownership: String,
    catalog: serde_json::Value,
    transformed_sha256: String,
    deadline: Instant,
}
impl FullPayload {
    fn new(
        input: VerifiedFullImport,
        roles: &[u8],
        deadline: Instant,
    ) -> Result<Self, BackupError> {
        let mut prefix = vec![0; 4096];
        let n = input.checked_reader_from(0)?.read(&mut prefix)?;
        prefix.truncate(n);
        let marker = b"--\n-- Name: ";
        let offset = prefix
            .windows(marker.len())
            .position(|p| p == marker)
            .ok_or(BackupError::Invalid("full header boundary"))?;
        let ownership = trusted_ownership()?;
        let mut hash = Sha256::new();
        hash.update(&prefix[..offset]);
        hash.update(FULL_SETTINGS.as_bytes());
        let mut reader = input.checked_reader_from(offset as u64)?;
        let mut buffer = [0u8; 65536];
        loop {
            if Instant::now() >= deadline {
                return Err(BackupError::Invalid("full import preparation deadline"));
            }
            let n = reader.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
        }
        drop(reader);
        hash.update(ownership.as_bytes());
        Ok(Self {
            header: prefix[..offset].to_vec(),
            offset: offset as u64,
            input,
            ownership,
            catalog: expected_catalog(roles)?,
            transformed_sha256: hex::encode(hash.finalize()),
            deadline,
        })
    }
}
const FULL_SETTINGS: &str = "SET statement_timeout='900s'; SET lock_timeout='5s'; SET idle_in_transaction_session_timeout='30s'; SET transaction_timeout='900s';\n";
impl CheckedPayload for FullPayload {
    fn prelude(&self, expected: &WriterExpected) -> Result<Vec<u8>, ImportFailure> {
        let mut bytes = b"BEGIN;\n".to_vec();
        bytes.extend_from_slice(&self.header);
        bytes.extend_from_slice(FULL_SETTINGS.as_bytes());
        bytes.extend_from_slice(
            writer_sql::assertion(&writer_sql::identity_predicate(expected), "IDENTITY").as_bytes(),
        );
        bytes.extend_from_slice(writer_sql::receipt(expected, "READY").as_bytes());
        Ok(bytes)
    }
    fn postcheck(&self, expected: &WriterExpected, writer: &WriterIdentity) -> Vec<u8> {
        let (pid, start, xid) = writer.parts();
        let predicate = format!(
            "{} AND pg_catalog.pg_backend_pid()={pid} AND (SELECT (EXTRACT(EPOCH FROM a.backend_start)*1000000)::bigint FROM pg_catalog.pg_stat_activity a WHERE a.pid=pg_catalog.pg_backend_pid())={start} AND pg_catalog.pg_current_xact_id()::text='{xid}'",
            writer_sql::identity_predicate(expected)
        );
        let catalog_json = self.catalog.to_string().replace('\'', "''");
        let catalog_predicate = format!(
            "({})::jsonb = '{}'::jsonb",
            CATALOG_SQL.trim_end_matches(';'),
            catalog_json
        );
        #[cfg(test)]
        let observation_nonce = is_case(FullFault::Success).then(|| expected.parts().5.hex());
        #[cfg(test)]
        let observation_owned = catalog_observation::sql(
            observation_nonce.as_deref(),
            CATALOG_SQL,
            &self.catalog.to_string(),
        );
        #[cfg(test)]
        let observation_sql = observation_owned.as_str();
        #[cfg(not(test))]
        let observation_sql = "";
        let catalog_check = catalog_context::guard(
            &writer_sql::assertion(&catalog_predicate, "CATALOG"),
            observation_sql,
        );
        format!(
            "{}{}{}{}",
            self.ownership,
            writer_sql::assertion(&predicate, "IDENTITY"),
            catalog_check,
            writer_sql::receipt(expected, "PRECOMMIT")
        )
        .into_bytes()
    }
    fn reader(&self) -> Box<dyn Read + Send + '_> {
        Box::new(
            self.input
                .checked_reader_from(self.offset)
                .expect("validated full header offset"),
        )
    }
    fn duration(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }
}
enum FullOrigin<'a> {
    Complete(&'a CompleteBackup),
    #[cfg(test)]
    Fresh(&'a crate::SourceLocalPin),
}
struct FullIo<'a> {
    target: &'a mut ImportedTarget,
    payload: Option<FullPayload>,
    stream: Option<stream::StreamOwner>,
    deadline: Instant,
    manifest: &'a BackupManifestV1,
    origin: FullOrigin<'a>,
    roles: &'a [u8],
}
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum FullFault {
    Success,
    PrecommitEof,
    Cancel,
    CommitUnknown,
    WrongEndpoint,
    BadInput,
    BadRole,
    DirtyTarget,
    AssetCorrupt,
}
#[cfg(test)]
use crate::full_restore::negative::{Boundary, NegativeEvidence};
#[cfg(test)]
use crate::full_restore::rehearsal_diagnostic::{Diagnostic, Stage};
#[cfg(test)]
#[derive(Clone)]
pub(super) struct AdmissionAudit {
    stage: Boundary,
    evidence: Arc<std::sync::Mutex<NegativeEvidence>>,
}
#[cfg(test)]
impl AdmissionAudit {
    fn for_case(case: FullFault) -> Option<Self> {
        let stage = match case {
            FullFault::BadInput => Boundary::DumpFreeze,
            FullFault::DirtyTarget => Boundary::CleanTarget,
            FullFault::BadRole => Boundary::Roles,
            FullFault::WrongEndpoint => Boundary::WriterRoute,
            FullFault::AssetCorrupt => Boundary::OriginalSet,
            _ => return None,
        };
        Some(Self {
            stage,
            evidence: Arc::new(std::sync::Mutex::new(NegativeEvidence::default())),
        })
    }
    pub(super) fn failed(&self) {
        self.evidence.lock().unwrap().failed();
    }
    fn injection(&self) {
        self.evidence.lock().unwrap().injected(self.stage);
    }
    fn refusal(&self, stage: Boundary) {
        self.evidence.lock().unwrap().refused(stage);
    }
    fn validate(&self, error: bool) -> Result<(), BackupError> {
        self.evidence.lock().unwrap().validate(self.stage, error)
    }
    pub(super) async fn before_admission(
        &self,
        conn: &mut PgConnection,
        config: &RestorePreflightConfig,
        control: &BackupDir,
        assets: &BackupDir,
    ) -> Result<(), BackupError> {
        if self.stage == Boundary::OriginalSet {
            return Ok(());
        }
        if self.stage == Boundary::Roles {
            let limit: i32 = sqlx::query_scalar(
                "SELECT rolconnlimit FROM pg_catalog.pg_roles WHERE rolname='learning_runtime'",
            )
            .fetch_one(&mut *conn)
            .await?;
            if limit != 0 {
                return Err(BackupError::Invalid("bad role fixture absent"));
            }
            self.injection();
        }
        if self.stage == Boundary::CleanTarget {
            let exists: bool = sqlx::query_scalar(
                "SELECT pg_catalog.to_regclass('public.full_restore_dirty_probe') IS NOT NULL",
            )
            .fetch_one(&mut *conn)
            .await?;
            if !exists {
                return Err(BackupError::Invalid("dirty target fixture absent"));
            }
            self.injection();
        }
        let before = negative_inventory(conn, config, control, assets).await?;
        self.evidence.lock().unwrap().before(before);
        Ok(())
    }
    pub(super) async fn dirty_refusal(
        &self,
        facts: &crate::RestoreTargetFacts,
        conn: &mut PgConnection,
        config: &RestorePreflightConfig,
        control: &BackupDir,
        assets: &BackupDir,
    ) -> Result<(), BackupError> {
        if self.stage != Boundary::CleanTarget {
            return Ok(());
        }
        // Fixed CREATE TABLE produces one relation plus its two row types.
        // All other availability predicates must still be clean.
        if facts.non_system_relations != 3 {
            return Err(BackupError::Invalid("unexpected dirty catalog baseline"));
        }
        let mut available = *facts;
        available.non_system_relations = 0;
        available.validate()?;
        self.refusal(Boundary::CleanTarget);
        self.after(conn, config, control, assets).await
    }
    async fn after(
        &self,
        conn: &mut PgConnection,
        config: &RestorePreflightConfig,
        control: &BackupDir,
        assets: &BackupDir,
    ) -> Result<(), BackupError> {
        let after = negative_inventory(conn, config, control, assets).await?;
        self.evidence.lock().unwrap().after(after);
        Ok(())
    }
    async fn before_target(&self, target: &mut ImportedTarget) -> Result<(), BackupError> {
        let admitted = target.admitted();
        let value = negative_inventory(
            admitted.preflight.sql_session.lease_mut(),
            &admitted.config,
            &admitted.control,
            &admitted.assets,
        )
        .await?;
        self.evidence.lock().unwrap().before(value);
        Ok(())
    }
    async fn after_target(&self, target: &mut ImportedTarget) -> Result<(), BackupError> {
        let admitted = target.admitted();
        self.after(
            admitted.preflight.sql_session.lease_mut(),
            &admitted.config,
            &admitted.control,
            &admitted.assets,
        )
        .await
    }
}
#[cfg(test)]
fn current_audit() -> Option<AdmissionAudit> {
    FULL_TEST.try_with(|c| c.audit.clone()).ok().flatten()
}
#[cfg(test)]
async fn negative_inventory(
    conn: &mut PgConnection,
    config: &RestorePreflightConfig,
    control: &BackupDir,
    assets: &BackupDir,
) -> Result<serde_json::Value, BackupError> {
    // Closed catalog inventory: all structural catalog families, not only
    // application table counts. Names are compile-time constants, never SQL input.
    const CATALOGS: &[&str] = &[
        "pg_class",
        "pg_attribute",
        "pg_attrdef",
        "pg_namespace",
        "pg_proc",
        "pg_type",
        "pg_enum",
        "pg_range",
        "pg_constraint",
        "pg_index",
        "pg_trigger",
        "pg_rewrite",
        "pg_sequence",
        "pg_extension",
        "pg_event_trigger",
        "pg_publication",
        "pg_publication_rel",
        "pg_publication_namespace",
        "pg_largeobject_metadata",
        "pg_collation",
        "pg_conversion",
        "pg_operator",
        "pg_opclass",
        "pg_opfamily",
        "pg_am",
        "pg_amop",
        "pg_amproc",
        "pg_ts_config",
        "pg_ts_config_map",
        "pg_ts_dict",
        "pg_ts_parser",
        "pg_ts_template",
        "pg_default_acl",
        "pg_foreign_data_wrapper",
        "pg_foreign_server",
        "pg_foreign_table",
        "pg_language",
        "pg_transform",
        "pg_parameter_acl",
        "pg_db_role_setting",
        "pg_tablespace",
        "pg_database",
        "pg_roles",
        "pg_auth_members",
        "pg_policy",
        "pg_depend",
        "pg_shdepend",
        "pg_cast",
        "pg_inherits",
        "pg_partitioned_table",
        "pg_description",
        "pg_shdescription",
        "pg_seclabel",
        "pg_shseclabel",
        "pg_init_privs",
        "pg_statistic_ext",
        "pg_subscription_rel",
        "pg_user_mappings",
    ];
    let mut parts=CATALOGS.iter().map(|name|format!("'{name}',(SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM pg_catalog.{name} t)")).collect::<Vec<_>>();
    parts.push("'pg_subscription',(SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM (SELECT oid,subdbid,subname,subowner,subenabled,subpublications FROM pg_catalog.pg_subscription) t)".into());
    // Fixed chunks keep every jsonb_build_object below its 100-argument cap.
    let objects = parts
        .chunks(40)
        .map(|chunk| format!("jsonb_build_object({})", chunk.join(",")))
        .collect::<Vec<_>>();
    let sql = format!("SELECT {}", objects.join(" || "));
    let catalog: serde_json::Value = tokio::time::timeout(
        Duration::from_secs(10),
        sqlx::query_scalar(&sql).fetch_one(conn),
    )
    .await
    .map_err(|_| BackupError::Invalid("negative catalog timeout"))??;
    if serde_json::to_vec(&catalog)?.len() > 32 * 1024 * 1024 {
        return Err(BackupError::Capacity("negative catalog"));
    }
    let asset_inventory = negative_files(assets)?;
    let mut attempts = serde_json::Map::new();
    for name in [
        preflight::restore_attempt_name(&config.expected_database)?,
        format!("{}.restore.commit-attempt", config.expected_database),
    ] {
        let value = match control.kind(&name) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::Value::Null,
            Err(e) => return Err(e.into()),
            Ok(_) => negative_file(&control.open_file(&name)?)?,
        };
        attempts.insert(name, value);
    }
    Ok(serde_json::json!({"catalog":catalog,"assets":asset_inventory,"attempts":attempts}))
}
#[cfg(test)]
fn negative_file(file: &std::fs::File) -> Result<serde_json::Value, BackupError> {
    use std::os::unix::fs::MetadataExt;
    let meta = file.metadata()?;
    if meta.len() > 64 * 1024 * 1024 {
        return Err(BackupError::Capacity("negative file"));
    }
    let mut input = file;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut count = 0u64;
    loop {
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        count += n as u64;
        if count > meta.len() {
            return Err(BackupError::Invalid("negative file changed"));
        }
        hash.update(&buffer[..n]);
    }
    if count != meta.len() {
        return Err(BackupError::Invalid("negative file length"));
    }
    Ok(
        serde_json::json!({"kind":"file","dev":meta.dev(),"ino":meta.ino(),"mode":meta.mode(),"uid":meta.uid(),"gid":meta.gid(),"size":count,"sha256":hex::encode(hash.finalize())}),
    )
}
#[cfg(test)]
fn negative_files(root: &BackupDir) -> Result<serde_json::Value, BackupError> {
    use learning_assets::backup_fs::BackupEntryKind;
    let mut rows = serde_json::Map::new();
    let mut total_bytes = 0u64;
    let mut pending = vec![(String::new(), root.try_clone()?)];
    while let Some((prefix, dir)) = pending.pop() {
        let (dev, ino) = dir.identity()?;
        rows.insert(
            prefix.clone(),
            serde_json::json!({"kind":"directory","dev":dev,"ino":ino}),
        );
        for name in dir.list_bounded(100010)? {
            if rows.len() + pending.len() > 100010 {
                return Err(BackupError::Capacity("negative inventory entries"));
            }
            let path = format!("{prefix}/{name}");
            match dir.kind(&name)? {
                BackupEntryKind::Directory => pending.push((path, dir.open_dir(&name)?)),
                BackupEntryKind::File => {
                    let file = dir.open_file(&name)?;
                    total_bytes = total_bytes
                        .checked_add(file.metadata()?.len())
                        .ok_or(BackupError::Overflow)?;
                    if total_bytes > 64 * 1024 * 1024 {
                        return Err(BackupError::Capacity("negative inventory total bytes"));
                    }
                    rows.insert(path, negative_file(&file)?);
                }
                _ => return Err(BackupError::Invalid("negative inventory entry")),
            }
        }
    }
    Ok(serde_json::Value::Object(rows))
}
#[cfg(test)]
struct FullTestControl {
    case: FullFault,
    cancel: watch::Sender<bool>,
    wrong_container: std::sync::Mutex<Option<String>>,
    observation: std::sync::Mutex<Option<CandidateImportReport>>,
    audit: Option<AdmissionAudit>,
    diagnostic: Diagnostic,
    writer_observation: Option<(
        supervisor_observation::Observation,
        Result<BackupDir, BackupError>,
    )>,
}
#[cfg(test)]
tokio::task_local! { static FULL_TEST:FullTestControl; }
#[cfg(test)]
fn full_stage(stage: Stage) {
    let _ = FULL_TEST.try_with(|control| control.diagnostic.enter(stage));
}
#[cfg(test)]
fn is_case(case: FullFault) -> bool {
    FULL_TEST
        .try_with(|control| control.case == case)
        .unwrap_or(false)
}
impl CandidateIo for FullIo<'_> {
    type Payload = FullPayload;
    #[cfg(test)]
    fn observation(&self) -> Option<supervisor_observation::Observation> {
        FULL_TEST
            .try_with(|control| {
                control
                    .writer_observation
                    .as_ref()
                    .map(|value| value.0.clone())
            })
            .ok()
            .flatten()
    }
    #[cfg(test)]
    fn observe_failure(&mut self, boundary: SupervisorBoundary, failure: ImportFailure) {
        let _ = FULL_TEST.try_with(|control| boundary.record(&control.diagnostic, failure));
    }
    fn retain_success(&self) -> bool {
        true
    }
    fn expected(&self) -> &WriterExpected {
        &self.target.admission.as_ref().unwrap().expected
    }
    async fn prepare(&mut self) -> Result<FullPayload, ImportFailure> {
        self.payload.take().ok_or(ImportFailure::Fixture)
    }
    async fn start(&mut self, header: Vec<u8>) -> Result<(), ImportFailure> {
        let admission = self.target.admitted();
        let container = admission.preflight.bound_target.claim.container_id.clone();
        #[cfg(test)]
        let container = if is_case(FullFault::WrongEndpoint) {
            FULL_TEST
                .with(|control| control.wrong_container.lock().unwrap().clone())
                .ok_or(ImportFailure::Identity)?
        } else {
            container
        };
        let fixed =
            commands::FixedImportCommand::writer(&container, &admission.config.expected_database)?;
        binding::linux::trusted_docker_path().map_err(|_| ImportFailure::Identity)?;
        let mut command = tokio::process::Command::new("/usr/bin/docker");
        command
            .args(&fixed.argv()[1..])
            .env_clear()
            .env("DOCKER_HOST", "unix:///var/run/docker.sock");
        let budget = stream::StreamBudget::full_writer(self.deadline);
        #[cfg(test)]
        let budget = if is_case(FullFault::WrongEndpoint) {
            stream::StreamBudget::full_wrong_route(
                self.deadline,
                &admission.config.expected_database,
            )?
        } else {
            budget
        };
        #[cfg(test)]
        let budget = if let Some(observation) = self.observation() {
            budget.observed(observation)
        } else {
            budget
        };
        self.stream = Some(stream::spawn_stream(command, budget)?);
        #[cfg(test)]
        if is_case(FullFault::WrongEndpoint) {
            current_audit().ok_or(ImportFailure::Fixture)?.injection();
        }
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
        self.target.admitted().recheck(self.deadline).await
    }
    async fn attempt(
        &mut self,
        sql: &FullPayload,
        writer: WriterIdentity,
    ) -> Result<DurableAttempt, ImportFailure> {
        let admission = self.target.admitted();
        let dump = self
            .manifest
            .files
            .iter()
            .find(|r| r.path == "database.dump")
            .ok_or(ImportFailure::Fixture)?;
        let context = candidate_attempt::CandidateAttemptContext::full(
            admission.config.expected_database.clone(),
            admission.birth_sha256,
            admission.inspection_sha256,
            self.manifest.backup_id,
            self.manifest
                .canonical_sha256()
                .map_err(|_| ImportFailure::Fixture)?,
            match &self.origin {
                FullOrigin::Complete(complete) => Some(complete.receipt_sha256().to_owned()),
                #[cfg(test)]
                FullOrigin::Fresh(pin) => {
                    if pin.manifest().backup_id != self.manifest.backup_id {
                        return Err(ImportFailure::Identity);
                    }
                    None
                }
            },
            dump.sha256.clone(),
            sql.input.decoded_sha256().to_owned(),
            sql.transformed_sha256.clone(),
            writer,
        )?;
        let control = admission
            .control
            .try_clone()
            .map_err(|_| ImportFailure::Journal)?;
        tokio::task::spawn_blocking(move || candidate_attempt::persist_attempt(&control, &context))
            .await
            .map_err(|_| ImportFailure::Journal)?
    }
    async fn send(&mut self, bytes: &[u8]) -> Result<(), ImportFailure> {
        #[cfg(test)]
        if is_case(FullFault::CommitUnknown) && bytes.starts_with(b"SELECT 'KW_C4|") {
            let stream = self.stream.as_mut().ok_or(ImportFailure::Io)?;
            stream.close_input().await?;
            stream.finish().await?;
            return Err(ImportFailure::Protocol);
        }
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
        #[cfg(test)]
        if is_case(FullFault::PrecommitEof) {
            let stream = self.stream.as_mut().ok_or(ImportFailure::Io)?;
            stream.close_input().await?;
            stream.finish().await?;
            return Err(ImportFailure::Protocol);
        }
        #[cfg(test)]
        if is_case(FullFault::Cancel) {
            FULL_TEST.with(|control| control.cancel.send_replace(true));
            return Err(ImportFailure::Cancelled);
        }
        let control = self
            .target
            .admitted()
            .control
            .try_clone()
            .map_err(|_| ImportFailure::Journal)?;
        tokio::task::spawn_blocking(move || {
            candidate_attempt::persist_commit_intent(&control, &attempt, &writer)
        })
        .await
        .map_err(|_| ImportFailure::Journal)?
    }
    async fn finish(&mut self) -> Result<(), ImportFailure> {
        let stream = self.stream.as_mut().ok_or(ImportFailure::Io)?;
        stream.close_input().await?;
        stream.finish().await
    }
    async fn cleanup(&mut self) -> Result<(), ImportFailure> {
        if let Some(mut stream) = self.stream.take() {
            #[cfg(test)]
            if is_case(FullFault::WrongEndpoint) {
                let outcome = stream.finish().await;
                if outcome == Err(ImportFailure::Exit) && stream.observed_missing_database() {
                    current_audit()
                        .ok_or(ImportFailure::Fixture)?
                        .refusal(Boundary::WriterRoute);
                }
            }
            stream.kill_and_wait().await?;
        }
        Ok(())
    }
    async fn readback(&mut self, writer: WriterIdentity, committed: bool) -> bool {
        let result=async {
            self.recheck().await?;
            let (pid,start,_)=writer.parts();
            let exit_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            loop {
                let live: bool = tokio::time::timeout_at(exit_deadline, sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_stat_activity WHERE pid=$1 AND (EXTRACT(EPOCH FROM backend_start)*1000000)::bigint=$2)").bind(pid).bind(start).fetch_one(self.target.connection()))
                    .await.map_err(|_| ImportFailure::Deadline)?.map_err(|_| ImportFailure::Session)?;
                if !live { break; }
                if tokio::time::Instant::now() >= exit_deadline { return Err(ImportFailure::Session); }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            if !committed {preflight::target_facts(self.target.connection(),0).await.map_err(|_|ImportFailure::Fixture)?.validate().map_err(|_|ImportFailure::Fixture)?;}
            else { verify_roles(self.target.connection(),self.roles).await.map_err(|_|ImportFailure::Identity)?; verify_catalog(self.target.connection(),self.roles).await.map_err(|_|ImportFailure::Fixture)?; }
            self.recheck().await?;Ok::<_,ImportFailure>(())
        }.await;
        result.is_ok()
    }
    async fn quarantine(&mut self) -> bool {
        #[cfg(test)]
        if let Some(audit) = current_audit().filter(|a| a.stage == Boundary::WriterRoute) {
            if audit.after_target(self.target).await.is_err() {
                audit.failed();
            }
        }
        self.target.stop().await.is_ok()
    }
}

async fn restore_originals(
    imported: &mut ImportedTarget,
    package: &VerifiedPackage,
) -> Result<AssetRestoreReport, BackupError> {
    imported.recheck().await?;
    let rows:Vec<(uuid::Uuid,uuid::Uuid,String,i64,String)>=sqlx::query_as("SELECT space_id,id,sha256,byte_size,storage_key FROM public.asset WHERE status='ready' ORDER BY space_id,id LIMIT 100001").fetch_all(imported.connection()).await?;
    let actual = rows
        .into_iter()
        .map(
            |(space_id, id, sha256, byte_size, storage_key)| crate::AssetRow {
                space_id,
                id,
                sha256,
                byte_size,
                storage_key,
            },
        )
        .collect();
    crate::validate_restored_assets(&package.plan, actual)?;
    let directory = package.directory.try_clone()?;
    let target = imported.admitted().assets.try_clone()?;
    let plan = package.plan.clone();
    tokio::task::spawn_blocking(move || {
        crate::full_restore::assets::restore_all(&directory, &target, &plan)
    })
    .await
    .map_err(|_| BackupError::Invalid("original restore owner lost"))??;
    #[cfg(test)]
    if is_case(FullFault::AssetCorrupt) {
        let first = package
            .plan
            .asset_files()
            .first()
            .ok_or(BackupError::Invalid("asset fault fixture empty"))?;
        let shard = imported
            .admitted()
            .assets
            .open_dir("sha256")?
            .open_dir(&first.sha256[..2])?;
        let extra = shard.create_file("unexpected-original")?;
        extra.sync_all()?;
        shard.sync()?;
        imported.admitted().assets.sync()?;
        let audit = current_audit().ok_or(BackupError::Invalid("asset audit absent"))?;
        audit.injection();
        audit.before_target(imported).await?;
        let refusal =
            crate::full_restore::assets::verify_all(&imported.admitted().assets, &package.plan);
        if matches!(&refusal, Err(BackupError::Invalid("extra original bytes"))) {
            audit.refusal(Boundary::OriginalSet);
            if audit.after_target(imported).await.is_err() {
                audit.failed();
            }
        }
        refusal?;
    }
    imported.recheck().await?;
    Ok(AssetRestoreReport {
        logical_assets: package.plan.logical_asset_count(),
        distinct_originals: package.plan.asset_files().len() as u64,
        original_bytes: package.plan.unique_asset_bytes(),
    })
}
fn error(failure: ImportFailure) -> BackupError {
    #[cfg(test)]
    let _ = FULL_TEST.try_with(|control| {
        control
            .diagnostic
            .import_failure(import_failure_code(failure));
    });
    match failure {
        ImportFailure::CommitUnknown => {
            BackupError::Invalid("restore commit unknown; unusable; no replay")
        }
        ImportFailure::Cancelled => BackupError::Invalid("restore cancelled; unusable; no replay"),
        _ => BackupError::Invalid("guarded full restore rejected; unusable"),
    }
}
#[cfg(test)]
fn import_failure_code(failure: ImportFailure) -> &'static str {
    failure.diagnostic_code()
}
fn child_error(error: ChildFailure) -> ImportFailure {
    match error {
        ChildFailure::Session => ImportFailure::Session,
        ChildFailure::Deadline => ImportFailure::Deadline,
        _ => ImportFailure::Identity,
    }
}

async fn verify_roles(conn: &mut PgConnection, raw: &[u8]) -> Result<(), BackupError> {
    crate::validate_role_recipe(raw)?;
    let expected: serde_json::Value = serde_json::from_slice(raw)?;
    let actual: serde_json::Value = sqlx::query_scalar(ROLE_SQL).fetch_one(&mut *conn).await?;
    if actual != expected {
        return Err(BackupError::Invalid("full restore role recipe differs"));
    }
    let closed:bool=sqlx::query_scalar("SELECT NOT pg_catalog.has_database_privilege('learning_runtime',pg_catalog.current_database(),'CONNECT') AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_database d CROSS JOIN LATERAL pg_catalog.aclexplode(d.datacl) a WHERE d.datname=pg_catalog.current_database() AND a.grantee=0 AND a.privilege_type='CONNECT')").fetch_one(conn).await?;
    if !closed {
        return Err(BackupError::Invalid("full restore runtime is available"));
    }
    Ok(())
}
const ROLE_SQL: &str = "SELECT jsonb_build_object('format_version',1,'roles',(SELECT jsonb_agg(jsonb_build_object('name',rolname::text,'login',rolcanlogin,'inherit',rolinherit,'superuser',rolsuper,'createdb',rolcreatedb,'createrole',rolcreaterole,'bypassrls',rolbypassrls,'replication',rolreplication,'connection_limit',rolconnlimit) ORDER BY rolname) FROM pg_catalog.pg_roles WHERE rolname IN ('learning_admin','learning_auth_lock','learning_runtime')),'memberships',(SELECT jsonb_agg(jsonb_build_object('role',r.rolname::text,'member',m.rolname::text,'inherit',a.inherit_option,'set',a.set_option,'admin',a.admin_option) ORDER BY r.rolname,m.rolname) FROM pg_catalog.pg_auth_members a JOIN pg_catalog.pg_roles r ON r.oid=a.roleid JOIN pg_catalog.pg_roles m ON m.oid=a.member WHERE r.rolname IN ('learning_admin','learning_auth_lock','learning_runtime') OR m.rolname IN ('learning_admin','learning_auth_lock','learning_runtime')))";
async fn verify_catalog(conn: &mut PgConnection, roles: &[u8]) -> Result<(), BackupError> {
    let actual = read_catalog(conn).await?;
    let expected = expected_catalog(roles)?;
    if actual != expected {
        return Err(BackupError::Invalid(
            "full restore catalog ownership or ACL differs",
        ));
    }
    Ok(())
}

async fn read_catalog(conn: &mut PgConnection) -> Result<serde_json::Value, BackupError> {
    // SQLx creates a savepoint inside a managed caller transaction, otherwise
    // its own transaction. Rolling back this read scope restores local GUCs
    // without committing or releasing the caller's transaction.
    let mut scope = conn.begin().await?;
    let result: Result<serde_json::Value, sqlx::Error> = async {
        sqlx::query("SELECT pg_catalog.set_config('search_path',$1,true)")
            .bind(catalog_context::PATH)
            .execute(&mut *scope)
            .await?;
        sqlx::query_scalar(CATALOG_SQL).fetch_one(&mut *scope).await
    }
    .await;
    let rollback = scope.rollback().await;
    match result {
        Err(primary) => Err(primary.into()),
        Ok(actual) => {
            rollback?;
            Ok(actual)
        }
    }
}

const CATALOG_SQL: &str = r###"SELECT json_build_object(
 'database',(SELECT json_build_object('owner',pg_get_userbyid(datdba),'acl',datacl::text) FROM pg_database WHERE datname=current_database()),
 'schemas',(SELECT json_agg(json_build_object('name',nspname,'owner',pg_get_userbyid(nspowner),'acl',nspacl::text) ORDER BY nspname) FROM pg_namespace WHERE nspname !~ '^pg_' AND nspname<>'information_schema'),
 'relations',(SELECT json_agg(json_build_object('name',c.relname,'kind',c.relkind,'owner',pg_get_userbyid(c.relowner),'acl',c.relacl::text) ORDER BY c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public'),
 'column_acl',(SELECT coalesce(json_agg(json_build_object('table',c.relname,'column',a.attname,'acl',a.attacl::text) ORDER BY c.relname,a.attnum),'[]') FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND a.attacl IS NOT NULL),
 'functions',(SELECT json_agg(json_build_object('name',p.proname,'args',pg_get_function_identity_arguments(p.oid),'owner',pg_get_userbyid(p.proowner),'acl',p.proacl::text,'security_definer',p.prosecdef,'config',p.proconfig) ORDER BY p.proname,pg_get_function_identity_arguments(p.oid)) FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public'),
 'roles',(SELECT json_agg(json_build_object('name',rolname,'superuser',rolsuper,'inherit',rolinherit,'create_role',rolcreaterole,'create_db',rolcreatedb,'login',rolcanlogin,'replication',rolreplication,'bypass_rls',rolbypassrls,'connection_limit',rolconnlimit) ORDER BY rolname) FROM pg_roles WHERE rolname IN ('learning_admin','learning_runtime','learning_auth_lock')),
 'memberships',(SELECT json_agg(json_build_object('role',r.rolname,'member',m.rolname,'grantor',g.rolname,'admin',a.admin_option,'inherit',a.inherit_option,'set',a.set_option) ORDER BY r.rolname,m.rolname) FROM pg_auth_members a JOIN pg_roles r ON r.oid=a.roleid JOIN pg_roles m ON m.oid=a.member JOIN pg_roles g ON g.oid=a.grantor WHERE r.rolname LIKE 'learning_%' OR m.rolname LIKE 'learning_%'),
 'default_acl',(SELECT coalesce(json_agg(json_build_object('role',pg_get_userbyid(defaclrole),'schema',coalesce(n.nspname,''),'type',defaclobjtype,'acl',defaclacl::text) ORDER BY defaclrole,defaclnamespace,defaclobjtype),'[]') FROM pg_default_acl a LEFT JOIN pg_namespace n ON n.oid=a.defaclnamespace),
 'constraints',(SELECT json_agg(json_build_object('table',c.relname,'name',p.conname,'type',p.contype,'definition',pg_get_constraintdef(p.oid,true)) ORDER BY c.relname,p.conname) FROM pg_constraint p JOIN pg_class c ON c.oid=p.conrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public'),
 'triggers',(SELECT json_agg(json_build_object('table',c.relname,'name',t.tgname,'definition',pg_get_triggerdef(t.oid,true),'enabled',t.tgenabled) ORDER BY c.relname,t.tgname) FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND NOT t.tgisinternal));"###;

fn expected_catalog(roles: &[u8]) -> Result<serde_json::Value, BackupError> {
    crate::validate_role_recipe(roles)?;
    let recipe: serde_json::Value = serde_json::from_slice(roles)?;
    let mut expected = SchemaContract::embedded()?.catalog().clone();
    expected["database"] =
        serde_json::json!({"owner":"learning_admin","acl":"{learning_admin=CTc/learning_admin}"});
    for (target, source) in expected["roles"]
        .as_array_mut()
        .ok_or(BackupError::Invalid("contract roles"))?
        .iter_mut()
        .zip(
            recipe["roles"]
                .as_array()
                .ok_or(BackupError::Invalid("recipe roles"))?,
        )
    {
        target["inherit"] = source["inherit"].clone();
        target["connection_limit"] = source["connection_limit"].clone();
    }
    Ok(expected)
}

#[cfg(test)]
mod catalog_context_tests {
    use super::*;
    use sqlx::{Connection, TransactionManager, postgres::PgConnectOptions};
    use std::{io::Write, os::unix::fs::MetadataExt, path::Path, str::FromStr};

    const EMPTY: &str = "SELECT (SELECT count(*) FROM pg_catalog.pg_class WHERE relnamespace='public'::pg_catalog.regnamespace)+(SELECT count(*) FROM pg_catalog.pg_proc WHERE pronamespace='public'::pg_catalog.regnamespace)+(SELECT count(*) FROM pg_catalog.pg_type WHERE typnamespace='public'::pg_catalog.regnamespace)";
    const SETUP: &str = "CREATE TABLE public.c6_parent(id integer PRIMARY KEY); CREATE TABLE public.c6_child(id integer PRIMARY KEY,parent_id integer REFERENCES public.c6_parent(id)); CREATE FUNCTION public.c6_touch() RETURNS trigger LANGUAGE plpgsql AS $c6$ BEGIN RETURN NEW; END $c6$; CREATE TRIGGER c6_touch BEFORE INSERT ON public.c6_child FOR EACH ROW EXECUTE FUNCTION public.c6_touch(); GRANT SELECT ON public.c6_parent TO learning_runtime;";
    const CLEANUP: &str =
        "DROP TABLE public.c6_child; DROP TABLE public.c6_parent; DROP FUNCTION public.c6_touch();";

    async fn fixture() -> (PgConnection, BackupDir, String) {
        assert_eq!(
            unsafe { libc::geteuid() },
            0,
            "C6 requires the fresh Root fixture"
        );
        let database = std::env::var("TEST_C4_C6_DATABASE").expect("C6 exact database missing");
        let suffix = database
            .strip_prefix("learning_catalog_c6_")
            .expect("C6 dedicated namespace required");
        let id = uuid::Uuid::parse_str(suffix).expect("C6 canonical UUID missing");
        assert!(
            id.get_version_num() == 4
                && id.get_variant() == uuid::Variant::RFC4122
                && id.to_string() == suffix
        );
        let dsn_path =
            std::env::var("TEST_C4_C6_ADMIN_DSN_FILE").expect("C6 private DSN file missing");
        let dsn_path = Path::new(&dsn_path);
        assert!(dsn_path.is_absolute());
        let parent = BackupDir::open_trusted_private_root(dsn_path.parent().unwrap())
            .expect("C6 private credential directory required");
        let file = parent
            .open_file(dsn_path.file_name().unwrap().to_str().unwrap())
            .expect("C6 no-follow DSN file required");
        let meta = file.metadata().unwrap();
        assert!(
            meta.is_file()
                && meta.uid() == 0
                && meta.mode() & 0o7777 == 0o600
                && meta.nlink() == 1
                && meta.len() <= 16384
        );
        let mut raw = Vec::new();
        file.take(16385).read_to_end(&mut raw).unwrap();
        let dsn = std::str::from_utf8(&raw).unwrap_or_else(|_| panic!("C6 DSN encoding rejected"));
        let options = PgConnectOptions::from_str(dsn.trim())
            .unwrap_or_else(|_| panic!("C6 DSN grammar rejected"));
        assert!(
            options.get_database() == Some(database.as_str())
                && options.get_username() == "learning_admin"
                && options.get_socket().map(|p| p.as_path())
                    == Some(Path::new("/var/run/postgresql")),
            "C6 DSN identity/socket rejected"
        );
        let mut conn = PgConnection::connect_with(&options)
            .await
            .unwrap_or_else(|_| panic!("C6 private PG connection rejected"));
        let identity: (String, String, String, i32) = sqlx::query_as("SELECT pg_catalog.current_database()::text,current_user::text,session_user::text,pg_catalog.current_setting('server_version_num')::integer").fetch_one(&mut conn).await.unwrap();
        assert!(
            identity.0 == database
                && identity.1 == "learning_admin"
                && identity.2 == "learning_admin"
                && identity.3 == 180006,
            "C6 server/database/session rejected"
        );
        let roles = serde_json::json!({"format_version":1,"roles":[
            {"name":"learning_admin","login":true,"inherit":true,"superuser":false,"createdb":false,"createrole":false,"bypassrls":false,"replication":false,"connection_limit":-1},
            {"name":"learning_auth_lock","login":false,"inherit":true,"superuser":false,"createdb":false,"createrole":false,"bypassrls":false,"replication":false,"connection_limit":-1},
            {"name":"learning_runtime","login":true,"inherit":true,"superuser":false,"createdb":false,"createrole":false,"bypassrls":false,"replication":false,"connection_limit":-1}],
            "memberships":[{"role":"learning_auth_lock","member":"learning_admin","inherit":false,"set":true,"admin":false}]});
        verify_roles(&mut conn, &serde_json::to_vec(&roles).unwrap())
            .await
            .expect("C6 safe role recipe and runtime closure required");
        let facts: (String, bool, bool, bool, bool) = sqlx::query_as("SELECT pg_catalog.pg_get_userbyid(nspowner)::text,pg_catalog.has_schema_privilege('learning_admin','public','CREATE'),pg_catalog.has_schema_privilege('learning_runtime','public','CREATE'),pg_catalog.has_function_privilege('learning_admin','pg_catalog.pg_control_system()','EXECUTE'),pg_catalog.has_function_privilege('learning_runtime','pg_catalog.pg_control_system()','EXECUTE') FROM pg_catalog.pg_namespace WHERE nspname='public'").fetch_one(&mut conn).await.unwrap();
        assert!(
            facts.0 == "pg_database_owner" && facts.1 && !facts.2 && facts.3 && !facts.4,
            "C6 fixture privilege boundary rejected"
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(EMPTY)
                .fetch_one(&mut conn)
                .await
                .unwrap(),
            0,
            "C6 fresh empty public required"
        );
        let proof_root =
            std::env::var("TEST_C4_C6_PROOF_ROOT").expect("C6 fresh private proof root missing");
        let proof = BackupDir::open_trusted_private_root(Path::new(&proof_root))
            .expect("C6 Root-only no-follow proof directory required");
        (conn, proof, database)
    }

    async fn path(conn: &mut PgConnection, value: &str) {
        sqlx::query("SELECT pg_catalog.set_config('search_path',$1,true)")
            .bind(value)
            .execute(conn)
            .await
            .unwrap();
    }
    async fn setup(conn: &mut PgConnection) -> serde_json::Value {
        sqlx::raw_sql(SETUP).execute(&mut *conn).await.unwrap();
        path(conn, "pg_catalog,public").await;
        sqlx::query_scalar(CATALOG_SQL)
            .fetch_one(conn)
            .await
            .unwrap()
    }
    async fn identity(conn: &mut PgConnection) -> (i32, i64, String) {
        sqlx::query_as("SELECT pg_catalog.pg_backend_pid(),(SELECT (EXTRACT(EPOCH FROM a.backend_start)*1000000)::bigint FROM pg_catalog.pg_stat_activity a WHERE a.pid=pg_catalog.pg_backend_pid()),pg_catalog.pg_current_xact_id()::text").fetch_one(conn).await.unwrap()
    }
    fn assertion(expected: &serde_json::Value) -> String {
        writer_sql::assertion(
            &format!(
                "({})::jsonb='{}'::jsonb",
                CATALOG_SQL.trim_end_matches(';'),
                expected.to_string().replace('\'', "''")
            ),
            "CATALOG",
        )
    }
    fn catalog_refusal(result: &Result<sqlx::postgres::PgQueryResult, sqlx::Error>) -> bool {
        matches!(result, Err(sqlx::Error::Database(error)) if error.message() == "KW_C4_CATALOG")
    }
    fn proof(root: &BackupDir, name: &str, value: serde_json::Value) {
        let bytes = serde_json::to_vec(&value).unwrap();
        assert!(bytes.len() <= 4096);
        let mut file = root
            .create_file(name)
            .expect("C6 create-new proof required");
        file.write_all(&bytes).unwrap();
        file.sync_all().unwrap();
        root.sync().unwrap();
    }

    #[tokio::test]
    #[ignore = "fresh Root PG18.6 C6 database/socket/credential/proof fixture required"]
    async fn c6_writer_guard_restores_empty_path_and_keeps_strict_catalog() {
        let (mut conn, root, database) = fixture().await;
        let mut tx = conn.begin().await.unwrap();
        let expected = setup(&mut tx).await;
        path(&mut tx, "").await;
        let before = identity(&mut tx).await;
        let mut attempt = tx.begin().await.unwrap();
        let result = sqlx::raw_sql(&catalog_context::guard(&assertion(&expected), ""))
            .execute(&mut *attempt)
            .await;
        let normal_restore = if result.is_ok() {
            Some(
                sqlx::query_scalar::<_, String>("SELECT pg_catalog.current_setting('search_path')")
                    .fetch_one(&mut *attempt)
                    .await
                    .unwrap()
                    .is_empty(),
            )
        } else {
            None
        };
        let original_refusal = catalog_refusal(&result);
        attempt.rollback().await.unwrap();
        let same_identity = before == identity(&mut tx).await;
        let restored: String =
            sqlx::query_scalar("SELECT pg_catalog.current_setting('search_path')")
                .fetch_one(&mut *tx)
                .await
                .unwrap();
        tx.rollback().await.unwrap();
        let empty = sqlx::query_scalar::<_, i64>(EMPTY)
            .fetch_one(&mut conn)
            .await
            .unwrap()
            == 0;
        proof(
            &root,
            "c6-writer.json",
            serde_json::json!({"scope":"C6_ISOLATED_RENDER_TEST_NOT_AUTHORITY","database":database,"passed":result.is_ok(),"original_catalog_refusal":original_refusal,"normal_restore":normal_restore,"same_identity":same_identity,"rollback_restore":restored.is_empty(),"public_empty":empty}),
        );
        assert!(same_identity && restored.is_empty() && empty);
        assert!(
            result.is_ok() && normal_restore == Some(true),
            "C6_WRITER_RENDER_CONTEXT_RED"
        );
    }

    #[tokio::test]
    #[ignore = "fresh Root PG18.6 C6 database/socket/credential/proof fixture required"]
    async fn c6_readback_uses_same_render_context_and_restores_path() {
        let (mut conn, root, database) = fixture().await;
        let mut tx = conn.begin().await.unwrap();
        let expected = setup(&mut tx).await;
        path(&mut tx, "").await;
        let before = identity(&mut tx).await;
        let caller_depth = sqlx::postgres::PgTransactionManager::get_transaction_depth(&tx);
        let actual = read_catalog(&mut tx).await.unwrap();
        let caller_depth_retained =
            caller_depth == sqlx::postgres::PgTransactionManager::get_transaction_depth(&tx);
        let strict_equal = actual == expected;
        let changed: Vec<&str> = [
            "database",
            "schemas",
            "relations",
            "column_acl",
            "functions",
            "roles",
            "memberships",
            "default_acl",
            "constraints",
            "triggers",
        ]
        .into_iter()
        .filter(|key| actual[*key] != expected[*key])
        .collect();
        let same_identity = before == identity(&mut tx).await;
        let restored: String =
            sqlx::query_scalar("SELECT pg_catalog.current_setting('search_path')")
                .fetch_one(&mut *tx)
                .await
                .unwrap();
        tx.rollback().await.unwrap();
        let rolled_back_fixture_empty = sqlx::query_scalar::<_, i64>(EMPTY)
            .fetch_one(&mut conn)
            .await
            .unwrap()
            == 0;
        assert!(rolled_back_fixture_empty);
        // Only this fresh isolated test database may persist a tiny fixture.
        // The production reader must restore context without a caller TX too.
        let original_path: String =
            sqlx::query_scalar("SELECT pg_catalog.current_setting('search_path')")
                .fetch_one(&mut conn)
                .await
                .unwrap();
        let mut fixture_tx = conn.begin().await.unwrap();
        let no_tx_expected = setup(&mut fixture_tx).await;
        fixture_tx.commit().await.unwrap();
        sqlx::query("SELECT pg_catalog.set_config('search_path','',false)")
            .execute(&mut conn)
            .await
            .unwrap();
        let no_tx_depth_before = sqlx::postgres::PgTransactionManager::get_transaction_depth(&conn);
        let no_tx_actual = read_catalog(&mut conn).await;
        let no_tx_strict_equal = no_tx_actual
            .as_ref()
            .is_ok_and(|actual| *actual == no_tx_expected);
        let no_tx_depth_after = sqlx::postgres::PgTransactionManager::get_transaction_depth(&conn);
        let no_tx_restored =
            sqlx::query_scalar::<_, String>("SELECT pg_catalog.current_setting('search_path')")
                .fetch_one(&mut conn)
                .await
                .is_ok_and(|path| path.is_empty());
        let cleanup_ok = sqlx::raw_sql(CLEANUP).execute(&mut conn).await.is_ok();
        let session_restore_ok =
            sqlx::query("SELECT pg_catalog.set_config('search_path',$1,false)")
                .bind(&original_path)
                .execute(&mut conn)
                .await
                .is_ok();
        let empty = sqlx::query_scalar::<_, i64>(EMPTY)
            .fetch_one(&mut conn)
            .await
            .is_ok_and(|count| count == 0);
        proof(
            &root,
            "c6-readback.json",
            serde_json::json!({"scope":"C6_ISOLATED_RENDER_TEST_NOT_AUTHORITY","database":database,"strict_equal":strict_equal,"differing_keys":changed,"same_identity":same_identity,"path_restored":restored.is_empty(),"caller_depth_retained":caller_depth_retained,"public_empty":empty,"without_caller_tx":{"strict_equal":no_tx_strict_equal,"path_restored":no_tx_restored,"depth_before":no_tx_depth_before,"depth_after":no_tx_depth_after,"fixture_cleanup":cleanup_ok,"session_restore":session_restore_ok}}),
        );
        assert!(same_identity && restored.is_empty() && caller_depth_retained && empty);
        assert!(
            no_tx_strict_equal
                && no_tx_restored
                && no_tx_depth_before == 0
                && no_tx_depth_after == 0
                && cleanup_ok
                && session_restore_ok,
            "C6_NO_CALLER_TX_RENDER_CONTEXT_FAILED"
        );
        assert!(strict_equal, "C6_READBACK_RENDER_CONTEXT_RED");
    }

    #[tokio::test]
    #[ignore = "fresh Root PG18.6 C6 database/socket/credential/proof fixture required"]
    async fn c6_real_definition_and_acl_changes_still_reject() {
        let (mut conn, root, database) = fixture().await;
        let mut observed = Vec::new();
        for (key, mutation) in [
            (
                "constraints",
                "ALTER TABLE public.c6_child DROP CONSTRAINT c6_child_parent_id_fkey; ALTER TABLE public.c6_child ADD CONSTRAINT c6_child_parent_id_fkey FOREIGN KEY(parent_id) REFERENCES public.c6_parent(id) ON DELETE CASCADE;",
            ),
            (
                "relations",
                "REVOKE SELECT ON public.c6_parent FROM learning_runtime;",
            ),
            (
                "triggers",
                "ALTER TABLE public.c6_child DISABLE TRIGGER c6_touch;",
            ),
        ] {
            let mut tx = conn.begin().await.unwrap();
            let expected = setup(&mut tx).await;
            sqlx::raw_sql(mutation).execute(&mut *tx).await.unwrap();
            let actual: serde_json::Value = sqlx::query_scalar(CATALOG_SQL)
                .fetch_one(&mut *tx)
                .await
                .unwrap();
            let real_change = actual[key] != expected[key];
            path(&mut tx, "").await;
            let before = identity(&mut tx).await;
            let mut attempt = tx.begin().await.unwrap();
            let result = sqlx::raw_sql(&catalog_context::guard(&assertion(&expected), ""))
                .execute(&mut *attempt)
                .await;
            let refused = catalog_refusal(&result);
            attempt.rollback().await.unwrap();
            let same_identity = before == identity(&mut tx).await;
            tx.rollback().await.unwrap();
            let empty = sqlx::query_scalar::<_, i64>(EMPTY)
                .fetch_one(&mut conn)
                .await
                .unwrap()
                == 0;
            observed.push(serde_json::json!({"key":key,"real_change":real_change,"strict_refusal":refused,"same_identity":same_identity,"public_empty":empty}));
        }
        let passed = observed.iter().all(|row| {
            [
                "real_change",
                "strict_refusal",
                "same_identity",
                "public_empty",
            ]
            .into_iter()
            .all(|key| row[key] == true)
        });
        proof(
            &root,
            "c6-mutations.json",
            serde_json::json!({"scope":"C6_ISOLATED_RENDER_TEST_NOT_AUTHORITY","database":database,"mutations":observed,"passed":passed}),
        );
        assert!(passed, "C6_REAL_MUTATION_STRICT_REFUSAL_FAILED");
    }
}

#[cfg(test)]
struct FreshLocalCapture {
    pin: crate::SourceLocalPin,
    root: BackupDir,
}
#[cfg(test)]
impl FreshLocalCapture {
    async fn capture(
        fixture: &mut crate::source::lifecycle_tests::live::Fixture,
    ) -> Result<Self, BackupError> {
        // Only the actual current prepare invocation can mint this type. No
        // serde, caller ID flag, historical receipt, or path-only adapter.
        let pin =
            crate::prepare_source_backup(&fixture.pool, &fixture.assets, &fixture.config).await?;
        if pin.manifest().backup_id != fixture.config.backup_id {
            return Err(BackupError::Invalid("fresh capture identity"));
        }
        let root = BackupDir::open_trusted_private_root(&fixture.config.local_pin_root)?;
        Ok(Self { pin, root })
    }
    fn package(self) -> Result<VerifiedPackage, BackupError> {
        let manifest = self.pin.manifest().clone();
        let directory = self
            .root
            .open_dir(&format!("{}.sealed", manifest.backup_id))?;
        let raw = preflight::read_limited(&directory, "manifest.json", 64 * 1024 * 1024)?;
        if raw != manifest.canonical_bytes()? {
            return Err(BackupError::Invalid("fresh manifest differs"));
        }
        let raw = preflight::read_limited(
            &directory,
            "asset-index.json",
            crate::MAX_ASSET_INDEX_BYTES as u64,
        )?;
        manifest.validate_with_index(&raw)?;
        let parsed: preflight::AssetIndex = serde_json::from_slice(&raw)?;
        let plan = BackupPlan::from_rows(parsed.assets)?;
        if raw != plan.asset_index_bytes() {
            return Err(BackupError::Invalid("fresh index differs"));
        }
        let roles = preflight::read_limited(&directory, "roles.json", 16384)?;
        crate::validate_role_recipe(&roles)?;
        for record in &manifest.files {
            crate::full_restore::assets::copy_original(
                &mut crate::full_restore::assets::open_relative(&directory, &record.path)?,
                &mut std::io::sink(),
                record,
            )?;
        }
        Ok(VerifiedPackage {
            directory,
            manifest,
            plan,
            roles,
            authority: PackageAuthority::Fresh(self.pin),
        })
    }
}
#[cfg(test)]
struct FreshTargetFixture {
    config: RestorePreflightConfig,
}
#[cfg(test)]
impl FreshTargetFixture {
    fn configured(diagnostic: &Diagnostic) -> Result<Self, BackupError> {
        let root = std::path::PathBuf::from(
            live_tests::required("KNOWWEAVE_C4_FULL_TARGET_ROOT").map_err(|failure| {
                diagnostic.import_failure(import_failure_code(failure));
                error(failure)
            })?,
        );
        let batch = root
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or(BackupError::Invalid("fresh target root"))?;
        let id =
            uuid::Uuid::parse_str(batch).map_err(|_| BackupError::Invalid("fresh target UUID"))?;
        if id.to_string() != batch || id.get_version_num() != 4 {
            return Err(BackupError::Invalid("fresh target UUID"));
        }
        let config = RestorePreflightConfig {
            destination_root: root.join("destination"),
            control_root: root.join("control"),
            asset_root: root.join("assets"),
            expected_database: format!("learning_restore_c4_{batch}"),
            trust_path: std::path::PathBuf::from(
                "/var/lib/knowweave-c4-destination/trust/receipt.json",
            ),
        };
        config.validate()?;
        Ok(Self { config })
    }
    async fn admit(self, package: &VerifiedPackage) -> Result<ImportedTarget, BackupError> {
        full_stage(Stage::AdmissionGuard);
        let guard = binding::acquire_for_restore(&self.config)?;
        full_stage(Stage::Profile);
        crate::full_restore::profile::FullRehearsalProfile::read_installed()?;
        let options = sqlx::postgres::PgConnectOptions::new()
            .socket("/var/run/knowweave-target")
            .port(5432)
            .username("learning_admin")
            .database(&self.config.expected_database);
        full_stage(Stage::TargetConnection);
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options)
            .await?;
        drop(guard);
        full_stage(Stage::CandidateAdmission);
        let original = linux::admit_full_candidate_target(self.config, pool, current_audit())
            .await
            .map_err(error)?;
        if is_case(FullFault::WrongEndpoint) {
            let wrong = crate::full_restore::profile::FullRehearsalProfile::read_installed()?
                .source_container_id()
                .to_owned();
            FULL_TEST.with(|control| *control.wrong_container.lock().unwrap() = Some(wrong));
        }
        full_stage(Stage::Preflight);
        let (preflight, config) = original
            .into_full_preflight(package.manifest.clone(), package.plan.clone())
            .await?;
        full_stage(Stage::OwnedAdmission);
        let admission =
            OwnedRestoreAdmission::from_preflight(preflight, config, &package.roles).await?;
        Ok(ImportedTarget {
            admission: Some(admission),
            runtime: tokio::runtime::Handle::current(),
            quarantined: false,
        })
    }
}
#[cfg(test)]
struct RehearsalReport {
    assets: AssetRestoreReport,
    stopped: bool,
}
#[cfg(test)]
async fn rehearse_fresh_local_capture(
    source: FreshLocalCapture,
    target: FreshTargetFixture,
) -> Result<RehearsalReport, BackupError> {
    let diagnostic = FULL_TEST.with(|control| control.diagnostic.clone());
    let package = diagnostic.run(Stage::Package, || source.package())?;
    let mut imported = diagnostic
        .run_async(Stage::TargetAdmission, target.admit(&package))
        .await?;
    let result = async {
        full_stage(Stage::Spool);
        let spool = imported
            .admitted()
            .control
            .create_dir(&format!("full-spool-{}", uuid::Uuid::new_v4()))?;
        full_stage(Stage::DumpOpen);
        let record = package
            .manifest
            .files
            .iter()
            .find(|r| r.path == "database.dump")
            .ok_or(BackupError::Invalid("fresh dump"))?;
        let mut input_file = package.directory.open_file("database.dump")?;
        if is_case(FullFault::BadInput) {
            full_stage(Stage::DumpFault);
            use std::io::{Seek, SeekFrom, Write};
            let mut corrupt = spool.create_file("corrupt.dump")?;
            std::io::copy(&mut input_file, &mut corrupt)?;
            corrupt.seek(SeekFrom::Start(0))?;
            corrupt.write_all(b"BAD!!")?;
            corrupt.sync_all()?;
            spool.sync()?;
            input_file = spool.open_file("corrupt.dump")?;
            current_audit()
                .ok_or(BackupError::Invalid("dump audit absent"))?
                .injection();
        }
        full_stage(Stage::DumpFreeze);
        let frozen_result = freeze_full_dump(&mut input_file, record, &spool);
        diagnostic.returned(&frozen_result);
        if is_case(FullFault::BadInput)
            && matches!(
                &frozen_result,
                Err(BackupError::Invalid("custom dump header"))
            )
        {
            let audit = current_audit().ok_or(BackupError::Invalid("dump audit absent"))?;
            audit.refusal(Boundary::DumpFreeze);
            if audit.after_target(&mut imported).await.is_err() {
                audit.failed();
            }
        }
        let frozen = frozen_result?;
        full_stage(Stage::DumpValidation);
        let observer = crate::full_restore::dump_observer::Observer::new(diagnostic.clone());
        let contract = SchemaContract::embedded_observed(Some(&observer))?;
        let input =
            crate::full_restore::validate_full_dump_observed(frozen, &contract, &spool, observer)
                .await?;
        let PackageAuthority::Fresh(pin) = &package.authority else {
            return Err(BackupError::Invalid("fresh-only rehearsal"));
        };
        let (_keepalive, mut receiver) = watch::channel(false);
        if let Ok(test_receiver) = FULL_TEST.try_with(|control| control.cancel.subscribe()) {
            receiver = test_receiver;
        }
        import_verified(
            &mut imported,
            input,
            &package,
            FullOrigin::Fresh(pin),
            receiver,
        )
        .await?;
        full_stage(Stage::Originals);
        let assets = restore_originals(&mut imported, &package).await?;
        full_stage(Stage::Recheck);
        imported.recheck().await?;
        recovery_tests::after_import(&mut imported, &package).await?;
        Ok::<_, BackupError>(assets)
    }
    .await;
    diagnostic.returned(&result);
    if is_case(FullFault::BadInput) {
        diagnostic
            .settle_async(Stage::NegativeTargetAudit, async {
                preflight::target_facts(imported.connection(), 0)
                    .await?
                    .validate()?;
                if !imported.admitted().assets.list()?.is_empty() {
                    return Err(BackupError::Invalid("bad full input wrote originals"));
                }
                Ok(())
            })
            .await?;
    }
    diagnostic
        .settle_async(Stage::Stop, imported.stop())
        .await?;
    let assets = result?;
    Ok(RehearsalReport {
        assets,
        stopped: true,
    })
}
#[cfg(test)]
pub(crate) async fn real_full_import_rehearsal(case: FullFault) -> Result<(), BackupError> {
    let diagnostic = Diagnostic::new();
    let result = real_full_import_rehearsal_inner(case, diagnostic.clone()).await;
    // Live callers unwrap this test result. Never expose arbitrary Error Debug.
    diagnostic.public_result(result)
}
#[cfg(test)]
async fn real_full_import_rehearsal_inner(
    case: FullFault,
    diagnostic: Diagnostic,
) -> Result<(), BackupError> {
    diagnostic.enter(Stage::Fixture);
    let mut fixture = crate::source::lifecycle_tests::live::Fixture::new().await;
    recovery_tests::seed(&fixture.pool).await?;
    let source = diagnostic
        .run_async(Stage::Capture, FreshLocalCapture::capture(&mut fixture))
        .await?;
    let expected = source.pin.manifest().clone();
    let target = diagnostic.run(Stage::TargetConfiguration, || {
        FreshTargetFixture::configured(&diagnostic)
    })?;
    let control_root = target.config.control_root.clone();
    let database = target.config.expected_database.clone();
    let (cancel, _receiver) = watch::channel(false);
    let (result, observed, audit) = FULL_TEST
        .scope(
            FullTestControl {
                case,
                cancel,
                wrong_container: std::sync::Mutex::new(None),
                observation: std::sync::Mutex::new(None),
                audit: AdmissionAudit::for_case(case),
                diagnostic: diagnostic.clone(),
                writer_observation: (case == FullFault::Success).then(|| {
                    (
                        supervisor_observation::Observation::new(),
                        BackupDir::open_trusted_private_root(&fixture.proof_root)
                            .map_err(BackupError::from),
                    )
                }),
            },
            async {
                let result = rehearse_fresh_local_capture(source, target).await;
                // Evidence failure is secondary; preserve the operational
                // result/report and let the existing settlement gate reject it.
                FULL_TEST.with(|control| {
                    if let Some((observation, root)) = &control.writer_observation {
                        observation.persist_secondary(&control.diagnostic, root.as_ref());
                    }
                });
                let observed = FULL_TEST.with(|control| control.observation.lock().unwrap().take());
                let audit = current_audit();
                (result, observed, audit)
            },
        )
        .await;
    if matches!(
        case,
        FullFault::BadInput
            | FullFault::BadRole
            | FullFault::DirtyTarget
            | FullFault::WrongEndpoint
            | FullFault::AssetCorrupt
    ) {
        diagnostic.settle(Stage::NegativeAudit, || {
            audit
                .as_ref()
                .ok_or(BackupError::Invalid("required negative audit absent"))?
                .validate(result.is_err())
        })?;
    }
    // An expected operational refusal cannot grant credit for a failed stop
    // or audit. The required negative audit above always runs first.
    diagnostic.require_settlement()?;
    let (result, observed) = diagnostic.observe(
        result,
        observed,
        !matches!(
            case,
            FullFault::BadInput | FullFault::BadRole | FullFault::DirtyTarget
        ),
    )?;
    diagnostic.settle(Stage::Observation, || {
        match case {
            FullFault::BadInput | FullFault::BadRole | FullFault::DirtyTarget => {
                if observed.is_some() {
                    return Err(BackupError::Invalid("bad input started writer"));
                }
            }
            _ => {
                let observation = observed
                    .as_ref()
                    .ok_or(BackupError::Invalid("full writer observation missing"))?;
                let expected_failure = match case {
                    FullFault::Success | FullFault::AssetCorrupt => None,
                    FullFault::Cancel => Some(ImportFailure::Cancelled),
                    FullFault::CommitUnknown => Some(ImportFailure::CommitUnknown),
                    FullFault::PrecommitEof | FullFault::WrongEndpoint => {
                        Some(ImportFailure::Protocol)
                    }
                    FullFault::BadInput | FullFault::BadRole | FullFault::DirtyTarget => {
                        unreachable!()
                    }
                };
                // The mandatory audit above already requires this SAME native
                // writer's complete, settled, exact database-missing primary error
                // plus equal inventories. Transport codes alone grant no credit.
                let failure_matches = if case == FullFault::WrongEndpoint {
                    matches!(
                        observation.failure,
                        Some(ImportFailure::Protocol | ImportFailure::Exit | ImportFailure::Stderr)
                    )
                } else {
                    observation.failure == expected_failure
                };
                if !failure_matches
                    || observation.commit_attempted
                        != matches!(
                            case,
                            FullFault::Success | FullFault::AssetCorrupt | FullFault::CommitUnknown
                        )
                    || (matches!(case, FullFault::PrecommitEof | FullFault::Cancel)
                        && !observation.content_verified)
                    || (!matches!(case, FullFault::Success | FullFault::AssetCorrupt)
                        && !observation.stop_confirmed)
                {
                    return Err(BackupError::Invalid("full fault protocol outcome differs"));
                }
            }
        }
        Ok(())
    })?;
    if case == FullFault::Success {
        let report = result?;
        diagnostic.enter(Stage::Report);
        if !report.stopped || report.assets.logical_assets() != expected.logical_asset_count {
            return Err(BackupError::Invalid("full rehearsal report"));
        }
    } else {
        diagnostic.enter(Stage::Report);
        if result.is_ok() {
            return Err(BackupError::Invalid("full fault accepted"));
        }
        diagnostic.enter(Stage::DurableInventory);
        let control = BackupDir::open_trusted_private_root(&control_root)?;
        let attempt = control
            .kind(&preflight::restore_attempt_name(&database)?)
            .is_ok();
        let intent = control
            .kind(&format!("{database}.restore.commit-attempt"))
            .is_ok();
        let expected_attempt = !matches!(
            case,
            FullFault::WrongEndpoint
                | FullFault::BadInput
                | FullFault::BadRole
                | FullFault::DirtyTarget
        );
        let expected_intent = matches!(case, FullFault::CommitUnknown | FullFault::AssetCorrupt);
        if attempt != expected_attempt || intent != expected_intent {
            return Err(BackupError::Invalid("full fault durable inventory"));
        }
        if attempt
            && preflight::reject_existing_attempt(
                control.kind(&preflight::restore_attempt_name(&database)?),
            )
            .is_ok()
        {
            return Err(BackupError::Invalid("full replay admitted"));
        }
    }
    diagnostic.enter(Stage::StopInspection);
    let profile = crate::full_restore::profile::FullRehearsalProfile::read_installed()?;
    let target_id = profile.target_container_id();
    let inspected = binding::linux::inspect("container", target_id)?;
    if inspected["Id"].as_str() != Some(target_id)
        || inspected["State"]["Running"].as_bool() != Some(false)
        || inspected["State"]["Pid"].as_u64() != Some(0)
        || inspected
            .get("ExecIDs")
            .is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()))
    {
        return Err(BackupError::Invalid("full body target stop unconfirmed"));
    }
    let case_name = match case {
        FullFault::Success => "full-import",
        FullFault::PrecommitEof => "full-precommit-eof",
        FullFault::Cancel => "full-cancel",
        FullFault::CommitUnknown => "full-commit-unknown",
        FullFault::WrongEndpoint => "full-wrong-endpoint",
        FullFault::BadInput => "full-bad-input",
        FullFault::BadRole => "full-bad-role",
        FullFault::DirtyTarget => "full-dirty-target",
        FullFault::AssetCorrupt => "full-asset-corrupt",
    };
    diagnostic.enter(Stage::Proof);
    let negative_evidence = if let Some(audit) = &audit {
        use std::io::Write;
        let bytes = serde_json::to_vec(&*audit.evidence.lock().unwrap())?;
        if bytes.len() > 64 * 1024 * 1024 {
            return Err(BackupError::Capacity("negative inventory proof"));
        }
        let root = BackupDir::open_trusted_private_root(&fixture.proof_root)?;
        let mut file = root.create_file("full-negative-inventory.json")?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        root.sync()?;
        Some(
            serde_json::json!({"stage":audit.stage,"sha256":hex::encode(Sha256::digest(&bytes)),"bytes":bytes.len(),"unchanged":true}),
        )
    } else {
        None
    };
    let proof = serde_json::json!({"format_version":1,"classification":"FULL_IMPORT_CASE_PASSED_SINGLE_HOST_QUARANTINED_NOT_COMPLETE","case":case_name,"backup_id":expected.backup_id,"manifest_sha256":expected.canonical_sha256()?,"target_database":database,"target_container_id":target_id,"target_stopped":true,"commit_attempted":observed.as_ref().is_some_and(|o|o.commit_attempted),"rollback_verified":matches!(case,FullFault::PrecommitEof|FullFault::Cancel)&&observed.as_ref().is_some_and(|o|o.content_verified),"import_catalog_verified":observed.as_ref().is_some_and(|o|o.content_verified&&o.failure.is_none()),"originals_verified":case==FullFault::Success,"logical_assets":expected.logical_asset_count,"unique_asset_bytes":expected.unique_asset_bytes,"negative_evidence":negative_evidence,"wrong_endpoint_proof":if case==FullFault::WrongEndpoint {Some("different_database_route_only_not_same_id_clone")}else{None}});
    use std::io::Write;
    let root = BackupDir::open_trusted_private_root(&fixture.proof_root)?;
    let mut file = root.create_file("full-import-result.json")?;
    file.write_all(&serde_json::to_vec(&proof)?)?;
    file.sync_all()?;
    root.sync()?;
    fixture.pool.close().await;
    Ok(())
}

/// Read-only installed management bridge. It exposes observations, never a
/// constructor or serialized replacement for CompleteBackup authority.
pub fn verified_complete_role_recipe(backup_id: uuid::Uuid) -> Result<Vec<u8>, BackupError> {
    let trust_path = std::path::Path::new(crate::DESTINATION_TRUST_PATH);
    BackupDir::open_trusted_private_root(
        trust_path
            .parent()
            .ok_or(BackupError::Invalid("installed trust parent"))?,
    )?;
    let trust = crate::complete::load_trust(trust_path)?;
    let complete = crate::open_complete_backup(
        std::path::Path::new(&trust.destination_root),
        trust_path,
        backup_id,
    )?;
    let package = VerifiedPackage::open_at(
        &complete,
        std::path::Path::new(&trust.destination_root),
        trust_path,
    )?;
    let proof = match &package.authority {
        PackageAuthority::Complete { _proof, .. } => _proof,
        #[cfg(test)]
        PackageAuthority::Fresh(_) => {
            return Err(BackupError::Invalid("complete role reader authority"));
        }
    };
    if option_env!("KNOWWEAVE_BUILD_ID_SHA256")
        != Some(package.manifest.source.application_build_sha256.as_str())
    {
        return Err(BackupError::Invalid("role reader package build differs"));
    }
    let value = serde_json::json!({"format_version":1,"capability":"complete_role_recipe_v1","backup_id":complete.backup_id(),"manifest_sha256":complete.manifest_sha256(),"receipt_sha256":complete.receipt_sha256(),"proof_manifest_sha256":proof.manifest_sha256(),"application_build_sha256":package.manifest.source.application_build_sha256,"roles":serde_json::from_slice::<serde_json::Value>(&package.roles)?});
    let bytes = serde_json::to_vec(&value)?;
    if bytes.len() > 16384 {
        return Err(BackupError::Capacity("verified role observation"));
    }
    Ok(bytes)
}

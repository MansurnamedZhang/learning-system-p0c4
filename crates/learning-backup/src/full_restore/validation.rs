//! Recovery validation owns the original admitted target. Disk observations are
//! never a replacement for CompleteBackup or for a successful live operation.
use super::*;
use crate::full_restore::{data, jobs};
use serde_json::{Value, json};
use std::io::Write;
use std::sync::atomic::AtomicU8;
use std::time::SystemTime;

const SESSION_SECONDS: u64 = 15;
const TRANSACTION_SECONDS: u64 = 60;
const RECORD_CAP: u64 = 16 * 1024 * 1024;
type ValidationFuture<'a, T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, BackupError>> + Send + 'a>>;
// One inherited absolute deadline, checked before polling any phase. Selecting
// cancellation drops the borrowed phase before its owner begins quarantine.
fn validation_phase<'a, T: Send + 'a>(
    cancel: &'a mut watch::Receiver<bool>,
    end: Instant,
    operation: ValidationFuture<'a, T>,
) -> ValidationFuture<'a, T> {
    Box::pin(async move {
        gate(cancel, end).map_err(error)?;
        tokio::select! {
            biased;
            _=cancel.changed()=>Err(error(ImportFailure::Cancelled)),
            _=tokio::time::sleep_until(tokio::time::Instant::from_std(end))=>Err(error(ImportFailure::Deadline)),
            result=operation=>result,
        }
    })
}
type MetadataBudget = Arc<std::sync::Mutex<crate::registry::ScanBudget>>;
fn charge_metadata(budget: &MetadataBudget, bytes: u64) -> Result<(), BackupError> {
    budget
        .lock()
        .map_err(|_| BackupError::Invalid("recovery metadata budget poisoned"))?
        .charge(bytes)
}
fn charge_value(budget: &MetadataBudget, value: &impl serde::Serialize) -> Result<(), BackupError> {
    charge_metadata(budget, serde_json::to_vec(value)?.len() as u64)
}
fn read_record(
    root: &BackupDir,
    name: &str,
    cap: u64,
    budget: &MetadataBudget,
) -> Result<Vec<u8>, BackupError> {
    use std::os::unix::fs::MetadataExt;
    let file = root.open_file(name)?;
    let meta = file.metadata()?;
    if meta.uid() != 0
        || meta.mode() & 0o777 != 0o600
        || meta.nlink() != 1
        || meta.len() == 0
        || meta.len() > cap
    {
        return Err(BackupError::Invalid("recovery private metadata file"));
    }
    // Reserve the possible growth byte before allocation/read; a changed size
    // is an error, not permission to read uncharged replacement metadata.
    let bound = meta.len().checked_add(1).ok_or(BackupError::Overflow)?;
    charge_metadata(budget, bound)?;
    let mut bytes = Vec::new();
    file.take(bound).read_to_end(&mut bytes)?;
    if bytes.len() as u64 != meta.len() {
        return Err(BackupError::Invalid("recovery metadata size changed"));
    }
    Ok(bytes)
}

/// The cleanup task owns only an exact claim; ImportedTarget retains the actual
/// admission until this task settles, including when a borrowed future is lost.
pub(super) struct RecoveryFence {
    claim: binding::DockerClaim,
    failed: AtomicBool,
    started: AtomicBool,
    outcome: AtomicU8,
    completed: tokio::sync::Notify,
}
impl RecoveryFence {
    pub(super) fn new(claim: binding::DockerClaim) -> Self {
        Self {
            claim,
            failed: AtomicBool::new(false),
            started: AtomicBool::new(false),
            outcome: AtomicU8::new(0),
            completed: tokio::sync::Notify::new(),
        }
    }
    pub(super) fn require_usable(&self) -> Result<(), BackupError> {
        if self.failed.load(Ordering::Acquire) {
            Err(BackupError::Invalid(
                "recovery operation permanently unusable",
            ))
        } else {
            Ok(())
        }
    }
    pub(super) fn started(&self) -> bool {
        self.started.load(Ordering::Acquire)
    }
    pub(super) fn fail(self: &Arc<Self>, runtime: &tokio::runtime::Handle) {
        self.failed.store(true, Ordering::Release);
        if !self.started.swap(true, Ordering::AcqRel) {
            let owned = Arc::clone(self);
            runtime.spawn(async move {
                let stopped =
                    linux_child::candidate_quarantine(&owned.claim).await == Isolation::Stopped;
                owned
                    .outcome
                    .store(if stopped { 1 } else { 2 }, Ordering::Release);
                owned.completed.notify_waiters();
            });
        }
    }
    pub(super) async fn wait(&self) -> bool {
        loop {
            let notified = self.completed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            match self.outcome.load(Ordering::Acquire) {
                1 => return true,
                2 => return false,
                _ => notified.await,
            }
        }
    }
}

fn canonical(value: &Value) -> Result<Vec<u8>, BackupError> {
    Ok(serde_json::to_vec(value)?)
}
fn write_record(
    root: &BackupDir,
    name: &str,
    value: &Value,
    budget: &MetadataBudget,
) -> Result<String, BackupError> {
    write_record_capped(root, name, value, RECORD_CAP, budget)
}
fn write_record_capped(
    root: &BackupDir,
    name: &str,
    value: &Value,
    cap: u64,
    budget: &MetadataBudget,
) -> Result<String, BackupError> {
    let bytes = canonical(value)?;
    if bytes.len() as u64 > cap {
        return Err(BackupError::Capacity("recovery record"));
    }
    charge_metadata(budget, bytes.len() as u64)?;
    let mut file = root.create_file(name)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    root.sync()?;
    let readback = read_record(root, name, cap, budget)?;
    if readback != bytes {
        return Err(BackupError::Invalid("recovery record readback"));
    }
    Ok(crate::digest(&bytes))
}

pub(super) fn bind_import_attempt(
    target: &mut ImportedTarget,
    package: &VerifiedPackage,
) -> Result<(), BackupError> {
    let budget = target.recovery_metadata_budget();
    let admission = target.admitted();
    let database = admission.config.expected_database.clone();
    let name = preflight::restore_attempt_name(&database)?;
    let attempt = read_record(&admission.control, &name, 4096, &budget)?;
    let intent = read_record(
        &admission.control,
        &format!("{database}.restore.commit-attempt"),
        4096,
        &budget,
    )?;
    let a: Value = serde_json::from_slice(&attempt)?;
    let i: Value = serde_json::from_slice(&intent)?;
    let complete = matches!(&package.authority, PackageAuthority::Complete { .. });
    let kind = if complete {
        "FULL_BACKUP_IMPORT"
    } else {
        "FULL_LOCAL_REHEARSAL"
    };
    if a["record_type"] != kind
        || a["phase"] != "ATTEMPT"
        || a["database"] != database
        || a["backup_id"] != package.manifest.backup_id.to_string()
        || a["manifest_sha256"] != package.manifest.canonical_sha256()?
        || i["record_type"] != kind
        || i["phase"] != "COMMIT_ATTEMPTED"
        || i["database"] != database
        || i["attempt_sha256"] != crate::digest(&attempt)
        || i["writer_sha256"] != a["writer_sha256"]
    {
        return Err(BackupError::Invalid("recovery import attempt linkage"));
    }
    target.recovery_binding["import_attempt_sha256"] = json!(crate::digest(&attempt));
    target.recovery_binding["import_commit_intent_sha256"] = json!(crate::digest(&intent));
    Ok(())
}

pub(super) fn recovery_control(target: &mut ImportedTarget) -> Result<BackupDir, BackupError> {
    target.recovery_fence.require_usable()?;
    if let Some(root) = &target.recovery_directory {
        return Ok(root.try_clone()?);
    }
    if target
        .recovery_binding
        .get("import_attempt_sha256")
        .is_none()
    {
        return Err(BackupError::Invalid(
            "recovery requires completed import linkage",
        ));
    }
    let identity = target.recovery_identity();
    let budget = target.recovery_metadata_budget();
    let admission = target.admitted();
    let id = admission
        .config
        .expected_database
        .strip_prefix("learning_restore_c4_")
        .ok_or(BackupError::Invalid("recovery target database"))?;
    let directory = admission.control.create_dir(&format!("recovery-{id}"))?;
    // Any visible residue is unreplayable; there is no reopen-and-complete API.
    admission.control.sync()?;
    write_record(&directory, "identity.json", &identity, &budget)?;
    target.recovery_directory = Some(directory);
    Ok(target.recovery_directory.as_ref().unwrap().try_clone()?)
}

/// A distinct top-level SQL transaction, freshly proven against the original
/// held target. It is neither a savepoint nor a public caller SQL capability.
pub(crate) struct RecoveryTransaction {
    challenge: Option<binding::LockChallenge<sqlx::Transaction<'static, sqlx::Postgres>>>,
    pid: i32,
    oid: u64,
    identity: Value,
    control: BackupDir,
    deadline: Instant,
    fence: Arc<RecoveryFence>,
    runtime: tokio::runtime::Handle,
    finished: bool,
    budget: MetadataBudget,
}
pub(super) async fn begin_recovery_transaction(
    target: &mut ImportedTarget,
) -> Result<RecoveryTransaction, BackupError> {
    let budget = target.recovery_metadata_budget();
    struct AdmissionFence {
        fence: Arc<RecoveryFence>,
        runtime: tokio::runtime::Handle,
        complete: bool,
    }
    impl Drop for AdmissionFence {
        fn drop(&mut self) {
            if !self.complete {
                self.fence.fail(&self.runtime);
            }
        }
    }
    let mut admission_fence = AdmissionFence {
        fence: Arc::clone(&target.recovery_fence),
        runtime: target.runtime.clone(),
        complete: false,
    };
    let result = async {
        target.recheck().await?;
        let control = target.recovery_control()?;
        let identity = target.recovery_identity();
        write_record(&control, "transaction-begin.json", &json!({"format_version":1,"capability":"recovery_transaction_begin_v1","identity":identity}),&budget,)?;
        let admin = target.admitted().recovery_admin.clone();
        let deadline = target.recovery_phase_deadline(Duration::from_secs(SESSION_SECONDS));
        let (mut challenge, pid, oid) = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), preflight::begin_sql_session(&admin))
            .await.map_err(|_| BackupError::Invalid("recovery transaction admission deadline"))??;
        let admission = target.admitted();
        linux_child::candidate_recheck(&mut admission.preflight.bound_target, &mut challenge, pid, oid, deadline)
            .await.map_err(|e| error(child_error(e)))?;
        let users:(String,String)=tokio::time::timeout_at(tokio::time::Instant::from_std(deadline),sqlx::query_as("SELECT current_user::text,session_user::text").fetch_one(&mut **challenge.lease_mut()))
            .await.map_err(|_|BackupError::Invalid("recovery transaction user deadline"))??;
        if users!=("learning_admin".into(),"learning_admin".into()){
            return Err(BackupError::Invalid("recovery transaction admin role differs"));
        }
        // Limit every statement on the new connection as well as the outer owner.
        for sql in ["SET LOCAL statement_timeout='15s'", "SET LOCAL lock_timeout='5s'", "SET LOCAL idle_in_transaction_session_timeout='30s'", "SET LOCAL transaction_timeout='60s'"] {
            tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), sqlx::query(sql).execute(&mut **challenge.lease_mut()))
                .await.map_err(|_| BackupError::Invalid("recovery transaction setup deadline"))??;
        }
        target.recheck().await?;
        Ok(RecoveryTransaction { challenge: Some(challenge), pid, oid, identity, control,
            deadline: target.recovery_phase_deadline(Duration::from_secs(TRANSACTION_SECONDS)), fence:Arc::clone(&target.recovery_fence),
            runtime:target.runtime.clone(), finished:false, budget })
    }.await;
    if result.is_err() {
        let _ = target.quarantine().await;
    }
    if result.is_ok() {
        admission_fence.complete = true;
    }
    result
}
impl RecoveryTransaction {
    pub(crate) fn connection(&mut self) -> &mut PgConnection {
        self.challenge
            .as_mut()
            .expect("owned recovery transaction")
            .lease_mut()
    }
    pub(crate) async fn commit(mut self, target: &mut ImportedTarget) -> Result<(), BackupError> {
        let budget = Arc::clone(&self.budget);
        let result = async {
            if !Arc::ptr_eq(&self.fence, &target.recovery_fence) || self.identity != target.recovery_identity() {
                return Err(BackupError::Invalid("recovery transaction target changed"));
            }
            target.recheck().await?;
            let admission = target.admitted();
            linux_child::candidate_recheck(&mut admission.preflight.bound_target, self.challenge.as_mut().unwrap(), self.pid, self.oid, self.deadline)
                .await.map_err(|e| error(child_error(e)))?;
            let normalization = self.control.open_dir("normalization")?;
            let intent = jobs::verify_normalization_intent(&normalization, &self.identity, &budget)?;
            write_record(&self.control, "transaction-commit-intent.json", &json!({"format_version":1,"capability":"recovery_transaction_commit_intent_v1","identity":self.identity,"normalization_intent_sha256":intent,"backend_pid":self.pid,"database_oid":self.oid}),&budget,)?;
            target.recovery_operation_gate()?;
            let challenge = self.challenge.take().unwrap();
            // Only this fresh transaction is consumed. The original challenge
            // remains held and rechecks the endpoint after durable COMMIT.
            tokio::time::timeout_at(tokio::time::Instant::from_std(self.deadline), challenge.lease.commit())
                .await.map_err(|_| BackupError::Invalid("recovery normalization commit unknown"))??;
            target.recheck().await?;
            write_record(&self.control, "transaction-committed.json", &json!({"format_version":1,"capability":"recovery_transaction_committed_v1","identity":self.identity,"normalization_intent_sha256":intent}),&budget,)?;
            Ok(())
        }.await;
        if result.is_err() {
            let _ = write_record(
                &self.control,
                "transaction-unknown.json",
                &json!({"format_version":1,"state":"UNKNOWN_UNUSABLE_UNREPLAYABLE","identity":self.identity}),
                &budget,
            );
            let isolation = target.quarantine().await;
            self.finished = true;
            isolation?;
            return result;
        }
        self.finished = true;
        result
    }
}
impl Drop for RecoveryTransaction {
    fn drop(&mut self) {
        let budget = Arc::clone(&self.budget);
        if !self.finished {
            let _ = write_record(
                &self.control,
                "transaction-abandoned.json",
                &json!({"format_version":1,"state":"ABANDONED_UNUSABLE_UNREPLAYABLE","identity":self.identity}),
                &budget,
            );
            self.fence.fail(&self.runtime);
        }
    }
}

/// Opaque evidence that all pre-normalization rows were checked on this target.
pub(super) struct PreparedRecoveryValidation {
    identity: Value,
    rows: Value,
    rows_sha256: String,
    expected: data::ExpectedRecoveryRows,
}

fn prepare_validation<'a>(
    target: &'a mut ImportedTarget,
    package: &'a VerifiedPackage,
    expected: &'a data::ExpectedRecoveryRows,
) -> ValidationFuture<'a, PreparedRecoveryValidation> {
    Box::pin(async move {
        target.recheck().await?;
        let root = target.recovery_control()?;
        let scratch = root.create_dir("row-comparison")?;
        let rows = data::verify_target_rows(target.connection(), expected, &scratch).await?;
        charge_value(&target.recovery_metadata_budget(), &rows.evidence())?;
        if rows.table_count() != 64 {
            return Err(BackupError::Invalid("recovery row inventory incomplete"));
        }
        verify_roles(target.connection(), &package.roles).await?;
        verify_catalog(target.connection(), &package.roles, &package.contract).await?;
        verify_originals(target, package).await?;
        target.recheck().await?;
        Ok(PreparedRecoveryValidation {
            identity: target.recovery_identity(),
            rows: rows.evidence(),
            rows_sha256: rows.canonical_sha256(),
            expected: expected.clone(),
        })
    })
}

fn verify_originals<'a>(
    target: &'a mut ImportedTarget,
    package: &'a VerifiedPackage,
) -> ValidationFuture<'a, ()> {
    Box::pin(async move {
        let rows: Vec<(uuid::Uuid,uuid::Uuid,String,i64,String)> = sqlx::query_as("SELECT space_id,id,sha256,byte_size,storage_key FROM public.asset WHERE status='ready' ORDER BY space_id,id LIMIT 100001")
        .fetch_all(target.connection()).await?;
        crate::validate_restored_assets(
            &package.plan,
            rows.into_iter()
                .map(
                    |(space_id, id, sha256, byte_size, storage_key)| crate::AssetRow {
                        space_id,
                        id,
                        sha256,
                        byte_size,
                        storage_key,
                    },
                )
                .collect(),
        )?;
        let original = target.admitted().assets.try_clone()?;
        let plan = package.plan.clone();
        tokio::task::spawn_blocking(move || {
            crate::full_restore::assets::verify_all(&original, &plan)
        })
        .await
        .map_err(|_| BackupError::Invalid("recovery originals verification owner lost"))??;
        Ok(())
    })
}

/// Eligibility only, never an activation or a connection permit. Construction
/// requires both the original CompleteBackup and a non-serialized live receipt.
pub struct RecoveryValidation {
    pending: RecoveryPending,
    receipt: CompletedReceipt,
}
pub struct RecoveredInstance {
    pending: RecoveryPending,
    receipt: CompletedReceipt,
}
struct CompletedReceipt {
    sha256: String,
}
impl RecoveryValidation {
    pub fn finish_complete(self) -> Result<RecoveredInstance, BackupError> {
        self.pending.target.recovery_fence.require_usable()?;
        if !matches!(
            &self.pending.package.authority,
            PackageAuthority::Complete { .. }
        ) {
            return Err(BackupError::Invalid(
                "recovery requires retained CompleteBackup",
            ));
        }
        Ok(RecoveredInstance {
            pending: self.pending,
            receipt: self.receipt,
        })
    }
}
impl RecoveredInstance {
    pub fn backup_id(&self) -> uuid::Uuid {
        self.pending.complete.backup_id()
    }
    pub fn validation_receipt_sha256(&self) -> &str {
        &self.receipt.sha256
    }
}
pub async fn validate_recovery(
    pending: RecoveryPending,
) -> Result<RecoveryValidation, BackupError> {
    validate_recovery_owned(pending, None).await
}

/// Consume a genuine destination-only copy; no caller observations or paths
/// can supply recovery effect authority. CompleteBackup remains mandatory.
pub async fn validate_recovery_with_snapshot_effects(
    pending: RecoveryPending,
    copied: crate::destination::snapshot_effects::CopiedSnapshotEffectsV2,
) -> Result<RecoveryValidation, BackupError> {
    validate_recovery_owned(pending, Some(copied)).await
}

async fn validate_recovery_owned(
    pending: RecoveryPending,
    copied: Option<crate::destination::snapshot_effects::CopiedSnapshotEffectsV2>,
) -> Result<RecoveryValidation, BackupError> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let (cancel, rx) = watch::channel(false);
    let _cancel = Cancellation(cancel);
    tokio::spawn(async move {
        let result = validate_owned(pending, copied, rx).await;
        let _ = sender.send(result);
    });
    receiver
        .await
        .map_err(|_| BackupError::Invalid("recovery validation owner lost; target unusable"))?
}
async fn validate_owned(
    mut pending: RecoveryPending,
    copied: Option<crate::destination::snapshot_effects::CopiedSnapshotEffectsV2>,
    mut cancel: watch::Receiver<bool>,
) -> Result<RecoveryValidation, BackupError> {
    let end = Instant::now() + Duration::from_secs(900);
    pending.target.recovery_supervision = Some((cancel.clone(), end));
    let result = async {
        let prepared = validation_phase(
            &mut cancel,
            end,
            prepare_validation(&mut pending.target, &pending.package, &pending.expected),
        )
        .await?;
        gate(&cancel, end).map_err(error)?;
        let lease = match &pending.package.authority {
            PackageAuthority::Complete {
                _destination: destination,
                ..
            } => destination.lease.clone_held()?,
            #[cfg(test)]
            PackageAuthority::Fresh(_) => {
                return Err(BackupError::Invalid(
                    "normal recovery requires CompleteBackup",
                ));
            }
        };
        let effects = match copied {
            Some(copied) => {
                validation_phase(
                    &mut cancel,
                    end,
                    jobs::inspect_copied_external_effects(&mut pending.target, &lease, copied),
                )
                .await?
            }
            None => {
                validation_phase(
                    &mut cancel,
                    end,
                    jobs::inspect_external_effects(&mut pending.target, &lease),
                )
                .await?
            }
        };
        gate(&cancel, end).map_err(error)?;
        let report = validation_phase(
            &mut cancel,
            end,
            jobs::normalize_restored_jobs(&mut pending.target, &effects, SystemTime::now()),
        )
        .await?;
        let barriers = validation_phase(
            &mut cancel,
            end,
            finish_barriers(
                &mut pending.target,
                &pending.package,
                prepared,
                &report,
                &effects,
            ),
        )
        .await?;
        gate(&cancel, end).map_err(error)?;
        let receipt = persist_validation_receipt(
            &mut pending.target,
            &pending.package,
            &report,
            barriers,
            "RECOVERY_VALIDATION_PASSED_COMPLETE_BACKED_NOT_ACTIVATED",
        )?;
        // Synchronous receipt publication must settle before this checkpoint;
        // a visible record cannot create a capability after caller loss.
        gate(&cancel, end).map_err(error)?;
        validation_phase(&mut cancel, end, Box::pin(pending.target.recheck())).await?;
        gate(&cancel, end).map_err(error)?;
        Ok(receipt)
    }
    .await;
    match result {
        Ok(receipt) => {
            pending.target.recovery_supervision = None;
            Ok(RecoveryValidation { pending, receipt })
        }
        Err(primary) => {
            pending.target.quarantine().await?;
            Err(primary)
        }
    }
}

fn finish_barriers<'a>(
    target: &'a mut ImportedTarget,
    package: &'a VerifiedPackage,
    prepared: PreparedRecoveryValidation,
    report: &'a jobs::JobRecoveryReport,
    effects: &'a jobs::ExternalReconciliation,
) -> ValidationFuture<'a, Value> {
    Box::pin(async move {
        let budget = target.recovery_metadata_budget();
        if prepared.identity != target.recovery_identity() {
            return Err(BackupError::Invalid("recovery validation target changed"));
        }
        let root = target.recovery_control()?;
        write_record(
            &root,
            "source-row-validation.json",
            &json!({"format_version":1,"identity":prepared.identity,"source_rows_sha256":prepared.rows_sha256,"rows":prepared.rows}),
            &budget,
        )?;
        if report.has_holds() {
            return Err(BackupError::Invalid("recovery external effects unresolved"));
        }
        verify_roles(target.connection(), &package.roles).await?;
        verify_catalog(target.connection(), &package.roles, &package.contract).await?;
        verify_originals(target, package).await?;
        let integrity = verify_integrity_results(target).await?;
        let snapshots = jobs::verify_copied_deliveries(target, effects).await?;
        let snapshot_file = write_record_capped(
            &root,
            "snapshot-deliveries.json",
            &snapshots,
            64 * 1024 * 1024,
            &budget,
        )?;
        let reading = learning_db::ReadingStore::validate_recovery_views(
            target.recovery_reading_transaction(),
        )
        .await
        .map_err(|_| BackupError::Invalid("recovery runtime projections or denials failed"))?;
        charge_value(&budget, reading.evidence())?;
        let reading_file = write_record_capped(
            &root,
            "runtime-projections.json",
            reading.evidence(),
            64 * 1024 * 1024,
            &budget,
        )?;
        // Recheck every unchanged table after application-core reads, then the
        // exact six-table committed delta. Together these cover all 64 tables;
        // job.updated_at and lease normalization are not a general exclusion.
        let after_scratch = root.create_dir("post-normalization-row-comparison")?;
        let unchanged = data::verify_unchanged_rows_after_normalization(
            target.connection(),
            &prepared.expected,
            &after_scratch,
        )
        .await?;
        let normalized = jobs::verify_committed_rows(target, report).await?;
        let after_rows = json!({"format_version":1,"capability":"recovery_all_64_tables_after_normalization_v1",
            "unchanged":unchanged.evidence(),"normalized":normalized,"covered_tables":64});
        let after_rows_file =
            write_record(&root, "post-normalization-rows.json", &after_rows, &budget)?;
        target.recheck().await?;
        #[cfg(test)]
        {
            record_barrier_observation(
                json!({"asset_integrity":integrity,"reading":reading.evidence(),"reading_report_sha256":reading.canonical_sha256()}),
            );
            fault(ReceiptFault::DerivedDigestMismatch)?;
        }
        Ok(
            json!({"source_rows_sha256":prepared.rows_sha256,"source_tables":64,
        "intentional_recovery_delta_sha256":report.canonical_sha256()?,
        "post_normalization_all_tables":after_rows,"post_normalization_rows_file_sha256":after_rows_file,
        "asset_integrity":integrity,"snapshot_deliveries":snapshots,
        "snapshot_delivery_file_sha256":snapshot_file,"reading_report_sha256":reading.canonical_sha256(),
        "reading_evidence_file":"runtime-projections.json","reading_evidence_file_sha256":reading_file,
        "reading_execution":"REAL_APPLICATION_CORE_ON_HELD_MANAGEMENT_TRANSACTION_NOT_ORDINARY_RUNTIME_CONNECTION",
        "sql_acl_and_runtime_connect_closed_checked_separately":true,
        "preview_generator":"NOT_IMPLEMENTED_NOT_REBUILT","search_generator":"NOT_IMPLEMENTED_NOT_REBUILT"}),
        )
    })
}

// Result-v1 is a flat object with fixed ASCII keys and scalar values. These are
// exactly learning_core::canonical_json's Object and scalar branches; compare
// the SQL helper byte-for-byte too, rather than assuming JSONB text is equal.
fn result_v1_canonical(value: &Value) -> Result<String, BackupError> {
    let map = value
        .as_object()
        .ok_or(BackupError::Invalid("recovery integrity result object"))?;
    if map.values().any(|v| v.is_array() || v.is_object()) {
        return Err(BackupError::Invalid("recovery integrity result scalar"));
    }
    let sorted: std::collections::BTreeMap<_, _> = map.iter().collect();
    let parts = sorted
        .into_iter()
        .map(|(k, v)| Ok(format!("{}:{}", serde_json::to_string(k)?, v)))
        .collect::<Result<Vec<String>, BackupError>>()?;
    Ok(format!("{{{}}}", parts.join(",")))
}
fn verify_integrity_results<'a>(target: &'a mut ImportedTarget) -> ValidationFuture<'a, Value> {
    Box::pin(async move {
        let rows:Vec<(Value,bool)>=sqlx::query_as(r#"SELECT to_jsonb(r), COALESCE(
      j.status='succeeded' AND j.output_digest=r.output_digest
      AND j.idempotency_key=r.idempotency_key AND e.business_key=r.idempotency_key
      AND e.event_type='asset_integrity_requested' AND e.payload_version=1
      AND e.processor_version=1 AND r.processor_version=1 AND r.result_version=1
      AND e.payload->>'actor_id'=e.actor_id::text AND e.payload->>'space_id'=r.space_id::text
      AND e.payload->>'block_id'=r.block_id::text AND e.payload->>'revision_id'=r.revision_id::text
      AND a.id=r.asset_id AND a.status='ready' AND a.sha256=r.sha256
      AND a.byte_size=r.byte_size AND a.media_type=r.media_type AND b.id=r.revision_id,false)
      FROM public.asset_integrity_result r
      LEFT JOIN public.job j ON j.id=r.job_id
      LEFT JOIN public.job_outbox e ON e.id=j.outbox_id
      LEFT JOIN public.block_asset_use u ON (u.space_id,u.block_id,u.revision_id,u.asset_id)=(r.space_id,r.block_id,r.revision_id,r.asset_id)
      LEFT JOIN public.block_revision b ON (b.space_id,b.block_id,b.id)=(r.space_id,r.block_id,r.revision_id)
      LEFT JOIN public.asset a ON (a.space_id,a.id)=(u.space_id,u.asset_id)
      ORDER BY r.job_id LIMIT 100001"#).fetch_all(target.connection()).await?;
        if rows.len() > crate::MAX_LOGICAL_ASSETS {
            return Err(BackupError::Capacity("recovery integrity results"));
        }
        let budget = target.recovery_metadata_budget();
        charge_value(&budget, &rows)?;
        let mut hashes = Vec::with_capacity(rows.len());
        for (row, linked) in rows {
            if !linked {
                return Err(BackupError::Invalid(
                    "recovery integrity result references differ",
                ));
            }
            let value = json!({"version":1,"kind":"asset_integrity","processor_version":1,
            "space_id":row["space_id"],"block_id":row["block_id"],"revision_id":row["revision_id"],
            "asset_id":row["asset_id"],"sha256":row["sha256"],"byte_size":row["byte_size"],"media_type":row["media_type"]});
            let canonical = result_v1_canonical(&value)?;
            let sql_canonical: String = sqlx::query_scalar("SELECT public.p0c3_canonical_json($1)")
                .bind(&value)
                .fetch_one(target.connection())
                .await?;
            charge_metadata(&budget, sql_canonical.len() as u64)?;
            if sql_canonical != canonical
                || row["output_digest"] != crate::digest(canonical.as_bytes())
            {
                return Err(BackupError::Invalid("recovery derived digest mismatch"));
            }
            hashes.push(json!({"job_id":row["job_id"],"output_digest":row["output_digest"]}));
        }
        let missing:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.job j JOIN public.job_outbox e ON e.id=j.outbox_id LEFT JOIN public.asset_integrity_result r ON r.job_id=j.id WHERE j.status='succeeded' AND e.event_type='asset_integrity_requested' AND r.job_id IS NULL)")
        .fetch_one(target.connection()).await?;
        if missing {
            return Err(BackupError::Invalid("recovery derived result missing"));
        }
        Ok(
            json!({"result_version":1,"count":hashes.len(),"results":hashes,"canonical_algorithm":"LEARNING_CORE_CANONICAL_JSON_FLAT_RESULT_V1_AND_SQL_BYTE_EQUAL","source_digest_and_exact_reference_checks":true}),
        )
    })
}

fn persist_validation_receipt(
    target: &mut ImportedTarget,
    package: &VerifiedPackage,
    report: &jobs::JobRecoveryReport,
    barriers: Value,
    state: &str,
) -> Result<CompletedReceipt, BackupError> {
    let budget = target.recovery_metadata_budget();
    let root = target.recovery_control()?.create_dir("validation")?;
    let value = json!({"format_version":1,"capability":"recovery_validation_v1","state":state,"identity":target.recovery_identity(),"manifest_sha256":package.manifest.canonical_sha256()?,"recovery_delta_sha256":report.canonical_sha256()?,"barriers":barriers,"runtime_activated":false,"complete_backup_manufactured":false});
    let sha256 = write_validation_receipt(&root, &value, &budget)?;
    Ok(CompletedReceipt { sha256 })
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ReceiptFault {
    None,
    BeforeFileSync,
    BeforeDirectorySync,
    DerivedDigestMismatch,
}
#[cfg(test)]
tokio::task_local! { static RECEIPT_FAULT:(ReceiptFault,Arc<std::sync::atomic::AtomicUsize>,Arc<std::sync::Mutex<Option<Value>>>); }
#[cfg(test)]
pub(super) async fn with_receipt_fault<T>(
    fault: ReceiptFault,
    operation: impl std::future::Future<Output = T>,
) -> (T, usize) {
    let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let result = RECEIPT_FAULT
        .scope(
            (
                fault,
                Arc::clone(&hits),
                Arc::new(std::sync::Mutex::new(None)),
            ),
            operation,
        )
        .await;
    (result, hits.load(Ordering::SeqCst))
}
#[cfg(test)]
fn record_barrier_observation(value: Value) {
    let _ = RECEIPT_FAULT.try_with(|(_, _, observation)| {
        *observation.lock().unwrap_or_else(|e| e.into_inner()) = Some(value);
    });
}
#[cfg(test)]
pub(super) fn barrier_observation() -> Option<Value> {
    RECEIPT_FAULT
        .try_with(|(_, _, observation)| {
            observation
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
        })
        .ok()
        .flatten()
}
#[cfg(test)]
fn fault(point: ReceiptFault) -> Result<(), BackupError> {
    if RECEIPT_FAULT
        .try_with(|(selected, hits, _)| {
            if *selected == point {
                hits.fetch_add(1, Ordering::SeqCst);
                true
            } else {
                false
            }
        })
        .unwrap_or(false)
    {
        Err(BackupError::Invalid(
            if point == ReceiptFault::DerivedDigestMismatch {
                "recovery derived digest mismatch"
            } else {
                "recovery validation receipt sync failed"
            },
        ))
    } else {
        Ok(())
    }
}
fn write_validation_receipt(
    root: &BackupDir,
    value: &Value,
    budget: &MetadataBudget,
) -> Result<String, BackupError> {
    let bytes = canonical(value)?;
    if bytes.len() as u64 > RECORD_CAP {
        return Err(BackupError::Capacity("recovery validation receipt"));
    }
    charge_metadata(budget, bytes.len() as u64)?;
    let mut file = root.create_file("receipt.json")?;
    file.write_all(&bytes)?;
    #[cfg(test)]
    fault(ReceiptFault::BeforeFileSync)?;
    file.sync_all()?;
    drop(file);
    #[cfg(test)]
    fault(ReceiptFault::BeforeDirectorySync)?;
    root.sync()?;
    if read_record(root, "receipt.json", RECORD_CAP, budget)? != bytes {
        return Err(BackupError::Invalid("recovery validation receipt readback"));
    }
    Ok(crate::digest(&bytes))
}
#[cfg(test)]
pub(super) async fn prepare_rehearsal_validation(
    target: &mut ImportedTarget,
    package: &VerifiedPackage,
    _assets: &AssetRestoreReport,
    expected: &data::ExpectedRecoveryRows,
) -> Result<PreparedRecoveryValidation, BackupError> {
    if !matches!(&package.authority, PackageAuthority::Fresh(_)) {
        return Err(BackupError::Invalid("fresh-only recovery rehearsal"));
    }
    prepare_validation(target, package, expected).await
}
#[cfg(test)]
pub(super) async fn validate_rehearsal_after_normalization(
    target: &mut ImportedTarget,
    package: &VerifiedPackage,
    _assets: &AssetRestoreReport,
    prepared: PreparedRecoveryValidation,
    report: &jobs::JobRecoveryReport,
    effects: &jobs::ExternalReconciliation,
    _recovery_time: SystemTime,
) -> Result<(), BackupError> {
    let budget = target.recovery_metadata_budget();
    let result=async{
        let barriers=finish_barriers(target,package,prepared,report,effects).await?;
        let root=target.recovery_control()?.create_dir("validation")?;
        let hash=write_validation_receipt(&root,&json!({"format_version":1,"capability":"rehearsal_validation_checks_v1","state":"VALIDATION_CHECKS_PASSED_EXACT_STOP_PENDING_UNUSABLE_NOT_COMPLETE","identity":target.recovery_identity(),"barriers":barriers,"complete_backup":false,"usable":false,"stop_required":true}),&budget,)?;
        target.rehearsal_validation_receipt=Some(hash);
        target.recheck().await?;
        Ok(())
    }.await;
    if result.is_err() {
        target.quarantine().await?;
    }
    // No PASS receipt while a mandatory barrier is pending; this test route
    // cannot construct RecoveryPending, CompleteBackup or RecoveredInstance.
    result
}
#[cfg(test)]
pub(super) fn finish_rehearsal_receipt(
    target: &mut ImportedTarget,
    package: &VerifiedPackage,
) -> Result<(), BackupError> {
    let Some(checks) = target.rehearsal_validation_receipt.take() else {
        return Ok(());
    };
    if !target.quarantined || !matches!(&package.authority, PackageAuthority::Fresh(_)) {
        return Err(BackupError::Invalid(
            "rehearsal validation requires exact stop",
        ));
    }
    let budget = target.recovery_metadata_budget();
    let root = target.recovery_control()?.open_dir("validation")?;
    write_record(
        &root,
        "rehearsal-passed.json",
        &json!({"format_version":1,"state":"FULL_RESTORE_REHEARSAL_PASSED_SINGLE_HOST_QUARANTINED_NOT_COMPLETE","identity":target.recovery_identity(),"validation_checks_sha256":checks,"exact_target_stopped":true,"complete_backup":false,"usable":false}),
        &budget,
    )?;
    Ok(())
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[tokio::test]
    async fn recovery_validation_cancelled_read_phase_never_enters_normalizer() {
        let (cancel, mut receiver) = watch::channel(false);
        let (started, observed) = tokio::sync::oneshot::channel();
        let writes = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&writes);
        let owner = tokio::spawn(async move {
            let end = Instant::now() + Duration::from_secs(10);
            validation_phase(
                &mut receiver,
                end,
                Box::pin(async move {
                    let _ = started.send(());
                    std::future::pending::<Result<(), BackupError>>().await
                }),
            )
            .await?;
            // This is the real phase boundary that protects normalization.
            counted.fetch_add(1, Ordering::SeqCst);
            Ok::<_, BackupError>(())
        });
        observed.await.unwrap();
        cancel.send_replace(true);
        assert!(owner.await.unwrap().is_err());
        assert_eq!(writes.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn recovery_validation_cancel_drops_mutating_future_before_cleanup() {
        struct OperationDrop(Arc<AtomicBool>);
        impl Drop for OperationDrop {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let (cancel, mut receiver) = watch::channel(false);
        let (started, observed) = tokio::sync::oneshot::channel();
        let dropped = Arc::new(AtomicBool::new(false));
        let witness = Arc::clone(&dropped);
        let owner = tokio::spawn(async move {
            let end = Instant::now() + Duration::from_secs(10);
            let result = validation_phase(
                &mut receiver,
                end,
                Box::pin(async move {
                    let _owned = OperationDrop(witness);
                    let _ = started.send(());
                    std::future::pending::<Result<(), BackupError>>().await
                }),
            )
            .await;
            // Production follows the same return with target.quarantine().
            // The phase's transaction Drop fence must already have run.
            assert!(dropped.load(Ordering::SeqCst));
            result
        });
        observed.await.unwrap();
        cancel.send_replace(true);
        assert!(owner.await.unwrap().is_err());
    }

    #[tokio::test]
    async fn recovery_validation_expired_absolute_end_never_polls_phase() {
        let (_cancel, mut receiver) = watch::channel(false);
        let polled = Arc::new(AtomicBool::new(false));
        let witness = Arc::clone(&polled);
        let end = Instant::now() - Duration::from_secs(1);
        let result = validation_phase(
            &mut receiver,
            end,
            Box::pin(async move {
                witness.store(true, Ordering::SeqCst);
                Ok(())
            }),
        )
        .await;
        assert!(result.is_err());
        assert!(!polled.load(Ordering::SeqCst));
    }
}

//! Fresh Attention acceptance; descriptions and hashes grant no authority.
use super::*;
use crate::source::{lifecycle_tests::live::Fixture, snapshot_effects::EnrolledSnapshotBroker};
use crate::{
    destination::snapshot_effects::copy_current_pin_snapshot_effects,
    full_restore::{attention_fixture, data},
};
use attention_fixture::live::{AttentionFixture, SourceJob, require};
use learning_core::*;
use learning_db::snapshot_broker::{BeforeWriteTransition, BrokerSignalResult};
use learning_db::{JobStore, ReadingStore, SnapshotStore};
use serde_json::{Value, json};
use std::{
    io::Write,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::SystemTime,
};
use uuid::Uuid;
#[derive(Clone, Copy, PartialEq, Eq)]
enum Case {
    FullChain,
    MissingAsset,
    CorruptDump,
    InterruptedTransfer,
    UnknownEffect,
    RootReplacement,
}
impl Case {
    fn name(self) -> &'static str {
        match self {
            Self::FullChain => "attention-full-chain",
            Self::MissingAsset => "attention-missing-asset",
            Self::CorruptDump => "attention-corrupt-dump",
            Self::InterruptedTransfer => "attention-interrupted-transfer",
            Self::UnknownEffect => "attention-unknown-effect",
            Self::RootReplacement => "attention-root-replacement",
        }
    }
    fn rejection(self) -> Option<&'static str> {
        match self {
            Self::FullChain => None,
            Self::MissingAsset => Some("MISSING_ASSET"),
            Self::CorruptDump => Some("CORRUPT_DUMP"),
            Self::InterruptedTransfer => Some("INTERRUPTED_TRANSFER"),
            Self::UnknownEffect => Some("UNKNOWN_EFFECT"),
            Self::RootReplacement => Some("ROOT_REPLACEMENT"),
        }
    }
}
struct AuthoritativeInventory {
    rows: data::SourceRecoveryRows,
    evidence: Value,
}
struct Captured {
    captured: crate::source::snapshot_effects::CapturedSnapshotEffectsV2,
    lease: crate::RegistryLease,
}
struct State {
    case: Case,
    seed: Mutex<Option<AttentionFixture>>,
    inventory: Mutex<Option<AuthoritativeInventory>>,
    captured: Mutex<Option<Captured>>,
    backup: Mutex<Option<Uuid>>,
    proof: Mutex<Option<std::path::PathBuf>>,
    pins: Mutex<Option<std::path::PathBuf>>,
    result: Mutex<Option<Value>>,
    negative: Mutex<Option<Value>>,
    secondary_capture: AtomicBool,
    secondary_capture_evidence: Mutex<Option<Value>>,
}
tokio::task_local! {static ATTENTION:State;}
pub(super) fn active() -> bool {
    ATTENTION.try_with(|_| ()).is_ok()
}
fn case() -> Case {
    ATTENTION.with(|s| s.case)
}
fn put(root: &BackupDir, name: &str, value: &Value) -> Result<String, BackupError> {
    let raw = serde_json::to_vec(value)?;
    require(
        !raw.is_empty() && raw.len() <= 16 * 1024 * 1024,
        "attention proof bound",
    )?;
    let mut f = root.create_file(name)?;
    f.write_all(&raw)?;
    f.sync_all()?;
    root.sync()?;
    Ok(crate::digest(&raw))
}
fn proof_root() -> Result<BackupDir, BackupError> {
    let p = ATTENTION
        .with(|s| s.proof.lock().unwrap().clone())
        .ok_or(BackupError::Invalid("attention proof root absent"))?;
    Ok(BackupDir::open_trusted_private_root(&p)?)
}
/// Original admitted source, drained/CONNECT closed, before the real dump.
pub(crate) async fn observe_closed_source(
    conn: &mut PgConnection,
    roles: &[u8],
    config: &crate::SourceBackupConfig,
) -> Result<(), BackupError> {
    if !active() {
        return Ok(());
    }
    if ATTENTION.with(|s| s.secondary_capture.load(Ordering::SeqCst)) {
        require(
            case() == Case::InterruptedTransfer,
            "attention secondary capture scope",
        )?;
        return Ok(());
    }
    require(
        ATTENTION.with(|s| *s.backup.lock().unwrap()) == Some(config.backup_id),
        "attention source capture identity",
    )?;
    let roster = ATTENTION
        .with(|s| s.seed.lock().unwrap().as_ref().map(|s| s.roster.clone()))
        .ok_or(BackupError::Invalid("attention typed roster absent"))?;
    let contract = SchemaContract::for_role_recipe(roles)?;
    let scratch = proof_root()?.create_dir("attention-source-rows")?;
    let mut tx = conn.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    let closed:(String,String,String,bool,bool,i64)=sqlx::query_as("SELECT current_database(),current_user::text,session_user::text,has_database_privilege('learning_runtime',current_database(),'CONNECT'),has_database_privilege('learning_snapshot_claim_broker',current_database(),'CONNECT'),(SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND pid<>pg_backend_pid())").fetch_one(&mut *tx).await?;
    require(
        closed
            == (
                config.expected_database.clone(),
                "learning_admin".into(),
                "learning_admin".into(),
                false,
                false,
                0,
            ),
        "attention source not exclusively closed",
    )?;
    let rows = data::capture_source_rows(&mut tx, &contract, &scratch).await?;
    let actual = read_catalog_profile(&mut tx, crate::restore_policy::RoleProfile::C0D1V1).await?;
    let target_catalog = expected_catalog(roles, &contract)?;
    let expected_source = attention_fixture::closed_source_catalog(&target_catalog)?;
    require(
        actual == expected_source,
        "attention full source function and ACL catalog",
    )?;
    let definitions = function_definitions(&mut tx).await?;
    let reading = ReadingStore::validate_recovery_views(&mut tx)
        .await
        .map_err(|_| BackupError::Invalid("attention source application projections"))?;
    let evidence = json!({"format_version":1,"capability":"attention_authoritative_inventory_v1","backup_id":config.backup_id,"source_view":"CLOSED_DRAINED_SAME_ADMITTED_CONNECTION_REPEATABLE_READ_READ_ONLY_BEFORE_DUMP","all_table_count":64,"rows":rows.evidence(),"catalog":actual,"target_expected_catalog":target_catalog,"function_definitions":definitions,"reading":reading.evidence(),"typed_roster":roster,"catalog_recovery_delta":[{"role":"learning_snapshot_claim_broker","field":"login","source":true,"target":false},{"object":"database","field":"acl","source":"{learning_admin=CTc/learning_admin,learning_runtime=T/learning_admin}","target":"{learning_admin=CTc/learning_admin}"}],"complete_backup":false,"usable":false});
    tx.commit().await?;
    put(&proof_root()?, "full-attention-inventory.json", &evidence)?;
    ATTENTION
        .with(|s| *s.inventory.lock().unwrap() = Some(AuthoritativeInventory { rows, evidence }));
    Ok(())
}
async fn function_definitions(conn: &mut PgConnection) -> Result<Value, BackupError> {
    let mut tx = conn.begin().await?;
    sqlx::query("SELECT pg_catalog.set_config('search_path',$1,true)")
        .bind(catalog_context::PATH)
        .execute(&mut *tx)
        .await?;
    let result:Value=sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('name',p.proname,'args',pg_get_function_identity_arguments(p.oid),'definition',pg_get_functiondef(p.oid)) ORDER BY p.proname,pg_get_function_identity_arguments(p.oid)),'[]'::jsonb) FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public'").fetch_one(&mut *tx).await?;
    tx.rollback().await?;
    require(
        result.as_array().is_some_and(|a| a.len() == 48)
            && serde_json::to_vec(&result)?.len() <= 1024 * 1024,
        "attention complete function definitions",
    )?;
    Ok(result)
}
async fn enqueue(
    f: &Fixture,
    runtime: &PgPool,
    actor: Principal,
    reading: ReadingRef,
) -> Result<Uuid, BackupError> {
    let store = SnapshotStore::new(runtime.clone());
    let request = SnapshotRequest {
        reading,
        mode: ReadingMode::Original,
        include_personal: false,
        include_originals: false,
        resource_versions: vec![],
        source_segments: vec![],
    };
    let key = Uuid::new_v4();
    let id = store
        .enqueue_export(actor, key, request.clone())
        .await
        .map_err(|_| BackupError::Invalid("attention snapshot enqueue"))?;
    require(
        store
            .enqueue_export(actor, key, request)
            .await
            .map_err(|_| BackupError::Invalid("attention enqueue replay"))?
            == id,
        "attention snapshot replay identity",
    )?;
    require(
        JobStore::new(runtime.clone())
            .dispatch_pending(256)
            .await
            .map_err(|_| BackupError::Invalid("attention snapshot dispatch"))?
            == 1,
        "attention exact snapshot dispatch",
    )?;
    let ready:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.job WHERE id=$1 AND status='snapshot_queued' AND attempt_count=0)").bind(id).fetch_one(&f.pool).await?;
    require(ready, "attention dispatched snapshot missing")?;
    Ok(id)
}
pub(super) async fn capture(f: &mut Fixture) -> Result<FreshLocalCapture, BackupError> {
    ATTENTION.with(|s| {
        *s.proof.lock().unwrap() = Some(f.proof_root.clone());
        *s.pins.lock().unwrap() = Some(f.config.local_pin_root.clone());
        *s.backup.lock().unwrap() = Some(f.config.backup_id);
    });
    let runtime = snapshot_effects_tests::pool("learning_runtime").await?;
    let source:(String,i64,String)=sqlx::query_as("SELECT current_database(),oid::bigint,pg_postmaster_start_time()::text FROM pg_database WHERE datname=current_database()").fetch_one(&f.pool).await?;
    let actual:(String,i64,String)=sqlx::query_as("SELECT current_database(),oid::bigint,pg_postmaster_start_time()::text FROM pg_database WHERE datname=current_database()").fetch_one(&runtime).await?;
    require(source == actual, "attention runtime is not admitted source")?;
    let mut seed = Box::pin(attention_fixture::live::seed_attention_source(
        &f.pool,
        &f.assets,
        &f.proof_root,
        &runtime,
    ))
    .await?;
    let committed = enqueue(f, &runtime, seed.actor, seed.historical.clone()).await?;
    runtime.close().await;
    drop(runtime);
    let broker = EnrolledSnapshotBroker::install_fresh(
        snapshot_effects_tests::pool("learning_admin").await?,
        snapshot_effects_tests::pool("learning_snapshot_claim_broker").await?,
        snapshot_effects_tests::pool("learning_runtime").await?,
        &f.assets,
        &f.config,
    )
    .await?;
    require(
        snapshot_effects_tests::signal(f, &broker, None, 30_000).await?
            == BrokerSignalResult::Completed,
        "attention committed snapshot absent",
    )?;
    let runtime = snapshot_effects_tests::pool("learning_runtime").await?;
    let retry = enqueue(f, &runtime, seed.actor, seed.reuse.clone()).await?;
    runtime.close().await;
    drop(runtime);
    require(
        snapshot_effects_tests::signal(
            f,
            &broker,
            Some(BeforeWriteTransition::TransientFailure),
            30_000,
        )
        .await?
            == BrokerSignalResult::RevokedBeforeWrite,
        "attention transient transition absent",
    )?;
    snapshot_effects_tests::wait_eligible(&f.pool, retry).await?;
    require(
        snapshot_effects_tests::signal(f, &broker, Some(BeforeWriteTransition::None), 30_000)
            .await?
            == BrokerSignalResult::RevokedBeforeWrite,
        "attention genuine no-effect retry absent",
    )?;
    let runtime = snapshot_effects_tests::pool("learning_runtime").await?;
    let queued = enqueue(f, &runtime, seed.actor, seed.current.clone()).await?;
    runtime.close().await;
    drop(runtime);
    for id in [committed, retry, queued] {
        seed.jobs.push(SourceJob {
            id,
            snapshot: true,
            tokens: Vec::new(),
        });
    }
    ATTENTION.with(|s| *s.seed.lock().unwrap() = Some(seed));
    let input = broker.quiesce_for_capture(f.config.backup_id).await?;
    f.refresh().await;
    let (pin, captured) =
        crate::prepare_source_backup_with_snapshot_effects(&f.pool, &f.assets, &f.config, input)
            .await?;
    require(
        pin.manifest().logical_asset_count == 6 && pin.manifest().unique_asset_bytes == 601,
        "attention complete original budget",
    )?;
    require(
        captured.ledger.jobs.len() == 2
            && captured
                .ledger
                .jobs
                .iter()
                .map(|j| j.attempts.len())
                .sum::<usize>()
                == 3,
        "attention complete broker history",
    )?;
    ATTENTION.with(|s| {
        let mut seed = s.seed.lock().unwrap();
        for c in &captured.ledger.jobs {
            let j = seed
                .as_mut()
                .unwrap()
                .jobs
                .iter_mut()
                .find(|j| j.id == c.job_id)
                .unwrap();
            j.tokens = c.attempts.iter().map(|a| a.claim_token).collect();
        }
    });
    if case() == Case::InterruptedTransfer {
        let previous = f.config.clone();
        f.next().await;
        ATTENTION.with(|s| s.secondary_capture.store(true, Ordering::SeqCst));
        crate::source::lifecycle_tests::fault("capture_dump_durable", 0);
        let failed = crate::prepare_source_backup(&f.pool, &f.assets, &f.config).await;
        ATTENTION.with(|s| s.secondary_capture.store(false, Ordering::SeqCst));
        require(
            matches!(
                failed,
                Err(BackupError::Invalid("controlled lifecycle test fault"))
            ),
            "attention secondary capture unrelated refusal",
        )?;
        require(
            f.journal().record().phase() == crate::GatePhase::DumpAndIndexDurable,
            "attention secondary capture wrong journal phase",
        )?;
        let closed:bool=sqlx::query_scalar("SELECT NOT has_database_privilege('learning_runtime',current_database(),'CONNECT') AND NOT has_database_privilege('learning_snapshot_claim_broker',current_database(),'CONNECT')").fetch_one(&f.pool).await?;
        require(closed, "attention failed new capture reopened source")?;
        let failed_id = f.config.backup_id;
        require(
            !f.config
                .local_pin_root
                .join(format!("{failed_id}.sealed"))
                .exists(),
            "attention failed new capture published pin",
        )?;
        require(
            crate::verify_sealed(&previous.local_pin_root, previous.backup_id)?.manifest_sha256()
                == pin.sealed().manifest_sha256(),
            "attention old pin damaged by failed capture",
        )?;
        ATTENTION.with(|s|*s.secondary_capture_evidence.lock().unwrap()=Some(json!({"failed_new_capture_preserves_previous_pin":true,"failed_new_backup_id":failed_id,"failed_new_capture_stage":"DUMP_DURABLE_BEFORE_LOCAL_SEAL","previous_pin_verified_after_failed_capture":true,"operation_order":["GOOD_CAPTURE","FAILED_NEW_CAPTURE_DUMP_DURABLE","INTERRUPTED_NEW_DESTINATION_COPY","PREVIOUS_PIN_REAL_RESTORE_AND_EXACT_STOP"]})));
        f.config = previous;
    }
    let registry = crate::ManagementRegistry::open_installed()?;
    let lease = registry.try_lock()?;
    ATTENTION.with(|s| *s.captured.lock().unwrap() = Some(Captured { captured, lease }));
    Ok(FreshLocalCapture {
        pin,
        root: BackupDir::open_trusted_private_root(&f.config.local_pin_root)?,
    })
}
async fn target_inventory(target: &mut ImportedTarget) -> Result<Value, BackupError> {
    let a = target.admitted();
    negative_inventory(
        a.preflight.sql_session.lease_mut(),
        &a.config,
        &a.control,
        &a.assets,
    )
    .await
}
/// The rejected copy is never converted to an authority. The original good
/// pin is subsequently restored, proving a failed new copy did not replace it.
pub(super) async fn before_import(
    target: &mut ImportedTarget,
    package: &VerifiedPackage,
) -> Result<(), BackupError> {
    if !active() || !matches!(case(), Case::MissingAsset | Case::InterruptedTransfer) {
        return Ok(());
    }
    let before = target_inventory(target).await?;
    let pins = ATTENTION
        .with(|s| s.pins.lock().unwrap().clone())
        .ok_or(BackupError::Invalid("attention pins absent"))?;
    let root = proof_root()?;
    root.create_dir("attention-negative-transfer")?;
    let dest = ATTENTION
        .with(|s| s.proof.lock().unwrap().clone())
        .unwrap()
        .join("attention-negative-transfer");
    let id = package.manifest.backup_id;
    let boundary = if case() == Case::InterruptedTransfer {
        require(
            matches!(
                crate::sealed::interrupt_test_transfer(&pins, &dest, id),
                Err(BackupError::Invalid(
                    "controlled destination transfer interruption"
                ))
            ),
            "attention transfer exact interruption absent",
        )?;
        require(
            !dest.join(format!("{id}.sealed")).exists(),
            "attention interrupted transfer published sealed",
        )?;
        "DESTINATION_AFTER_TARGET_CHUNK"
    } else {
        crate::transfer_sealed_backup(&pins, &dest, id)?;
        let first = package
            .plan
            .asset_files()
            .first()
            .ok_or(BackupError::Invalid("attention original roster empty"))?;
        let copy = BackupDir::open_private_root(&dest)?.open_dir(&format!("{id}.sealed"))?;
        let shard = copy
            .open_dir("assets")?
            .open_dir("sha256")?
            .open_dir(&first.sha256[..2])?;
        let retained = root.create_dir("attention-retained-missing-original")?;
        let path = dest.join(format!("{id}.sealed/assets/sha256/{}", &first.sha256[..2]));
        use std::os::unix::fs::PermissionsExt;
        // Only this fresh fault copy is made writable for the explicit injection.
        // The retained source pin and its enrolled roots never change permissions.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
        shard.rename_entry_to_noreplace_without_sync(&first.sha256, &retained, "original")?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o500))?;
        shard.sync()?;
        retained.sync()?;
        require(
            matches!(crate::verify_sealed(&dest,id),Err(BackupError::Io(ref e))if e.kind()==std::io::ErrorKind::NotFound),
            "attention missing original exact open refusal absent",
        )?;
        "DESTINATION_DECLARED_ORIGINAL_OPEN"
    };
    require(
        crate::verify_sealed(&pins, id)?.manifest_sha256()
            == package.manifest.canonical_sha256()?,
        "attention retained pin full reread differs",
    )?;
    let after = target_inventory(target).await?;
    require(
        before == after,
        "attention failed copy wrote target before import",
    )?;
    let before_sha = put(&root, "full-attention-negative-before.json", &before)?;
    let after_sha = put(&root, "full-attention-negative-after.json", &after)?;
    ATTENTION.with(|s|*s.negative.lock().unwrap()=Some(json!({"boundary":boundary,"injected":true,"refused":true,"before_sha256":before_sha,"after_sha256":after_sha,"unchanged":true,"bad_input_import_started":false,"audit_timing":"GOOD_SOURCE_PACKAGE_ADMITTED_READ_ONLY_BEFORE_ANY_IMPORT","previous_pin_verified":true,"previous_pin_restore_verified":false,"failed_new_capture_preserves_previous_pin":case()==Case::InterruptedTransfer})));
    ATTENTION.with(|s| {
        if let Some(extra) = s.secondary_capture_evidence.lock().unwrap().as_ref() {
            let mut negative = s.negative.lock().unwrap();
            let fields = negative.as_mut().unwrap().as_object_mut().unwrap();
            for (key, value) in extra.as_object().unwrap() {
                fields.insert(key.clone(), value.clone());
            }
        }
    });
    Ok(())
}
async fn reject_tokens(
    target: &mut ImportedTarget,
    jobs: &[SourceJob],
) -> Result<usize, BackupError> {
    let mut count = 0;
    for job in jobs {
        for token in &job.tokens {
            let sql = if job.snapshot {
                [
                    "SELECT public.p0c3_renew_snapshot_job($1,$2,30000)",
                    "SELECT public.p0c3_checkpoint_snapshot_job($1,$2,$3)",
                    "SELECT public.p0c3_succeed_snapshot_job($1,$2,$3)",
                    "SELECT public.p0c3_fail_snapshot_job($1,$2,'transient_storage',true)",
                ]
            } else {
                [
                    "SELECT public.p0c2_renew_job($1,$2,30000)",
                    "SELECT public.p0c2_checkpoint_job($1,$2,$3)",
                    "SELECT public.p0c2_succeed_job($1,$2,$3)",
                    "SELECT public.p0c2_fail_job($1,$2,'transient_storage',true)",
                ]
            };
            for (i, statement) in sql.into_iter().enumerate() {
                let q = sqlx::query_scalar::<_, bool>(statement)
                    .bind(job.id)
                    .bind(token);
                let changed = match i {
                    1 => {
                        q.bind(json!({"forbidden_old_attention_worker":true}))
                            .fetch_one(target.connection())
                            .await?
                    }
                    2 => {
                        q.bind("a".repeat(64))
                            .fetch_one(target.connection())
                            .await?
                    }
                    _ => q.fetch_one(target.connection()).await?,
                };
                require(!changed, "attention old token changed restored job")?;
                count += 1;
            }
        }
    }
    Ok(count)
}
pub(super) async fn after_import(
    target: &mut ImportedTarget,
    package: &VerifiedPackage,
    assets: &AssetRestoreReport,
    expected: &data::ExpectedRecoveryRows,
) -> Result<(), BackupError> {
    if !active() {
        return Ok(());
    }
    let inventory = ATTENTION
        .with(|s| s.inventory.lock().unwrap().take())
        .ok_or(BackupError::Invalid("attention closed inventory absent"))?;
    inventory.rows.require_dump(expected)?;
    let seed = ATTENTION
        .with(|s| s.seed.lock().unwrap().take())
        .ok_or(BackupError::Invalid("attention source fixture absent"))?;
    require(seed.jobs.len() == 10, "attention supported job roster")?;
    let prepared =
        validation::prepare_rehearsal_validation(target, package, assets, expected).await?;
    require(
        read_catalog_profile(
            target.connection(),
            crate::restore_policy::RoleProfile::C0D1V1,
        )
        .await?
            == inventory.evidence["target_expected_catalog"],
        "attention full target ACL differs",
    )?;
    require(
        function_definitions(target.connection()).await?
            == inventory.evidence["function_definitions"],
        "attention full function definitions differ",
    )?;
    let Captured { captured, lease } = ATTENTION
        .with(|s| s.captured.lock().unwrap().take())
        .ok_or(BackupError::Invalid(
            "attention captured capabilities absent",
        ))?;
    let effects = if case() == Case::UnknownEffect {
        drop(captured);
        crate::full_restore::jobs::inspect_external_effects_without_export_authority(target).await?
    } else {
        let binding = package.current_pin_copy_binding(&captured)?;
        let copied = copy_current_pin_snapshot_effects(target, binding, &lease, captured)?;
        crate::full_restore::jobs::inspect_current_pin_external_effects(target, &lease, copied)
            .await?
    };
    let report =
        crate::full_restore::jobs::normalize_restored_jobs(target, &effects, SystemTime::now())
            .await?;
    let rejected = reject_tokens(target, &seed.jobs).await?;
    require(rejected == 40, "attention complete token fencing roster")?;
    let normalization_sha = put(
        &proof_root()?,
        "full-attention-normalization.json",
        report.evidence(),
    )?;
    if case() == Case::UnknownEffect {
        require(
            report.has_holds() && report.held_jobs() == 3,
            "attention UNKNOWN holds incomplete",
        )?;
        let ids = seed
            .jobs
            .iter()
            .filter(|j| j.snapshot)
            .map(|j| j.id)
            .collect::<Vec<_>>();
        let runnable:i64=sqlx::query_scalar("SELECT count(*) FROM public.job WHERE id=ANY($1) AND (status='snapshot_queued' OR (status='snapshot_retry_wait' AND next_attempt_at<=clock_timestamp()) OR (status='snapshot_running' AND lease_expires_at<=clock_timestamp()))").bind(&ids).fetch_one(target.connection()).await?;
        require(runnable == 0, "attention UNKNOWN snapshot runnable")?;
    }
    let validated = validation::validate_rehearsal_after_normalization(
        target,
        package,
        assets,
        prepared,
        &report,
        &effects,
        SystemTime::now(),
    )
    .await;
    let (reading, delivery, post) = if case() == Case::UnknownEffect {
        require(
            matches!(
                validated,
                Err(BackupError::Invalid("recovery external effects unresolved"))
            ),
            "attention UNKNOWN bypassed validation",
        )?;
        ATTENTION.with(|s|*s.negative.lock().unwrap()=Some(json!({"boundary":"POST_IMPORT_EXTERNAL_EFFECT_VALIDATION","injected":true,"refused":true,"unchanged":false,"bad_input_import_started":true,"snapshot_holds":3,"snapshot_runnable":0,"previous_pin_verified":true,"previous_pin_restore_verified":false})));
        (Value::Null, Value::Null, Value::Null)
    } else {
        validated?;
        require(
            !report.has_holds()
                && report.held_jobs() == 0
                && report.evidence()["positive_snapshot_findings"] == 3,
            "attention positive effects incomplete",
        )?;
        let control = target.recovery_control()?;
        let raw = preflight::read_limited(
            &control.open_dir("validation")?,
            "receipt.json",
            16 * 1024 * 1024,
        )?;
        let receipt: Value = serde_json::from_slice(&raw)?;
        let reading_raw =
            preflight::read_limited(&control, "runtime-projections.json", 64 * 1024 * 1024)?;
        let reading: Value = serde_json::from_slice(&reading_raw)?;
        require(
            reading == inventory.evidence["reading"],
            "attention historical current personal or denial changed",
        )?;
        let delivery = receipt["barriers"]["snapshot_deliveries"].clone();
        require(
            delivery["count"] == 1
                && delivery["source_access"] == false
                && delivery["runtime_reconnected"] == false
                && delivery["republished"] == false,
            "attention held management delivery absent",
        )?;
        require(
            delivery["deliveries"][0]["files"]
                .as_array()
                .is_some_and(|a| a.len() == 4),
            "attention ReadingCopy four-file roster",
        )?;
        let post_raw =
            preflight::read_limited(&control, "post-normalization-rows.json", 1024 * 1024)?;
        let post: Value = serde_json::from_slice(&post_raw)?;
        require(
            post["covered_tables"] == 64
                && post["unchanged"]["unchanged_tables"] == 63
                && post["normalized"]["all_columns_compared"] == true,
            "attention final all-table barrier absent",
        )?;
        ATTENTION.with(|s| {
            if let Some(n) = s.negative.lock().unwrap().as_mut() {
                n["previous_pin_restore_verified"] = json!(true);
            }
        });
        (reading, delivery, post)
    };
    let inventory_raw = preflight::read_limited(
        &proof_root()?,
        "full-attention-inventory.json",
        16 * 1024 * 1024,
    )?;
    let target_id = crate::full_restore::profile::FullRehearsalProfile::read_installed()?
        .target_container_id()
        .to_owned();
    let result = json!({"format_version":1,"classification":if case()==Case::FullChain{"FULL_RESTORE_REHEARSAL_PASSED_SINGLE_HOST_QUARANTINED_NOT_COMPLETE"}else{"EXPECTED_REJECTION_PASSED_SINGLE_HOST_QUARANTINED_NOT_COMPLETE"},"case":case().name(),"backup_id":package.manifest.backup_id,"manifest_sha256":package.manifest.canonical_sha256()?,"target_database":target.admitted().config.expected_database,"target_container_id":target_id,"target_stopped":false,"source_inventory_sha256":crate::digest(&inventory_raw),"normalization_report_sha256":normalization_sha,"pre_normalization_all_64":true,"covered_tables":64,"logical_assets":6,"distinct_originals":5,"unique_original_bytes":601,"old_token_rejections":rejected,"jobs":10,"snapshot_jobs":3,"snapshot_claims":3,"snapshot_holds":report.held_jobs(),"reading_comparison":reading,"typed_semantics":inventory.evidence["typed_roster"],"post_normalization":post,"snapshot_delivery":delivery,"expected_rejection":case().rejection(),"negative":ATTENTION.with(|s|s.negative.lock().unwrap().clone()),"complete_backup":false,"usable":false,"task7_complete":false,"stop_required":true});
    ATTENTION.with(|s| *s.result.lock().unwrap() = Some(result));
    Ok(())
}
fn base_negative(full: &Value, inventory_sha: &str) -> Value {
    json!({"format_version":1,"classification":"EXPECTED_REJECTION_PASSED_SINGLE_HOST_QUARANTINED_NOT_COMPLETE","case":case().name(),"backup_id":full["backup_id"],"manifest_sha256":full["manifest_sha256"],"target_database":full["target_database"],"target_container_id":full["target_container_id"],"target_stopped":true,"target_import_started":false,"source_inventory_sha256":inventory_sha,"normalization_report_sha256":null,"pre_normalization_all_64":false,"covered_tables":0,"logical_assets":6,"distinct_originals":5,"unique_original_bytes":601,"expected_rejection":case().rejection(),"negative":null,"reading_comparison":null,"typed_semantics":null,"snapshot_delivery":null,"complete_backup":false,"usable":false,"task7_complete":false,"stop_required":true})
}
pub(super) async fn finish(_f: &mut Fixture) -> Result<(), BackupError> {
    if !active() {
        return Ok(());
    }
    let root = proof_root()?;
    let raw = preflight::read_limited(&root, "full-import-result.json", 1024 * 1024)?;
    let full: Value = serde_json::from_slice(&raw)?;
    require(
        full["target_stopped"] == true,
        "attention exact target stop missing",
    )?;
    let inventory_raw =
        preflight::read_limited(&root, "full-attention-inventory.json", 16 * 1024 * 1024)?;
    let mut result = if case() == Case::CorruptDump {
        require(
            full["case"] == "full-bad-input"
                && full["commit_attempted"] == false
                && full["negative_evidence"]["unchanged"] == true,
            "attention precise corrupt-dump audit missing",
        )?;
        let mut result = base_negative(&full, &crate::digest(&inventory_raw));
        let audit =
            preflight::read_limited(&root, "full-negative-inventory.json", 64 * 1024 * 1024)?;
        let audit: Value = serde_json::from_slice(&audit)?;
        require(
            audit["injected"] == "dump_freeze"
                && audit["refused"] == "dump_freeze"
                && audit["before"] == audit["after"]
                && audit["audit_failed"] == false,
            "attention corrupt dump unrelated refusal",
        )?;
        result["negative"] = json!({"boundary":"DUMP_FREEZE_CUSTOM_HEADER","injected":true,"refused":true,"unchanged":true,"bad_input_import_started":false,"before_sha256":crate::digest(&serde_json::to_vec(&audit["before"])?),"after_sha256":crate::digest(&serde_json::to_vec(&audit["after"])?),"original_negative_inventory_sha256":full["negative_evidence"]["sha256"],"previous_pin_verified":true,"previous_pin_restore_verified":false});
        result
    } else {
        ATTENTION
            .with(|s| s.result.lock().unwrap().take())
            .ok_or(BackupError::Invalid("attention full proof missing"))?
    };
    result["target_stopped"] = json!(true);
    result["target_import_started"] = json!(case() != Case::CorruptDump);
    result["original_full_import_proof_sha256"] = json!(crate::digest(&raw));
    put(&root, "full-attention-result.json", &result)?;
    Ok(())
}
async fn root_replacement() -> Result<(), BackupError> {
    let mut f = Fixture::attention().await;
    let source = Box::pin(capture(&mut f)).await?;
    let package = source.package()?;
    let diagnostic = Diagnostic::new();
    let target = FreshTargetFixture::configured(&diagnostic)?;
    let mut imported = target.admit(&package).await?;
    let before = target_inventory(&mut imported).await?;
    let path = imported.admitted().config.control_root.clone();
    let parent = path
        .parent()
        .ok_or(BackupError::Invalid("attention control parent"))?;
    let leaf = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or(BackupError::Invalid("attention control leaf"))?;
    let parent = BackupDir::open_trusted_private_root(parent)?;
    let old = imported.admitted().control.identity()?;
    parent.rename_noreplace(leaf, "attention-retained-control")?;
    let replacement = parent.create_dir(leaf)?;
    let new = replacement.identity()?;
    require(old != new, "attention replacement did not change inode")?;
    let refused = imported.recheck().await;
    require(
        matches!(
            refused,
            Err(BackupError::Invalid(
                "guarded full restore rejected; unusable"
            ))
        ),
        "attention held root replacement not precisely rejected",
    )?;
    let after = target_inventory(&mut imported).await?;
    require(
        before == after,
        "attention root replacement changed pre-import inventory",
    )?;
    let root = proof_root()?;
    let b = put(&root, "full-attention-negative-before.json", &before)?;
    let a = put(&root, "full-attention-negative-after.json", &after)?;
    imported.stop().await?;
    let profile = crate::full_restore::profile::FullRehearsalProfile::read_installed()?;
    let id = profile.target_container_id();
    let actual = binding::linux::inspect("container", id)?;
    require(
        actual["Id"] == id && actual["State"]["Running"] == false && actual["State"]["Pid"] == 0,
        "attention root replacement target stop unconfirmed",
    )?;
    let inv = preflight::read_limited(&root, "full-attention-inventory.json", 16 * 1024 * 1024)?;
    let full = json!({"backup_id":package.manifest.backup_id,"manifest_sha256":package.manifest.canonical_sha256()?,"target_database":imported.admitted().config.expected_database,"target_container_id":id});
    let mut result = base_negative(&full, &crate::digest(&inv));
    result["negative"] = json!({"boundary":"HELD_TARGET_CONTROL_CURRENT_NAME_IDENTITY","injected":true,"refused":true,"unchanged":true,"bad_input_import_started":false,"before_sha256":b,"after_sha256":a,"held_identity":old,"replacement_identity":new,"previous_pin_verified":true,"previous_pin_restore_verified":false});
    put(&root, "full-attention-result.json", &result)?;
    f.pool.close().await;
    Ok(())
}
async fn run(case: Case) {
    let state = State {
        case,
        seed: Mutex::new(None),
        inventory: Mutex::new(None),
        captured: Mutex::new(None),
        backup: Mutex::new(None),
        proof: Mutex::new(None),
        pins: Mutex::new(None),
        result: Mutex::new(None),
        negative: Mutex::new(None),
        secondary_capture: AtomicBool::new(false),
        secondary_capture_evidence: Mutex::new(None),
    };
    ATTENTION
        .scope(state, async {
            if case == Case::RootReplacement {
                root_replacement().await.unwrap();
            } else {
                let fault = if case == Case::CorruptDump {
                    FullFault::BadInput
                } else {
                    FullFault::Success
                };
                Box::pin(real_full_import_rehearsal(fault)).await.unwrap();
            }
        })
        .await;
}
macro_rules! native_case {
    ($name:ident,$case:ident) => {
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        #[ignore = "fresh Root full Attention, real original APIs and broker, isolated exact-stop"]
        async fn $name() {
            run(Case::$case).await;
        }
    };
}
native_case!(
    attention_full_chain_preserves_authoritative_inventory_and_originals,
    FullChain
);
native_case!(attention_missing_asset, MissingAsset);
native_case!(attention_corrupt_dump, CorruptDump);
native_case!(attention_interrupted_transfer, InterruptedTransfer);
native_case!(attention_unknown_effect, UnknownEffect);
native_case!(attention_root_replacement, RootReplacement);

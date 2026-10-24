//! Fresh native tests. Only normal Root APIs create the cut and copied adjunct.
//! These current-Pin rehearsals cannot construct CompleteBackup/RecoveredInstance.
use super::*;
use crate::destination::snapshot_effects::copy_current_pin_snapshot_effects;
use crate::source::{lifecycle_tests::live::Fixture, snapshot_effects::EnrolledSnapshotBroker};
use learning_core::*;
use learning_db::snapshot_broker::{BeforeWriteTransition, BrokerSignalResult};
use learning_db::{CompositionStore, JobStore, ReadingStore, SnapshotStore, VersionedContentStore};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{io::Write, os::unix::fs::MetadataExt, path::Path, sync::Mutex, time::SystemTime};
use uuid::Uuid;

const LEASE_MS: u64 = 30_000;
const REQUEST_SECONDS: u64 = 90;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Case {
    P1,
    P2,
    P3Live,
    P3Expired,
    P4,
    P5One,
    P5Two,
    P5Terminal,
    P6,
    Queued,
}
impl Case {
    fn name(self) -> &'static str {
        match self {
            Self::P1 => "snapshot-p1",
            Self::P2 => "snapshot-p2",
            Self::P3Live => "snapshot-p3-live",
            Self::P3Expired => "snapshot-p3-expired",
            Self::P4 => "snapshot-p4",
            Self::P5One => "snapshot-p5-one",
            Self::P5Two => "snapshot-p5-two",
            Self::P5Terminal => "snapshot-p5-terminal",
            Self::P6 => "snapshot-p6",
            Self::Queued => "snapshot-queued-zero",
        }
    }
    fn attempts(self) -> i32 {
        match self {
            Self::Queued => 0,
            Self::P2 | Self::P5Two => 2,
            Self::P3Live | Self::P3Expired | Self::P4 => 3,
            _ => 1,
        }
    }
}
struct SourceState {
    job: Uuid,
    tokens: Vec<Uuid>,
    before: Value,
    receipt: Vec<Value>,
    claims: usize,
}
struct CapturedState {
    captured: crate::source::snapshot_effects::CapturedSnapshotEffectsV2,
    lease: crate::RegistryLease,
    source: SourceState,
}
struct State {
    case: Case,
    captured: Mutex<Option<CapturedState>>,
}
tokio::task_local! { static SNAPSHOT: State; }
pub(super) fn active() -> bool {
    SNAPSHOT.try_with(|_| ()).is_ok()
}
fn check(value: bool, message: &'static str) -> Result<(), BackupError> {
    if value {
        Ok(())
    } else {
        Err(BackupError::Invalid(message))
    }
}
fn private(path: &Path) -> Result<Vec<u8>, BackupError> {
    let m = std::fs::symlink_metadata(path)?;
    check(
        m.is_file()
            && !m.file_type().is_symlink()
            && m.uid() == 0
            && m.mode() & 0o7777 == 0o600
            && m.nlink() == 1
            && m.len() > 0
            && m.len() <= 4096,
        "snapshot private fixture file",
    )?;
    Ok(std::fs::read(path)?)
}
pub(super) async fn pool(role: &'static str) -> Result<PgPool, BackupError> {
    let key = match role {
        "learning_admin" => "ADMIN_DSN_FILE",
        "learning_runtime" => "RUNTIME_DSN_FILE",
        "learning_snapshot_claim_broker" => "BROKER_DSN_FILE",
        _ => return Err(BackupError::Invalid("snapshot fixed role")),
    };
    let value = std::env::var(format!("TEST_C4_LIFECYCLE_{key}"))
        .map_err(|_| BackupError::Invalid("snapshot DSN fixture absent"))?;
    let bytes = private(Path::new(&value))?;
    let dsn =
        std::str::from_utf8(&bytes).map_err(|_| BackupError::Invalid("snapshot DSN encoding"))?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .min_connections(0)
        .connect(dsn.trim())
        .await?;
    let users: (String, String) = sqlx::query_as("SELECT current_user::text,session_user::text")
        .fetch_one(&pool)
        .await?;
    check(
        users == (role.into(), role.into()),
        "snapshot actual source role differs",
    )?;
    Ok(pool)
}
fn put(root: &BackupDir, name: &str, bytes: &[u8]) -> Result<(), BackupError> {
    check(
        !bytes.is_empty() && bytes.len() <= 1024 * 1024,
        "snapshot fixture evidence bound",
    )?;
    let mut f = root.create_file(name)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    root.sync()?;
    Ok(())
}

#[derive(Serialize)]
struct SignalRequest<'a> {
    format_version: u8,
    operation: &'static str,
    backup_id: Uuid,
    database: &'a str,
    compose_project: &'a str,
    request_id: Uuid,
    epoch_id: Uuid,
    binding_id: Uuid,
    work_socket_path: &'a Path,
    lease_ms: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignalAck {
    format_version: u8,
    operation: String,
    backup_id: Uuid,
    request_id: Uuid,
    epoch_id: Uuid,
    binding_id: Uuid,
    disposition: String,
    client_exit: i32,
}
// Root starts a real, separately measured UID/GID65532 worker with a single
// epoch leaf RO mount. This request carries no actor, job, token or credential.
pub(super) async fn signal(
    f: &Fixture,
    broker: &EnrolledSnapshotBroker,
    transition: Option<BeforeWriteTransition>,
    lease_ms: u64,
) -> Result<BrokerSignalResult, BackupError> {
    let deadline = Instant::now() + Duration::from_secs(REQUEST_SECONDS);
    let request_id = Uuid::new_v4();
    let root = BackupDir::open_trusted_private_root(&f.proof_root)?;
    let request = SignalRequest {
        format_version: 2,
        operation: "snapshot_broker_signal",
        backup_id: f.config.backup_id,
        database: &f.config.expected_database,
        compose_project: &f.config.expected_compose_project,
        request_id,
        epoch_id: broker.epoch_id(),
        binding_id: broker.binding_id(),
        work_socket_path: broker.work_socket_path(),
        lease_ms,
    };
    put(
        &root,
        &format!("snapshot-request-{request_id}.json"),
        &serde_json::to_vec(&request)?,
    )?;
    let result = match transition {
        Some(t) => broker.accept_next_signal_for_capture(t).await?,
        None => broker.accept_next_signal().await?,
    };
    let disposition = match result {
        BrokerSignalResult::NoWork => "NO_WORK",
        BrokerSignalResult::Completed => "COMPLETED",
        BrokerSignalResult::RevokedBeforeWrite => "REVOKED_BEFORE_WRITE",
        BrokerSignalResult::NotAcquired => "NOT_ACQUIRED",
    };
    let ack = f
        .proof_root
        .join(format!("snapshot-ready-{request_id}.json"));
    loop {
        check(
            Instant::now() < deadline,
            "snapshot driver acknowledgment deadline",
        )?;
        if ack.try_exists()? {
            let ack: SignalAck = serde_json::from_slice(&private(&ack)?)?;
            check(
                ack.format_version == 2
                    && ack.operation == "snapshot_broker_signal"
                    && ack.backup_id == f.config.backup_id
                    && ack.request_id == request_id
                    && ack.epoch_id == broker.epoch_id()
                    && ack.binding_id == broker.binding_id()
                    && ack.disposition == disposition
                    && ack.client_exit == 0,
                "snapshot exact worker acknowledgment",
            )?;
            check(
                Instant::now() < deadline,
                "snapshot driver acknowledgment deadline",
            )?;
            return Ok(result);
        }
        check(
            Instant::now() < deadline,
            "snapshot driver acknowledgment deadline",
        )?;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
async fn job(pool: &PgPool, id: Uuid) -> Result<Value, BackupError> {
    Ok(
        sqlx::query_scalar("SELECT to_jsonb(j) FROM public.job j WHERE id=$1")
            .bind(id)
            .fetch_one(pool)
            .await?,
    )
}
pub(super) async fn wait_eligible(pool: &PgPool, id: Uuid) -> Result<(), BackupError> {
    let end = Instant::now() + Duration::from_secs(40);
    loop {
        let due:bool=sqlx::query_scalar("SELECT (status='snapshot_running' AND lease_expires_at<=clock_timestamp()) OR (status='snapshot_retry_wait' AND next_attempt_at<=clock_timestamp()) FROM public.job WHERE id=$1").bind(id).fetch_one(pool).await?;
        check(
            Instant::now() < end,
            "snapshot real database clock deadline",
        )?;
        if due {
            return Ok(());
        }
        check(
            Instant::now() < end,
            "snapshot real database clock deadline",
        )?;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
async fn enqueue(f: &Fixture, runtime: &PgPool) -> Result<Uuid, BackupError> {
    let actor = Principal {
        actor_id: Uuid::new_v4(),
    };
    let space = Uuid::new_v4();
    let peer = Uuid::new_v4();
    for user in [actor.actor_id, peer] {
        sqlx::query("INSERT INTO public.app_user(id) VALUES($1)")
            .bind(user)
            .execute(&f.pool)
            .await?;
    }
    sqlx::query("INSERT INTO public.space(id,owner_id) VALUES($1,$2)")
        .bind(space)
        .bind(actor.actor_id)
        .execute(&f.pool)
        .await?;
    for user in [actor.actor_id, peer] {
        sqlx::query(
            "INSERT INTO public.space_grant(actor_id,space_id,can_write) VALUES($1,$2,true)",
        )
        .bind(user)
        .bind(space)
        .execute(&f.pool)
        .await?;
    }
    let revision = VersionedContentStore::new(runtime.clone())
        .create(
            actor,
            space,
            CreateContent {
                request_id: Uuid::new_v4(),
                draft: ContentDraft::V1(TextDraft {
                    kind: BlockKind::Text,
                    intent: Intent::Note,
                    language: "en".into(),
                    title: "Text-only recovery".into(),
                    payload: TextPayload {
                        format: TextFormat::Markdown,
                        text: "Durable reading text without original assets".into(),
                    },
                }),
                reason: "fresh native snapshot fixture".into(),
            },
        )
        .await
        .map_err(|_| BackupError::Invalid("snapshot runtime content API"))?;
    let document = CompositionStore::new(runtime.clone())
        .save(
            actor,
            space,
            SaveComposition {
                request_id: Uuid::new_v4(),
                composition_id: None,
                base_revision_id: None,
                kind: CompositionKind::Document,
                title: "Text-only document".into(),
                nodes: vec![NodeDraft {
                    occurrence_id: None,
                    target: NodeTarget::Block(BlockRef {
                        block_id: revision.block_id,
                        revision_id: revision.revision_id,
                    }),
                }],
                reason: "fresh native snapshot fixture".into(),
            },
        )
        .await
        .map_err(|_| BackupError::Invalid("snapshot runtime composition API"))?;
    let readings = ReadingStore::new(runtime.clone());
    let reading = readings
        .create(
            actor,
            space,
            CreateReading {
                request_id: Uuid::new_v4(),
                base: document.reference,
                title: "Text-only reading".into(),
                reason: "fresh native snapshot fixture".into(),
            },
        )
        .await
        .map_err(|_| BackupError::Invalid("snapshot runtime reading API"))?;
    let request = SnapshotRequest {
        reading: reading.view,
        mode: ReadingMode::Original,
        include_personal: false,
        include_originals: false,
        resource_versions: vec![],
        source_segments: vec![],
    };
    let store = SnapshotStore::new(runtime.clone());
    let key = Uuid::new_v4();
    let id = store
        .enqueue_export(actor, key, request.clone())
        .await
        .map_err(|_| BackupError::Invalid("snapshot runtime enqueue API"))?;
    check(
        store
            .enqueue_export(actor, key, request)
            .await
            .map_err(|_| BackupError::Invalid("snapshot runtime enqueue replay"))?
            == id,
        "snapshot request key replay changed job",
    )?;
    check(
        JobStore::new(runtime.clone())
            .dispatch_pending(256)
            .await
            .map_err(|_| BackupError::Invalid("snapshot runtime dispatch API"))?
            == 1,
        "snapshot exact runtime dispatch",
    )?;
    let counts:(i64,i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM public.asset),(SELECT count(*) FROM public.snapshot_export_request),(SELECT count(*) FROM public.job),(SELECT count(*) FROM public.reading_view_revision)").fetch_one(&f.pool).await?;
    check(
        counts == (0, 1, 1, 1),
        "snapshot nonempty text-only fixture shape",
    )?;
    Ok(id)
}

pub(super) async fn capture(f: &mut Fixture) -> Result<FreshLocalCapture, BackupError> {
    let case = SNAPSHOT.with(|s| s.case);
    #[cfg(test)]
    eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=CAPTURE_RUNTIME_POOL_BEFORE");
    let runtime = pool("learning_runtime").await?;
    #[cfg(test)]
    eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=CAPTURE_RUNTIME_POOL_AFTER");
    #[cfg(test)]
    eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=CAPTURE_ENQUEUE_BEFORE");
    let id = enqueue(f, &runtime).await?;
    #[cfg(test)]
    eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=CAPTURE_ENQUEUE_AFTER");
    runtime.close().await;
    #[cfg(test)]
    eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=CAPTURE_INSTALL_FRESH_BEFORE");
    let broker = EnrolledSnapshotBroker::install_fresh(
        pool("learning_admin").await?,
        pool("learning_snapshot_claim_broker").await?,
        pool("learning_runtime").await?,
        &f.assets,
        &f.config,
    )
    .await?;
    #[cfg(test)]
    eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=CAPTURE_INSTALL_FRESH_AFTER");
    for attempt in 1..=case.attempts() {
        if attempt > 1 {
            wait_eligible(&f.pool, id).await?;
        }
        let transition = match case {
            Case::P6 => None,
            Case::P5One | Case::P5Two => Some(BeforeWriteTransition::TransientFailure),
            Case::P5Terminal => Some(BeforeWriteTransition::TerminalFailure),
            _ => Some(BeforeWriteTransition::None),
        };
        let lease =
            if attempt == case.attempts() && matches!(case, Case::P1 | Case::P2 | Case::P3Live) {
                180_000
            } else {
                LEASE_MS
            };
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=CAPTURE_SIGNAL_BEFORE");
        let result = signal(f, &broker, transition, lease).await?;
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_CAPTURE_CHECKPOINT step=CAPTURE_SIGNAL_AFTER");
        check(
            result
                == if case == Case::P6 {
                    BrokerSignalResult::Completed
                } else {
                    BrokerSignalResult::RevokedBeforeWrite
                },
            "snapshot actual original operation disposition",
        )?;
        check(
            job(&f.pool, id).await?["attempt_count"] == attempt,
            "snapshot real attempt count",
        )?;
    }
    if matches!(case, Case::P3Expired | Case::P4) {
        wait_eligible(&f.pool, id).await?;
    }
    if case == Case::P4 {
        check(
            signal(f, &broker, Some(BeforeWriteTransition::None), LEASE_MS).await?
                == BrokerSignalResult::NotAcquired,
            "snapshot ceiling must return actual None",
        )?;
    }
    let before = job(&f.pool, id).await?;
    if case == Case::P3Live {
        let live: bool = sqlx::query_scalar(
            "SELECT lease_expires_at>clock_timestamp() FROM public.job WHERE id=$1",
        )
        .bind(id)
        .fetch_one(&f.pool)
        .await?;
        check(live, "snapshot third source lease must remain live")?;
    }
    let receipt: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(r) FROM public.snapshot_export_result r ORDER BY job_id",
    )
    .fetch_all(&f.pool)
    .await?;
    check(
        receipt.len() == usize::from(case == Case::P6),
        "snapshot genuine receipt roster",
    )?;
    let input = broker.quiesce_for_capture(f.config.backup_id).await?;
    // Re-observe after all worker clients naturally exited; no worker/lifecycle
    // statement is treated as the independent Root isolation attestation.
    f.refresh().await;
    let (pin, captured) =
        crate::prepare_source_backup_with_snapshot_effects(&f.pool, &f.assets, &f.config, input)
            .await?;
    if matches!(case, Case::P1 | Case::P2 | Case::P3Live) {
        let live: bool = sqlx::query_scalar(
            "SELECT lease_expires_at>clock_timestamp() FROM public.job WHERE id=$1",
        )
        .bind(id)
        .fetch_one(&f.pool)
        .await?;
        check(live, "snapshot capture of unexpired variant became late")?;
    }
    check(
        pin.manifest().logical_asset_count == 0 && pin.manifest().unique_asset_bytes == 0,
        "snapshot text-only source original budget",
    )?;
    let tokens = if case == Case::Queued {
        check(
            captured.ledger.jobs.is_empty(),
            "queued zero unexpectedly claimed",
        )?;
        vec![]
    } else {
        check(
            captured.ledger.jobs.len() == 1 && captured.ledger.jobs[0].job_id == id,
            "snapshot complete ledger roster",
        )?;
        let attempts = &captured.ledger.jobs[0].attempts;
        check(
            attempts.len() == case.attempts() as usize
                && attempts
                    .iter()
                    .enumerate()
                    .all(|(i, a)| a.attempt == i as i32 + 1),
            "snapshot full attempt history",
        )?;
        if case != Case::P6 {
            check(
                attempts.iter().all(|a| {
                    a.revoked_before_write.is_some()
                        && a.write_issued.is_none()
                        && a.business_commit.is_none()
                        && a.write_closed.is_none()
                }),
                "no-effect history contains write authority",
            )?;
        } else {
            check(
                attempts[0].write_issued.is_some()
                    && attempts[0].business_commit.is_some()
                    && attempts[0].write_closed.is_some(),
                "committed history lacks durable closure",
            )?;
        }
        attempts.iter().map(|a| a.claim_token).collect()
    };
    let registry = crate::ManagementRegistry::open_installed()?;
    let lease = registry.try_lock()?;
    let base_root = BackupDir::open_trusted_private_root(&f.config.local_pin_root)?;
    let source = SourceState {
        job: id,
        tokens,
        before,
        receipt,
        claims: case.attempts() as usize,
    };
    SNAPSHOT.with(|s| {
        *s.captured.lock().unwrap() = Some(CapturedState {
            captured,
            lease,
            source,
        })
    });
    Ok(FreshLocalCapture {
        pin,
        root: base_root,
    })
}

async fn reject_tokens(
    target: &mut ImportedTarget,
    source: &SourceState,
) -> Result<usize, BackupError> {
    let mut count = 0;
    for token in &source.tokens {
        for (sql, kind) in [
            ("SELECT public.p0c3_renew_snapshot_job($1,$2,300000)", 0),
            ("SELECT public.p0c3_checkpoint_snapshot_job($1,$2,$3)", 1),
            ("SELECT public.p0c3_succeed_snapshot_job($1,$2,$3)", 2),
            (
                "SELECT public.p0c3_fail_snapshot_job($1,$2,'transient_storage',true)",
                0,
            ),
        ] {
            let query = sqlx::query_scalar::<_, bool>(sql)
                .bind(source.job)
                .bind(token);
            let changed = match kind {
                1 => {
                    query
                        .bind(json!({"forbidden_old_worker":true}))
                        .fetch_one(target.connection())
                        .await?
                }
                2 => {
                    query
                        .bind("a".repeat(64))
                        .fetch_one(target.connection())
                        .await?
                }
                _ => query.fetch_one(target.connection()).await?,
            };
            check(
                !changed,
                "snapshot old original token changed recovered job",
            )?;
            count += 1;
        }
    }
    Ok(count)
}
pub(super) async fn after_import(
    target: &mut ImportedTarget,
    package: &VerifiedPackage,
    assets: &AssetRestoreReport,
    expected: &crate::full_restore::data::ExpectedRecoveryRows,
) -> Result<(), BackupError> {
    if !active() {
        return Ok(());
    }
    let case = SNAPSHOT.with(|s| s.case);
    let CapturedState {
        captured,
        lease,
        source,
    } = SNAPSHOT
        .with(|s| s.captured.lock().unwrap().take())
        .ok_or(BackupError::Invalid(
            "snapshot original captured capability absent",
        ))?;
    let prepared =
        validation::prepare_rehearsal_validation(target, package, assets, expected).await?;
    sqlx::query("SELECT pg_catalog.set_config('TimeZone','UTC',true)")
        .execute(target.connection())
        .await?;
    let before: Value = sqlx::query_scalar("SELECT to_jsonb(j) FROM public.job j WHERE id=$1")
        .bind(source.job)
        .fetch_one(target.connection())
        .await?;
    let receipts: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(r) FROM public.snapshot_export_result r ORDER BY job_id",
    )
    .fetch_all(target.connection())
    .await?;
    for field in [
        "id",
        "outbox_id",
        "idempotency_key",
        "attempt_count",
        "checkpoint",
        "output_digest",
    ] {
        check(
            before[field] == source.before[field],
            "snapshot imported protected source field differs",
        )?;
    }
    check(
        receipts.len() == source.receipt.len(),
        "snapshot original receipt count differs",
    )?;
    target.recheck().await?;
    let binding = package.current_pin_copy_binding(&captured)?;
    let copied = copy_current_pin_snapshot_effects(target, binding, &lease, captured)?;
    target.recheck().await?;
    let effects =
        crate::full_restore::jobs::inspect_current_pin_external_effects(target, &lease, copied)
            .await?;
    let report =
        crate::full_restore::jobs::normalize_restored_jobs(target, &effects, SystemTime::now())
            .await?;
    check(
        !report.has_holds()
            && report.held_jobs() == 0
            && report.evidence()["positive_snapshot_findings"] == 1,
        "snapshot opaque positive finding missing",
    )?;
    let after: Value = sqlx::query_scalar("SELECT to_jsonb(j) FROM public.job j WHERE id=$1")
        .bind(source.job)
        .fetch_one(target.connection())
        .await?;
    check(
        after["attempt_count"] == case.attempts()
            && after["lease_token"].is_null()
            && after["lease_expires_at"].is_null(),
        "snapshot retry count or lease differs",
    )?;
    let changed = matches!(case, Case::P1 | Case::P2 | Case::P3Live | Case::P3Expired);
    if changed {
        check(
            after["status"]
                == if case.attempts() == 3 {
                    "failed"
                } else {
                    "snapshot_retry_wait"
                },
            "snapshot recovered retry family/ceiling",
        )?;
    } else {
        check(
            after == before,
            "snapshot terminal/queued/waiting row changed",
        )?;
    }
    let old_token_rejections = reject_tokens(target, &source).await?;
    check(
        old_token_rejections == 4 * source.claims,
        "snapshot complete old-token probe coverage",
    )?;
    let unchanged: Value = sqlx::query_scalar("SELECT to_jsonb(j) FROM public.job j WHERE id=$1")
        .bind(source.job)
        .fetch_one(target.connection())
        .await?;
    check(
        unchanged == after,
        "snapshot old-token probes changed normalized row",
    )?;
    let unchanged_receipts: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(r) FROM public.snapshot_export_result r ORDER BY job_id",
    )
    .fetch_all(target.connection())
    .await?;
    check(
        unchanged_receipts == receipts,
        "snapshot immutable receipt rewritten",
    )?;
    validation::validate_rehearsal_after_normalization(
        target,
        package,
        assets,
        prepared,
        &report,
        &effects,
        SystemTime::now(),
    )
    .await?;
    // Read the actual barrier result. Delivery is deliberately executed once:
    // its private third-copy destination refuses replay/residue.
    let receipt_root = target.recovery_control()?.open_dir("validation")?;
    let validation_raw = preflight::read_limited(&receipt_root, "receipt.json", 16 * 1024 * 1024)?;
    let validation: Value = serde_json::from_slice(&validation_raw)?;
    let delivery = validation["barriers"]["snapshot_deliveries"].clone();
    check(
        delivery["execution"]
            == "REAL_SNAPSHOT_DELIVERY_CORE_ON_HELD_MANAGEMENT_TRANSACTION_RECOVERY_CONTROL_COPY"
            && delivery["recovery_copy_receipt_sha256"]
                .as_str()
                .is_some_and(crate::valid_digest),
        "snapshot actual recovery-control copy receipt absent",
    )?;
    check(
        delivery["count"] == usize::from(case == Case::P6),
        "snapshot committed delivery coverage",
    )?;
    if case == Case::P6 {
        let files = delivery["deliveries"][0]["files"]
            .as_array()
            .ok_or(BackupError::Invalid("snapshot delivered files absent"))?;
        check(
            files.len() == 4
                && files.iter().all(|r| {
                    r["bytes"].as_u64().is_some_and(|n| n > 0)
                        && r["sha256"].as_str().is_some_and(crate::valid_digest)
                }),
            "snapshot text-only package must contain four verified nonempty files",
        )?;
        check(
            delivery["source_access"] == false
                && delivery["runtime_reconnected"] == false
                && delivery["republished"] == false,
            "snapshot recovery re-entered source publication",
        )?;
    }
    let recovery = target.recovery_control()?;
    let post_raw = preflight::read_limited(&recovery, "post-normalization-rows.json", 1024 * 1024)?;
    let post: Value = serde_json::from_slice(&post_raw)?;
    check(
        post["capability"] == "recovery_all_64_tables_after_normalization_v1"
            && post["covered_tables"] == 64
            && post["unchanged"]["unchanged_tables"] == 63
            && post["unchanged"]["tables"]
                .as_array()
                .is_some_and(|rows| rows.len() == 63)
            && post["unchanged"]["excluded_columns"] == json!([])
            && post["normalized"]["all_columns_compared"] == true
            && post["normalized"]["tables"]
                == json!([
                    "asset_integrity_result",
                    "job",
                    "job_outbox",
                    "request_key",
                    "snapshot_export_request",
                    "snapshot_export_result"
                ])
            && post["normalized"]["normalization_report_sha256"] == report.canonical_sha256()?,
        "snapshot final 64-table row barrier missing",
    )?;
    let root = BackupDir::open_trusted_private_root(Path::new("/var/lib/knowweave-source/proofs"))?;
    let report_bytes = serde_json::to_vec(report.evidence())?;
    let target_id = crate::full_restore::profile::FullRehearsalProfile::read_installed()?
        .target_container_id()
        .to_owned();
    let evidence = json!({"format_version":2,"classification":"FULL_SNAPSHOT_EFFECT_NATIVE_CASE_PASSED_EXACT_STOP_PENDING_NOT_COMPLETE","case":case.name(),"backup_id":package.manifest.backup_id,"manifest_sha256":package.manifest.canonical_sha256()?,"target_database":target.admitted().config.expected_database,"target_container_id":target_id,"job_id":source.job,"attempts":case.attempts(),"old_token_rejections":old_token_rejections,"positive_snapshot_findings":1,"snapshot_holds":0,"normalization_report_sha256":report.canonical_sha256()?,"normalization_report_bytes":report_bytes.len(),"normalization_commit_known":true,"post_normalization_covered_tables":64,"post_normalization_rows_sha256":crate::digest(&post_raw),"logical_assets":0,"unique_original_bytes":0,"snapshot_delivery":delivery,"source_effect_handles_consumed_by_copy":true,"complete_backup":false,"usable":false,"task6_complete":false,"stop_required":true});
    put(
        &root,
        "full-snapshot-effect-result.json",
        &serde_json::to_vec(&evidence)?,
    )?;
    put(
        &root,
        "full-snapshot-effect-normalization.json",
        &report_bytes,
    )?;
    Ok(())
}
async fn run(case: Case) {
    SNAPSHOT
        .scope(
            State {
                case,
                captured: Mutex::new(None),
            },
            real_full_import_rehearsal(FullFault::Success),
        )
        .await
        .unwrap();
}
macro_rules! native_case {
    ($name:ident,$case:ident)=>{
        #[tokio::test(flavor="multi_thread",worker_threads=2)]
        #[ignore="fresh Root full-C profile, real UID65532 leaf-RO worker, independent destination copy"]
        async fn $name(){run(Case::$case).await;}
    }
}
native_case!(recovery_snapshot_p1_revoke_unexpired, P1);
native_case!(recovery_snapshot_p2_revoke_full_history, P2);
native_case!(recovery_snapshot_p3_ceiling_unexpired, P3Live);
native_case!(recovery_snapshot_p3_ceiling_expired, P3Expired);
native_case!(recovery_snapshot_p4_real_none_after_third_expiry, P4);
native_case!(recovery_snapshot_p5_transient_one, P5One);
native_case!(recovery_snapshot_p5_transient_two, P5Two);
native_case!(recovery_snapshot_p5_terminal_unchanged, P5Terminal);
native_case!(recovery_snapshot_p6_committed_copy_held_delivery, P6);
native_case!(recovery_snapshot_queued_zero_no_effect, Queued);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "fresh Root full-C profile; genuine direct-claim history gap must not produce an opaque capture"]
async fn recovery_snapshot_direct_claim_history_gap_never_positive() {
    let diagnostic = Diagnostic::new();
    diagnostic.public_result(history_gap().await).unwrap();
}
async fn history_gap() -> Result<(), BackupError> {
    let mut f = Fixture::text_only().await;
    let runtime = pool("learning_runtime").await?;
    let id = enqueue(&f, &runtime).await?;
    runtime.close().await;
    let broker = EnrolledSnapshotBroker::install_fresh(
        pool("learning_admin").await?,
        pool("learning_snapshot_claim_broker").await?,
        pool("learning_runtime").await?,
        &f.assets,
        &f.config,
    )
    .await?;
    check(
        signal(&f, &broker, Some(BeforeWriteTransition::None), LEASE_MS).await?
            == BrokerSignalResult::RevokedBeforeWrite,
        "history gap requires original revoked first claim",
    )?;
    let first = job(&f.pool, id).await?;
    check(
        first["attempt_count"] == 1 && first["status"] == "snapshot_running",
        "history gap first actual claim missing",
    )?;
    wait_eligible(&f.pool, id).await?;
    // A real legacy producer bypasses the original epoch. The new claim and
    // token are returned by canonical SQL, never written or invented here.
    let runtime = pool("learning_runtime").await?;
    let foreign = JobStore::new(runtime.clone())
        .claim_snapshot(id, Duration::from_millis(LEASE_MS))
        .await
        .map_err(|_| BackupError::Invalid("history gap canonical direct claim failed"))?
        .ok_or(BackupError::Invalid(
            "history gap actual second claim absent",
        ))?;
    check(
        foreign.attempt_count == 2 && first["lease_token"] != foreign.token.to_string(),
        "history gap did not obtain an actual new second token",
    )?;
    runtime.close().await;
    // SQLx 0.8.6 may requeue an in-flight return after close's idle scan.
    // Release the legacy producer's final pool handle before capture so
    // that late idle connection cannot stay alive through this fixture.
    drop(runtime);
    let input = broker.quiesce_for_capture(f.config.backup_id).await?;
    f.refresh().await;
    let result =
        crate::prepare_source_backup_with_snapshot_effects(&f.pool, &f.assets, &f.config, input)
            .await;
    check(
        matches!(
            result,
            Err(BackupError::Invalid("snapshot effects v2 contract"))
        ),
        "history gap unexpectedly returned an opaque capture or another failure",
    )?;
    // Admit the already Root-pinned clean target, without a manifest or import,
    // then use its original exact quarantine path. No target SQL is supplied.
    let target = FreshTargetFixture::configured(&Diagnostic::new())?;
    let options = sqlx::postgres::PgConnectOptions::new()
        .socket("/var/run/knowweave-target")
        .port(5432)
        .username("learning_admin")
        .database(&target.config.expected_database);
    let admin = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .min_connections(0)
        .connect_with(options)
        .await?;
    let mut admitted = linux::admit_full_candidate_target(target.config, admin, None)
        .await
        .map_err(error)?;
    admitted.quarantine().await;
    let profile = crate::full_restore::profile::FullRehearsalProfile::read_installed()?;
    let proof = json!({"format_version":2,"classification":"FULL_SNAPSHOT_HISTORY_GAP_REFUSED_NO_CAPTURE_NOT_COMPLETE",
        "case":"snapshot-history-gap","backup_id":f.config.backup_id,"job_id":id,"original_attempts":1,
        "actual_attempts":2,"original_claim_revoked":true,"new_token_observed":true,
        "failure_code":"SNAPSHOT_EFFECTS_V2_CONTRACT","opaque_capture_returned":false,
        "destination_copy_entered":false,"target_import_entered":false,"normalization_entered":false,
        "target_admitted_clean":true,"target_container_id":profile.target_container_id(),
        "stop_required":true,"usable":false,"task6_complete":false});
    put(
        &BackupDir::open_trusted_private_root(&f.proof_root)?,
        "full-snapshot-history-gap-result.json",
        &serde_json::to_vec(&proof)?,
    )?;
    Ok(())
}

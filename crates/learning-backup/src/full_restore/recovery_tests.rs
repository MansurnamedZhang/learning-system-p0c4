//! Fixed fresh recovery foundations; no registered export authority exists.
use super::*;
use learning_core::{
    Affinity, AssetRef, BlockKind, BlockRef, BodyV3, CompositionKind, ContentDraft, ContentV3,
    CreateContent, CreateReading, EditReading, GapAnchor, InsertTarget, Intent, JobInput,
    NodeDraft, NodeTarget, Principal, ReadingEdit, ReadingMode, SaveComposition, TextDraft,
    TextFormat, TextPayload,
};
use learning_db::{
    AssetIntegrityProcessor, AssetProcessOutcome, AssetStore, CompositionStore, JobStore,
    ReadingStore, VersionedContentStore,
};
use serde_json::{Value, json};
use std::{
    io::Write,
    path::Path,
    sync::Mutex,
    time::{Duration, SystemTime},
};
use uuid::Uuid;

#[derive(Clone, Copy)]
pub(crate) enum RecoveryCase {
    Leases,
    RetryLimit,
    Effects,
    Derived,
    ReceiptFailure,
}
impl RecoveryCase {
    fn name(self) -> &'static str {
        match self {
            Self::Leases => "recovery-leases",
            Self::RetryLimit => "recovery-retry-limit",
            Self::Effects => "recovery-effects",
            Self::Derived => "recovery-derived",
            Self::ReceiptFailure => "recovery-receipt-failure",
        }
    }
    fn coverage(self) -> (usize, usize, usize) {
        match self {
            Self::Leases => (4, 16, 2),
            Self::RetryLimit => (17, 48, 6),
            Self::Effects => (7, 8, 7),
            Self::Derived | Self::ReceiptFailure => (2, 4, 0),
        }
    }
}
#[derive(Clone)]
struct SourceJob {
    id: Uuid,
    snapshot: bool,
    token: Option<Uuid>,
}
struct RecoveryState {
    case: RecoveryCase,
    jobs: Mutex<Vec<SourceJob>>,
}
tokio::task_local! { static RECOVERY: RecoveryState; }
pub(crate) async fn run(case: RecoveryCase) -> Result<(), BackupError> {
    RECOVERY
        .scope(
            RecoveryState {
                case,
                jobs: Mutex::new(Vec::new()),
            },
            real_full_import_rehearsal(FullFault::Success),
        )
        .await
}
pub(crate) async fn leases() -> Result<(), BackupError> {
    run(RecoveryCase::Leases).await
}
fn current_case() -> Option<RecoveryCase> {
    RECOVERY.try_with(|state| state.case).ok()
}
fn require(value: bool, message: &'static str) -> Result<(), BackupError> {
    if value {
        Ok(())
    } else {
        Err(BackupError::Invalid(message))
    }
}
#[derive(Clone, Copy)]
enum InitialState {
    Running(i32, bool),
    Queued,
    Waiting,
    Succeeded,
    Failed,
    Cancelled,
}

async fn seed_job(
    pool: &PgPool,
    actor: Uuid,
    space: Uuid,
    snapshot: bool,
    state: InitialState,
) -> Result<SourceJob, BackupError> {
    let id = Uuid::new_v4();
    let block = Uuid::new_v4();
    let revision = Uuid::new_v4();
    let (event, payload, key) = if snapshot {
        let request = json!({"reading":{"view_id":block,"revision_id":revision},"mode":"original","include_personal":false,"include_originals":false,"resource_versions":[],"source_segments":[]});
        let canonical: String = sqlx::query_scalar("SELECT public.p0c3_canonical_json($1)")
            .bind(&request)
            .fetch_one(pool)
            .await?;
        (
            "snapshot_export_requested",
            json!({"version":1,"kind":"snapshot_export","actor_id":actor,"space_id":space,"request":request}),
            format!(
                "snapshot-export:v1:{actor}:{space}:{}",
                hex::encode(Sha256::digest(canonical.as_bytes()))
            ),
        )
    } else {
        (
            "asset_integrity_requested",
            json!({"version":1,"kind":"asset_integrity","actor_id":actor,"space_id":space,"block_id":block,"revision_id":revision}),
            format!("asset-integrity:v1:{space}:{block}:{revision}"),
        )
    };
    sqlx::query("INSERT INTO public.job_outbox(id,business_key,event_type,payload_version,payload,actor_id,processor_version) VALUES($1,$2,$3,1,$4,$5,1)").bind(id).bind(&key).bind(event).bind(payload).bind(actor).execute(pool).await?;
    sqlx::query(
        "INSERT INTO public.job(id,outbox_id,idempotency_key,status) VALUES($1,$1,$2,'queued')",
    )
    .bind(id)
    .bind(key)
    .execute(pool)
    .await?;
    sqlx::query("UPDATE public.job_outbox SET dispatched_at=clock_timestamp() WHERE id=$1")
        .bind(id)
        .execute(pool)
        .await?;
    if matches!(state, InitialState::Queued | InitialState::Cancelled) {
        if matches!(state, InitialState::Cancelled) {
            let sql = if snapshot {
                "SELECT public.p0c3_cancel_snapshot_job($1)"
            } else {
                "SELECT public.p0c2_cancel_job($1)"
            };
            require(
                sqlx::query_scalar(sql).bind(id).fetch_one(pool).await?,
                "recovery fixture cancellation",
            )?;
        }
        return Ok(SourceJob {
            id,
            snapshot,
            token: None,
        });
    }
    let attempts = match state {
        InitialState::Running(n, _) => n,
        _ => 1,
    };
    let mut token = Uuid::nil();
    for attempt in 1..=attempts {
        let sql = if snapshot {
            "SELECT token FROM public.p0c3_claim_snapshot_job($1,300000)"
        } else {
            "SELECT token FROM public.p0c2_claim_job($1,300000)"
        };
        token = sqlx::query_scalar(sql).bind(id).fetch_one(pool).await?;
        if attempt < attempts {
            // The next real claim spends an attempt; never refund or directly
            // overwrite attempt_count to manufacture a ceiling fixture.
            sqlx::query("UPDATE public.job SET lease_expires_at=clock_timestamp()-interval '1 hour' WHERE id=$1").bind(id).execute(pool).await?;
        }
    }
    sqlx::query("UPDATE public.job SET checkpoint=$2 WHERE id=$1")
        .bind(id)
        .bind(json!({"recovery_fixture":true}))
        .execute(pool)
        .await?;
    match state {
        InitialState::Running(_, expired) => {
            sqlx::query("UPDATE public.job SET lease_expires_at=CASE WHEN $2 THEN clock_timestamp()-interval '1 hour' ELSE clock_timestamp()+interval '1 hour' END WHERE id=$1").bind(id).bind(expired).execute(pool).await?;
            Ok(SourceJob {
                id,
                snapshot,
                token: Some(token),
            })
        }
        InitialState::Waiting | InitialState::Failed => {
            let sql = if snapshot {
                "SELECT public.p0c3_fail_snapshot_job($1,$2,$3,$4)"
            } else {
                "SELECT public.p0c2_fail_job($1,$2,$3,$4)"
            };
            let retry = matches!(state, InitialState::Waiting);
            require(
                sqlx::query_scalar(sql)
                    .bind(id)
                    .bind(token)
                    .bind(if retry {
                        "transient_storage"
                    } else {
                        "invalid_input"
                    })
                    .bind(retry)
                    .fetch_one(pool)
                    .await?,
                "recovery fixture failure transition",
            )?;
            Ok(SourceJob {
                id,
                snapshot,
                token: None,
            })
        }
        InitialState::Succeeded => {
            require(snapshot, "only snapshot DB receipt fixture is supported")?;
            // This is a real immutable DB receipt without export bytes or
            // registry authority. It MUST remain Unknown, never AlreadyCommitted.
            let manifest = hex::encode(Sha256::digest(b"unbound recovery fixture receipt"));
            let plan = hex::encode(Sha256::digest(b"unbound recovery fixture plan"));
            require(
                sqlx::query_scalar(
                    "SELECT public.p0c3_complete_snapshot_job($1,$2,'reading_copy_v1',$3,$4,$5)",
                )
                .bind(id)
                .bind(token)
                .bind(manifest)
                .bind(plan)
                .bind(vec![space])
                .fetch_one(pool)
                .await?,
                "recovery fixture immutable snapshot receipt",
            )?;
            Ok(SourceJob {
                id,
                snapshot,
                token: None,
            })
        }
        InitialState::Queued | InitialState::Cancelled => unreachable!(),
    }
}

// These are real fresh-source application writes and a genuine byte-verifying
// C2 processor, never an admin-created digest or a positive snapshot finding.
async fn seed_derived(
    pool: &PgPool,
    files: &learning_assets::FsAssetStore,
    actor_id: Uuid,
    space: Uuid,
) -> Result<SourceJob, BackupError> {
    let peer_id = Uuid::new_v4();
    sqlx::query("INSERT INTO public.app_user(id) VALUES($1)")
        .bind(peer_id)
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO public.space_grant(actor_id,space_id,can_write) VALUES($1,$2,true)")
        .bind(peer_id)
        .bind(space)
        .execute(pool)
        .await?;
    let asset_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM public.asset WHERE space_id=$1 AND status='ready' ORDER BY id LIMIT 1",
    )
    .bind(space)
    .fetch_one(pool)
    .await?;
    // Before source capture/drain only. No recovery target connection or
    // replacement for its held management guard is constructed here.
    let runtime = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_with(
            pool.connect_options()
                .as_ref()
                .clone()
                .username("learning_runtime"),
        )
        .await?;
    let result = async {
        let users: (String, String) = sqlx::query_as("SELECT current_user::text,session_user::text")
            .fetch_one(&runtime).await?;
        require(users == ("learning_runtime".into(), "learning_runtime".into()),
            "recovery source fixture requires actual runtime role")?;
        let actor = Principal { actor_id };
        let saved = VersionedContentStore::new(runtime.clone()).create(actor, space, CreateContent {
            request_id: Uuid::new_v4(),
            draft: ContentDraft::V3(ContentV3 {
                intent: Intent::Note, language: "en".into(), title: "Recovery original".into(),
                body: BodyV3::Attachment { asset: AssetRef { space_id: space, asset_id },
                    display_name: "Verified recovery original".into() },
                basis_refs: vec![], requires_context: vec![], source_run: None,
            }), reason: "fresh recovery fixture".into(),
        }).await.map_err(|_| BackupError::Invalid("recovery source content API failed"))?;
        let block = BlockRef { block_id: saved.block_id, revision_id: saved.revision_id };
        let input = JobInput::asset_integrity(actor_id, space, block.clone())
            .map_err(|_| BackupError::Invalid("recovery source integrity input failed"))?;
        let jobs = JobStore::new(runtime.clone());
        require(jobs.dispatch_pending(256).await
            .map_err(|_| BackupError::Invalid("recovery source dispatch failed"))? == 1,
            "recovery source exact integrity dispatch")?;
        let job_id: Uuid = sqlx::query_scalar("SELECT id FROM public.job WHERE idempotency_key=$1")
            .bind(input.business_key()).fetch_one(pool).await?;
        let lease = jobs.claim(job_id, Duration::from_secs(30)).await
            .map_err(|_| BackupError::Invalid("recovery source integrity claim failed"))?
            .ok_or(BackupError::Invalid("recovery source integrity lease absent"))?;
        let processor = AssetIntegrityProcessor::new(AssetStore::new(runtime.clone(), files.clone()), jobs);
        require(processor.process(&lease).await
            .map_err(|_| BackupError::Invalid("recovery source byte verification failed"))?
            == AssetProcessOutcome::Succeeded, "recovery source result-v1 not completed")?;
        let document = CompositionStore::new(runtime.clone()).save(actor, space, SaveComposition {
            request_id: Uuid::new_v4(), composition_id: None, base_revision_id: None,
            kind: CompositionKind::Document, title: "Recovery document".into(),
            nodes: vec![NodeDraft { occurrence_id: None, target: NodeTarget::Block(block) }],
            reason: "fresh recovery document".into(),
        }).await.map_err(|_| BackupError::Invalid("recovery source composition API failed"))?;
        let readings = ReadingStore::new(runtime.clone());
        let first = readings.create(actor, space, CreateReading {
            request_id: Uuid::new_v4(), base: document.reference.clone(),
            title: "Recovery reading".into(), reason: "fresh recovery reading".into(),
        }).await.map_err(|_| BackupError::Invalid("recovery source reading API failed"))?;
        let second = readings.edit(actor, first.overlay.overlay_id, EditReading {
            request_id: Uuid::new_v4(), expected_overlay_revision: first.overlay.revision_id,
            expected_reading_view_revision: first.view.revision_id,
            edit: ReadingEdit::InsertNew {
                drafts: vec![TextDraft { kind: BlockKind::Text, intent: Intent::Note,
                    language: "en".into(), title: "Recovery personal note".into(),
                    payload: TextPayload { format: TextFormat::Markdown,
                        text: "A real personal reading layer".into() } }],
                target: InsertTarget::NewGroup { anchor: GapAnchor {
                    base: document.reference.clone(), parent_occurrence_path: vec![],
                    left_occurrence_id: Some(document.nodes[0].occurrence_id),
                    right_occurrence_id: None, affinity: Affinity::AfterLeft,
                } },
            }, reason: "create historical and current views".into(),
        }).await.map_err(|_| BackupError::Invalid("recovery source reading edit API failed"))?;
        require(first.view.view_id == second.view.view_id && first.view.revision_id != second.view.revision_id,
            "recovery reading fixture lacks two revisions")?;
        for reference in [first.view, second.view] {
            for mode in [ReadingMode::Original, ReadingMode::Personal, ReadingMode::Fused] {
                require(readings.read_versioned(actor, reference.clone(), mode).await
                    .map_err(|_| BackupError::Invalid("recovery source owner projection failed"))?.is_some(),
                    "recovery source owner projection absent")?;
                require(readings.read_versioned(Principal { actor_id: peer_id }, reference.clone(), mode).await
                    .map_err(|_| BackupError::Invalid("recovery source granted peer denial failed"))?.is_none(),
                    "recovery source private reading leaked to granted peer")?;
            }
        }
        // The shared projection bridge is a management-only observation API.
        // An ordinary runtime transaction must fail its actual role gate even
        // though normal owner projections above genuinely work on this pool.
        let mut runtime_probe = runtime.begin().await?;
        let management_read = ReadingStore::validate_recovery_views(&mut runtime_probe).await;
        runtime_probe.rollback().await?;
        require(matches!(management_read,
            Err(learning_core::ContentError::Invalid(message))
                if message == "recovery_reading_management_role"),
            "ordinary runtime admitted to recovery management reader")?;
        let counts: (i64,i64,i64) = sqlx::query_as("SELECT (SELECT count(*) FROM public.asset_integrity_result),(SELECT count(*) FROM public.block_asset_use),(SELECT count(*) FROM public.reading_view_revision)")
            .fetch_one(pool).await?;
        require(counts == (1,1,2), "recovery derived source fixture is empty or unexpected")?;
        Ok(SourceJob { id: job_id, snapshot: false, token: None })
    }.await;
    runtime.close().await;
    result
}

pub(super) async fn seed(
    pool: &PgPool,
    files: &learning_assets::FsAssetStore,
) -> Result<(), BackupError> {
    let Some(case) = current_case() else {
        return Ok(());
    };
    let existing: i64 = sqlx::query_scalar("SELECT count(*) FROM public.job")
        .fetch_one(pool)
        .await?;
    require(
        existing == 0,
        "recovery fixture requires no historical jobs",
    )?;
    let (actor, space): (Uuid, Uuid) =
        sqlx::query_as("SELECT owner_id,id FROM public.space ORDER BY id LIMIT 1")
            .fetch_one(pool)
            .await?;
    // The source full-import fixture creates an owner, not a runtime grant.
    // The genuine snapshot completion function must observe its normal grant
    // rather than disabling that check to manufacture an immutable receipt.
    sqlx::query("INSERT INTO public.space_grant(actor_id,space_id,can_write) VALUES($1,$2,true) ON CONFLICT(actor_id,space_id) DO NOTHING")
        .bind(actor).bind(space).execute(pool).await?;
    let mut jobs = Vec::new();
    match case {
        RecoveryCase::Leases | RecoveryCase::RetryLimit => {
            let maximum = if matches!(case, RecoveryCase::RetryLimit) {
                3
            } else {
                1
            };
            for snapshot in [false, true] {
                for attempt in 1..=maximum {
                    for expired in [false, true] {
                        jobs.push(
                            seed_job(
                                pool,
                                actor,
                                space,
                                snapshot,
                                InitialState::Running(attempt, expired),
                            )
                            .await?,
                        );
                    }
                }
            }
            if matches!(case, RecoveryCase::RetryLimit) {
                for state in [
                    InitialState::Queued,
                    InitialState::Waiting,
                    InitialState::Failed,
                    InitialState::Cancelled,
                    InitialState::Queued,
                ] {
                    jobs.push(seed_job(pool, actor, space, false, state).await?);
                }
            }
        }
        RecoveryCase::Effects => {
            for state in [
                InitialState::Queued,
                InitialState::Running(1, false),
                InitialState::Running(1, true),
                InitialState::Waiting,
                InitialState::Succeeded,
                InitialState::Failed,
                InitialState::Cancelled,
            ] {
                jobs.push(seed_job(pool, actor, space, true, state).await?);
            }
        }
        RecoveryCase::Derived | RecoveryCase::ReceiptFailure => {
            jobs.push(seed_derived(pool, files, actor, space).await?);
            jobs.push(seed_job(pool, actor, space, false, InitialState::Running(1, false)).await?);
        }
    }
    require(jobs.len() == case.coverage().0, "recovery fixture coverage")?;
    RECOVERY.with(|state| *state.jobs.lock().unwrap() = jobs);
    Ok(())
}
async fn jobs(target: &mut ImportedTarget) -> Result<Vec<Value>, BackupError> {
    Ok(
        sqlx::query_scalar("SELECT to_jsonb(j) FROM public.job AS j ORDER BY j.id")
            .fetch_all(target.connection())
            .await?,
    )
}
async fn receipts(target: &mut ImportedTarget) -> Result<Vec<Value>, BackupError> {
    Ok(sqlx::query_scalar(
        "SELECT to_jsonb(r) FROM public.snapshot_export_result AS r ORDER BY r.job_id",
    )
    .fetch_all(target.connection())
    .await?)
}
async fn reject_old_token(
    target: &mut ImportedTarget,
    job: &SourceJob,
) -> Result<usize, BackupError> {
    let Some(token) = job.token else {
        return Ok(0);
    };
    let sql = if job.snapshot {
        [
            "SELECT public.p0c3_renew_snapshot_job($1,$2,300000)",
            "SELECT public.p0c3_checkpoint_snapshot_job($1,$2,$3)",
            "SELECT public.p0c3_succeed_snapshot_job($1,$2,$3)",
            "SELECT public.p0c3_fail_snapshot_job($1,$2,'transient_storage',true)",
        ]
    } else {
        [
            "SELECT public.p0c2_renew_job($1,$2,300000)",
            "SELECT public.p0c2_checkpoint_job($1,$2,$3)",
            "SELECT public.p0c2_succeed_job($1,$2,$3)",
            "SELECT public.p0c2_fail_job($1,$2,'transient_storage',true)",
        ]
    };
    let renew: bool = sqlx::query_scalar(sql[0])
        .bind(job.id)
        .bind(token)
        .fetch_one(target.connection())
        .await?;
    let checkpoint: bool = sqlx::query_scalar(sql[1])
        .bind(job.id)
        .bind(token)
        .bind(json!({"forbidden_old_worker":true}))
        .fetch_one(target.connection())
        .await?;
    let succeed: bool = sqlx::query_scalar(sql[2])
        .bind(job.id)
        .bind(token)
        .bind("a".repeat(64))
        .fetch_one(target.connection())
        .await?;
    let fail: bool = sqlx::query_scalar(sql[3])
        .bind(job.id)
        .bind(token)
        .fetch_one(target.connection())
        .await?;
    require(
        !renew && !checkpoint && !succeed && !fail,
        "recovery old source token survived",
    )?;
    Ok(4)
}

pub(super) async fn after_import(
    target: &mut ImportedTarget,
    package: &VerifiedPackage,
    assets: &AssetRestoreReport,
    expected: &crate::full_restore::data::ExpectedRecoveryRows,
) -> Result<(), BackupError> {
    let Some(case) = current_case() else {
        return Ok(());
    };
    let source_jobs = RECOVERY.with(|state| state.jobs.lock().unwrap().clone());
    // Match the normalizer's transaction-local timestamp rendering before
    // recording complete row images for the independent fencing probes.
    sqlx::query("SELECT pg_catalog.set_config('TimeZone','UTC',true)")
        .execute(target.connection())
        .await?;
    let before = jobs(target).await?;
    let before_receipts = receipts(target).await?;
    let before_integrity: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(r) FROM public.asset_integrity_result r ORDER BY r.job_id",
    )
    .fetch_all(target.connection())
    .await?;
    let derived_case = matches!(case, RecoveryCase::Derived | RecoveryCase::ReceiptFailure);
    require(
        before_integrity.len() == usize::from(derived_case),
        "recovery exact genuine result-v1 fixture missing",
    )?;
    if derived_case {
        let counts: (i64,i64,i64,i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM public.block_asset_use),count(*),count(*) FILTER (WHERE v.id=h.head_revision_id),count(*) FILTER (WHERE v.id<>h.head_revision_id) FROM public.reading_view_revision v JOIN public.reading_view h ON h.id=v.view_id",
        ).fetch_one(target.connection()).await?;
        require(
            counts == (1, 2, 1, 1),
            "recovery imported derived fixture is empty",
        )?;
    }
    require(
        before.len() == source_jobs.len(),
        "recovery imported job roster differs",
    )?;
    // Verify all source rows BEFORE any intentional normalization mutation.
    let prepared =
        super::validation::prepare_rehearsal_validation(target, package, assets, expected).await?;
    let recovery_time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_900_000_000);
    let effects =
        crate::full_restore::jobs::inspect_external_effects_without_export_authority(target)
            .await?;
    let report =
        crate::full_restore::jobs::normalize_restored_jobs(target, &effects, recovery_time).await?;
    let after = jobs(target).await?;
    require(
        before.len() == after.len(),
        "recovery normalization lost jobs",
    )?;
    let mut rejected = 0;
    for seeded in &source_jobs {
        let id = seeded.id.to_string();
        let original = before
            .iter()
            .find(|row| row["id"].as_str() == Some(id.as_str()))
            .ok_or(BackupError::Invalid("recovery original job absent"))?;
        let restored = after
            .iter()
            .find(|row| row["id"].as_str() == Some(id.as_str()))
            .ok_or(BackupError::Invalid("recovery normalized job absent"))?;
        for field in [
            "id",
            "outbox_id",
            "idempotency_key",
            "attempt_count",
            "checkpoint",
            "created_at",
        ] {
            require(
                original[field] == restored[field],
                "recovery changed protected job data",
            )?;
        }
        require(
            restored["lease_token"].is_null() && restored["lease_expires_at"].is_null(),
            "RECOVERY_SOURCE_LEASES_SURVIVED_IMPORT",
        )?;
        let status = original["status"]
            .as_str()
            .ok_or(BackupError::Invalid("recovery fixture status"))?;
        if matches!(status, "succeeded" | "failed" | "cancelled")
            || (!seeded.snapshot && !status.ends_with("running"))
        {
            require(
                original == restored,
                "recovery changed immutable or waiting job",
            )?;
        } else if seeded.snapshot {
            require(
                restored["status"] == "cancelled" && restored["next_attempt_at"].is_null(),
                "unknown snapshot remained claimable",
            )?;
        } else {
            let attempts = original["attempt_count"]
                .as_i64()
                .ok_or(BackupError::Invalid("recovery fixture attempts"))?;
            require(
                if attempts == 3 {
                    restored["status"] == "failed" && restored["next_attempt_at"].is_null()
                } else {
                    restored["status"] == "retry_wait" && !restored["next_attempt_at"].is_null()
                },
                "recovery retry ceiling or deadline differs",
            )?;
        }
        rejected += reject_old_token(target, seeded).await?;
        if seeded.snapshot || restored["status"] == "failed" {
            let sql = if seeded.snapshot {
                "SELECT token FROM public.p0c3_claim_snapshot_job($1,300000)"
            } else {
                "SELECT token FROM public.p0c2_claim_job($1,300000)"
            };
            let token: Option<Uuid> = sqlx::query_scalar(sql)
                .bind(seeded.id)
                .fetch_optional(target.connection())
                .await?;
            require(
                token.is_none(),
                "recovery held or exhausted job was claimed",
            )?;
        }
    }
    require(
        jobs(target).await? == after,
        "old-token probes changed recovered jobs",
    )?;
    require(
        receipts(target).await? == before_receipts,
        "recovery rewrote snapshot receipt",
    )?;
    let after_integrity: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(r) FROM public.asset_integrity_result r ORDER BY r.job_id",
    )
    .fetch_all(target.connection())
    .await?;
    require(
        before_integrity == after_integrity,
        "recovery rewrote real result-v1 receipt",
    )?;
    let snapshot_ids: Vec<_> = source_jobs
        .iter()
        .filter(|job| job.snapshot)
        .map(|job| job.id)
        .collect();
    let runnable:i64=sqlx::query_scalar("SELECT count(*) FROM public.job WHERE id=ANY($1) AND (status='snapshot_queued' OR (status='snapshot_retry_wait' AND next_attempt_at<=clock_timestamp()) OR (status='snapshot_running' AND lease_expires_at<=clock_timestamp()))").bind(snapshot_ids).fetch_one(target.connection()).await?;
    let (job_count, rejection_count, holds) = case.coverage();
    require(
        runnable == 0
            && rejected == rejection_count
            && report.held_jobs() == holds
            && report.has_holds() == (holds > 0),
        "recovery hold or fencing coverage differs",
    )?;
    let fault = match case {
        RecoveryCase::Derived => super::validation::ReceiptFault::DerivedDigestMismatch,
        RecoveryCase::ReceiptFailure => super::validation::ReceiptFault::BeforeDirectorySync,
        _ => super::validation::ReceiptFault::None,
    };
    let ((validation, observation), hits) = super::validation::with_receipt_fault(fault, async {
        let result = super::validation::validate_rehearsal_after_normalization(
            target,
            package,
            assets,
            prepared,
            &report,
            recovery_time,
        )
        .await;
        (result, super::validation::barrier_observation())
    })
    .await;
    let derived_validation = if derived_case {
        let observation = observation.ok_or(BackupError::Invalid(
            "recovery nonempty application barriers not observed",
        ))?;
        let reading = &observation["reading"];
        for (key, value) in [
            ("view_revisions", 2_u64),
            ("current_revisions", 1),
            ("historical_revisions", 1),
            ("owner_present", 6),
            ("nonexistent_actor_denied", 6),
            ("existing_granted_nonowner_denied", 6),
            ("views_without_existing_granted_nonowner", 0),
            ("owner_without_grant_denied", 0),
        ] {
            require(
                reading[key].as_u64() == Some(value),
                "recovery actual reading coverage incomplete",
            )?;
        }
        require(
            observation["asset_integrity"]["count"].as_u64() == Some(1),
            "recovery actual result-v1 coverage empty",
        )?;
        let reading_sha256 = hex::encode(Sha256::digest(serde_json::to_vec(reading)?));
        require(
            observation["reading_report_sha256"] == reading_sha256,
            "recovery actual reading report digest differs",
        )?;
        json!({"result_v1_count":1,"block_asset_use_count":1,"reading_view_revisions":2,
            "reading_current_revisions":1,"reading_historical_revisions":1,"owner_projections":6,
            "nonexistent_actor_denials":6,"granted_peer_denials":6,"views_without_granted_peer":0,
            "reading_report_sha256":reading_sha256})
    } else {
        require(
            observation.is_none(),
            "recovery held snapshot bypassed application gate",
        )?;
        Value::Null
    };
    let (state, reason, fault_expected) = match case {
        RecoveryCase::Derived => (
            "REJECTED_DERIVED_DIGEST",
            "recovery derived digest mismatch",
            true,
        ),
        RecoveryCase::ReceiptFailure => (
            "REJECTED_RECEIPT_SYNC",
            "recovery validation receipt sync failed",
            true,
        ),
        _ => (
            "HELD_UNKNOWN_SNAPSHOT_EFFECT",
            "recovery external effects unresolved",
            false,
        ),
    };
    require(
        matches!(validation,Err(BackupError::Invalid(message)) if message==reason)
            && hits == usize::from(fault_expected),
        "recovery expected barrier was not observed",
    )?;
    let report_bytes = serde_json::to_vec(report.evidence())?;
    require(
        !report_bytes.is_empty() && report_bytes.len() <= 1024 * 1024,
        "recovery fixture report capacity",
    )?;
    let report_sha = hex::encode(Sha256::digest(&report_bytes));
    require(
        report.canonical_sha256()? == report_sha,
        "recovery report canonical digest differs",
    )?;
    let database = target.admitted().config.expected_database.clone();
    let target_id = crate::full_restore::profile::FullRehearsalProfile::read_installed()?
        .target_container_id()
        .to_owned();
    let proof = json!({"format_version":1,"classification":"FULL_RECOVERY_FOUNDATION_CASE_PASSED_SINGLE_HOST_QUARANTINED_NOT_COMPLETE","case":case.name(),"backup_id":package.manifest.backup_id,"manifest_sha256":package.manifest.canonical_sha256()?,"target_database":database,"target_container_id":target_id,"stop_required":true,"snapshot_authority":"UNAVAILABLE_ALL_SNAPSHOTS_UNKNOWN","external_effect_bytes_verified":false,"normalization_report_sha256":report_sha,"normalization_report_bytes":report_bytes.len(),"normalization_verified":true,"attempt_counts_preserved":true,"running_leases_cleared":true,"source_job_count":job_count,"old_token_rejections":rejected,"snapshot_holds":holds,"terminal_receipts_unchanged":true,"validation_state":state,"fault_observed":fault_expected,"derived_validation":derived_validation,"usable":false,"task6_complete":false});
    // This hook precedes exact-stop. The separate import Success proof and
    // actual stopped target observation must both be joined by the runner.
    let root = BackupDir::open_trusted_private_root(Path::new("/var/lib/knowweave-source/proofs"))?;
    for (name, bytes) in [
        ("full-recovery-normalization.json", report_bytes),
        ("full-recovery-result.json", serde_json::to_vec(&proof)?),
    ] {
        let mut file = root.create_file(name)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        root.sync()?;
    }
    Ok(())
}

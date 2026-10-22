use super::{
    history::{Claim, Rows},
    writer,
};
use chrono::{TimeDelta, Utc};
use learning_core::{JobInput, ReadingMode, ReadingRef, SnapshotRequest};
use serde_json::json;
use uuid::Uuid;

fn observed() -> (Rows, Claim) {
    let job = Uuid::new_v4();
    let token = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let now = Utc::now();
    let expiry = now + TimeDelta::minutes(5);
    let input = JobInput::snapshot_export(
        actor,
        Uuid::new_v4(),
        SnapshotRequest {
            reading: ReadingRef {
                view_id: Uuid::new_v4(),
                revision_id: Uuid::new_v4(),
            },
            mode: ReadingMode::Fused,
            include_personal: false,
            include_originals: false,
            resource_versions: vec![],
            source_segments: vec![],
        },
    )
    .unwrap();
    let rows = serde_json::from_value(json!({
        "job": {"id": job, "outbox_id": job, "idempotency_key": input.business_key(),
            "status": "snapshot_running", "attempt_count": 1, "lease_token": token,
            "lease_expires_at": expiry, "next_attempt_at": null, "last_error_class": null,
            "output_digest": null, "checkpoint": null, "created_at": now, "updated_at": now},
        "outbox": {"id": job, "business_key": input.business_key(),
            "event_type": "snapshot_export_requested", "payload_version": 1,
            "payload": input.to_value(), "actor_id": actor, "processor_version": 1,
            "created_at": now, "dispatched_at": now}, "observed_at": now,
    }))
    .unwrap();
    (
        rows,
        Claim {
            job_id: job,
            token,
            attempts: 1,
            expires_at: expiry,
        },
    )
}

// A token/lease mutation between the original known claim and issue must reject
// before the sticky issue point or any package filesystem entry is created.
#[test]
fn c2_issue_rejects_changed_token_and_expired_database_clock() {
    let (original, claim) = observed();
    let mut current: Rows =
        serde_json::from_value(serde_json::to_value(&original).unwrap()).unwrap();
    current.job.lease_token = Some(Uuid::new_v4());
    assert!(
        writer::validate_issue_rows(&original, &claim, &current, original.observed_at).is_err()
    );
    let current: Rows = serde_json::from_value(serde_json::to_value(&original).unwrap()).unwrap();
    assert!(writer::validate_issue_rows(&original, &claim, &current, claim.expires_at).is_err());
}

#[test]
fn c2_issue_rejects_any_full_row_or_outbox_mutation() {
    let (original, claim) = observed();
    let mut current: Rows =
        serde_json::from_value(serde_json::to_value(&original).unwrap()).unwrap();
    assert!(writer::validate_issue_rows(&original, &claim, &current, original.observed_at).is_ok());
    current.outbox.actor_id = Uuid::new_v4();
    assert!(
        writer::validate_issue_rows(&original, &claim, &current, original.observed_at).is_err()
    );
    let mut current: Rows =
        serde_json::from_value(serde_json::to_value(&original).unwrap()).unwrap();
    current.job.checkpoint = Some(json!({"external": true}));
    assert!(
        writer::validate_issue_rows(&original, &claim, &current, original.observed_at).is_err()
    );
}

#[cfg(target_os = "linux")]
mod linux {
    use super::super::{
        history::runtime::{self, EpochWork, Hooks, Stage},
        install,
    };
    use crate::{CompositionStore, ContentStore, JobStore, MIGRATOR, ReadingStore, SnapshotStore};
    use learning_assets::{RootSnapshotCustody, verify_snapshot};
    use learning_core::*;
    use sqlx::{PgPool, postgres::PgPoolOptions};
    use std::{
        path::{Path, PathBuf},
        sync::{Arc, Mutex, atomic::Ordering},
        time::Duration,
    };
    use tokio::{sync::Notify, time::timeout};
    use uuid::Uuid;

    struct Rig {
        owner: PgPool,
        runtime: PgPool,
        broker: PgPool,
        cluster: PgPool,
    }
    async fn pool(role: &str) -> PgPool {
        PgPoolOptions::new()
            .max_connections(3)
            .connect(
                &std::env::var(format!("TEST_SNAPSHOT_CLAIM_P1_{role}_DATABASE_URL"))
                    .expect("explicit fresh C2 PG18 DSN; no skip"),
            )
            .await
            .expect("actual role LOGIN")
    }
    impl Rig {
        async fn new() -> Self {
            let owner = pool("ADMIN").await;
            let runtime = pool("RUNTIME").await;
            let cluster = pool("CLUSTER").await;
            let identity: (String, String, String) =
                sqlx::query_as("SELECT session_user::text,current_user::text,current_database()")
                    .fetch_one(&owner)
                    .await
                    .unwrap();
            assert_eq!(
                (&*identity.0, &*identity.1),
                ("learning_admin", "learning_admin")
            );
            let suffix = identity
                .2
                .strip_prefix("learning_task6_claim_p1_")
                .expect("fresh P1/C2 database");
            assert_eq!(Uuid::parse_str(suffix).unwrap().to_string(), suffix);
            MIGRATOR.run(&owner).await.unwrap();
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM public.job")
                .fetch_one(&owner)
                .await
                .unwrap();
            assert_eq!(count, 0, "fresh isolated database required");
            install(&owner).await.unwrap();
            let broker = pool("BROKER").await;
            Self {
                owner,
                runtime,
                broker,
                cluster,
            }
        }
        async fn enqueue(&self, exact: bool) -> (Uuid, Uuid) {
            let actor = Principal {
                actor_id: Uuid::new_v4(),
            };
            let space = Uuid::new_v4();
            sqlx::query("INSERT INTO public.app_user(id) VALUES($1)")
                .bind(actor.actor_id)
                .execute(&self.owner)
                .await
                .unwrap();
            sqlx::query("INSERT INTO public.space(id,owner_id) VALUES($1,$2)")
                .bind(space)
                .bind(actor.actor_id)
                .execute(&self.owner)
                .await
                .unwrap();
            sqlx::query(
                "INSERT INTO public.space_grant(actor_id,space_id,can_write) VALUES($1,$2,true)",
            )
            .bind(actor.actor_id)
            .bind(space)
            .execute(&self.owner)
            .await
            .unwrap();
            let block = ContentStore::new(self.runtime.clone()).create(actor, space, CreateCommand {
                request_id: Uuid::new_v4(), reason: "C2 real domain fixture".into(),
                draft: serde_json::from_value(serde_json::json!({"kind":"text", "intent":"note", "language":"zh-CN", "title":"DB prepared copy", "payload":{"format":"markdown","text":"Real enqueued nonempty reading. 真实非空阅读。"}})).unwrap(),
            }).await.unwrap();
            let document = CompositionStore::new(self.runtime.clone())
                .save(
                    actor,
                    space,
                    SaveComposition {
                        request_id: Uuid::new_v4(),
                        composition_id: None,
                        base_revision_id: None,
                        kind: CompositionKind::Document,
                        title: "C2 Reading".into(),
                        nodes: vec![NodeDraft {
                            occurrence_id: None,
                            target: NodeTarget::Block(BlockRef {
                                block_id: block.block_id,
                                revision_id: block.revision_id,
                            }),
                        }],
                        reason: "C2 domain fixture".into(),
                    },
                )
                .await
                .unwrap();
            let reading = ReadingStore::new(self.runtime.clone())
                .create(
                    actor,
                    space,
                    CreateReading {
                        request_id: Uuid::new_v4(),
                        base: document.reference,
                        title: "C2".into(),
                        reason: "C2 domain fixture".into(),
                    },
                )
                .await
                .unwrap();
            let request = Uuid::new_v4();
            let job = SnapshotStore::new(self.runtime.clone())
                .enqueue_export(
                    actor,
                    request,
                    SnapshotRequest {
                        reading: reading.view,
                        mode: ReadingMode::Fused,
                        include_personal: exact,
                        include_originals: exact,
                        resource_versions: vec![],
                        source_segments: vec![],
                    },
                )
                .await
                .unwrap();
            JobStore::new(self.runtime.clone())
                .dispatch_pending(256)
                .await
                .unwrap();
            (job, request)
        }
        async fn close(self) {
            self.broker.close().await;
            // Normal successful fixture teardown, matching original P1. The
            // next fresh database must not inherit this test's direct CONNECT
            // grant in the shared cluster. Failures remain failures, never an
            // installer/profile repair or a best-effort cleanup-and-retry.
            sqlx::query(
                "DROP FUNCTION public.p0c4_snapshot_claim_locked_observation(uuid,boolean)",
            )
            .execute(&self.owner)
            .await
            .unwrap();
            sqlx::query("REVOKE USAGE ON SCHEMA public FROM learning_snapshot_claim_broker")
                .execute(&self.owner)
                .await
                .unwrap();
            let sql: String = sqlx::query_scalar("SELECT format('REVOKE CONNECT ON DATABASE %I FROM learning_snapshot_claim_broker',current_database())")
                .fetch_one(&self.owner).await.unwrap();
            sqlx::query(&sql).execute(&self.owner).await.unwrap();
            sqlx::raw_sql("REVOKE EXECUTE ON FUNCTION public.p0c3_claim_snapshot_job(uuid,bigint),public.p0c3_renew_snapshot_job(uuid,uuid,bigint),public.p0c3_fail_snapshot_job(uuid,uuid,text,boolean) FROM learning_snapshot_claim_broker")
                .execute(&self.owner).await.unwrap();
            self.runtime.close().await;
            self.owner.close().await;
            self.cluster.close().await;
        }
        async fn no_result(&self, job: Uuid) {
            let fact: (String, i32, i64) = sqlx::query_as("SELECT status,attempt_count,(SELECT count(*) FROM public.snapshot_export_result WHERE job_id=$1) FROM public.job WHERE id=$1")
                .bind(job).fetch_one(&self.owner).await.unwrap();
            assert_eq!(fact, ("snapshot_running".into(), 1, 0));
        }
    }
    fn fixture(r: &Rig, hooks: Option<Arc<Hooks>>) -> (EpochWork, RootSnapshotCustody, PathBuf) {
        let path = PathBuf::from("/run").join(format!("task6-c2-{}", Uuid::new_v4()));
        let work =
            runtime::real_fixture(r.broker.clone(), &path, hooks).expect("original Root owner");
        let custody = RootSnapshotCustody::create_fresh(&path.join("exports"))
            .expect("fresh Root OS custody");
        (work, custody, path)
    }
    fn hooks(stage: Stage) -> Arc<Hooks> {
        Arc::new(Hooks {
            stage,
            arrived: Notify::new(),
            resume: Notify::new(),
            lose_commit_knowledge: false,
            blocking_return: false,
            blocking_release: (Mutex::new(false), std::sync::Condvar::new()),
            post_spawn_join: false,
            spawned: Notify::new(),
            join_resume: Notify::new(),
        })
    }
    struct Release(Arc<Hooks>);
    impl Drop for Release {
        fn drop(&mut self) {
            *self.0.blocking_release.0.lock().unwrap() = true;
            self.0.blocking_release.1.notify_all();
            self.0.resume.notify_one();
        }
    }
    async fn arrived(h: &Hooks) {
        timeout(Duration::from_secs(30), h.arrived.notified())
            .await
            .expect("actual original stage arrived");
    }
    fn frames(path: &Path) -> Vec<serde_json::Value> {
        let mut files: Vec<_> = std::fs::read_dir(path.join("journal"))
            .unwrap()
            .map(|f| f.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "frame"))
            .collect();
        files.sort();
        files
            .into_iter()
            .map(|p| {
                let bytes = std::fs::read(p).unwrap();
                assert_eq!(&bytes[..8], b"KWSJBYT2");
                serde_json::from_slice(&bytes[120..]).unwrap()
            })
            .collect()
    }
    fn no_package(path: &Path, job: Uuid) {
        assert!(!path.join("exports").join(job.to_string()).exists());
    }

    #[tokio::test]
    #[ignore = "fresh PG18 four actual LOGINs and Root0 /run; no skip"]
    async fn c2_real_enqueued_copy_reaches_ordered_issue_and_verified_package() {
        let r = Rig::new().await;
        let (job, request) = r.enqueue(false).await;
        let (mut work, custody, path) = fixture(&r, None);
        let pending = work
            .start_copy(
                job,
                request,
                Duration::from_secs(60),
                r.runtime.clone(),
                &custody,
            )
            .unwrap();
        assert!(
            custody.try_capture().is_err(),
            "SH must precede registration return"
        );
        let staged = pending.await.unwrap();
        work.drain().await.unwrap();
        let verified = verify_snapshot(staged.directory()).unwrap();
        assert!(matches!(
            verified.manifest,
            SnapshotManifest::ReadingCopyV1(_)
        ));
        assert_eq!(verified.files.len(), 3);
        let bytes: u64 = verified.files.iter().map(|f| f.size).sum();
        assert!(bytes > 100);
        let records = frames(&path);
        assert_eq!(records.len(), 4);
        assert_eq!(records[0]["schema"], "knowweave.snapshot-claim.p1.intent");
        assert_eq!(
            records[1]["schema"],
            "knowweave.snapshot-claim.p1.committed"
        );
        assert_eq!(records[2]["event"], "WRITE_AUTHORITY_ISSUED");
        assert_eq!(records[3]["event"], "PACKAGE_STAGED");
        assert_eq!(records[3]["verified_files"], 3);
        assert_eq!(records[3]["verified_bytes"], bytes);
        assert_eq!(
            records[3]["manifest_sha256"],
            staged.directory().manifest_sha256()
        );
        assert_eq!(records[3]["issue"]["sequence"], 3);
        assert_eq!(staged.sequences(), (3, 4));
        assert!(work.original().issued_for(job));
        assert_eq!(work.original().sql_calls.load(Ordering::SeqCst), 1);
        assert!(
            custody.try_capture().is_err(),
            "private staged result retains its SH"
        );
        r.no_result(job).await;
        drop(staged);
        assert!(custody.try_capture().is_ok());
        r.close().await;
    }

    #[tokio::test]
    #[ignore = "fresh PG18 four actual LOGINs and Root0 /run; no skip"]
    async fn c2_changed_lease_before_issue_rejects_without_package_files() {
        let r = Rig::new().await;
        let (job, request) = r.enqueue(false).await;
        let h = hooks(Stage::BeforeIssue);
        let _release = Release(h.clone());
        let (mut work, custody, path) = fixture(&r, Some(h.clone()));
        let pending = work
            .start_copy(
                job,
                request,
                Duration::from_secs(60),
                r.runtime.clone(),
                &custody,
            )
            .unwrap();
        arrived(&h).await;
        // Canonical renewal induces actual expiry drift under the same token;
        // this is neither natural expiry nor a token takeover/direct UPDATE.
        let (token, expiry): (Uuid, chrono::DateTime<chrono::Utc>) =
            sqlx::query_as("SELECT lease_token,lease_expires_at FROM public.job WHERE id=$1")
                .bind(job)
                .fetch_one(&r.owner)
                .await
                .unwrap();
        assert!(
            JobStore::new(r.runtime.clone())
                .renew(job, token, Duration::from_secs(300))
                .await
                .unwrap()
        );
        let renewed: (String, i32, Uuid, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
            "SELECT status,attempt_count,lease_token,lease_expires_at FROM public.job WHERE id=$1",
        )
        .bind(job)
        .fetch_one(&r.owner)
        .await
        .unwrap();
        assert_eq!(renewed.0, "snapshot_running");
        assert_eq!(renewed.1, 1);
        assert!(
            renewed.2 == token,
            "canonical renewal keeps the original token"
        );
        assert_ne!(renewed.3, expiry, "actual DB lease expiry must change");
        h.resume.notify_one();
        assert!(pending.await.is_err());
        work.drain().await.unwrap();
        assert!(!work.original().issued_for(job));
        assert!(work.original().held());
        assert_eq!(frames(&path).len(), 2);
        no_package(&path, job);
        r.no_result(job).await;
        assert!(custody.try_capture().is_ok());
        r.close().await;
    }

    #[tokio::test]
    #[ignore = "fresh PG18 four actual LOGINs and Root0 /run; no skip"]
    async fn c2_durable_issue_cancel_before_first_create_stays_issued_hold() {
        let r = Rig::new().await;
        let (job, request) = r.enqueue(false).await;
        let h = hooks(Stage::AfterIssue);
        let _release = Release(h.clone());
        let (mut work, custody, path) = fixture(&r, Some(h.clone()));
        let pending = work
            .start_copy(
                job,
                request,
                Duration::from_secs(60),
                r.runtime.clone(),
                &custody,
            )
            .unwrap();
        arrived(&h).await;
        assert_eq!(frames(&path)[2]["event"], "WRITE_AUTHORITY_ISSUED");
        assert!(work.original().issued_for(job));
        no_package(&path, job);
        drop(pending);
        assert!(custody.try_capture().is_err());
        h.resume.notify_one();
        work.drain().await.unwrap();
        assert!(work.original().held());
        assert!(work.original().issued_for(job));
        assert_eq!(frames(&path).len(), 3);
        no_package(&path, job);
        r.no_result(job).await;
        assert!(custody.try_capture().is_ok());
        // The same original meter can admit the small prepared-copy encoding
        // yet reject the complete staged-work prepayment. Issued stays sticky
        // and no job/partial/ready may be created after that capacity failure.
        let (job, request) = r.enqueue(false).await;
        let h = hooks(Stage::AfterIssue);
        let _release = Release(h.clone());
        let (mut work, custody, path) = fixture(&r, Some(h.clone()));
        let pending = work
            .start_copy(
                job,
                request,
                Duration::from_secs(60),
                r.runtime.clone(),
                &custody,
            )
            .unwrap();
        arrived(&h).await;
        let budget = &work.original().budget;
        let used = budget.usage().unwrap().charged_bytes;
        budget.charge(512 * 1024 * 1024 - used - 4096, 0).unwrap();
        h.resume.notify_one();
        assert!(pending.await.is_err());
        work.drain().await.unwrap();
        assert!(work.original().issued_for(job));
        assert!(work.original().held());
        assert!(work.original().budget.usage().unwrap().exhausted);
        no_package(&path, job);
        assert_eq!(frames(&path).len(), 3);
        r.no_result(job).await;
        assert!(custody.try_capture().is_ok());
        r.close().await;
    }

    #[tokio::test]
    #[ignore = "fresh PG18 four actual LOGINs and Root0 /run; no skip"]
    async fn c2_started_blocking_copy_retains_shared_lock_until_original_settlement() {
        let r = Rig::new().await;
        for expire in [false, true] {
            let (job, request) = r.enqueue(false).await;
            let h = hooks(Stage::AfterStageCopy);
            let _release = Release(h.clone());
            let (mut work, custody, path) = fixture(&r, Some(h.clone()));
            let pending = work
                .start_copy(
                    job,
                    request,
                    Duration::from_secs(if expire { 10 } else { 60 }),
                    r.runtime.clone(),
                    &custody,
                )
                .unwrap();
            arrived(&h).await;
            let original = work.original().clone();
            if expire {
                assert!(pending.await.is_err());
            } else {
                drop(pending);
            }
            assert!(original.issued_for(job));
            assert!(original.held());
            assert!(custody.try_capture().is_err());
            {
                let drain = work.drain();
                tokio::pin!(drain);
                tokio::select! { _=&mut drain=>panic!("real blocking writer detached"), _=tokio::time::sleep(Duration::from_millis(10))=>{} }
            }
            assert!(original.active());
            assert!(custody.try_capture().is_err());
            *h.blocking_release.0.lock().unwrap() = true;
            h.blocking_release.1.notify_all();
            work.drain().await.unwrap();
            assert!(!original.active());
            assert!(custody.try_capture().is_ok());
            assert_eq!(frames(&path).len(), 3);
            r.no_result(job).await;
        }
        // A Root fixture injects a real ENOTDIR at the actual job-directory
        // open. This is an error seam, never a successful package or authority.
        let (job, request) = r.enqueue(false).await;
        let h = hooks(Stage::AfterStageError);
        let _release = Release(h.clone());
        let (mut work, custody, path) = fixture(&r, Some(h.clone()));
        std::fs::write(
            path.join("exports").join(job.to_string()),
            b"fixture blocks actual directory creation",
        )
        .unwrap();
        let pending = work
            .start_copy(
                job,
                request,
                Duration::from_secs(60),
                r.runtime.clone(),
                &custody,
            )
            .unwrap();
        arrived(&h).await;
        assert!(work.original().issued_for(job));
        assert!(work.original().held());
        assert!(
            custody.try_capture().is_err(),
            "original SH outlives actual consumed-writer error"
        );
        assert_eq!(frames(&path).len(), 3);
        drop(pending);
        h.resume.notify_one();
        work.drain().await.unwrap();
        assert!(custody.try_capture().is_ok());
        r.no_result(job).await;
        r.close().await;
    }

    #[tokio::test]
    #[ignore = "fresh PG18 four actual LOGINs and Root0 /run; no skip"]
    async fn c2_wrong_runtime_or_custody_root_never_reclaims_or_writes() {
        let r = Rig::new().await;
        let wrong_endpoint = PgPoolOptions::new().max_connections(1).connect_lazy_with(
            r.runtime
                .connect_options()
                .as_ref()
                .clone()
                .host("c2-source-mismatch.invalid"),
        );
        for rejected_pool in [r.owner.clone(), wrong_endpoint.clone()] {
            let (job, request) = r.enqueue(false).await;
            let (mut work, custody, path) = fixture(&r, None);
            let pending = work
                .start_copy(
                    job,
                    request,
                    Duration::from_secs(60),
                    rejected_pool,
                    &custody,
                )
                .unwrap();
            assert!(
                work.start_copy(
                    job,
                    request,
                    Duration::from_secs(60),
                    r.runtime.clone(),
                    &custody
                )
                .is_err()
            );
            assert!(
                work.start_copy(
                    job,
                    Uuid::new_v4(),
                    Duration::from_secs(60),
                    r.runtime.clone(),
                    &custody
                )
                .is_err()
            );
            assert!(pending.await.is_err());
            work.drain().await.unwrap();
            assert!(!work.original().issued_for(job));
            assert!(frames(&path).len() <= 2);
            no_package(&path, job);
            assert_eq!(work.original().sql_calls.load(Ordering::SeqCst), 1);
            r.no_result(job).await;
        }
        wrong_endpoint.close().await;
        let (job, _) = r.enqueue(false).await;
        let (mut work, custody, path) = fixture(&r, None);
        std::fs::rename(path.join("exports"), path.join("exports-moved")).unwrap();
        assert!(
            work.start_copy(
                job,
                Uuid::new_v4(),
                Duration::from_secs(60),
                r.runtime.clone(),
                &custody
            )
            .is_err()
        );
        assert_eq!(work.original().sql_calls.load(Ordering::SeqCst), 0);
        assert!(frames(&path).is_empty());
        r.close().await;
    }

    #[tokio::test]
    #[ignore = "fresh PG18 four actual LOGINs and Root0 /run; no skip"]
    async fn c2_exact_request_is_rejected_before_issue_or_files() {
        let r = Rig::new().await;
        let (job, request) = r.enqueue(true).await;
        let (mut work, custody, path) = fixture(&r, None);
        assert!(
            work.start_copy(
                job,
                request,
                Duration::from_secs(60),
                r.runtime.clone(),
                &custody
            )
            .unwrap()
            .await
            .is_err()
        );
        work.drain().await.unwrap();
        assert_eq!(frames(&path).len(), 2);
        assert!(!work.original().issued_for(job));
        no_package(&path, job);
        r.no_result(job).await;
        r.close().await;
    }
}

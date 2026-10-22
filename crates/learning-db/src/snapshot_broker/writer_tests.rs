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

    const OBSERVER_SIGNATURE: &str =
        "public.p0c4_snapshot_publication_observe_v1(uuid,uuid,uuid,timestamptz,text,boolean)";
    const OBSERVE: &str =
        "SELECT * FROM public.p0c4_snapshot_publication_observe_v1($1,$2,$3,$4,$5,$6)";

    async fn observer_catalog(r: &Rig) -> String {
        sqlx::query_scalar("SELECT COALESCE(jsonb_agg(to_jsonb(p)||jsonb_build_object('_xmin',p.xmin::text,'_ctid',p.ctid::text) ORDER BY p.oid),'[]'::jsonb)::text FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='public' AND p.proname='p0c4_snapshot_publication_observe_v1'")
            .fetch_one(&r.owner).await.unwrap()
    }
    async fn runtime_job_boundary(r: &Rig) {
        let denied: bool = sqlx::query_scalar("SELECT NOT pg_catalog.has_column_privilege('learning_runtime','public.job','lease_token','SELECT') AND (SELECT bool_and(NOT pg_catalog.has_column_privilege('learning_runtime','public.job',c,'UPDATE')) FROM unnest(ARRAY['status','attempt_count','lease_token','lease_expires_at','next_attempt_at','last_error_class','output_digest','checkpoint']) c)")
            .fetch_one(&r.owner).await.unwrap();
        assert!(denied, "runtime retains its original job column boundary");
    }
    async fn observer_setup(r: &Rig) {
        super::super::writer::install_publication_observation(&r.owner)
            .await
            .expect("D1 observation owner install must be supported");
        runtime_job_boundary(r).await;
    }
    async fn owner_lock_probe(r: &Rig, job: Uuid, locked: bool) {
        let mut tx = r.owner.begin().await.unwrap();
        let result = sqlx::query("SELECT id FROM public.job WHERE id=$1 FOR UPDATE NOWAIT")
            .bind(job)
            .fetch_one(&mut *tx)
            .await;
        if locked {
            let error = result.expect_err("valid original before retains the actual row lock");
            assert_eq!(
                error.as_database_error().and_then(|e| e.code()).as_deref(),
                Some("55P03")
            );
        } else {
            result.expect("invalid observer binding must not lock the target job");
        }
        tx.rollback().await.unwrap();
    }

    #[tokio::test]
    #[ignore = "fresh PG18 four actual LOGINs and Root0 /run; no skip"]
    async fn d1_publication_observer_rejects_wrong_token_expiry_request_and_phase() {
        use sqlx::Row;
        let r = Rig::new().await;
        observer_setup(&r).await;
        let (job, request) = r.enqueue(false).await;
        let lease = JobStore::new(r.runtime.clone())
            .claim_snapshot(job, Duration::from_secs(60))
            .await
            .unwrap()
            .unwrap();
        let business_before: String = sqlx::query_scalar("SELECT jsonb_build_object('job',to_jsonb(j),'outbox',to_jsonb(e),'receipts',(SELECT count(*) FROM public.snapshot_export_result WHERE job_id=j.id))::text FROM public.job j JOIN public.job_outbox e ON e.id=j.outbox_id WHERE j.id=$1")
            .bind(job).fetch_one(&r.owner).await.unwrap();
        for (token, expiry, request_arg, phase) in [
            (Uuid::new_v4(), lease.lease_expires_at, request, "before"),
            (
                lease.token,
                lease.lease_expires_at + chrono::Duration::milliseconds(1),
                request,
                "before",
            ),
            (
                lease.token,
                lease.lease_expires_at,
                Uuid::new_v4(),
                "before",
            ),
            (lease.token, lease.lease_expires_at, request, "invalid"),
            (lease.token, lease.lease_expires_at, request, "after"),
        ] {
            let mut tx = r.runtime.begin().await.unwrap();
            let result = sqlx::query(OBSERVE)
                .bind(job)
                .bind(token)
                .bind(request_arg)
                .bind(expiry)
                .bind(phase)
                .bind(true)
                .fetch_optional(&mut *tx)
                .await;
            if phase == "invalid" {
                let error = result.expect_err("unknown phase must reject before observation");
                assert_eq!(
                    error.as_database_error().and_then(|e| e.code()).as_deref(),
                    Some("22023")
                );
            } else {
                assert!(
                    result.unwrap().is_none(),
                    "wrong binding or premature after cannot disclose a body"
                );
            }
            // Keep the original runtime transaction open during this independent
            // owner NOWAIT probe; a rollback-before-probe would miss a row lock.
            owner_lock_probe(&r, job, false).await;
            tx.rollback().await.unwrap();
        }
        let mut tx = r.runtime.begin().await.unwrap();
        let header = sqlx::query(OBSERVE)
            .bind(job)
            .bind(lease.token)
            .bind(request)
            .bind(lease.lease_expires_at)
            .bind("before")
            .bind(false)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        for field in [
            "job_row",
            "outbox_row",
            "request_row",
            "key_row",
            "receipt_row",
        ] {
            assert!(header.get::<Option<serde_json::Value>, _>(field).is_none());
        }
        owner_lock_probe(&r, job, true).await;
        let body = sqlx::query(OBSERVE)
            .bind(job)
            .bind(lease.token)
            .bind(request)
            .bind(lease.lease_expires_at)
            .bind("before")
            .bind(true)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        for field in [
            "job_octets",
            "outbox_octets",
            "request_octets",
            "key_octets",
            "receipt_octets",
        ] {
            assert_eq!(header.get::<i64, _>(field), body.get::<i64, _>(field));
        }
        for (field, length) in [
            ("job_row", 13),
            ("outbox_row", 9),
            ("request_row", 4),
            ("key_row", 4),
        ] {
            assert_eq!(
                body.get::<serde_json::Value, _>(field)
                    .as_object()
                    .unwrap()
                    .len(),
                length
            );
        }
        assert_eq!(
            body.get::<serde_json::Value, _>("job_row")["lease_token"],
            lease.token.to_string()
        );
        assert!(
            body.get::<Option<serde_json::Value>, _>("receipt_row")
                .is_none()
        );
        tx.rollback().await.unwrap();
        owner_lock_probe(&r, job, false).await;
        let business_after: String = sqlx::query_scalar("SELECT jsonb_build_object('job',to_jsonb(j),'outbox',to_jsonb(e),'receipts',(SELECT count(*) FROM public.snapshot_export_result WHERE job_id=j.id))::text FROM public.job j JOIN public.job_outbox e ON e.id=j.outbox_id WHERE j.id=$1")
            .bind(job).fetch_one(&r.owner).await.unwrap();
        assert_eq!(
            business_before, business_after,
            "observer never changes business rows"
        );
        runtime_job_boundary(&r).await;
        r.close().await;
    }

    #[tokio::test]
    #[ignore = "fresh PG18 four actual LOGINs and Root0 /run; no skip"]
    async fn d1_publication_observer_profile_drift_is_rejected_without_repair() {
        let r = Rig::new().await;
        for drift in ["signature", "source", "acl"] {
            observer_setup(&r).await;
            let definition: String =
                sqlx::query_scalar("SELECT pg_catalog.pg_get_functiondef($1::regprocedure)")
                    .bind(OBSERVER_SIGNATURE)
                    .fetch_one(&r.owner)
                    .await
                    .unwrap();
            match drift {
                "signature" => {
                    sqlx::raw_sql("DROP FUNCTION public.p0c4_snapshot_publication_observe_v1(uuid,uuid,uuid,timestamptz,text,boolean); CREATE FUNCTION public.p0c4_snapshot_publication_observe_v1(uuid) RETURNS boolean LANGUAGE plpgsql AS $drift$ BEGIN RETURN false; END $drift$; REVOKE ALL ON FUNCTION public.p0c4_snapshot_publication_observe_v1(uuid) FROM PUBLIC; GRANT EXECUTE ON FUNCTION public.p0c4_snapshot_publication_observe_v1(uuid) TO learning_runtime")
                        .execute(&r.owner).await.unwrap();
                }
                "source" => {
                    assert_eq!(definition.matches("AS $function$").count(), 1);
                    let changed = definition.replacen(
                        "AS $function$",
                        "AS $function$\n-- deliberate observer source drift\n",
                        1,
                    );
                    sqlx::raw_sql(&changed).execute(&r.owner).await.unwrap();
                }
                "acl" => {
                    // Runtime WITH GRANT OPTION isolates the new observer
                    // profile; the old three-function broker profile is intact.
                    sqlx::raw_sql("GRANT EXECUTE ON FUNCTION public.p0c4_snapshot_publication_observe_v1(uuid,uuid,uuid,timestamptz,text,boolean) TO learning_runtime WITH GRANT OPTION")
                        .execute(&r.owner).await.unwrap();
                }
                _ => unreachable!(),
            }
            let drifted = observer_catalog(&r).await;
            assert!(
                super::super::writer::install_publication_observation(&r.owner)
                    .await
                    .is_err()
            );
            assert_eq!(
                observer_catalog(&r).await,
                drifted,
                "owner installer must not repair drift"
            );
            let (job, request) = r.enqueue(false).await;
            let (mut work, custody, path) = fixture(&r, None);
            let pending = work
                .start_publish(
                    job,
                    request,
                    Duration::from_secs(60),
                    r.runtime.clone(),
                    &custody,
                )
                .unwrap();
            assert!(pending.await.is_err());
            work.drain().await.unwrap();
            assert!(work.original().held());
            assert!(work.original().issued_for(job));
            assert_eq!(
                frames(&path).len(),
                4,
                "new observer profile rejects after original staging"
            );
            r.no_result(job).await;
            assert_eq!(
                observer_catalog(&r).await,
                drifted,
                "runtime validation must not repair drift"
            );
            assert!(custody.try_capture().is_ok());
            // Explicit fixture removal after rejection is established, never
            // an installer retry that overwrites a mismatched function.
            let drop = if drift == "signature" {
                "DROP FUNCTION public.p0c4_snapshot_publication_observe_v1(uuid)"
            } else {
                "DROP FUNCTION public.p0c4_snapshot_publication_observe_v1(uuid,uuid,uuid,timestamptz,text,boolean)"
            };
            sqlx::raw_sql(drop).execute(&r.owner).await.unwrap();
        }
        r.close().await;
    }

    #[tokio::test]
    #[ignore = "fresh PG18 four actual LOGINs and Root0 /run; no skip"]
    async fn d1_publication_observer_installer_requires_owner_and_is_verify_only_on_repeat() {
        let r = Rig::new().await;
        let absent = observer_catalog(&r).await;
        assert_eq!(absent, "[]");
        for pool in [&r.runtime, &r.broker, &r.cluster] {
            assert!(
                super::super::writer::install_publication_observation(pool)
                    .await
                    .is_err()
            );
            assert_eq!(
                observer_catalog(&r).await,
                absent,
                "non-owner must reject before any DDL"
            );
        }
        super::super::writer::install_publication_observation(&r.owner)
            .await
            .expect("D1 observation owner install must be supported");
        let installed = observer_catalog(&r).await;
        assert_ne!(installed, absent);
        for _ in 0..2 {
            super::super::writer::install_publication_observation(&r.owner)
                .await
                .unwrap();
            assert_eq!(
                observer_catalog(&r).await,
                installed,
                "repeat install only verifies OID/source/ACL/catalog"
            );
        }
        for pool in [&r.runtime, &r.broker, &r.cluster] {
            assert!(
                super::super::writer::install_publication_observation(pool)
                    .await
                    .is_err()
            );
            assert_eq!(observer_catalog(&r).await, installed);
        }
        runtime_job_boundary(&r).await;
        r.close().await;
    }

    // Missing publication, an incomplete receipt, an early SH release, or either
    // missing durable success event must fail this real domain-to-DB contract.
    #[tokio::test]
    #[ignore = "fresh PG18 four actual LOGINs and Root0 /run; no skip"]
    async fn d1_real_enqueued_copy_publishes_receipt_and_six_durable_frames() {
        use crate::snapshot_observation::{JobObservation, OutboxObservation};
        use sqlx::types::Json;

        #[derive(serde::Serialize, sqlx::FromRow)]
        struct Receipt {
            job_id: Uuid,
            stage_token: Uuid,
            capability: String,
            manifest_sha256: String,
            plan_sha256: String,
            required_spaces: Vec<Uuid>,
            created_at: chrono::DateTime<chrono::Utc>,
        }
        type BoundRows = (
            Json<JobObservation>,
            Json<OutboxObservation>,
            Json<serde_json::Value>,
            Json<serde_json::Value>,
        );
        const BOUND_ROWS: &str = "SELECT to_jsonb(j),to_jsonb(e),to_jsonb(r),to_jsonb(k) FROM public.job j JOIN public.job_outbox e ON e.id=j.outbox_id JOIN public.snapshot_export_request r ON r.job_id=j.id AND r.outbox_id=e.id AND r.actor_id=e.actor_id JOIN public.request_key k ON k.actor_id=r.actor_id AND k.request_id=r.request_id WHERE j.id=$1 AND r.request_id=$2";

        let r = Rig::new().await;
        observer_setup(&r).await;
        let (job, request) = r.enqueue(false).await;
        let before: BoundRows = sqlx::query_as(BOUND_ROWS)
            .bind(job)
            .bind(request)
            .fetch_one(&r.owner)
            .await
            .unwrap();
        assert_eq!(before.0.0.status, "snapshot_queued");
        assert_eq!(before.0.0.attempt_count, 0);
        let JobInput::SnapshotExport {
            actor_id,
            space_id,
            request: input,
        } = JobInput::from_value(before.1.0.payload.clone()).unwrap()
        else {
            panic!("actual snapshot input required")
        };
        assert_eq!(actor_id, before.1.0.actor_id);
        assert!(!input.include_personal);
        assert!(!input.include_originals);
        assert_eq!(before.2.0.as_object().unwrap().len(), 4);
        assert_eq!(before.3.0.as_object().unwrap().len(), 4);
        assert_eq!(before.3.0["operation"], "snapshot_export");
        assert_eq!(before.2.0["request_id"], request.to_string());
        let no_receipt: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM public.snapshot_export_result WHERE job_id=$1",
        )
        .bind(job)
        .fetch_one(&r.owner)
        .await
        .unwrap();
        assert_eq!(no_receipt, 0);

        let (mut work, custody, path) = fixture(&r, None);
        let pending = work
            .start_publish(
                job,
                request,
                Duration::from_secs(60),
                r.runtime.clone(),
                &custody,
            )
            .expect("D1 same-original publication must be supported");
        let _published = pending
            .await
            .expect("D1 must complete the actual runtime transaction and durable close");
        work.drain().await.unwrap();
        assert!(!work.original().held());
        assert!(work.original().issued_for(job));
        assert_eq!(work.original().sql_calls.load(Ordering::SeqCst), 1);

        let after: BoundRows = sqlx::query_as(BOUND_ROWS)
            .bind(job)
            .bind(request)
            .fetch_one(&r.owner)
            .await
            .unwrap();
        let receipt: Receipt = sqlx::query_as("SELECT job_id,stage_token,capability,manifest_sha256,plan_sha256,required_spaces,created_at FROM public.snapshot_export_result WHERE job_id=$1")
            .bind(job).fetch_one(&r.owner).await.unwrap();
        assert_eq!(receipt.job_id, job);
        assert_eq!(receipt.capability, "reading_copy_v1");
        assert_eq!(receipt.required_spaces, vec![space_id]);
        assert_eq!(receipt.plan_sha256.len(), 64);
        assert!(
            receipt
                .plan_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
        assert!(receipt.created_at >= before.0.0.updated_at);
        assert_eq!(after.0.0.status, "succeeded");
        assert_eq!(after.0.0.attempt_count, 1);
        assert_eq!(after.0.0.id, before.0.0.id);
        assert_eq!(after.0.0.outbox_id, before.0.0.outbox_id);
        assert_eq!(after.0.0.idempotency_key, before.0.0.idempotency_key);
        assert_eq!(after.0.0.created_at, before.0.0.created_at);
        assert_eq!(after.0.0.checkpoint, before.0.0.checkpoint);
        assert_eq!(after.0.0.lease_token, None);
        assert_eq!(after.0.0.lease_expires_at, None);
        assert_eq!(after.0.0.next_attempt_at, None);
        assert_eq!(after.0.0.last_error_class, None);
        assert_eq!(
            after.0.0.output_digest.as_deref(),
            Some(receipt.manifest_sha256.as_str())
        );
        assert!(after.0.0.updated_at >= before.0.0.updated_at);
        assert_eq!(after.1.0, before.1.0);
        assert_eq!(after.2.0, before.2.0);
        assert_eq!(after.3.0, before.3.0);

        let records = frames(&path);
        assert_eq!(records.len(), 6);
        assert_eq!(records[0]["schema"], "knowweave.snapshot-claim.p1.intent");
        assert_eq!(
            records[1]["schema"],
            "knowweave.snapshot-claim.p1.committed"
        );
        assert_eq!(records[2]["event"], "WRITE_AUTHORITY_ISSUED");
        assert_eq!(records[3]["event"], "PACKAGE_STAGED");
        assert_eq!(records[4]["event"], "BUSINESS_COMMIT_KNOWN");
        assert_eq!(records[5]["event"], "WRITE_CLOSED");
        assert_eq!(records[4]["previous"]["sequence"], 4);
        assert_eq!(records[5]["previous"]["sequence"], 5);
        assert_eq!(
            records[1]["claim"]["token"],
            receipt.stage_token.to_string()
        );
        assert_eq!(records[3]["manifest_sha256"], receipt.manifest_sha256);
        assert_eq!(records[4]["before"]["job"], records[1]["after"]["job"]);
        assert_eq!(
            records[4]["before"]["outbox"],
            serde_json::to_value(&before.1.0).unwrap()
        );
        assert_eq!(
            records[4]["after"]["job"],
            serde_json::to_value(&after.0.0).unwrap()
        );
        assert_eq!(
            records[4]["after"]["outbox"],
            serde_json::to_value(&after.1.0).unwrap()
        );
        for side in ["before", "after"] {
            assert_eq!(records[4][side]["request"], before.2.0);
            assert_eq!(records[4][side]["key"], before.3.0);
        }
        assert!(records[4]["before"]["receipt"].is_null());
        assert_eq!(
            records[4]["after"]["receipt"],
            serde_json::to_value(&receipt).unwrap()
        );

        // Independently read and hash every actual package byte after the
        // original operation has settled, without reopening any capability.
        let capture = custody
            .try_capture()
            .expect("SH released after durable close");
        capture.revalidate().unwrap();
        let ready = path
            .join("exports")
            .join(job.to_string())
            .join(format!("{}.ready", receipt.stage_token));
        let manifest_bytes = std::fs::read(ready.join("manifest.json")).unwrap();
        assert_eq!(hex_digest(&manifest_bytes), receipt.manifest_sha256);
        let SnapshotManifest::ReadingCopyV1(manifest) =
            serde_json::from_slice(&manifest_bytes).unwrap()
        else {
            panic!("actual reading-copy manifest required")
        };
        assert_eq!(manifest.copy_id, job);
        assert_eq!(manifest.files.len(), 3);
        let names: Vec<_> = manifest.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(names, ["reading.html", "reading.md", "validation.json"]);
        let mut verified_bytes = 0u64;
        for file in manifest.files {
            let bytes = std::fs::read(ready.join(&file.path)).unwrap();
            assert_eq!(bytes.len() as u64, file.size);
            assert_eq!(hex_digest(&bytes), file.sha256);
            verified_bytes += file.size;
        }
        assert!(verified_bytes > 100);
        assert_eq!(records[3]["verified_files"], 3);
        assert_eq!(records[3]["verified_bytes"], verified_bytes);
        drop(capture);
        r.close().await;
    }

    async fn actual_input(r: &Rig, job: Uuid) -> (Principal, Uuid, SnapshotRequest) {
        let payload: serde_json::Value =
            sqlx::query_scalar("SELECT payload FROM public.job_outbox WHERE id=$1")
                .bind(job)
                .fetch_one(&r.owner)
                .await
                .unwrap();
        let JobInput::SnapshotExport {
            actor_id,
            space_id,
            request,
        } = JobInput::from_value(payload).unwrap()
        else {
            panic!("actual snapshot input required")
        };
        (Principal { actor_id }, space_id, request)
    }
    async fn business_fact(r: &Rig, job: Uuid) -> (String, i32, i64) {
        sqlx::query_as("SELECT status,attempt_count,(SELECT count(*) FROM public.snapshot_export_result WHERE job_id=$1) FROM public.job WHERE id=$1")
            .bind(job).fetch_one(&r.owner).await.unwrap()
    }
    async fn cross_space_copy(r: &Rig) -> (Uuid, Uuid, Principal, Uuid) {
        let (seed, _) = r.enqueue(false).await;
        let (actor, root_space, _) = actual_input(r, seed).await;
        let foreign = Uuid::new_v4();
        sqlx::query("INSERT INTO public.space(id,owner_id) VALUES($1,$2)")
            .bind(foreign)
            .bind(actor.actor_id)
            .execute(&r.owner)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO public.space_grant(actor_id,space_id,can_write) VALUES($1,$2,true)",
        )
        .bind(actor.actor_id)
        .bind(foreign)
        .execute(&r.owner)
        .await
        .unwrap();
        let block = ContentStore::new(r.runtime.clone()).create(actor, foreign, CreateCommand {
            request_id: Uuid::new_v4(), reason: "D1 foreign original fixture".into(),
            draft: serde_json::from_value(serde_json::json!({"kind":"text", "intent":"note", "language":"zh-CN", "title":"Foreign original", "payload":{"format":"markdown","text":"Foreign actual original must change projection after grant revocation."}})).unwrap(),
        }).await.unwrap();
        let document = CompositionStore::new(r.runtime.clone())
            .save(
                actor,
                root_space,
                SaveComposition {
                    request_id: Uuid::new_v4(),
                    composition_id: None,
                    base_revision_id: None,
                    kind: CompositionKind::Document,
                    title: "D1 foreign original document".into(),
                    nodes: vec![NodeDraft {
                        occurrence_id: None,
                        target: NodeTarget::Block(BlockRef {
                            block_id: block.block_id,
                            revision_id: block.revision_id,
                        }),
                    }],
                    reason: "D1 actual cross-space fixture".into(),
                },
            )
            .await
            .unwrap();
        let reading = ReadingStore::new(r.runtime.clone())
            .create(
                actor,
                root_space,
                CreateReading {
                    request_id: Uuid::new_v4(),
                    base: document.reference,
                    title: "D1 drift".into(),
                    reason: "D1 actual cross-space reading".into(),
                },
            )
            .await
            .unwrap();
        let request = Uuid::new_v4();
        let job = SnapshotStore::new(r.runtime.clone())
            .enqueue_export(
                actor,
                request,
                SnapshotRequest {
                    reading: reading.view,
                    mode: ReadingMode::Fused,
                    include_personal: false,
                    include_originals: false,
                    resource_versions: vec![],
                    source_segments: vec![],
                },
            )
            .await
            .unwrap();
        JobStore::new(r.runtime.clone())
            .dispatch_pending(256)
            .await
            .unwrap();
        (job, request, actor, foreign)
    }

    #[tokio::test]
    #[ignore = "fresh PG18 four actual LOGINs and Root0 /run; no skip"]
    async fn d1_runtime_recheck_rejects_revocation_or_changed_plan() {
        let r = Rig::new().await;
        observer_setup(&r).await;
        for drift in [false, true] {
            let (job, request, actor, revoked_space) = if drift {
                cross_space_copy(&r).await
            } else {
                let (job, request) = r.enqueue(false).await;
                let (actor, space, _) = actual_input(&r, job).await;
                (job, request, actor, space)
            };
            let (_, _, input) = actual_input(&r, job).await;
            let before_projection = ReadingStore::new(r.runtime.clone())
                .read_versioned(actor, input.reading.clone(), input.mode)
                .await
                .unwrap()
                .unwrap();
            let before_copy =
                learning_assets::sanitize_reading_copy(&before_projection, input.mode, false)
                    .unwrap();
            let h = hooks(Stage::BeforePublish);
            let _release = Release(h.clone());
            let (mut work, custody, path) = fixture(&r, Some(h.clone()));
            let pending = work
                .start_publish(
                    job,
                    request,
                    Duration::from_secs(60),
                    r.runtime.clone(),
                    &custody,
                )
                .unwrap();
            arrived(&h).await;
            assert_eq!(frames(&path).len(), 4);
            assert!(custody.try_capture().is_err());
            assert_eq!(
                sqlx::query("DELETE FROM public.space_grant WHERE actor_id=$1 AND space_id=$2")
                    .bind(actor.actor_id)
                    .bind(revoked_space)
                    .execute(&r.owner)
                    .await
                    .unwrap()
                    .rows_affected(),
                1
            );
            if drift {
                let after_projection = ReadingStore::new(r.runtime.clone())
                    .read_versioned(actor, input.reading.clone(), input.mode)
                    .await
                    .unwrap()
                    .unwrap();
                let after_copy =
                    learning_assets::sanitize_reading_copy(&after_projection, input.mode, false)
                        .unwrap();
                assert_ne!(
                    before_copy, after_copy,
                    "actual foreign grant changes the fixed original's clipped copy"
                );
            }
            h.resume.notify_one();
            assert!(pending.await.is_err());
            work.drain().await.unwrap();
            assert!(work.original().held());
            assert!(work.original().issued_for(job));
            assert_eq!(frames(&path).len(), 4);
            assert_eq!(
                business_fact(&r, job).await,
                ("snapshot_running".into(), 1, 0)
            );
            assert!(custody.try_capture().is_ok());
        }
        r.close().await;
    }

    #[tokio::test]
    #[ignore = "fresh PG18 four actual LOGINs and Root0 /run; no skip"]
    async fn d1_original_token_or_expiry_drift_prevents_publication() {
        let r = Rig::new().await;
        observer_setup(&r).await;
        for cancel in [false, true] {
            let (job, request) = r.enqueue(false).await;
            let h = hooks(Stage::BeforePublish);
            let _release = Release(h.clone());
            let (mut work, custody, path) = fixture(&r, Some(h.clone()));
            let pending = work
                .start_publish(
                    job,
                    request,
                    Duration::from_secs(60),
                    r.runtime.clone(),
                    &custody,
                )
                .unwrap();
            arrived(&h).await;
            let (token, expiry): (Uuid, chrono::DateTime<chrono::Utc>) =
                sqlx::query_as("SELECT lease_token,lease_expires_at FROM public.job WHERE id=$1")
                    .bind(job)
                    .fetch_one(&r.owner)
                    .await
                    .unwrap();
            if cancel {
                assert!(JobStore::new(r.runtime.clone()).cancel(job).await.unwrap());
                let invalidated: (Option<Uuid>, Option<chrono::DateTime<chrono::Utc>>) =
                    sqlx::query_as(
                        "SELECT lease_token,lease_expires_at FROM public.job WHERE id=$1",
                    )
                    .bind(job)
                    .fetch_one(&r.owner)
                    .await
                    .unwrap();
                assert_eq!(
                    invalidated,
                    (None, None),
                    "canonical cancel invalidates the token; this is not token takeover"
                );
            } else {
                assert!(
                    JobStore::new(r.runtime.clone())
                        .renew(job, token, Duration::from_secs(300))
                        .await
                        .unwrap()
                );
                let renewed: (Uuid, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
                    "SELECT lease_token,lease_expires_at FROM public.job WHERE id=$1",
                )
                .bind(job)
                .fetch_one(&r.owner)
                .await
                .unwrap();
                assert_eq!(renewed.0, token);
                assert_ne!(
                    renewed.1, expiry,
                    "actual canonical renewal changes expiry; not natural expiry"
                );
            }
            h.resume.notify_one();
            assert!(pending.await.is_err());
            work.drain().await.unwrap();
            assert!(work.original().held());
            assert!(work.original().issued_for(job));
            assert_eq!(frames(&path).len(), 4);
            assert_eq!(
                business_fact(&r, job).await,
                (
                    if cancel {
                        "cancelled"
                    } else {
                        "snapshot_running"
                    }
                    .into(),
                    1,
                    0
                )
            );
            assert!(custody.try_capture().is_ok());
        }
        r.close().await;
    }

    #[tokio::test]
    #[ignore = "fresh PG18 four actual LOGINs and Root0 /run; no skip"]
    async fn d1_original_commit_knowledge_loss_stays_hold_without_adoption() {
        let r = Rig::new().await;
        observer_setup(&r).await;
        let (job, request) = r.enqueue(false).await;
        let (mut work, custody, path) =
            fixture(&r, Some(hooks(Stage::LoseBusinessCommitKnowledge)));
        let pending = work
            .start_publish(
                job,
                request,
                Duration::from_secs(60),
                r.runtime.clone(),
                &custody,
            )
            .unwrap();
        assert!(pending.await.is_err());
        work.drain().await.unwrap();
        // This explicitly loses the already-executed original commit knowledge;
        // it does not simulate a network fault or adopt the independently read DB.
        assert_eq!(business_fact(&r, job).await, ("succeeded".into(), 1, 1));
        assert_eq!(frames(&path).len(), 4);
        assert!(work.original().held());
        assert!(work.original().issued_for(job));
        assert!(
            work.start_publish(
                job,
                request,
                Duration::from_secs(60),
                r.runtime.clone(),
                &custody
            )
            .is_err()
        );
        assert_eq!(work.original().sql_calls.load(Ordering::SeqCst), 1);
        assert!(custody.try_capture().is_ok());
        r.close().await;
    }

    #[tokio::test]
    #[ignore = "fresh PG18 four actual LOGINs and Root0 /run; no skip"]
    async fn d1_business_or_close_append_failure_stays_hold() {
        let r = Rig::new().await;
        observer_setup(&r).await;
        for (stage, ordinal) in [
            (Stage::BeforeBusinessEvent, 5),
            (Stage::BeforeCloseEvent, 6),
        ] {
            let (job, request) = r.enqueue(false).await;
            let h = hooks(stage);
            let _release = Release(h.clone());
            let (mut work, custody, path) = fixture(&r, Some(h.clone()));
            let pending = work
                .start_publish(
                    job,
                    request,
                    Duration::from_secs(60),
                    r.runtime.clone(),
                    &custody,
                )
                .unwrap();
            arrived(&h).await;
            assert_eq!(business_fact(&r, job).await, ("succeeded".into(), 1, 1));
            assert_eq!(frames(&path).len(), ordinal - 1);
            assert!(custody.try_capture().is_err());
            // Actual durable journal prefix rejection on a retained unknown
            // pending file. This does not claim a private fsync or ACK seam.
            let residue = path.join("journal").join(format!(".{ordinal:020}.pending"));
            std::fs::write(&residue, b"D1 Root fixture retained unknown pending").unwrap();
            h.resume.notify_one();
            assert!(pending.await.is_err());
            work.drain().await.unwrap();
            assert_eq!(frames(&path).len(), ordinal - 1);
            assert_eq!(
                std::fs::read(residue).unwrap(),
                b"D1 Root fixture retained unknown pending"
            );
            assert!(work.original().held());
            assert!(work.original().issued_for(job));
            assert!(
                work.start_publish(
                    job,
                    request,
                    Duration::from_secs(60),
                    r.runtime.clone(),
                    &custody
                )
                .is_err()
            );
            assert_eq!(work.original().sql_calls.load(Ordering::SeqCst), 1);
            assert!(custody.try_capture().is_ok());
        }
        r.close().await;
    }

    #[tokio::test]
    #[ignore = "fresh PG18 four actual LOGINs and Root0 /run; no skip"]
    async fn d1_started_commit_or_append_cancel_retains_shared_custody_until_settled() {
        let r = Rig::new().await;
        observer_setup(&r).await;
        for (stage, expected_frames, blocking) in [
            (Stage::AfterBusinessCommit, 4, false),
            (Stage::AfterBusinessEvent, 5, true),
            (Stage::AfterCloseEvent, 6, true),
        ] {
            for expire in [false, true] {
                let (job, request) = r.enqueue(false).await;
                let mut h = hooks(stage);
                Arc::get_mut(&mut h).unwrap().blocking_return = blocking;
                let _release = Release(h.clone());
                let (mut work, custody, path) = fixture(&r, Some(h.clone()));
                let pending = work
                    .start_publish(
                        job,
                        request,
                        Duration::from_secs(if expire { 10 } else { 60 }),
                        r.runtime.clone(),
                        &custody,
                    )
                    .unwrap();
                arrived(&h).await;
                assert_eq!(frames(&path).len(), expected_frames);
                assert_eq!(business_fact(&r, job).await, ("succeeded".into(), 1, 1));
                if expire {
                    assert!(pending.await.is_err());
                } else {
                    drop(pending);
                }
                assert!(work.original().held());
                assert!(work.original().issued_for(job));
                assert!(custody.try_capture().is_err());
                {
                    let drain = work.drain();
                    tokio::pin!(drain);
                    tokio::select! { _=&mut drain=>panic!("D1 original started commit/append detached"), _=tokio::time::sleep(Duration::from_millis(10))=>{} }
                }
                assert!(work.original().active());
                assert!(custody.try_capture().is_err());
                if blocking {
                    *h.blocking_release.0.lock().unwrap() = true;
                    h.blocking_release.1.notify_all();
                } else {
                    h.resume.notify_one();
                }
                work.drain().await.unwrap();
                assert!(!work.original().active());
                assert!(work.original().held());
                assert_eq!(frames(&path).len(), expected_frames);
                assert_eq!(work.original().sql_calls.load(Ordering::SeqCst), 1);
                assert!(custody.try_capture().is_ok());
            }
        }
        r.close().await;
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

    #[tokio::test]
    #[ignore = "fresh PG18 four actual LOGINs and Root0 /run; no skip"]
    async fn formal_quiesce_cancellation_retains_original_append_and_effect_sh_until_settled() {
        let r = Rig::new().await;
        let mut h = hooks(Stage::AfterQuiesceAppend);
        Arc::get_mut(&mut h).unwrap().blocking_return = true;
        let _release = Release(h.clone());
        let (work, custody, path) = fixture(&r, Some(h.clone()));
        let original = work.owner();
        let retention = custody.writer().unwrap();
        let pending = work
            .start_quiesce(move |mut work, owner, slot| async move {
                work.drain().await?;
                let _retention = retention;
                let bytes = super::super::history::encode(
                    &serde_json::json!({"test_component":"original_control_append"}),
                    &owner.budget,
                )?;
                owner.append(&slot, bytes).await?;
                slot.live()
            })
            .unwrap();
        arrived(&h).await;
        assert_eq!(
            frames(&path).len(),
            1,
            "real durable frame exists before its original append returns"
        );
        assert!(custody.try_capture().is_err());
        drop(pending);
        assert!(original.held());
        let waiting_original = original.clone();
        let mut waiting = tokio::spawn(async move { waiting_original.settle_control().await });
        assert!(
            timeout(Duration::from_millis(100), &mut waiting)
                .await
                .is_err(),
            "outer cancellation must not detach the real append"
        );
        assert!(
            custody.try_capture().is_err(),
            "retained SH prevents EX while actual append is blocked"
        );
        *h.blocking_release.0.lock().unwrap() = true;
        h.blocking_release.1.notify_all();
        waiting.await.unwrap().unwrap();
        assert!(original.held());
        assert_eq!(frames(&path).len(), 1);
        assert!(custody.try_capture().is_ok());
        r.close().await;
    }

    #[tokio::test]
    #[ignore = "fresh PG18 four actual LOGINs and Root0 /run; no skip"]
    async fn formal_quiesce_cancellation_drains_original_blocking_worker_before_releasing_sh() {
        let r = Rig::new().await;
        let (job, request) = r.enqueue(false).await;
        let h = hooks(Stage::AfterStageCopy);
        let _release = Release(h.clone());
        let (mut work, custody, path) = fixture(&r, Some(h.clone()));
        let original = work.owner();
        let worker = work
            .start_copy(
                job,
                request,
                Duration::from_secs(60),
                r.runtime.clone(),
                &custody,
            )
            .unwrap();
        arrived(&h).await;
        let pending = work
            .start_quiesce(|mut work, _owner, slot| async move {
                work.drain().await?;
                slot.live()
            })
            .unwrap();
        drop(pending);
        drop(worker);
        let waiting_original = original.clone();
        let mut waiting = tokio::spawn(async move { waiting_original.settle_control().await });
        assert!(
            timeout(Duration::from_millis(100), &mut waiting)
                .await
                .is_err()
        );
        assert!(
            custody.try_capture().is_err(),
            "real worker still owns its staged package SH and supervisor SH"
        );
        *h.blocking_release.0.lock().unwrap() = true;
        h.blocking_release.1.notify_all();
        waiting.await.unwrap().unwrap();
        assert!(original.held());
        assert!(original.issued_for(job));
        assert_eq!(
            frames(&path).len(),
            3,
            "cancelled issued worker cannot append package or CUT"
        );
        r.no_result(job).await;
        assert!(custody.try_capture().is_ok());
        r.close().await;
    }
}

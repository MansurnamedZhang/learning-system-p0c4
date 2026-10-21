//! Explicit component fixtures: real C LOGIN + original SQL + actual Root byte
//! journal. These tests are ignored, never silently skip, and mint no public cap.
use super::{
    history::{
        self, Intent, Outcome,
        runtime::{self, EpochWork, Hooks, Stage},
    },
    install,
};
use crate::{JobStore, MIGRATOR};
use learning_core::{JobInput, ReadingMode, ReadingRef, SnapshotRequest};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, atomic::Ordering},
    time::Duration,
};
use tokio::{
    sync::{Mutex as AsyncMutex, Notify},
    time::timeout,
};
use uuid::Uuid;
static SERIAL: AsyncMutex<()> = AsyncMutex::const_new(());

struct Rig {
    owner: PgPool,
    runtime: PgPool,
    broker: PgPool,
    cluster: PgPool,
}
async fn pool(role: &str) -> PgPool {
    let url = std::env::var(format!("TEST_SNAPSHOT_CLAIM_P1_{role}_DATABASE_URL"))
        .expect("explicit fresh P1 PG18 DSN; no skip");
    PgPoolOptions::new()
        .max_connections(3)
        .connect(&url)
        .await
        .expect("real P1 LOGIN")
}
async fn identity(pool: &PgPool) -> (String, String, String) {
    sqlx::query_as("SELECT session_user::text,current_user::text,current_database()")
        .fetch_one(pool)
        .await
        .unwrap()
}
impl Rig {
    async fn new() -> Self {
        let owner = pool("ADMIN").await;
        let runtime = pool("RUNTIME").await;
        let cluster = pool("CLUSTER").await;
        let id = identity(&owner).await;
        assert_eq!((&*id.0, &*id.1), ("learning_admin", "learning_admin"));
        let suffix =
            id.2.strip_prefix("learning_task6_claim_p1_")
                .expect("fresh P1 database only");
        assert_eq!(Uuid::parse_str(suffix).unwrap().to_string(), suffix);
        assert_eq!(
            identity(&runtime).await,
            (
                "learning_runtime".into(),
                "learning_runtime".into(),
                id.2.clone()
            )
        );
        assert_eq!(
            identity(&cluster).await,
            ("postgres".into(), "postgres".into(), id.2.clone())
        );
        MIGRATOR.run(&owner).await.unwrap();
        let rows: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM public.job),(SELECT count(*) FROM public.job_outbox)",
        )
        .fetch_one(&owner)
        .await
        .unwrap();
        assert_eq!(rows, (0, 0));
        // The existing installer validates the separately provisioned cluster
        // role and installs only the adopted C profile. No temporary grants.
        install(&owner).await.unwrap();
        let broker = pool("BROKER").await;
        assert_eq!(
            identity(&broker).await,
            (
                "learning_snapshot_claim_broker".into(),
                "learning_snapshot_claim_broker".into(),
                id.2
            )
        );
        let facts = "SELECT (SELECT oid::bigint FROM pg_catalog.pg_database WHERE datname=current_database()),pg_catalog.pg_postmaster_start_time()::text";
        let expected: (i64, String) = sqlx::query_as(facts).fetch_one(&owner).await.unwrap();
        let options = owner.connect_options();
        for other in [&runtime, &broker, &cluster] {
            let actual: (i64, String) = sqlx::query_as(facts).fetch_one(other).await.unwrap();
            assert_eq!(actual, expected);
            let c = other.connect_options();
            assert_eq!(c.get_host(), options.get_host());
            assert_eq!(c.get_port(), options.get_port());
            assert_eq!(c.get_database(), options.get_database());
        }
        Self {
            owner,
            runtime,
            broker,
            cluster,
        }
    }
    async fn seed(&self) -> Uuid {
        let actor = Uuid::new_v4();
        let id = Uuid::new_v4();
        let input = JobInput::snapshot_export(
            actor,
            Uuid::new_v4(),
            SnapshotRequest {
                reading: ReadingRef {
                    view_id: Uuid::new_v4(),
                    revision_id: Uuid::new_v4(),
                },
                mode: ReadingMode::Fused,
                include_personal: true,
                include_originals: true,
                resource_versions: vec![],
                source_segments: vec![],
            },
        )
        .unwrap();
        sqlx::query("INSERT INTO public.app_user(id) VALUES($1)")
            .bind(actor)
            .execute(&self.owner)
            .await
            .unwrap();
        sqlx::query("INSERT INTO public.job_outbox(id,business_key,event_type,payload_version,payload,actor_id,processor_version) VALUES($1,$2,'snapshot_export_requested',1,$3,$4,1)").bind(id).bind(input.business_key()).bind(input.to_value()).bind(actor).execute(&self.owner).await.unwrap();
        JobStore::new(self.runtime.clone())
            .dispatch_pending(256)
            .await
            .unwrap();
        id
    }
    async fn row(&self, id: Uuid) -> serde_json::Value {
        sqlx::query_scalar("SELECT to_jsonb(j) FROM public.job j WHERE id=$1")
            .bind(id)
            .fetch_one(&self.owner)
            .await
            .unwrap()
    }
    async fn cleanup(self) {
        self.broker.close().await;
        sqlx::query("DROP FUNCTION public.p0c4_snapshot_claim_locked_observation(uuid,boolean)")
            .execute(&self.owner)
            .await
            .unwrap();
        sqlx::query("REVOKE USAGE ON SCHEMA public FROM learning_snapshot_claim_broker")
            .execute(&self.owner)
            .await
            .unwrap();
        let sql:String=sqlx::query_scalar("SELECT format('REVOKE CONNECT ON DATABASE %I FROM learning_snapshot_claim_broker',current_database())").fetch_one(&self.owner).await.unwrap();
        sqlx::query(&sql).execute(&self.owner).await.unwrap();
        sqlx::raw_sql("REVOKE EXECUTE ON FUNCTION public.p0c3_claim_snapshot_job(uuid,bigint),public.p0c3_renew_snapshot_job(uuid,uuid,bigint),public.p0c3_fail_snapshot_job(uuid,uuid,text,boolean) FROM learning_snapshot_claim_broker").execute(&self.owner).await.unwrap();
        self.runtime.close().await;
        self.owner.close().await;
        self.cluster.close().await;
    }
}
fn root() -> PathBuf {
    PathBuf::from("/run").join(format!("task6-p1-{}", Uuid::new_v4()))
}
fn hooks(stage: Stage, blocking_return: bool, lose_commit_knowledge: bool) -> Arc<Hooks> {
    Arc::new(Hooks {
        stage,
        arrived: Notify::new(),
        resume: Notify::new(),
        lose_commit_knowledge,
        blocking_return,
        blocking_release: (Mutex::new(false), std::sync::Condvar::new()),
        post_spawn_join: false,
        spawned: Notify::new(),
        join_resume: Notify::new(),
    })
}
// Ensure a failing assertion cannot strand the actual blocking fixture forever.
// This releases private test gates; it never aborts an original operation.
struct ReleaseOnDrop(Arc<Hooks>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        let (lock, wake) = &self.0.blocking_release;
        if let Ok(mut value) = lock.lock() {
            *value = true;
            wake.notify_all();
        }
        self.0.join_resume.notify_one();
        self.0.resume.notify_one();
    }
}
fn fixture(r: &Rig, h: Option<Arc<Hooks>>) -> (EpochWork, PathBuf) {
    let path = root();
    (
        runtime::real_fixture(r.broker.clone(), &path, h)
            .expect("Root0 real private journal required"),
        path,
    )
}
async fn arrived(h: &Hooks) {
    timeout(Duration::from_secs(30), h.arrived.notified())
        .await
        .expect("actual owned phase reached before cancellation");
}
fn frames(path: &Path) -> Vec<PathBuf> {
    let mut names: Vec<_> = std::fs::read_dir(path.join("journal"))
        .unwrap()
        .map(|x| x.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "frame"))
        .collect();
    names.sort();
    names
}
fn body(p: &Path) -> Vec<u8> {
    let bytes = std::fs::read(p).unwrap();
    assert_eq!(&bytes[..8], b"KWSJBYT2");
    assert_eq!(
        u64::from_be_bytes(bytes[112..120].try_into().unwrap()) as usize,
        bytes.len() - 120
    );
    bytes[120..].to_vec()
}

#[tokio::test]
#[ignore = "explicit fresh PG18 four real LOGINs and Root0 private /run; no skip"]
async fn p1_fresh_claim_known_commit_has_exact_durable_intent_and_outcome() {
    let _serial = SERIAL.lock().await;
    let r = Rig::new().await;
    let id = r.seed().await;
    let (mut work, path) = fixture(&r, None);
    let result = work
        .start(id, Uuid::new_v4(), Duration::from_secs(60))
        .unwrap()
        .await
        .unwrap();
    assert_eq!(result.sequences(), (1, 2));
    assert_eq!(work.original().sql_calls.load(Ordering::SeqCst), 1);
    let f = frames(&path);
    assert_eq!(f.len(), 2);
    let intent: Intent = serde_json::from_slice(&body(&f[0])).unwrap();
    let outcome: Outcome = serde_json::from_slice(&body(&f[1])).unwrap();
    assert!(matches!(
        intent.source_build.mode,
        history::SourceMode::UnenrolledComponent
    ));
    assert!(intent.source_build.external_reference_not_authority);
    assert_eq!(
        intent.source_build.execution_binding_sha256,
        std::env::var("TEST_SNAPSHOT_CLAIM_P1_EXECUTION_BINDING_SHA256").unwrap()
    );
    assert_eq!(intent.before.job.status, "snapshot_queued");
    assert_eq!(intent.before.job.attempt_count, 0);
    assert_eq!(outcome.after.job.status, "snapshot_running");
    assert_eq!(outcome.after.job.attempt_count, 1);
    assert!(intent.before.outbox == outcome.after.outbox);
    assert!(intent.identity == outcome.identity);
    assert!(matches!(
        outcome.commit,
        history::CommitState::OriginalKnown
    ));
    let actual: crate::snapshot_observation::JobObservation =
        serde_json::from_value(r.row(id).await).unwrap();
    assert!(outcome.after.job == actual);
    assert!(
        work.start(id, Uuid::new_v4(), Duration::from_secs(60))
            .is_err()
    );
    work.drain().await.unwrap();
    r.cleanup().await;
}

#[tokio::test]
#[ignore = "explicit fresh PG18 four real LOGINs and Root0 private /run; no skip"]
async fn p1_unsupported_rows_issue_neither_intent_nor_claim() {
    let _serial = SERIAL.lock().await;
    let r = Rig::new().await;
    for existing in [false, true] {
        let id = if existing {
            r.seed().await
        } else {
            Uuid::new_v4()
        };
        if existing {
            sqlx::query("SELECT * FROM public.p0c3_claim_snapshot_job($1,300000)")
                .bind(id)
                .fetch_all(&r.runtime)
                .await
                .unwrap();
        }
        let before = if existing {
            Some(r.row(id).await)
        } else {
            None
        };
        let (mut work, path) = fixture(&r, None);
        assert!(
            work.start(id, Uuid::new_v4(), Duration::from_secs(60))
                .unwrap()
                .await
                .is_err()
        );
        assert_eq!(work.original().sql_calls.load(Ordering::SeqCst), 0);
        assert!(frames(&path).is_empty());
        if let Some(before) = before {
            assert!(r.row(id).await == before);
        }
        work.drain().await.unwrap();
    }
    r.cleanup().await;
}

#[tokio::test]
#[ignore = "explicit fresh PG18 four real LOGINs and Root0 private /run; no skip"]
async fn p1_cancelled_receiver_keeps_original_owner_and_never_publishes_late() {
    let _serial = SERIAL.lock().await;
    let r = Rig::new().await;
    for stage in [
        Stage::AfterIntent,
        Stage::AfterClaim,
        Stage::BeforeCommit,
        Stage::AfterCommit,
        Stage::BeforeOutcome,
        Stage::AfterOutcome,
    ] {
        let id = r.seed().await;
        let h = hooks(stage, false, false);
        let (mut work, _) = fixture(&r, Some(h.clone()));
        let pending = work
            .start(id, Uuid::new_v4(), Duration::from_secs(60))
            .unwrap();
        arrived(&h).await;
        drop(pending);
        if stage == Stage::AfterClaim {
            // Original owner is at the already observed private barrier. A
            // cancelled drain must not take/drop its live settlement handle.
            {
                let mut draining = Box::pin(work.drain());
                tokio::select! {
                    value=&mut draining => panic!("paused owner unexpectedly drained: {}",value.is_ok()),
                    _=tokio::time::sleep(Duration::from_millis(5))=>{}
                }
            }
            let mut draining = Box::pin(work.drain());
            tokio::select! {
                _=&mut draining=>panic!("original settlement owner was lost"),
                _=tokio::time::sleep(Duration::from_millis(5))=>{}
            }
            h.resume.notify_one();
            draining.await.unwrap();
        } else {
            h.resume.notify_one();
            work.drain().await.unwrap();
        }
        assert!(
            work.start(id, Uuid::new_v4(), Duration::from_secs(60))
                .is_err()
        );
        let row = r.row(id).await;
        let committed = matches!(
            stage,
            Stage::AfterCommit | Stage::BeforeOutcome | Stage::AfterOutcome
        );
        assert_eq!(row["attempt_count"], if committed { 1 } else { 0 });
        assert_eq!(
            work.original().sql_calls.load(Ordering::SeqCst),
            if stage == Stage::AfterIntent { 0 } else { 1 }
        );
    }
    r.cleanup().await;
}

#[tokio::test]
#[ignore = "explicit fresh PG18 four real LOGINs and Root0 private /run; no skip"]
async fn p1_durable_boundaries_and_lost_commit_knowledge_never_mint_result() {
    let _serial = SERIAL.lock().await;
    let r = Rig::new().await;
    for stage in [
        Stage::BeforeIntent,
        Stage::BeforeOutcome,
        Stage::AfterCommit,
    ] {
        let id = r.seed().await;
        let h = hooks(stage, false, stage == Stage::AfterCommit);
        let (mut work, path) = fixture(&r, Some(h.clone()));
        let pending = work
            .start(id, Uuid::new_v4(), Duration::from_secs(60))
            .unwrap();
        arrived(&h).await;
        if stage != Stage::AfterCommit {
            std::fs::rename(
                path.join("journal/epoch.header"),
                path.join("journal/epoch.moved-by-fixture"),
            )
            .unwrap();
        }
        h.resume.notify_one();
        assert!(pending.await.is_err());
        work.drain().await.unwrap();
        assert_eq!(
            work.original().sql_calls.load(Ordering::SeqCst),
            if stage == Stage::BeforeIntent { 0 } else { 1 }
        );
        assert_eq!(
            r.row(id).await["attempt_count"],
            if stage == Stage::BeforeIntent { 0 } else { 1 }
        );
        assert_eq!(
            frames(&path).len(),
            if stage == Stage::BeforeIntent { 0 } else { 1 }
        );
    }
    // These are actual prefix rejection and deliberate client knowledge loss;
    // they do not claim coverage of B's private fsync seams or network ACK loss.
    r.cleanup().await;
}

#[tokio::test]
#[ignore = "explicit fresh PG18 four real LOGINs and Root0 private /run; no skip"]
async fn p1_singleflight_and_deadline_fence_precede_first_await() {
    let _serial = SERIAL.lock().await;
    let r = Rig::new().await;
    let id = r.seed().await;
    let request = Uuid::new_v4();
    let h = hooks(Stage::AfterOutcome, true, false);
    let _release = ReleaseOnDrop(h.clone());
    let (mut work, path) = fixture(&r, Some(h.clone()));
    let pending = work.start(id, request, Duration::from_secs(10)).unwrap();
    assert!(work.start(id, request, Duration::from_secs(60)).is_err());
    assert!(
        work.start(id, Uuid::new_v4(), Duration::from_secs(60))
            .is_err()
    );
    arrived(&h).await;
    assert_eq!(frames(&path).len(), 2);
    assert!(pending.await.is_err());
    // The actual B append has returned to its original blocking closure, which
    // remains held here beyond the authority deadline. Never detach its owner.
    let (lock, wake) = &h.blocking_release;
    *lock.lock().unwrap() = true;
    wake.notify_one();
    work.drain().await.unwrap();
    assert_eq!(work.original().sql_calls.load(Ordering::SeqCst), 1);
    assert!(work.start(id, request, Duration::from_secs(60)).is_err());
    // Deterministic post-spawn cancellation: both the original async owner and
    // actual blocking append are observed at their separate private barriers.
    let id = r.seed().await;
    let mut h = hooks(Stage::AfterOutcome, true, false);
    Arc::get_mut(&mut h).unwrap().post_spawn_join = true;
    let _release2 = ReleaseOnDrop(h.clone());
    let (mut work, path) = fixture(&r, Some(h.clone()));
    let request = Uuid::new_v4();
    let pending = work.start(id, request, Duration::from_secs(60)).unwrap();
    timeout(Duration::from_secs(30), h.spawned.notified())
        .await
        .expect("actual post-spawn-before-join reached");
    arrived(&h).await;
    drop(pending);
    h.join_resume.notify_one();
    let mut draining = Box::pin(work.drain());
    tokio::select! {
        _=&mut draining=>panic!("blocking append detached after caller cancellation"),
        _=tokio::time::sleep(Duration::from_millis(10))=>{}
    }
    assert_eq!(frames(&path).len(), 2);
    let (lock, wake) = &h.blocking_release;
    *lock.lock().unwrap() = true;
    wake.notify_all();
    draining.await.unwrap();
    assert!(work.start(id, request, Duration::from_secs(60)).is_err());
    assert_eq!(work.original().sql_calls.load(Ordering::SeqCst), 1);
    r.cleanup().await;
}

#[tokio::test]
#[ignore = "explicit fresh PG18 four real LOGINs and Root0 private /run; no skip"]
async fn p1_same_work_budget_and_typed_record_bounds_are_enforced() {
    let _serial = SERIAL.lock().await;
    let r = Rig::new().await;
    let id = r.seed().await;
    let (mut work, path) = fixture(&r, None);
    let usage = work.original().budget.usage().unwrap();
    work.original()
        .budget
        .charge(512 * 1024 * 1024 - usage.charged_bytes - 1024 * 1024, 0)
        .unwrap();
    assert!(
        work.start(id, Uuid::new_v4(), Duration::from_secs(60))
            .unwrap()
            .await
            .is_err()
    );
    assert_eq!(work.original().sql_calls.load(Ordering::SeqCst), 0);
    assert!(frames(&path).is_empty());
    let usage = work.original().budget.usage().unwrap();
    assert!(usage.charged_bytes > 512 * 1024 * 1024 - 1024 * 1024);
    work.drain().await.unwrap();
    r.cleanup().await;
}

//! Explicit fresh PostgreSQL fixtures; no absent-DSN skip and no positive
//! snapshot/export authority. Root provisions the separate real LOGIN first.
use learning_core::{JobInput, ReadingMode, ReadingRef, SnapshotRequest};
use learning_db::{
    JobStore, MIGRATOR,
    snapshot_observation::{
        self as a1, ObservationBudget, ObservationError, ObservationTransaction,
    },
};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::time::Duration;
use tokio::sync::Mutex;
use uuid::Uuid;

static SERIAL: Mutex<()> = Mutex::const_new(());
struct Rig {
    owner: PgPool,
    runtime: PgPool,
    broker: PgPool,
    cluster: PgPool,
}
impl Rig {
    async fn new() -> Self {
        let owner = connect("TEST_SNAPSHOT_OBSERVATION_ADMIN_DATABASE_URL").await;
        let runtime = connect("TEST_SNAPSHOT_OBSERVATION_RUNTIME_DATABASE_URL").await;
        let cluster = connect("TEST_SNAPSHOT_OBSERVATION_CLUSTER_DATABASE_URL").await;
        let owner_id: (String, String, String) = identity(&owner).await;
        assert_eq!(
            (&*owner_id.0, &*owner_id.1),
            ("learning_admin", "learning_admin")
        );
        assert!(
            owner_id.2.starts_with("learning_task6_observation_"),
            "fresh dedicated A1 database required"
        );
        assert_eq!(
            identity(&runtime).await,
            (
                "learning_runtime".into(),
                "learning_runtime".into(),
                owner_id.2.clone()
            )
        );
        assert_eq!(
            identity(&cluster).await,
            ("postgres".into(), "postgres".into(), owner_id.2)
        );
        MIGRATOR.run(&owner).await.expect("canonical migrations");
        let broker_url = std::env::var("TEST_SNAPSHOT_OBSERVATION_BROKER_DATABASE_URL")
            .expect("real separately provisioned broker LOGIN DSN required");
        let broker = PgPoolOptions::new()
            .max_connections(3)
            .connect_lazy(&broker_url)
            .expect("broker DSN format");
        let roles: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_roles WHERE rolname='learning_snapshot_broker'",
        )
        .fetch_one(&owner)
        .await
        .unwrap();
        assert_eq!(
            roles, 1,
            "cluster provisioner must create exact role before the DB-owner installer"
        );
        Self {
            owner,
            runtime,
            broker,
            cluster,
        }
    }
    async fn seed(&self) -> Uuid {
        let (actor, space, id) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let input = JobInput::snapshot_export(
            actor,
            space,
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
        sqlx::query("INSERT INTO public.job_outbox(id,business_key,event_type,payload_version,payload,actor_id,processor_version) VALUES($1,$2,'snapshot_export_requested',1,$3,$4,1)")
            .bind(id).bind(input.business_key()).bind(input.to_value()).bind(actor).execute(&self.owner).await.unwrap();
        JobStore::new(self.runtime.clone())
            .dispatch_pending(256)
            .await
            .unwrap();
        id
    }
    async fn seed_c2(&self) -> Uuid {
        let (actor, outbox) = (Uuid::new_v4(), Uuid::new_v4());
        let input = JobInput::asset_integrity(
            actor,
            Uuid::new_v4(),
            learning_core::BlockRef {
                block_id: Uuid::new_v4(),
                revision_id: Uuid::new_v4(),
            },
        )
        .unwrap();
        sqlx::query("INSERT INTO public.app_user(id) VALUES($1)")
            .bind(actor)
            .execute(&self.owner)
            .await
            .unwrap();
        sqlx::query("INSERT INTO public.job_outbox(id,business_key,event_type,payload_version,payload,actor_id,processor_version) VALUES($1,$2,'asset_integrity_requested',1,$3,$4,1)")
            .bind(outbox).bind(input.business_key()).bind(input.to_value()).bind(actor).execute(&self.owner).await.unwrap();
        JobStore::new(self.runtime.clone())
            .dispatch_pending(256)
            .await
            .unwrap();
        sqlx::query_scalar("SELECT id FROM public.job WHERE outbox_id=$1")
            .bind(outbox)
            .fetch_one(&self.owner)
            .await
            .unwrap()
    }
    async fn assert_broker_identity(&self) {
        let database = identity(&self.owner).await.2;
        assert_eq!(
            identity(&self.broker).await,
            (
                "learning_snapshot_broker".into(),
                "learning_snapshot_broker".into(),
                database
            )
        );
    }
    async fn cleanup(self) {
        self.broker.close().await;
        sqlx::query("DROP FUNCTION public.p0c4_snapshot_locked_observation(uuid,boolean)")
            .execute(&self.owner)
            .await
            .unwrap();
        sqlx::query("REVOKE USAGE ON SCHEMA public FROM learning_snapshot_broker")
            .execute(&self.owner)
            .await
            .unwrap();
        let revoke:String=sqlx::query_scalar("SELECT format('REVOKE CONNECT ON DATABASE %I FROM learning_snapshot_broker',current_database())").fetch_one(&self.owner).await.unwrap();
        sqlx::query(&revoke).execute(&self.owner).await.unwrap();
        self.runtime.close().await;
        self.owner.close().await;
        self.cluster.close().await;
    }
}
async fn connect(key: &str) -> PgPool {
    let url = std::env::var(key).expect("explicit isolated A1 DSN required; no skip");
    PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("A1 isolated connection")
}
async fn identity(p: &PgPool) -> (String, String, String) {
    sqlx::query_as("SELECT session_user::text,current_user::text,current_database()")
        .fetch_one(p)
        .await
        .unwrap()
}
fn state(e: &sqlx::Error) -> Option<String> {
    e.as_database_error()
        .and_then(|e| e.code())
        .map(|s| s.into_owned())
}
fn budget() -> ObservationBudget {
    ObservationBudget::new(64 * 1024 * 1024).unwrap()
}
fn helper_ddl() -> &'static str {
    a1::INSTALL_SQL
        .split("-- INSTALL_HELPER\n")
        .nth(1)
        .unwrap()
        .split("-- INSTALL_ACL")
        .next()
        .unwrap()
}

#[tokio::test]
#[ignore = "explicit fresh PostgreSQL four real LOGIN DSNs; no silent skip"]
async fn a1_real_login_exact_rows_shared_budget_and_lock_until_rollback() {
    let _serial = SERIAL.lock().await;
    let r = Rig::new().await;
    a1::install(&r.owner).await.unwrap();
    r.assert_broker_identity().await;
    let id = r.seed().await;
    let c2 = r.seed_c2().await;
    let meter = budget();
    let mut tx = ObservationTransaction::begin(&r.broker, meter.clone(), Duration::from_secs(10))
        .await
        .unwrap();
    let observed = tx.observe(id).await.unwrap().unwrap();
    assert_eq!(observed.job.id, id);
    assert_eq!(observed.job.outbox_id, observed.outbox.id);
    assert_eq!(observed.job.status, "snapshot_queued");
    assert_eq!(observed.job.attempt_count, 0);
    assert_eq!(
        serde_json::to_value(&observed.job)
            .unwrap()
            .as_object()
            .unwrap()
            .len(),
        13
    );
    assert_eq!(
        serde_json::to_value(&observed.outbox)
            .unwrap()
            .as_object()
            .unwrap()
            .len(),
        9
    );
    assert!(observed.outbox.dispatched_at.is_some());
    assert!(observed.observed_at >= observed.job.created_at);
    assert!(observed.job.lease_token.is_none());
    let charged = meter.charged_bytes();
    assert!(charged > 1024);
    let mut probe = r.runtime.begin().await.unwrap();
    for sql in [
        "SET LOCAL statement_timeout='2s'",
        "SET LOCAL lock_timeout='2s'",
    ] {
        sqlx::query(sql).execute(&mut *probe).await.unwrap();
    }
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *probe)
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    // A polled owned transaction, never a detached spawned task. The actual
    // original claim runs as runtime, and its result is always rolled back.
    let mut waiting = Box::pin(async move {
        let reply:Result<Option<learning_db::JobLease>,sqlx::Error>=sqlx::query_as("SELECT job_id,token,attempts AS attempt_count,expires_at AS lease_expires_at FROM public.p0c3_claim_snapshot_job($1,$2)")
            .bind(id).bind(30000_i64).fetch_optional(&mut *probe).await;
        let rollback = probe.rollback().await;
        (reply, rollback)
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(150), &mut waiting)
            .await
            .is_err()
    );
    let locked:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM pg_catalog.pg_stat_activity WHERE pid=$1 AND wait_event_type='Lock' AND strpos(query,'public.p0c3_claim_snapshot_job')>0)").bind(pid).fetch_one(&r.cluster).await.unwrap();
    assert!(
        locked,
        "actual PostgreSQL backend must be waiting on a lock, not merely an unpolled task"
    );
    tx.rollback().await.unwrap();
    let (reply, rolled_back) = tokio::time::timeout_at(deadline, &mut waiting)
        .await
        .expect("absolute runtime probe deadline; no detached task");
    rolled_back.unwrap();
    let lease = reply.unwrap().unwrap();
    assert_eq!(
        lease.attempt_count, 1,
        "actual original function returned the first attempt before rollback"
    );
    let mut next = ObservationTransaction::begin(&r.broker, meter.clone(), Duration::from_secs(10))
        .await
        .unwrap();
    let after = next.observe(id).await.unwrap().unwrap();
    assert!(after.job.lease_token.is_none());
    assert_eq!(after.job.attempt_count, 0);
    assert_eq!(
        after.job, observed.job,
        "runtime probe was rolled back; A1 cannot change business rows"
    );
    assert_eq!(after.outbox, observed.outbox);
    assert!(meter.charged_bytes() > charged);
    assert!(next.observe(Uuid::new_v4()).await.unwrap().is_none());
    assert!(
        next.observe(c2).await.unwrap().is_none(),
        "actual C2 row must not be observable by the C3 helper"
    );
    assert!(matches!(
        next.observe(Uuid::nil()).await,
        Err(ObservationError::Invalid(_))
    ));
    next.rollback().await.unwrap();
    let mut small = ObservationTransaction::begin(
        &r.broker,
        ObservationBudget::new(1536).unwrap(),
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    assert!(
        matches!(small.observe(id).await, Err(ObservationError::Capacity)),
        "header obtains lock but body cannot be read without prepaid budget"
    );
    small.rollback().await.unwrap();
    r.cleanup().await;
}

#[tokio::test]
#[ignore = "explicit fresh PostgreSQL four real LOGIN DSNs; no silent skip"]
async fn a1_installer_rejects_role_and_privilege_drift_without_runtime_expansion() {
    let _serial = SERIAL.lock().await;
    let r = Rig::new().await;
    // The real catalog contains table, index, TOAST, partitioned-table, view,
    // materialized-view and sequence rows. SQL may reorder WHERE conjuncts;
    // only CASE guards may select the applicable privilege function.
    sqlx::raw_sql(
        "CREATE TABLE public.a1_kind_table(id integer PRIMARY KEY, body text); \
         CREATE TABLE public.a1_kind_partitioned(id integer) PARTITION BY RANGE(id); \
         CREATE TABLE public.a1_kind_partition PARTITION OF public.a1_kind_partitioned FOR VALUES FROM (0) TO (10); \
         CREATE VIEW public.a1_kind_view AS SELECT id FROM public.a1_kind_table; \
         CREATE MATERIALIZED VIEW public.a1_kind_materialized AS SELECT 1 AS id; \
         CREATE SEQUENCE public.a1_kind_sequence",
    )
    .execute(&r.owner)
    .await
    .unwrap();
    let kinds: Vec<String> =
        sqlx::query_scalar("SELECT DISTINCT relkind::text FROM pg_catalog.pg_class ORDER BY 1")
            .fetch_all(&r.owner)
            .await
            .unwrap();
    for expected in ["r", "i", "t", "p", "v", "m", "S"] {
        assert!(kinds.iter().any(|kind| kind == expected), "{expected}");
    }
    sqlx::query("ALTER ROLE learning_snapshot_broker INHERIT")
        .execute(&r.cluster)
        .await
        .unwrap();
    assert!(a1::install(&r.owner).await.is_err());
    sqlx::query("ALTER ROLE learning_snapshot_broker NOINHERIT")
        .execute(&r.cluster)
        .await
        .unwrap();
    sqlx::query("GRANT SELECT ON public.job TO learning_snapshot_broker")
        .execute(&r.owner)
        .await
        .unwrap();
    assert!(a1::install(&r.owner).await.is_err());
    sqlx::query("REVOKE SELECT ON public.job FROM learning_snapshot_broker")
        .execute(&r.owner)
        .await
        .unwrap();
    let privilege_drift = [
        (
            "GRANT USAGE ON SEQUENCE public.a1_kind_sequence TO learning_snapshot_broker",
            "REVOKE USAGE ON SEQUENCE public.a1_kind_sequence FROM learning_snapshot_broker",
        ),
        (
            "GRANT SELECT(body) ON public.a1_kind_table TO learning_snapshot_broker",
            "REVOKE SELECT(body) ON public.a1_kind_table FROM learning_snapshot_broker",
        ),
        (
            "GRANT SELECT ON public.a1_kind_view TO learning_snapshot_broker",
            "REVOKE SELECT ON public.a1_kind_view FROM learning_snapshot_broker",
        ),
        (
            "GRANT SELECT ON public.a1_kind_materialized TO learning_snapshot_broker",
            "REVOKE SELECT ON public.a1_kind_materialized FROM learning_snapshot_broker",
        ),
        (
            "GRANT SELECT ON public.a1_kind_partitioned TO learning_snapshot_broker",
            "REVOKE SELECT ON public.a1_kind_partitioned FROM learning_snapshot_broker",
        ),
    ];
    for (grant, revoke) in privilege_drift {
        sqlx::query(grant).execute(&r.owner).await.unwrap();
        let error = a1::install(&r.owner).await.unwrap_err();
        match error {
            ObservationError::Database(error) => {
                assert_eq!(state(&error).as_deref(), Some("42501"), "{grant}");
            }
            _ => panic!("installer must reject real privilege drift before mutation"),
        }
        sqlx::query(revoke).execute(&r.owner).await.unwrap();
    }
    a1::install(&r.owner).await.unwrap();
    r.assert_broker_identity().await;
    for (grant, revoke) in privilege_drift {
        sqlx::query(grant).execute(&r.owner).await.unwrap();
        assert!(matches!(
            ObservationTransaction::begin(&r.broker, budget(), Duration::from_secs(5)).await,
            Err(ObservationError::Invalid(
                "snapshot observation role or extension mismatch"
            ))
        ));
        sqlx::query(revoke).execute(&r.owner).await.unwrap();
    }
    let valid = ObservationTransaction::begin(&r.broker, budget(), Duration::from_secs(5))
        .await
        .unwrap();
    valid.rollback().await.unwrap();
    sqlx::raw_sql(
        "DROP MATERIALIZED VIEW public.a1_kind_materialized; \
         DROP VIEW public.a1_kind_view; \
         DROP TABLE public.a1_kind_partitioned,public.a1_kind_table; \
         DROP SEQUENCE public.a1_kind_sequence",
    )
    .execute(&r.owner)
    .await
    .unwrap();
    assert!(
        a1::install(&r.owner).await.is_err(),
        "duplicate installation cannot silently replace authority"
    );
    let id = r.seed().await;
    for pool in [&r.runtime, &r.owner] {
        assert!(matches!(
            ObservationTransaction::begin(pool, budget(), Duration::from_secs(5)).await,
            Err(ObservationError::Invalid(_))
        ));
    }
    for sql in [
        "SELECT lease_token FROM public.job LIMIT 1",
        "SELECT id FROM public.job FOR UPDATE",
        "UPDATE public.job SET status=status",
        "SELECT * FROM public.p0c4_snapshot_locked_observation(NULL,false)",
    ] {
        let error = sqlx::query(sql).execute(&r.runtime).await.unwrap_err();
        assert_eq!(state(&error).as_deref(), Some("42501"));
    }
    for sql in [
        "SELECT lease_token FROM public.job LIMIT 1",
        "UPDATE public.job SET status=status",
        "DELETE FROM public.job WHERE false",
        "INSERT INTO public.job_outbox DEFAULT VALUES",
        "CREATE TABLE public.a1_forbidden(id integer)",
        "SET ROLE learning_admin",
        "SET ROLE learning_runtime",
        "SELECT * FROM public.p0c3_claim_snapshot_job(NULL,30000)",
    ] {
        let error = sqlx::query(sql).execute(&r.broker).await.unwrap_err();
        assert_eq!(state(&error).as_deref(), Some("42501"), "{sql}");
    }
    let owner_error =
        sqlx::query("SELECT * FROM public.p0c4_snapshot_locked_observation($1,false)")
            .bind(id)
            .execute(&r.owner)
            .await
            .unwrap_err();
    assert_eq!(
        state(&owner_error).as_deref(),
        Some("42501"),
        "SECURITY DEFINER does not authenticate an owner session as broker"
    );
    let unchanged: (String, i32) =
        sqlx::query_as("SELECT status,attempt_count FROM public.job WHERE id=$1")
            .bind(id)
            .fetch_one(&r.owner)
            .await
            .unwrap();
    assert_eq!(unchanged, ("snapshot_queued".into(), 0));
    r.cleanup().await;
}

#[tokio::test]
#[ignore = "explicit fresh PostgreSQL four real LOGIN DSNs; no silent skip"]
async fn a1_observation_rejects_acl_body_owner_search_path_and_overload_drift() {
    let _serial = SERIAL.lock().await;
    let r = Rig::new().await;
    a1::install(&r.owner).await.unwrap();
    r.assert_broker_identity().await;
    let alterations = [
        (
            "GRANT EXECUTE ON FUNCTION public.p0c4_snapshot_locked_observation(uuid,boolean) TO PUBLIC",
            "REVOKE EXECUTE ON FUNCTION public.p0c4_snapshot_locked_observation(uuid,boolean) FROM PUBLIC",
        ),
        (
            "GRANT EXECUTE ON FUNCTION public.p0c4_snapshot_locked_observation(uuid,boolean) TO learning_runtime",
            "REVOKE EXECUTE ON FUNCTION public.p0c4_snapshot_locked_observation(uuid,boolean) FROM learning_runtime",
        ),
        (
            "ALTER FUNCTION public.p0c4_snapshot_locked_observation(uuid,boolean) SET search_path=public",
            "ALTER FUNCTION public.p0c4_snapshot_locked_observation(uuid,boolean) SET search_path=pg_catalog",
        ),
        (
            "ALTER FUNCTION public.p0c4_snapshot_locked_observation(uuid,boolean) SECURITY INVOKER",
            "ALTER FUNCTION public.p0c4_snapshot_locked_observation(uuid,boolean) SECURITY DEFINER",
        ),
    ];
    for (alter, restore) in alterations {
        sqlx::query(alter).execute(&r.owner).await.unwrap();
        assert!(
            ObservationTransaction::begin(&r.broker, budget(), Duration::from_secs(5))
                .await
                .is_err()
        );
        sqlx::query(restore).execute(&r.owner).await.unwrap();
    }
    sqlx::query(
        "ALTER FUNCTION public.p0c4_snapshot_locked_observation(uuid,boolean) OWNER TO postgres",
    )
    .execute(&r.cluster)
    .await
    .unwrap();
    assert!(
        ObservationTransaction::begin(&r.broker, budget(), Duration::from_secs(5))
            .await
            .is_err()
    );
    sqlx::query("ALTER FUNCTION public.p0c4_snapshot_locked_observation(uuid,boolean) OWNER TO learning_admin").execute(&r.cluster).await.unwrap();
    // Even a syntactically valid body change cannot pass the fixed body pin.
    let altered = helper_ddl()
        .replace("CREATE FUNCTION ", "CREATE OR REPLACE FUNCTION ")
        .replace("-- Exact 13 job columns", "-- Altered 13 job columns");
    sqlx::raw_sql(&altered).execute(&r.owner).await.unwrap();
    assert!(
        ObservationTransaction::begin(&r.broker, budget(), Duration::from_secs(5))
            .await
            .is_err()
    );
    sqlx::raw_sql(&helper_ddl().replace("CREATE FUNCTION ", "CREATE OR REPLACE FUNCTION "))
        .execute(&r.owner)
        .await
        .unwrap();
    sqlx::query("CREATE FUNCTION public.p0c4_snapshot_locked_observation(integer) RETURNS integer LANGUAGE sql AS 'SELECT 1'").execute(&r.owner).await.unwrap();
    assert!(
        ObservationTransaction::begin(&r.broker, budget(), Duration::from_secs(5))
            .await
            .is_err()
    );
    sqlx::query("DROP FUNCTION public.p0c4_snapshot_locked_observation(integer)")
        .execute(&r.owner)
        .await
        .unwrap();
    let id = r.seed().await;
    sqlx::query("ALTER TABLE public.job ADD COLUMN a1_unexpected integer")
        .execute(&r.owner)
        .await
        .unwrap();
    let mut wrong = ObservationTransaction::begin(&r.broker, budget(), Duration::from_secs(5))
        .await
        .unwrap();
    assert!(
        matches!(wrong.observe(id).await, Err(ObservationError::Database(_))),
        "a fourteenth visible job column must be rejected before a body is returned"
    );
    wrong.rollback().await.unwrap();
    sqlx::query("ALTER TABLE public.job DROP COLUMN a1_unexpected")
        .execute(&r.owner)
        .await
        .unwrap();
    let ok = ObservationTransaction::begin(&r.broker, budget(), Duration::from_secs(5))
        .await
        .unwrap();
    ok.rollback().await.unwrap();
    r.cleanup().await;
}

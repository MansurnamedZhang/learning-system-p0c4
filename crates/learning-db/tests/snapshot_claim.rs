//! Explicit fresh PostgreSQL fixtures; no absent-DSN skip and no positive
//! snapshot/export authority. Root provisions the separate real LOGIN first.
use learning_core::{JobInput, ReadingMode, ReadingRef, SnapshotRequest};
use learning_db::{
    JobStore, MIGRATOR,
    snapshot_broker::{self as c0, ObservationBudget, ObservationError, ObservationTransaction},
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
        let owner = connect("TEST_SNAPSHOT_CLAIM_ADMIN_DATABASE_URL").await;
        let runtime = connect("TEST_SNAPSHOT_CLAIM_RUNTIME_DATABASE_URL").await;
        let cluster = connect("TEST_SNAPSHOT_CLAIM_CLUSTER_DATABASE_URL").await;
        let owner_id: (String, String, String) = identity(&owner).await;
        assert_eq!(
            (&*owner_id.0, &*owner_id.1),
            ("learning_admin", "learning_admin")
        );
        let suffix = owner_id
            .2
            .strip_prefix("learning_task6_claim_")
            .expect("fresh dedicated C0 database required");
        assert_eq!(Uuid::parse_str(suffix).unwrap().to_string(), suffix);
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
        let rows: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM public.job),(SELECT count(*) FROM public.job_outbox)",
        )
        .fetch_one(&owner)
        .await
        .unwrap();
        assert_eq!(
            rows,
            (0, 0),
            "every exact C0 case starts with its own empty dedicated database"
        );
        let broker_url = std::env::var("TEST_SNAPSHOT_CLAIM_BROKER_DATABASE_URL")
            .expect("real separately provisioned broker LOGIN DSN required");
        let broker = PgPoolOptions::new()
            .max_connections(3)
            .connect_lazy(&broker_url)
            .expect("broker DSN format");
        let roles: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_roles WHERE rolname='learning_snapshot_claim_broker'",
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
                "learning_snapshot_claim_broker".into(),
                "learning_snapshot_claim_broker".into(),
                database
            )
        );
        // Same source endpoint facts without reading secrets or using role
        // substitution. No cross-server pool may stand in for this broker.
        let expected = self.owner.connect_options();
        for pool in [&self.runtime, &self.cluster, &self.broker] {
            let options = pool.connect_options();
            assert_eq!(options.get_host(), expected.get_host());
            assert_eq!(options.get_port(), expected.get_port());
            assert_eq!(options.get_database(), expected.get_database());
        }
        let fact = "SELECT (SELECT oid::bigint FROM pg_catalog.pg_database WHERE datname=current_database()),pg_catalog.pg_postmaster_start_time()::text";
        let expected: (i64, String) = sqlx::query_as(fact).fetch_one(&self.owner).await.unwrap();
        for pool in [&self.runtime, &self.cluster, &self.broker] {
            let actual: (i64, String) = sqlx::query_as(fact).fetch_one(pool).await.unwrap();
            assert_eq!(actual, expected);
        }
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
        let revoke:String=sqlx::query_scalar("SELECT format('REVOKE CONNECT ON DATABASE %I FROM learning_snapshot_claim_broker',current_database())").fetch_one(&self.owner).await.unwrap();
        sqlx::query(&revoke).execute(&self.owner).await.unwrap();
        sqlx::raw_sql("REVOKE EXECUTE ON FUNCTION public.p0c3_claim_snapshot_job(uuid,bigint), public.p0c3_renew_snapshot_job(uuid,uuid,bigint), public.p0c3_fail_snapshot_job(uuid,uuid,text,boolean) FROM learning_snapshot_claim_broker").execute(&self.owner).await.unwrap();
        self.runtime.close().await;
        self.owner.close().await;
        self.cluster.close().await;
    }
}
async fn connect(key: &str) -> PgPool {
    let url = std::env::var(key).expect("explicit isolated C0 DSN required; no skip");
    PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("C0 isolated connection")
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

// C0-only integration target explicitly adopted for future Root PG18 validation.
// Each exact ignored case requires a freshly provisioned dedicated database;
// runtime and broker authenticate independently, never via SET ROLE.
#[tokio::test]
#[ignore = "explicit fresh PG18 four real LOGIN DSNs; no silent skip"]
async fn c0_real_login_complete_rows_budget_and_broker_lock_until_rollback() {
    let _serial = SERIAL.lock().await;
    let r = Rig::new().await;
    c0::install(&r.owner).await.unwrap();
    r.assert_broker_identity().await;
    for p in [&r.owner, &r.runtime] {
        assert!(matches!(
            ObservationTransaction::begin(p, budget(), Duration::from_secs(10)).await,
            Err(ObservationError::Invalid(_))
        ));
    }
    for sql in [
        "SELECT rolpassword FROM pg_catalog.pg_authid",
        "SELECT passwd FROM pg_catalog.pg_shadow",
        "SELECT lease_token FROM public.job",
        "UPDATE public.job SET status=status",
        "SET ROLE learning_admin",
        "SET ROLE learning_runtime",
    ] {
        assert_eq!(
            state(&sqlx::query(sql).execute(&r.broker).await.unwrap_err()).as_deref(),
            Some("42501")
        );
    }
    let metadata: bool = sqlx::query_scalar("SELECT EXISTS(SELECT FROM pg_catalog.pg_roles WHERE rolname='learning_snapshot_claim_broker') AND EXISTS(SELECT FROM pg_catalog.pg_class) AND EXISTS(SELECT FROM pg_catalog.pg_attribute) AND EXISTS(SELECT FROM pg_catalog.pg_proc)")
        .fetch_one(&r.broker).await.unwrap();
    assert!(
        metadata,
        "ordinary default catalog metadata reads stay available"
    );
    let id = r.seed().await;
    let c2 = r.seed_c2().await;
    let meter = budget();
    let mut tx = ObservationTransaction::begin(&r.broker, meter.clone(), Duration::from_secs(10))
        .await
        .unwrap();
    let initial = meter.charged_bytes().unwrap();
    let observed = tx.observe(id).await.unwrap().unwrap();
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
    assert_eq!(observed.job.id, observed.outbox.id);
    assert_eq!(
        (observed.job.status.as_str(), observed.job.attempt_count),
        ("snapshot_queued", 0)
    );
    assert!(observed.job.lease_token.is_none());
    assert!(meter.charged_bytes().unwrap() > initial);
    let mut probe = r.broker.begin().await.unwrap();
    sqlx::query("SET LOCAL statement_timeout='2s'")
        .execute(&mut *probe)
        .await
        .unwrap();
    sqlx::query("SET LOCAL lock_timeout='2s'")
        .execute(&mut *probe)
        .await
        .unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *probe)
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let mut waiting = Box::pin(async move {
        let reply: Result<(i64,i64),sqlx::Error> = sqlx::query_as("SELECT job_octets,outbox_octets FROM public.p0c4_snapshot_claim_locked_observation($1,false)")
            .bind(id).fetch_one(&mut *probe).await;
        let rollback = probe.rollback().await;
        (reply, rollback)
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(150), &mut waiting)
            .await
            .is_err()
    );
    let locked:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM pg_catalog.pg_stat_activity WHERE pid=$1 AND wait_event_type='Lock' AND strpos(query,'public.p0c4_snapshot_claim_locked_observation')>0)").bind(pid).fetch_one(&r.cluster).await.unwrap();
    assert!(locked, "actual second C LOGIN backend must be lock-waiting");
    tx.rollback().await.unwrap();
    let (header, rollback) = tokio::time::timeout_at(deadline, &mut waiting)
        .await
        .expect("absolute non-detached C observation probe");
    rollback.unwrap();
    let (job_octets, outbox_octets) = header.unwrap();
    assert!(job_octets > 0 && outbox_octets > 0);
    let mut next = ObservationTransaction::begin(&r.broker, meter.clone(), Duration::from_secs(10))
        .await
        .unwrap();
    let after = next.observe(id).await.unwrap().unwrap();
    assert_eq!(after.job, observed.job);
    assert_eq!(after.outbox, observed.outbox);
    assert!(next.observe(c2).await.unwrap().is_none());
    next.rollback().await.unwrap();
    // Begin + header fits; the body/second-profile prepayment cannot fit.
    let mut small = ObservationTransaction::begin(
        &r.broker,
        ObservationBudget::new(initial * 2).unwrap(),
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    assert!(matches!(
        small.observe(id).await,
        Err(ObservationError::Capacity)
    ));
    assert!(matches!(
        small.observe(id).await,
        Err(ObservationError::Invalid(_))
    ));
    small.rollback().await.unwrap();
    // Keep the actual observer alive while only its borrowed observe future
    // is dropped. A pending query may continue server-side; it grants no right
    // to reuse this observer before explicit known rollback.
    let mut holder = ObservationTransaction::begin(&r.broker, budget(), Duration::from_secs(30))
        .await
        .unwrap();
    holder.observe(id).await.unwrap().unwrap();
    let options = (*r.broker.connect_options()).clone();
    let cancelled_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    let cancelled_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&cancelled_pool)
        .await
        .unwrap();
    let mut retained =
        ObservationTransaction::begin(&cancelled_pool, budget(), Duration::from_secs(10))
            .await
            .unwrap();
    let mut borrowed = Box::pin(retained.observe(id));
    // Keep polling the same borrowed future throughout catalog verification
    // and the actual header query. Its own original absolute deadline bounds
    // this select; no timer restart or fixed short delay proves lock arrival.
    tokio::select! {
        completed = &mut borrowed => match completed {
            Err(error) => panic!("observation ended before actual PG Lock: {error}; SQLSTATE={:?}", error.sqlstate()),
            Ok(_) => panic!("observation completed before actual PG Lock"),
        },
        () = async {
            loop {
                let waiting_on_lock: bool = sqlx::query_scalar("SELECT EXISTS(SELECT FROM pg_catalog.pg_stat_activity WHERE pid=$1 AND wait_event_type='Lock' AND strpos(query,'public.p0c4_snapshot_claim_locked_observation')>0)")
                    .bind(cancelled_pid).fetch_one(&r.cluster).await.unwrap();
                if waiting_on_lock { break; }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        } => {}
    }
    drop(borrowed);
    assert!(matches!(
        retained.observe(id).await,
        Err(ObservationError::Invalid(
            "snapshot claim observation permanently poisoned"
        ))
    ));
    holder.rollback().await.unwrap();
    retained.rollback().await.unwrap();
    cancelled_pool.close().await;
    let mut checked = ObservationTransaction::begin(&r.broker, budget(), Duration::from_secs(10))
        .await
        .unwrap();
    let unchanged = checked.observe(id).await.unwrap().unwrap();
    assert_eq!(unchanged.job, observed.job);
    assert_eq!(unchanged.outbox, observed.outbox);
    checked.rollback().await.unwrap();
    r.cleanup().await;
}

#[tokio::test]
#[ignore = "explicit fresh PG18 four real LOGIN DSNs; no silent skip"]
async fn c0_mixed_catalog_and_system_direct_acl_are_rejected_without_repair() {
    let _serial = SERIAL.lock().await;
    let r = Rig::new().await;
    sqlx::raw_sql("CREATE TABLE public.c0_kind_table(id integer PRIMARY KEY,body text); CREATE TABLE public.c0_kind_partitioned(id integer) PARTITION BY RANGE(id); CREATE TABLE public.c0_kind_partition PARTITION OF public.c0_kind_partitioned FOR VALUES FROM (0) TO (10); CREATE VIEW public.c0_kind_view AS SELECT id FROM public.c0_kind_table; CREATE MATERIALIZED VIEW public.c0_kind_materialized AS SELECT 1 AS id; CREATE SEQUENCE public.c0_kind_sequence").execute(&r.owner).await.unwrap();
    let kinds: Vec<String> =
        sqlx::query_scalar("SELECT DISTINCT relkind::text FROM pg_catalog.pg_class")
            .fetch_all(&r.owner)
            .await
            .unwrap();
    for kind in ["r", "i", "t", "p", "v", "m", "S"] {
        assert!(kinds.iter().any(|k| k == kind));
    }
    let drift = [
        (
            "GRANT SELECT ON pg_catalog.pg_authid TO learning_snapshot_claim_broker",
            "REVOKE SELECT ON pg_catalog.pg_authid FROM learning_snapshot_claim_broker",
            true,
        ),
        (
            "GRANT SELECT(rolname) ON pg_catalog.pg_authid TO learning_snapshot_claim_broker",
            "REVOKE SELECT(rolname) ON pg_catalog.pg_authid FROM learning_snapshot_claim_broker",
            true,
        ),
        (
            "GRANT SELECT(body) ON public.c0_kind_table TO learning_snapshot_claim_broker",
            "REVOKE SELECT(body) ON public.c0_kind_table FROM learning_snapshot_claim_broker",
            false,
        ),
        (
            "GRANT USAGE ON SEQUENCE public.c0_kind_sequence TO learning_snapshot_claim_broker",
            "REVOKE USAGE ON SEQUENCE public.c0_kind_sequence FROM learning_snapshot_claim_broker",
            false,
        ),
        (
            "GRANT SELECT ON public.c0_kind_materialized TO learning_snapshot_claim_broker",
            "REVOKE SELECT ON public.c0_kind_materialized FROM learning_snapshot_claim_broker",
            false,
        ),
        (
            "GRANT SELECT ON public.c0_kind_partitioned TO learning_snapshot_claim_broker",
            "REVOKE SELECT ON public.c0_kind_partitioned FROM learning_snapshot_claim_broker",
            false,
        ),
    ];
    // PUBLIC grants bypass a direct-grantee scan. Test real effective ACLs,
    // including PG18 MAINTAIN and both sensitive system relations/columns.
    // Grant/revoke below is confined to this fresh synthetic fixture; the
    // product must reject and must never repair any of these privileges.
    let public_drift = [
        (
            "GRANT MAINTAIN ON public.c0_kind_table TO PUBLIC",
            "REVOKE MAINTAIN ON public.c0_kind_table FROM PUBLIC",
            false,
            "SELECT pg_catalog.has_table_privilege('learning_snapshot_claim_broker','public.c0_kind_table','MAINTAIN')",
        ),
        (
            "GRANT SELECT ON pg_catalog.pg_authid TO PUBLIC",
            "REVOKE SELECT ON pg_catalog.pg_authid FROM PUBLIC",
            true,
            "SELECT pg_catalog.has_table_privilege('learning_snapshot_claim_broker','pg_catalog.pg_authid','SELECT')",
        ),
        (
            "GRANT SELECT(rolpassword) ON pg_catalog.pg_authid TO PUBLIC",
            "REVOKE SELECT(rolpassword) ON pg_catalog.pg_authid FROM PUBLIC",
            true,
            "SELECT pg_catalog.has_column_privilege('learning_snapshot_claim_broker','pg_catalog.pg_authid','rolpassword','SELECT')",
        ),
        (
            "GRANT SELECT ON pg_catalog.pg_shadow TO PUBLIC",
            "REVOKE SELECT ON pg_catalog.pg_shadow FROM PUBLIC",
            true,
            "SELECT pg_catalog.has_table_privilege('learning_snapshot_claim_broker','pg_catalog.pg_shadow','SELECT')",
        ),
        (
            "GRANT SELECT(passwd) ON pg_catalog.pg_shadow TO PUBLIC",
            "REVOKE SELECT(passwd) ON pg_catalog.pg_shadow FROM PUBLIC",
            true,
            "SELECT pg_catalog.has_column_privilege('learning_snapshot_claim_broker','pg_catalog.pg_shadow','passwd','SELECT')",
        ),
    ];
    for (grant, revoke, cluster, check_acl) in public_drift {
        let p = if cluster { &r.cluster } else { &r.owner };
        let initial: bool = sqlx::query_scalar(check_acl)
            .fetch_one(&r.owner)
            .await
            .unwrap();
        assert!(
            !initial,
            "fresh fixture baseline must not already carry the tested effective ACL"
        );
        sqlx::query(grant).execute(p).await.unwrap();
        let error = c0::install(&r.owner).await.unwrap_err();
        assert_eq!(error.sqlstate().as_deref(), Some("42501"));
        let preserved: bool = sqlx::query_scalar(check_acl)
            .fetch_one(&r.owner)
            .await
            .unwrap();
        assert!(
            preserved,
            "rejected installer must not silently revoke PUBLIC privileges"
        );
        sqlx::query(revoke).execute(p).await.unwrap();
    }
    for (grant, revoke, cluster) in drift {
        let p = if cluster { &r.cluster } else { &r.owner };
        sqlx::query(grant).execute(p).await.unwrap();
        let error = c0::install(&r.owner).await.unwrap_err();
        assert_eq!(error.sqlstate().as_deref(), Some("42501"));
        // The installer must not silently revoke or fix the rejected grant.
        let present:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM pg_catalog.pg_class c,LATERAL pg_catalog.aclexplode(c.relacl) a WHERE a.grantee='learning_snapshot_claim_broker'::regrole) OR EXISTS(SELECT FROM pg_catalog.pg_attribute c,LATERAL pg_catalog.aclexplode(c.attacl) a WHERE a.grantee='learning_snapshot_claim_broker'::regrole)").fetch_one(&r.owner).await.unwrap();
        assert!(present);
        sqlx::query(revoke).execute(p).await.unwrap();
    }
    c0::install(&r.owner).await.unwrap();
    r.assert_broker_identity().await;
    for (grant, revoke, cluster, check_acl) in public_drift {
        let p = if cluster { &r.cluster } else { &r.owner };
        sqlx::query(grant).execute(p).await.unwrap();
        let error =
            match ObservationTransaction::begin(&r.broker, budget(), Duration::from_secs(10)).await
            {
                Err(error) => error,
                Ok(tx) => {
                    tx.rollback().await.unwrap();
                    panic!("PUBLIC effective ACL must reject");
                }
            };
        assert_eq!(error.sqlstate().as_deref(), Some("42501"));
        let preserved: bool = sqlx::query_scalar(check_acl)
            .fetch_one(&r.owner)
            .await
            .unwrap();
        assert!(preserved);
        sqlx::query(revoke).execute(p).await.unwrap();
        let valid = ObservationTransaction::begin(&r.broker, budget(), Duration::from_secs(10))
            .await
            .unwrap();
        valid.rollback().await.unwrap();
    }
    for (grant, revoke, cluster) in drift {
        let p = if cluster { &r.cluster } else { &r.owner };
        sqlx::query(grant).execute(p).await.unwrap();
        let error =
            match ObservationTransaction::begin(&r.broker, budget(), Duration::from_secs(10)).await
            {
                Err(error) => error,
                Ok(tx) => {
                    tx.rollback().await.unwrap();
                    panic!("direct ACL must reject")
                }
            };
        assert_eq!(error.sqlstate().as_deref(), Some("42501"));
        sqlx::query(revoke).execute(p).await.unwrap();
    }
    let valid = ObservationTransaction::begin(&r.broker, budget(), Duration::from_secs(10))
        .await
        .unwrap();
    valid.rollback().await.unwrap();
    sqlx::raw_sql("DROP MATERIALIZED VIEW public.c0_kind_materialized; DROP VIEW public.c0_kind_view; DROP TABLE public.c0_kind_partitioned,public.c0_kind_table; DROP SEQUENCE public.c0_kind_sequence").execute(&r.owner).await.unwrap();
    r.cleanup().await;
}

#[tokio::test]
#[ignore = "explicit fresh PG18 four real LOGIN DSNs; no silent skip"]
async fn c0_exact_canonical_function_profile_and_role_drift_are_not_adopted() {
    let _serial = SERIAL.lock().await;
    let r = Rig::new().await;
    // All interventions are explicit fixture mutations by the isolated owner;
    // the product never repairs or adopts any of them.
    for (bad, restore) in [
        (
            "ALTER ROLE learning_snapshot_claim_broker INHERIT",
            "ALTER ROLE learning_snapshot_claim_broker NOINHERIT",
        ),
        (
            "ALTER ROLE learning_snapshot_claim_broker SET search_path=pg_catalog",
            "ALTER ROLE learning_snapshot_claim_broker RESET search_path",
        ),
        (
            "GRANT learning_runtime TO learning_snapshot_claim_broker",
            "REVOKE learning_runtime FROM learning_snapshot_claim_broker",
        ),
    ] {
        sqlx::query(bad).execute(&r.cluster).await.unwrap();
        assert!(c0::install(&r.owner).await.is_err());
        sqlx::query(restore).execute(&r.cluster).await.unwrap();
    }
    c0::install(&r.owner).await.unwrap();
    r.assert_broker_identity().await;
    for (bad, restore) in [
        (
            "ALTER FUNCTION public.p0c3_claim_snapshot_job(uuid,bigint) COST 101",
            "ALTER FUNCTION public.p0c3_claim_snapshot_job(uuid,bigint) COST 100",
        ),
        (
            "ALTER FUNCTION public.p0c3_renew_snapshot_job(uuid,uuid,bigint) SET search_path=pg_catalog",
            "ALTER FUNCTION public.p0c3_renew_snapshot_job(uuid,uuid,bigint) SET search_path=pg_catalog,public,pg_temp",
        ),
        (
            "GRANT EXECUTE ON FUNCTION public.p0c3_fail_snapshot_job(uuid,uuid,text,boolean) TO PUBLIC",
            "REVOKE EXECUTE ON FUNCTION public.p0c3_fail_snapshot_job(uuid,uuid,text,boolean) FROM PUBLIC",
        ),
        (
            "GRANT EXECUTE ON FUNCTION public.p0c3_claim_snapshot_job(uuid,bigint) TO learning_snapshot_claim_broker WITH GRANT OPTION",
            "REVOKE GRANT OPTION FOR EXECUTE ON FUNCTION public.p0c3_claim_snapshot_job(uuid,bigint) FROM learning_snapshot_claim_broker",
        ),
        (
            "ALTER FUNCTION public.p0c4_snapshot_claim_locked_observation(uuid,boolean) COST 101",
            "ALTER FUNCTION public.p0c4_snapshot_claim_locked_observation(uuid,boolean) COST 100",
        ),
        (
            "CREATE FUNCTION public.p0c4_snapshot_claim_locked_observation(integer) RETURNS integer LANGUAGE sql AS 'SELECT 1'",
            "DROP FUNCTION public.p0c4_snapshot_claim_locked_observation(integer)",
        ),
    ] {
        sqlx::query(bad).execute(&r.owner).await.unwrap();
        assert!(
            ObservationTransaction::begin(&r.broker, budget(), Duration::from_secs(10))
                .await
                .is_err()
        );
        sqlx::query(restore).execute(&r.owner).await.unwrap();
        let valid = ObservationTransaction::begin(&r.broker, budget(), Duration::from_secs(10))
            .await
            .unwrap();
        valid.rollback().await.unwrap();
    }
    let original: String = sqlx::query_scalar("SELECT pg_catalog.pg_get_functiondef('public.p0c3_renew_snapshot_job(uuid,uuid,bigint)'::regprocedure)")
        .fetch_one(&r.owner).await.unwrap();
    let changed = original.replacen("BEGIN\n", "BEGIN\nPERFORM 1;\n", 1);
    assert_ne!(original, changed);
    sqlx::raw_sql(&changed).execute(&r.owner).await.unwrap();
    assert!(
        ObservationTransaction::begin(&r.broker, budget(), Duration::from_secs(10))
            .await
            .is_err()
    );
    sqlx::raw_sql(&original).execute(&r.owner).await.unwrap();
    let valid = ObservationTransaction::begin(&r.broker, budget(), Duration::from_secs(10))
        .await
        .unwrap();
    valid.rollback().await.unwrap();
    // C0 grants only exact canonical3 plus helper. It does NOT exercise claim
    // via this broker or issue a capability; case1 uses only a second read helper.
    let extra:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM pg_catalog.pg_proc p,LATERAL pg_catalog.aclexplode(p.proacl) a WHERE a.grantee='learning_snapshot_claim_broker'::regrole AND p.oid NOT IN ('public.p0c4_snapshot_claim_locked_observation(uuid,boolean)'::regprocedure,'public.p0c3_claim_snapshot_job(uuid,bigint)'::regprocedure,'public.p0c3_renew_snapshot_job(uuid,uuid,bigint)'::regprocedure,'public.p0c3_fail_snapshot_job(uuid,uuid,text,boolean)'::regprocedure))").fetch_one(&r.broker).await.unwrap();
    assert!(!extra);
    r.cleanup().await;
}

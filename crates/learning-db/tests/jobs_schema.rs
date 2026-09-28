mod support;

use sqlx::PgPool;
use support::{TestRig, sqlstate};
use uuid::Uuid;

async fn outbox(pool: &PgPool, actor: Uuid) -> (Uuid, String) {
    let id = Uuid::new_v4();
    let space_id = Uuid::new_v4();
    let block_id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    let key = format!("asset-integrity:v1:{space_id}:{block_id}:{revision_id}");
    sqlx::query("INSERT INTO public.job_outbox(id,business_key,event_type,payload_version,payload,actor_id,processor_version) VALUES($1,$2,'asset_integrity_requested',1,$3,$4,1)")
        .bind(id)
        .bind(&key)
        .bind(serde_json::json!({"version":1,"kind":"asset_integrity","space_id":space_id,"block_id":block_id,"revision_id":revision_id,"actor_id":actor}))
        .bind(actor)
        .execute(pool)
        .await
        .unwrap();
    (id, key)
}

async fn job(pool: &PgPool, outbox_id: Uuid, key: &str) -> Uuid {
    let id = Uuid::new_v4();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("INSERT INTO public.job(id,outbox_id,idempotency_key,status,attempt_count) VALUES($1,$2,$3,'queued',0)")
        .bind(id)
        .bind(outbox_id)
        .bind(key)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE public.job_outbox SET dispatched_at=clock_timestamp() WHERE id=$1")
        .bind(outbox_id)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    id
}

#[tokio::test]
async fn business_key_and_outbox_identity_allow_only_one_job() {
    let rig = TestRig::from_env().await;
    let (actor, _) = rig.seed_actor_space(true).await;
    let (event, key) = outbox(&rig.runtime_pool, actor.actor_id).await;
    let duplicate = sqlx::query("INSERT INTO public.job_outbox(id,business_key,event_type,payload_version,payload,actor_id,processor_version) SELECT $1,business_key,event_type,payload_version,payload,actor_id,processor_version FROM public.job_outbox WHERE id=$2")
        .bind(Uuid::new_v4()).bind(event).execute(&rig.runtime_pool).await.unwrap_err();
    assert_eq!(sqlstate(&duplicate).as_deref(), Some("23505"));

    let wrong_job_key = sqlx::query("INSERT INTO public.job(id,outbox_id,idempotency_key,status,attempt_count) VALUES($1,$2,$3,'queued',0)")
        .bind(Uuid::new_v4()).bind(event)
        .bind(format!("asset-integrity:v1:{}", Uuid::new_v4()))
        .execute(&rig.runtime_pool).await.unwrap_err();
    assert_eq!(sqlstate(&wrong_job_key).as_deref(), Some("23503"));

    let first = job(&rig.runtime_pool, event, &key).await;
    let duplicate_job = sqlx::query("INSERT INTO public.job(id,outbox_id,idempotency_key,status,attempt_count) VALUES($1,$2,$3,'queued',0)")
        .bind(Uuid::new_v4()).bind(event).bind(&key).execute(&rig.runtime_pool).await.unwrap_err();
    assert_eq!(sqlstate(&duplicate_job).as_deref(), Some("23505"));
    let saved: (Uuid, Uuid, String) =
        sqlx::query_as("SELECT id,outbox_id,idempotency_key FROM public.job WHERE id=$1")
            .bind(first)
            .fetch_one(&rig.runtime_pool)
            .await
            .unwrap();
    assert_eq!(saved, (first, event, key));
}

#[tokio::test]
async fn business_key_must_bind_the_exact_payload_reference() {
    let rig = TestRig::from_env().await;
    let (actor, _) = rig.seed_actor_space(true).await;
    let payload = serde_json::json!({
        "version": 1,
        "kind": "asset_integrity",
        "space_id": Uuid::new_v4(),
        "block_id": Uuid::new_v4(),
        "revision_id": Uuid::new_v4(),
        "actor_id": actor.actor_id,
    });
    let error = sqlx::query("INSERT INTO public.job_outbox(id,business_key,event_type,payload_version,payload,actor_id,processor_version) VALUES($1,$2,'asset_integrity_requested',1,$3,$4,1)")
        .bind(Uuid::new_v4())
        .bind(format!("asset-integrity:v1:{}", Uuid::new_v4()))
        .bind(payload)
        .bind(actor.actor_id)
        .execute(&rig.runtime_pool)
        .await
        .unwrap_err();
    assert_eq!(sqlstate(&error).as_deref(), Some("23514"));
}

#[tokio::test]
async fn unknown_versions_bad_status_and_lease_shape_are_rejected() {
    let rig = TestRig::from_env().await;
    let (actor, _) = rig.seed_actor_space(true).await;
    let key = format!("asset-integrity:v1:{}", Uuid::new_v4());
    let bad_version = sqlx::query("INSERT INTO public.job_outbox(id,business_key,event_type,payload_version,payload,actor_id,processor_version) VALUES($1,$2,'asset_integrity_requested',99,'{}'::jsonb,$3,1)")
        .bind(Uuid::new_v4()).bind(&key).bind(actor.actor_id).execute(&rig.runtime_pool).await.unwrap_err();
    assert_eq!(sqlstate(&bad_version).as_deref(), Some("23514"));
    for payload in [
        serde_json::json!({"version":1,"kind":"asset_integrity","actor_id":actor.actor_id}),
        serde_json::json!({"version":1,"kind":"asset_integrity","space_id":Uuid::new_v4(),"block_id":Uuid::new_v4(),"revision_id":Uuid::new_v4(),"actor_id":Uuid::new_v4()}),
    ] {
        let bad_payload = sqlx::query("INSERT INTO public.job_outbox(id,business_key,event_type,payload_version,payload,actor_id,processor_version) VALUES($1,$2,'asset_integrity_requested',1,$3,$4,1)")
            .bind(Uuid::new_v4()).bind(&key).bind(payload).bind(actor.actor_id)
            .execute(&rig.runtime_pool).await.unwrap_err();
        assert_eq!(sqlstate(&bad_payload).as_deref(), Some("23514"));
    }
    let (event, key) = outbox(&rig.runtime_pool, actor.actor_id).await;
    for (status, attempts) in [("unknown", 0), ("queued", -1), ("running", 0)] {
        let error = sqlx::query("INSERT INTO public.job(id,outbox_id,idempotency_key,status,attempt_count) VALUES($1,$2,$3,$4,$5)")
            .bind(Uuid::new_v4()).bind(event).bind(&key).bind(status).bind(attempts)
            .execute(&rig.runtime_pool).await.unwrap_err();
        assert_eq!(
            sqlstate(&error).as_deref(),
            Some("23514"),
            "{status}/{attempts}"
        );
    }
    let bad_lease = sqlx::query("INSERT INTO public.job(id,outbox_id,idempotency_key,status,attempt_count,lease_token,lease_expires_at) VALUES($1,$2,$3,'queued',0,$4,clock_timestamp()+interval '1 minute')")
        .bind(Uuid::new_v4()).bind(event).bind(&key).bind(Uuid::new_v4())
        .execute(&rig.runtime_pool).await.unwrap_err();
    assert_eq!(sqlstate(&bad_lease).as_deref(), Some("23514"));
}

#[tokio::test]
async fn terminal_job_cannot_be_reopened_even_by_table_owner() {
    let rig = TestRig::from_env().await;
    let (actor, _) = rig.seed_actor_space(true).await;
    for terminal in ["cancelled", "succeeded", "failed"] {
        let (event, key) = outbox(&rig.runtime_pool, actor.actor_id).await;
        let id = job(&rig.runtime_pool, event, &key).await;
        if terminal != "cancelled" {
            sqlx::query("UPDATE public.job SET status='running',attempt_count=1,lease_token=$2,lease_expires_at=clock_timestamp()+interval '1 minute' WHERE id=$1")
                .bind(id).bind(Uuid::new_v4()).execute(&rig.admin_pool).await.unwrap();
        }
        sqlx::query(
            "UPDATE public.job SET status=$2,lease_token=NULL,lease_expires_at=NULL WHERE id=$1",
        )
        .bind(id)
        .bind(terminal)
        .execute(&rig.admin_pool)
        .await
        .unwrap();
        let error = sqlx::query("UPDATE public.job SET status='queued' WHERE id=$1")
            .bind(id)
            .execute(&rig.admin_pool)
            .await
            .unwrap_err();
        assert_eq!(sqlstate(&error).as_deref(), Some("23514"), "{terminal}");
    }
}

#[tokio::test]
async fn dispatch_marker_requires_job_in_same_transaction() {
    let rig = TestRig::from_env().await;
    let (actor, _) = rig.seed_actor_space(true).await;
    let (event, key) = outbox(&rig.runtime_pool, actor.actor_id).await;
    let mut incomplete = rig.runtime_pool.begin().await.unwrap();
    sqlx::query("UPDATE public.job_outbox SET dispatched_at=clock_timestamp() WHERE id=$1")
        .bind(event)
        .execute(&mut *incomplete)
        .await
        .unwrap();
    let error = incomplete.commit().await.unwrap_err();
    assert_eq!(sqlstate(&error).as_deref(), Some("23514"));
    let mut tx = rig.runtime_pool.begin().await.unwrap();
    sqlx::query("UPDATE public.job_outbox SET dispatched_at=clock_timestamp() WHERE id=$1")
        .bind(event)
        .execute(&mut *tx)
        .await
        .unwrap();
    let id = job_in_transaction(&mut tx, event, &key).await;
    tx.commit().await.unwrap();
    let linked: Uuid = sqlx::query_scalar("SELECT id FROM public.job WHERE outbox_id=$1")
        .bind(event)
        .fetch_one(&rig.runtime_pool)
        .await
        .unwrap();
    assert_eq!(linked, id);
}

#[tokio::test]
async fn job_insert_without_dispatch_marker_rolls_back() {
    let rig = TestRig::from_env().await;
    let (actor, _) = rig.seed_actor_space(true).await;
    let (event, key) = outbox(&rig.runtime_pool, actor.actor_id).await;
    let mut tx = rig.runtime_pool.begin().await.unwrap();
    sqlx::query("INSERT INTO public.job(id,outbox_id,idempotency_key,status,attempt_count) VALUES($1,$2,$3,'queued',0)")
        .bind(Uuid::new_v4()).bind(event).bind(&key).execute(&mut *tx).await.unwrap();
    let error = tx.commit().await.unwrap_err();
    assert_eq!(sqlstate(&error).as_deref(), Some("23514"));
}

#[tokio::test]
async fn outbox_insert_cannot_start_dispatched_without_a_job() {
    let rig = TestRig::from_env().await;
    let (actor, _) = rig.seed_actor_space(true).await;
    let space_id = Uuid::new_v4();
    let block_id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    let key = format!("asset-integrity:v1:{space_id}:{block_id}:{revision_id}");
    let payload = serde_json::json!({
        "version":1,"kind":"asset_integrity","space_id":space_id,
        "block_id":block_id,"revision_id":revision_id,"actor_id":actor.actor_id
    });
    let error = sqlx::query("INSERT INTO public.job_outbox(id,business_key,event_type,payload_version,payload,actor_id,processor_version,dispatched_at) VALUES($1,$2,'asset_integrity_requested',1,$3,$4,1,clock_timestamp())")
        .bind(Uuid::new_v4()).bind(key).bind(payload).bind(actor.actor_id)
        .execute(&rig.runtime_pool).await.unwrap_err();
    assert_eq!(sqlstate(&error).as_deref(), Some("23514"));
}

#[tokio::test]
async fn queued_job_cannot_start_with_result_or_checkpoint() {
    let rig = TestRig::from_env().await;
    let (actor, _) = rig.seed_actor_space(true).await;
    for statement in [
        "INSERT INTO public.job(id,outbox_id,idempotency_key,status,attempt_count,output_digest) VALUES($1,$2,$3,'queued',0,repeat('a',64))",
        "INSERT INTO public.job(id,outbox_id,idempotency_key,status,attempt_count,last_error_class) VALUES($1,$2,$3,'queued',0,'unexpected')",
        "INSERT INTO public.job(id,outbox_id,idempotency_key,status,attempt_count,checkpoint) VALUES($1,$2,$3,'queued',0,'{}'::jsonb)",
    ] {
        let (event, key) = outbox(&rig.runtime_pool, actor.actor_id).await;
        let mut tx = rig.runtime_pool.begin().await.unwrap();
        let insertion = sqlx::query(statement)
            .bind(Uuid::new_v4())
            .bind(event)
            .bind(&key)
            .execute(&mut *tx)
            .await;
        let error = match insertion {
            Err(error) => {
                tx.rollback().await.unwrap();
                error
            }
            Ok(_) => {
                sqlx::query(
                    "UPDATE public.job_outbox SET dispatched_at=clock_timestamp() WHERE id=$1",
                )
                .bind(event)
                .execute(&mut *tx)
                .await
                .unwrap();
                tx.commit().await.unwrap_err()
            }
        };
        assert_eq!(sqlstate(&error).as_deref(), Some("23514"), "{statement}");
    }
}

#[tokio::test]
async fn retry_wait_requires_a_scheduled_time() {
    let rig = TestRig::from_env().await;
    let (actor, _) = rig.seed_actor_space(true).await;
    let (event, key) = outbox(&rig.runtime_pool, actor.actor_id).await;
    let id = job(&rig.runtime_pool, event, &key).await;
    sqlx::query("UPDATE public.job SET status='running',attempt_count=1,lease_token=$2,lease_expires_at=clock_timestamp()+interval '1 minute' WHERE id=$1")
        .bind(id).bind(Uuid::new_v4()).execute(&rig.admin_pool).await.unwrap();
    let error = sqlx::query("UPDATE public.job SET status='retry_wait',lease_token=NULL,lease_expires_at=NULL WHERE id=$1")
        .bind(id).execute(&rig.admin_pool).await.unwrap_err();
    assert_eq!(sqlstate(&error).as_deref(), Some("23514"));
}

#[tokio::test]
async fn claim_transitions_increment_attempt_and_only_running_can_renew_in_place() {
    let rig = TestRig::from_env().await;
    let (actor, _) = rig.seed_actor_space(true).await;
    let (event, key) = outbox(&rig.runtime_pool, actor.actor_id).await;
    let id = job(&rig.runtime_pool, event, &key).await;
    let queued_noop = sqlx::query("UPDATE public.job SET status='queued' WHERE id=$1")
        .bind(id)
        .execute(&rig.admin_pool)
        .await
        .unwrap_err();
    assert_eq!(sqlstate(&queued_noop).as_deref(), Some("23514"));
    let no_first_attempt = sqlx::query("UPDATE public.job SET status='running',lease_token=$2,lease_expires_at=clock_timestamp()+interval '1 minute' WHERE id=$1")
        .bind(id).bind(Uuid::new_v4()).execute(&rig.admin_pool).await.unwrap_err();
    assert_eq!(sqlstate(&no_first_attempt).as_deref(), Some("23514"));
    let first_token = Uuid::new_v4();
    sqlx::query("UPDATE public.job SET status='running',attempt_count=1,lease_token=$2,lease_expires_at=clock_timestamp()+interval '1 minute' WHERE id=$1")
        .bind(id).bind(first_token).execute(&rig.admin_pool).await.unwrap();
    sqlx::query(
        "UPDATE public.job SET lease_expires_at=clock_timestamp()+interval '2 minutes' WHERE id=$1",
    )
    .bind(id)
    .execute(&rig.admin_pool)
    .await
    .unwrap();
    sqlx::query("UPDATE public.job SET status='retry_wait',lease_token=NULL,lease_expires_at=NULL,next_attempt_at=clock_timestamp()+interval '1 minute' WHERE id=$1")
        .bind(id).execute(&rig.admin_pool).await.unwrap();
    let wait_noop = sqlx::query("UPDATE public.job SET status='retry_wait' WHERE id=$1")
        .bind(id)
        .execute(&rig.admin_pool)
        .await
        .unwrap_err();
    assert_eq!(sqlstate(&wait_noop).as_deref(), Some("23514"));
    let no_second_attempt = sqlx::query("UPDATE public.job SET status='running',next_attempt_at=NULL,lease_token=$2,lease_expires_at=clock_timestamp()+interval '1 minute' WHERE id=$1")
        .bind(id).bind(Uuid::new_v4()).execute(&rig.admin_pool).await.unwrap_err();
    assert_eq!(sqlstate(&no_second_attempt).as_deref(), Some("23514"));
}

async fn job_in_transaction(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event: Uuid,
    key: &str,
) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO public.job(id,outbox_id,idempotency_key,status,attempt_count) VALUES($1,$2,$3,'queued',0)")
        .bind(id).bind(event).bind(key).execute(&mut **tx).await.unwrap();
    id
}

#[tokio::test]
async fn runtime_is_non_owner_and_cannot_administer_job_tables() {
    let rig = TestRig::from_env().await;
    let flags: (bool, bool, bool, bool) = sqlx::query_as("SELECT rolsuper,rolbypassrls,rolcreaterole,rolcreatedb FROM pg_roles WHERE rolname=current_user")
        .fetch_one(&rig.runtime_pool).await.unwrap();
    assert_eq!(flags, (false, false, false, false));
    for table in ["job_outbox", "job"] {
        let owner: bool = sqlx::query_scalar("SELECT tableowner=current_user FROM pg_tables WHERE schemaname='public' AND tablename=$1")
            .bind(table).fetch_one(&rig.runtime_pool).await.unwrap();
        assert!(!owner, "{table}");
        for verb in ["DELETE", "TRUNCATE"] {
            let allowed: bool =
                sqlx::query_scalar("SELECT has_table_privilege(current_user,$1,$2)")
                    .bind(format!("public.{table}"))
                    .bind(verb)
                    .fetch_one(&rig.runtime_pool)
                    .await
                    .unwrap();
            assert!(!allowed, "{table}/{verb}");
        }
        let public_insert: bool = sqlx::query_scalar("SELECT COALESCE(bool_or(a.grantee=0 AND a.privilege_type='INSERT'),false) FROM pg_class c LEFT JOIN LATERAL aclexplode(c.relacl) a ON true WHERE c.oid=$1::regclass")
            .bind(format!("public.{table}")).fetch_one(&rig.admin_pool).await.unwrap();
        assert!(!public_insert, "PUBLIC/{table}");
        let statement = format!("ALTER TABLE public.{table} ADD COLUMN forbidden integer");
        let error = sqlx::raw_sql(&statement)
            .execute(&rig.runtime_pool)
            .await
            .unwrap_err();
        assert_eq!(sqlstate(&error).as_deref(), Some("42501"));
    }
    for (table, column) in [("job_outbox", "business_key"), ("job", "idempotency_key")] {
        let allowed: bool =
            sqlx::query_scalar("SELECT has_column_privilege(current_user,$1,$2,'UPDATE')")
                .bind(format!("public.{table}"))
                .bind(column)
                .fetch_one(&rig.runtime_pool)
                .await
                .unwrap();
        assert!(!allowed, "runtime can rewrite {table}.{column}");
    }
    for column in ["payload", "actor_id", "processor_version", "event_type"] {
        let allowed: bool = sqlx::query_scalar(
            "SELECT has_column_privilege(current_user,'public.job_outbox',$1,'UPDATE')",
        )
        .bind(column)
        .fetch_one(&rig.runtime_pool)
        .await
        .unwrap();
        assert!(!allowed, "runtime can rewrite job_outbox.{column}");
    }
}

#[tokio::test]
async fn original_release_outbox_keeps_its_event_and_release_constraints() {
    let rig = TestRig::from_env().await;
    let release = Uuid::new_v4();
    let wrong_type = sqlx::query("INSERT INTO public.outbox_event(id,event_type,aggregate_id,payload_version) VALUES($1,'asset_integrity_requested',$2,1)")
        .bind(Uuid::new_v4()).bind(release).execute(&rig.runtime_pool).await.unwrap_err();
    assert_eq!(sqlstate(&wrong_type).as_deref(), Some("23514"));
    let missing_release = sqlx::query("INSERT INTO public.outbox_event(id,event_type,aggregate_id,payload_version) VALUES($1,'composition_released',$2,1)")
        .bind(Uuid::new_v4()).bind(release).execute(&rig.runtime_pool).await.unwrap_err();
    assert_eq!(sqlstate(&missing_release).as_deref(), Some("23503"));
}

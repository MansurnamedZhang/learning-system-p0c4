mod support;

use learning_core::{BlockRef, JobInput};
use learning_db::{JobFailureClass, JobStore};
use serde_json::json;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{sync::Arc, time::Duration};
use support::{TestRig, sqlstate};
use tokio::sync::Barrier;
use uuid::Uuid;

const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER_DIGEST: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

async fn queued_job(rig: &TestRig) -> Uuid {
    let (actor, space) = rig.seed_actor_space(true).await;
    let input = JobInput::asset_integrity(
        actor.actor_id,
        space,
        BlockRef {
            block_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
        },
    )
    .unwrap();
    let event = Uuid::new_v4();
    let job = Uuid::new_v4();
    sqlx::query("INSERT INTO public.job_outbox(id,business_key,event_type,payload_version,payload,actor_id,processor_version) VALUES($1,$2,'asset_integrity_requested',1,$3,$4,1)")
        .bind(event)
        .bind(input.business_key())
        .bind(input.to_value())
        .bind(actor.actor_id)
        .execute(&rig.runtime_pool)
        .await
        .unwrap();
    let mut tx = rig.runtime_pool.begin().await.unwrap();
    sqlx::query("INSERT INTO public.job(id,outbox_id,idempotency_key,status,attempt_count) VALUES($1,$2,$3,'queued',0)")
        .bind(job)
        .bind(event)
        .bind(input.business_key())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE public.job_outbox SET dispatched_at=clock_timestamp() WHERE id=$1")
        .bind(event)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    job
}

async fn runtime_store() -> (TestRig, JobStore, Uuid) {
    let rig = TestRig::from_env().await;
    let job = queued_job(&rig).await;
    let store = JobStore::new(rig.runtime_pool.clone());
    (rig, store, job)
}

async fn expire_as_fixture(admin: &PgPool, job: Uuid) {
    sqlx::query(
        "UPDATE public.job SET lease_expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(job)
    .execute(admin)
    .await
    .unwrap();
    let expired: bool = sqlx::query_scalar(
        "SELECT lease_expires_at < clock_timestamp() FROM public.job WHERE id=$1",
    )
    .bind(job)
    .fetch_one(admin)
    .await
    .unwrap();
    assert!(
        expired,
        "fixture must expire according to the database clock"
    );
}

#[tokio::test]
async fn runtime_cannot_update_job_state_or_result_columns_directly() {
    let (rig, _, job) = runtime_store().await;
    for statement in [
        "UPDATE public.job SET status='cancelled' WHERE id=$1",
        "UPDATE public.job SET output_digest='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' WHERE id=$1",
    ] {
        let error = sqlx::query(statement)
            .bind(job)
            .execute(&rig.runtime_pool)
            .await
            .unwrap_err();
        assert_eq!(sqlstate(&error).as_deref(), Some("42501"));
    }
}

#[tokio::test]
async fn runtime_cannot_read_another_workers_current_lease_token() {
    let (rig, _, job) = runtime_store().await;
    let token = Uuid::new_v4();
    sqlx::query("UPDATE public.job SET status='running',attempt_count=1,lease_token=$2,lease_expires_at=clock_timestamp()+interval '30 seconds' WHERE id=$1")
        .bind(job)
        .bind(token)
        .execute(&rig.admin_pool)
        .await
        .unwrap();
    let error =
        sqlx::query_scalar::<_, Option<Uuid>>("SELECT lease_token FROM public.job WHERE id=$1")
            .bind(job)
            .fetch_one(&rig.runtime_pool)
            .await
            .unwrap_err();
    assert_eq!(sqlstate(&error).as_deref(), Some("42501"));
}

#[tokio::test]
async fn lease_functions_are_not_executable_by_public() {
    let rig = TestRig::from_env().await;
    for signature in [
        "public.p0c2_claim_job(uuid,bigint)",
        "public.p0c2_renew_job(uuid,uuid,bigint)",
        "public.p0c2_checkpoint_job(uuid,uuid,jsonb)",
        "public.p0c2_succeed_job(uuid,uuid,text)",
        "public.p0c2_fail_job(uuid,uuid,text,boolean)",
        "public.p0c2_cancel_job(uuid)",
    ] {
        let public_can_execute: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_proc AS p \
             CROSS JOIN LATERAL pg_catalog.aclexplode(COALESCE(p.proacl,pg_catalog.acldefault('f',p.proowner))) AS a \
             WHERE p.oid=$1::regprocedure AND a.grantee=0 AND a.privilege_type='EXECUTE')",
        )
        .bind(signature)
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap();
        assert!(!public_can_execute, "PUBLIC can execute {signature}");
    }
}

#[tokio::test]
async fn runtime_cannot_reuse_an_old_token_when_a_retry_becomes_due() {
    let (rig, _, job) = runtime_store().await;
    let old_token = Uuid::new_v4();
    sqlx::query("UPDATE public.job SET status='running',attempt_count=1,lease_token=$2,lease_expires_at=clock_timestamp()+interval '30 seconds' WHERE id=$1")
        .bind(job)
        .bind(old_token)
        .execute(&rig.admin_pool)
        .await
        .unwrap();
    sqlx::query("UPDATE public.job SET status='retry_wait',lease_token=NULL,lease_expires_at=NULL,next_attempt_at=clock_timestamp()-interval '1 second',last_error_class='transient_storage' WHERE id=$1")
        .bind(job)
        .execute(&rig.admin_pool)
        .await
        .unwrap();

    // A runtime caller knows its previous token. The database must not let
    // that caller choose the next token after retry_wait clears the column.
    let chosen: Result<Uuid, sqlx::Error> =
        sqlx::query_scalar("SELECT token FROM public.p0c2_claim_job($1,$2,30000)")
            .bind(job)
            .bind(old_token)
            .fetch_one(&rig.runtime_pool)
            .await;
    let error = chosen.expect_err("runtime can still choose a lease token");
    assert!(
        matches!(sqlstate(&error).as_deref(), Some("42501" | "42883")),
        "unexpected token-choice failure: {error}"
    );
    let fresh = JobStore::new(rig.runtime_pool.clone())
        .claim(job, Duration::from_secs(30))
        .await
        .unwrap()
        .unwrap();
    assert_ne!(fresh.token, old_token, "old lease token was reused");
}

#[tokio::test]
async fn two_independent_connections_claim_the_same_job_once() {
    let (rig, _, job) = runtime_store().await;
    let url = std::env::var("TEST_DATABASE_URL").unwrap();
    let first = JobStore::new(
        PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap(),
    );
    let second = JobStore::new(
        PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap(),
    );
    let barrier = Arc::new(Barrier::new(3));
    let a = barrier.clone();
    let b = barrier.clone();
    let (left, right, _) = tokio::join!(
        async {
            a.wait().await;
            first.claim(job, Duration::from_secs(30)).await
        },
        async {
            b.wait().await;
            second.claim(job, Duration::from_secs(30)).await
        },
        barrier.wait(),
    );
    let claims = [left.unwrap(), right.unwrap()];
    assert_eq!(claims.iter().filter(|claim| claim.is_some()).count(), 1);
    let winner = claims.into_iter().flatten().next().unwrap();
    assert_eq!(winner.job_id, job);
    assert_eq!(winner.attempt_count, 1);
    let state: (String, i32, Uuid, chrono::DateTime<chrono::Utc>, bool) = sqlx::query_as(
        "SELECT status,attempt_count,lease_token,lease_expires_at,lease_expires_at>clock_timestamp() FROM public.job WHERE id=$1",
    )
    .bind(job)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(
        state,
        (
            "running".into(),
            1,
            winner.token,
            winner.lease_expires_at,
            true
        )
    );
}

#[tokio::test]
async fn expired_token_is_fenced_before_and_after_another_claim() {
    let (rig, store, job) = runtime_store().await;
    let first = store
        .claim(job, Duration::from_secs(30))
        .await
        .unwrap()
        .unwrap();
    expire_as_fixture(&rig.admin_pool, job).await;
    assert!(
        !store
            .renew(job, first.token, Duration::from_secs(30))
            .await
            .unwrap()
    );
    assert!(
        !store
            .checkpoint(job, first.token, json!({"part": 1}))
            .await
            .unwrap()
    );
    assert!(!store.succeed(job, first.token, DIGEST).await.unwrap());
    assert!(
        !store
            .fail(job, first.token, JobFailureClass::TransientStorage)
            .await
            .unwrap()
    );
    let before: (String, i32, Option<String>, Option<serde_json::Value>) = sqlx::query_as(
        "SELECT status,attempt_count,output_digest,checkpoint FROM public.job WHERE id=$1",
    )
    .bind(job)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(before, ("running".into(), 1, None, None));

    let second = store
        .claim(job, Duration::from_secs(30))
        .await
        .unwrap()
        .unwrap();
    assert_ne!(second.token, first.token);
    assert_eq!(second.attempt_count, 2);
    assert!(
        !store
            .renew(job, first.token, Duration::from_secs(30))
            .await
            .unwrap()
    );
    assert!(
        !store
            .checkpoint(job, first.token, json!({"part": 2}))
            .await
            .unwrap()
    );
    assert!(!store.succeed(job, first.token, DIGEST).await.unwrap());
    assert!(
        !store
            .fail(job, first.token, JobFailureClass::TransientStorage)
            .await
            .unwrap()
    );
    assert!(
        store
            .renew(job, second.token, Duration::from_secs(30))
            .await
            .unwrap()
    );
    assert!(
        store
            .checkpoint(job, second.token, json!({"part": 2}))
            .await
            .unwrap()
    );
    assert!(store.succeed(job, second.token, DIGEST).await.unwrap());
    let finished: (String, i32, Option<String>) =
        sqlx::query_as("SELECT status,attempt_count,output_digest FROM public.job WHERE id=$1")
            .bind(job)
            .fetch_one(&rig.admin_pool)
            .await
            .unwrap();
    assert_eq!(finished, ("succeeded".into(), 2, Some(DIGEST.into())));
}

#[tokio::test]
async fn retry_wait_uses_database_deadline_and_records_error_class() {
    let (rig, store, job) = runtime_store().await;
    let first = store
        .claim(job, Duration::from_secs(30))
        .await
        .unwrap()
        .unwrap();
    assert!(
        store
            .fail(job, first.token, JobFailureClass::TransientStorage)
            .await
            .unwrap()
    );
    let waiting: (String, String, bool, f64) = sqlx::query_as(
        "SELECT status,last_error_class,next_attempt_at>clock_timestamp(),EXTRACT(EPOCH FROM next_attempt_at-updated_at)::float8 FROM public.job WHERE id=$1",
    )
    .bind(job)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(
        (waiting.0.as_str(), waiting.1.as_str(), waiting.2),
        ("retry_wait", "transient_storage", true)
    );
    assert!(
        (0.9..=1.1).contains(&waiting.3),
        "first retry should wait one second: {}",
        waiting.3
    );
    assert!(
        store
            .claim(job, Duration::from_secs(30))
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("SELECT pg_sleep(GREATEST(0.0,EXTRACT(EPOCH FROM next_attempt_at-clock_timestamp()))::float8+0.05) FROM public.job WHERE id=$1")
        .bind(job)
        .execute(&rig.admin_pool)
        .await
        .unwrap();
    let second = store
        .claim(job, Duration::from_secs(30))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second.attempt_count, 2);
    assert_ne!(second.token, first.token);
    assert!(
        store
            .fail(job, second.token, JobFailureClass::TransientStorage)
            .await
            .unwrap()
    );
    let second_wait: f64 = sqlx::query_scalar(
        "SELECT EXTRACT(EPOCH FROM next_attempt_at-updated_at)::float8 FROM public.job WHERE id=$1",
    )
    .bind(job)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert!(
        (1.9..=2.1).contains(&second_wait),
        "second retry should wait two seconds: {second_wait}"
    );
}

#[tokio::test]
async fn queued_and_waiting_jobs_can_be_cancelled_without_resurrection() {
    let (rig, store, queued) = runtime_store().await;
    assert!(store.cancel(queued).await.unwrap());
    assert!(!store.cancel(queued).await.unwrap());
    assert!(
        store
            .claim(queued, Duration::from_secs(30))
            .await
            .unwrap()
            .is_none()
    );

    let waiting = queued_job(&rig).await;
    let claim = store
        .claim(waiting, Duration::from_secs(30))
        .await
        .unwrap()
        .unwrap();
    assert!(
        store
            .fail(waiting, claim.token, JobFailureClass::TransientStorage)
            .await
            .unwrap()
    );
    assert!(store.cancel(waiting).await.unwrap());
    assert!(
        store
            .claim(waiting, Duration::from_secs(30))
            .await
            .unwrap()
            .is_none()
    );
    let states: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT id,status FROM public.job WHERE id=$1 OR id=$2 ORDER BY id")
            .bind(queued)
            .bind(waiting)
            .fetch_all(&rig.admin_pool)
            .await
            .unwrap();
    assert_eq!(states.len(), 2);
    assert!(states.iter().all(|(_, status)| status == "cancelled"));
}

#[tokio::test]
async fn third_retryable_failure_is_terminal_and_permanent_error_never_retries() {
    let (rig, store, job) = runtime_store().await;
    let first = store
        .claim(job, Duration::from_secs(30))
        .await
        .unwrap()
        .unwrap();
    // The owner creates the later-attempt fixture without waiting through two
    // prior backoffs; the public failure method still decides the final state.
    expire_as_fixture(&rig.admin_pool, job).await;
    let second_token = Uuid::new_v4();
    sqlx::query("UPDATE public.job SET attempt_count=2,lease_token=$2,lease_expires_at=clock_timestamp()+interval '30 seconds' WHERE id=$1")
        .bind(job).bind(second_token).execute(&rig.admin_pool).await.unwrap();
    expire_as_fixture(&rig.admin_pool, job).await;
    let third_token = Uuid::new_v4();
    sqlx::query("UPDATE public.job SET attempt_count=3,lease_token=$2,lease_expires_at=clock_timestamp()+interval '30 seconds' WHERE id=$1")
        .bind(job).bind(third_token).execute(&rig.admin_pool).await.unwrap();
    assert_ne!(first.token, third_token);
    assert!(
        store
            .fail(job, third_token, JobFailureClass::TransientStorage)
            .await
            .unwrap()
    );
    let exhausted: (String, i32, String, Option<chrono::DateTime<chrono::Utc>>) = sqlx::query_as(
        "SELECT status,attempt_count,last_error_class,next_attempt_at FROM public.job WHERE id=$1",
    )
    .bind(job)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(
        exhausted,
        ("failed".into(), 3, "transient_storage".into(), None)
    );
    assert!(
        store
            .claim(job, Duration::from_secs(30))
            .await
            .unwrap()
            .is_none()
    );

    let permanent = queued_job(&rig).await;
    let claim = store
        .claim(permanent, Duration::from_secs(30))
        .await
        .unwrap()
        .unwrap();
    assert!(
        store
            .fail(permanent, claim.token, JobFailureClass::InvalidInput)
            .await
            .unwrap()
    );
    let state: (String, String, Option<chrono::DateTime<chrono::Utc>>) = sqlx::query_as(
        "SELECT status,last_error_class,next_attempt_at FROM public.job WHERE id=$1",
    )
    .bind(permanent)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(state, ("failed".into(), "invalid_input".into(), None));
}

#[tokio::test]
async fn expired_third_attempt_cannot_be_claimed_a_fourth_time() {
    let (rig, store, job) = runtime_store().await;
    let first = store
        .claim(job, Duration::from_secs(30))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.attempt_count, 1);
    expire_as_fixture(&rig.admin_pool, job).await;
    let second = store
        .claim(job, Duration::from_secs(30))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second.attempt_count, 2);
    expire_as_fixture(&rig.admin_pool, job).await;
    let third = store
        .claim(job, Duration::from_secs(30))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(third.attempt_count, 3);
    expire_as_fixture(&rig.admin_pool, job).await;
    assert!(
        store
            .claim(job, Duration::from_secs(30))
            .await
            .unwrap()
            .is_none()
    );
    let state: (String, i32, Option<Uuid>, String) = sqlx::query_as(
        "SELECT status,attempt_count,lease_token,last_error_class FROM public.job WHERE id=$1",
    )
    .bind(job)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(state, ("failed".into(), 3, None, "lease_expired".into()));
}

#[tokio::test]
async fn cancellation_racing_completion_has_one_terminal_winner() {
    let (rig, store, job) = runtime_store().await;
    let claim = store
        .claim(job, Duration::from_secs(30))
        .await
        .unwrap()
        .unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let a = barrier.clone();
    let b = barrier.clone();
    let url = std::env::var("TEST_DATABASE_URL").unwrap();
    let cancel = JobStore::new(
        PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap(),
    );
    let finish = JobStore::new(
        PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap(),
    );
    let (cancelled, succeeded, _) = tokio::join!(
        async {
            a.wait().await;
            cancel.cancel(job).await
        },
        async {
            b.wait().await;
            finish.succeed(job, claim.token, DIGEST).await
        },
        barrier.wait(),
    );
    assert_eq!(
        u8::from(cancelled.unwrap()) + u8::from(succeeded.unwrap()),
        1
    );
    let state: (String, Option<String>, Option<Uuid>) =
        sqlx::query_as("SELECT status,output_digest,lease_token FROM public.job WHERE id=$1")
            .bind(job)
            .fetch_one(&rig.admin_pool)
            .await
            .unwrap();
    match state.0.as_str() {
        "cancelled" => assert_eq!((state.1, state.2), (None, None)),
        "succeeded" => assert_eq!((state.1, state.2), (Some(DIGEST.into()), None)),
        other => panic!("unexpected terminal state: {other}"),
    }
}

#[tokio::test]
async fn completed_job_cannot_rewrite_output_or_be_cancelled() {
    let (rig, store, job) = runtime_store().await;
    let claim = store
        .claim(job, Duration::from_secs(30))
        .await
        .unwrap()
        .unwrap();
    assert!(store.succeed(job, claim.token, DIGEST).await.unwrap());
    assert!(!store.succeed(job, claim.token, OTHER_DIGEST).await.unwrap());
    assert!(
        !store
            .checkpoint(job, claim.token, json!({"late": true}))
            .await
            .unwrap()
    );
    assert!(
        !store
            .fail(job, claim.token, JobFailureClass::InvalidInput)
            .await
            .unwrap()
    );
    assert!(!store.cancel(job).await.unwrap());
    assert!(
        store
            .claim(job, Duration::from_secs(30))
            .await
            .unwrap()
            .is_none()
    );
    let state: (String, i32, Option<String>, Option<Uuid>) = sqlx::query_as(
        "SELECT status,attempt_count,output_digest,lease_token FROM public.job WHERE id=$1",
    )
    .bind(job)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(state, ("succeeded".into(), 1, Some(DIGEST.into()), None));
}

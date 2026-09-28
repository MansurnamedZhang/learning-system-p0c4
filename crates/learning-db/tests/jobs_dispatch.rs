mod support;

use learning_core::{BlockRef, JobInput, JobStatus};
use learning_db::JobStore;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use support::{TestRig, assembly as a};
use tokio::sync::Barrier;
use uuid::Uuid;

async fn pending(rig: &TestRig, actor: Uuid) -> (Uuid, String) {
    let input = JobInput::asset_integrity(
        actor,
        Uuid::new_v4(),
        BlockRef {
            block_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
        },
    )
    .unwrap();
    let event = Uuid::new_v4();
    let key = input.business_key();
    // The test database persists across integration binaries. Put this event
    // before every already-pending event so a bounded batch exercises it.
    sqlx::query("INSERT INTO public.job_outbox(id,business_key,event_type,payload_version,payload,actor_id,processor_version,created_at) VALUES($1,$2,'asset_integrity_requested',1,$3,$4,1,COALESCE((SELECT min(created_at)-interval '1 minute' FROM public.job_outbox WHERE dispatched_at IS NULL),clock_timestamp()))")
        .bind(event).bind(&key).bind(input.to_value()).bind(actor)
        .execute(&rig.runtime_pool).await.unwrap();
    (event, key)
}

async fn linked_job(rig: &TestRig, event: Uuid, key: &str) -> Uuid {
    let row: (Uuid, String, String, i32, bool) = sqlx::query_as(
        "SELECT j.id,j.idempotency_key,j.status,j.attempt_count,\
                e.dispatched_at IS NOT NULL \
           FROM public.job j JOIN public.job_outbox e ON e.id=j.outbox_id \
          WHERE j.outbox_id=$1",
    )
    .bind(event)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(row.1, key);
    assert_eq!(row.2, "queued");
    assert_eq!(row.3, 0);
    assert!(row.4);
    row.0
}

#[tokio::test]
async fn batch_limit_transfers_only_one_pending_event_per_call() {
    let rig = TestRig::from_env().await;
    let (actor, _) = rig.seed_actor_space(true).await;
    let events = [
        pending(&rig, actor.actor_id).await,
        pending(&rig, actor.actor_id).await,
    ];
    let store = JobStore::new(rig.runtime_pool.clone());
    assert_eq!(store.dispatch_pending(1).await.unwrap(), 1);
    let first_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public.job WHERE outbox_id=$1 OR outbox_id=$2")
            .bind(events[0].0)
            .bind(events[1].0)
            .fetch_one(&rig.admin_pool)
            .await
            .unwrap();
    assert_eq!(first_count, 1);
    assert_eq!(store.dispatch_pending(1).await.unwrap(), 1);
    linked_job(&rig, events[0].0, &events[0].1).await;
    linked_job(&rig, events[1].0, &events[1].1).await;
}

#[tokio::test]
async fn stopped_before_transfer_keeps_event_pending_until_retry() {
    let rig = TestRig::from_env().await;
    let (actor, _) = rig.seed_actor_space(true).await;
    let (event, key) = pending(&rig, actor.actor_id).await;
    let mut interrupted = rig.runtime_pool.begin().await.unwrap();
    let locked: Uuid =
        sqlx::query_scalar("SELECT id FROM public.job_outbox WHERE id=$1 FOR UPDATE")
            .bind(event)
            .fetch_one(&mut *interrupted)
            .await
            .unwrap();
    assert_eq!(locked, event);
    interrupted.rollback().await.unwrap();
    let store = JobStore::new(rig.runtime_pool.clone());
    assert!(store.dispatch_pending(8).await.unwrap() >= 1);
    let first_id = linked_job(&rig, event, &key).await;
    assert_eq!(
        store.get(first_id).await.unwrap().unwrap().status,
        JobStatus::Queued
    );
    store.dispatch_pending(8).await.unwrap();
    assert_eq!(linked_job(&rig, event, &key).await, first_id);
}

#[tokio::test]
async fn stopped_after_job_insert_before_dispatch_marker_rolls_back_and_recovers() {
    let rig = TestRig::from_env().await;
    let (actor, _) = rig.seed_actor_space(true).await;
    let (event, key) = pending(&rig, actor.actor_id).await;
    let mut interrupted = rig.runtime_pool.begin().await.unwrap();
    sqlx::query("INSERT INTO public.job(id,outbox_id,idempotency_key,status,attempt_count) VALUES($1,$2,$3,'queued',0)")
        .bind(Uuid::new_v4()).bind(event).bind(&key)
        .execute(&mut *interrupted).await.unwrap();
    interrupted.rollback().await.unwrap();
    let before: (i64, bool) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM public.job WHERE outbox_id=$1),\
                (SELECT dispatched_at IS NULL FROM public.job_outbox WHERE id=$1)",
    )
    .bind(event)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(before, (0, true));
    let store = JobStore::new(rig.runtime_pool.clone());
    assert!(store.dispatch_pending(8).await.unwrap() >= 1);
    linked_job(&rig, event, &key).await;
}

#[tokio::test]
async fn two_connections_dispatch_the_same_pending_events_once() {
    let rig = TestRig::from_env().await;
    let (actor, _) = rig.seed_actor_space(true).await;
    let events = [
        pending(&rig, actor.actor_id).await,
        pending(&rig, actor.actor_id).await,
    ];
    let url = std::env::var("TEST_DATABASE_URL").unwrap();
    let first_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    let second_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    let first = JobStore::new(first_pool);
    let second = JobStore::new(second_pool);
    let barrier = Arc::new(Barrier::new(3));
    let first_start = barrier.clone();
    let second_start = barrier.clone();
    let (a, b, _) = tokio::join!(
        async {
            first_start.wait().await;
            first.dispatch_pending(8).await
        },
        async {
            second_start.wait().await;
            second.dispatch_pending(8).await
        },
        barrier.wait(),
    );
    assert!(a.unwrap() + b.unwrap() >= 2);
    let ids = [
        linked_job(&rig, events[0].0, &events[0].1).await,
        linked_job(&rig, events[1].0, &events[1].1).await,
    ];
    assert_ne!(ids[0], ids[1]);
    first.dispatch_pending(8).await.unwrap();
    assert_eq!(linked_job(&rig, events[0].0, &events[0].1).await, ids[0]);
    assert_eq!(linked_job(&rig, events[1].0, &events[1].1).await, ids[1]);
    let job_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public.job WHERE outbox_id=$1 OR outbox_id=$2")
            .bind(events[0].0)
            .bind(events[1].0)
            .fetch_one(&rig.admin_pool)
            .await
            .unwrap();
    assert_eq!(job_count, 2);
}

#[tokio::test]
async fn release_only_outbox_is_left_untouched_by_job_transfer() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let original = rig
        .store
        .create(actor, space, support::command("v1 release body"))
        .await
        .unwrap();
    let composition = rig
        .compositions()
        .save(actor, space, a::doc(vec![a::block(&original)]))
        .await
        .unwrap();
    let release = rig
        .releases()
        .publish(actor, space, a::publish(vec![a::root(&composition, None)]))
        .await
        .unwrap();
    let legacy: (Uuid, String, i32) = sqlx::query_as(
        "SELECT id,event_type,payload_version FROM public.outbox_event WHERE aggregate_id=$1",
    )
    .bind(release.release_id)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(legacy.1, "composition_released");
    let (event, key) = pending(&rig, actor.actor_id).await;
    assert!(
        JobStore::new(rig.runtime_pool.clone())
            .dispatch_pending(8)
            .await
            .unwrap()
            >= 1
    );
    linked_job(&rig, event, &key).await;
    let after: (Uuid, String, i32) = sqlx::query_as(
        "SELECT id,event_type,payload_version FROM public.outbox_event WHERE aggregate_id=$1",
    )
    .bind(release.release_id)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(after, legacy);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM public.job j JOIN public.job_outbox e ON e.id=j.outbox_id WHERE e.actor_id=$1")
        .bind(actor.actor_id).fetch_one(&rig.admin_pool).await.unwrap();
    assert_eq!(count, 1);
}

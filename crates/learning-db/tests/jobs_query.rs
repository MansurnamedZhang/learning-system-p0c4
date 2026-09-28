mod support;

use learning_core::{BlockRef, JobInput, JobStatus};
use learning_db::JobStore;
use support::TestRig;
use uuid::Uuid;

#[tokio::test]
async fn job_query_returns_exact_input_and_absent_id_is_none() {
    let rig = TestRig::from_env().await;
    let (actor, space_id) = rig.seed_actor_space(true).await;
    let block = BlockRef {
        block_id: Uuid::new_v4(),
        revision_id: Uuid::new_v4(),
    };
    let input = JobInput::asset_integrity(actor.actor_id, space_id, block).unwrap();
    let key = input.business_key();
    let event = Uuid::new_v4();
    let job = Uuid::new_v4();
    sqlx::query("INSERT INTO public.job_outbox(id,business_key,event_type,payload_version,payload,actor_id,processor_version) VALUES($1,$2,'asset_integrity_requested',1,$3,$4,1)")
        .bind(event).bind(&key).bind(input.to_value()).bind(actor.actor_id)
        .execute(&rig.runtime_pool).await.unwrap();
    let mut tx = rig.runtime_pool.begin().await.unwrap();
    sqlx::query("INSERT INTO public.job(id,outbox_id,idempotency_key,status,attempt_count) VALUES($1,$2,$3,'queued',0)")
        .bind(job).bind(event).bind(&key).execute(&mut *tx).await.unwrap();
    sqlx::query("UPDATE public.job_outbox SET dispatched_at=clock_timestamp() WHERE id=$1")
        .bind(event)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let store = JobStore::new(rig.runtime_pool.clone());
    assert!(store.get(Uuid::new_v4()).await.unwrap().is_none());
    let saved = store.get(job).await.unwrap().unwrap();
    assert_eq!(saved.id, job);
    assert_eq!(saved.outbox_id, event);
    assert_eq!(saved.idempotency_key, key);
    assert_eq!(saved.input, input);
    assert_eq!(saved.status, JobStatus::Queued);
    assert_eq!(saved.attempt_count, 0);
}

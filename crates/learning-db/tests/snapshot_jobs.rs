#[path = "support/relation_store.rs"]
mod relations;
mod support;

use learning_core::{JobInput, ReadingMode, ReadingRef, SnapshotRequest};
use serde_json::json;
use uuid::Uuid;

fn request() -> SnapshotRequest {
    SnapshotRequest {
        reading: ReadingRef {
            view_id: Uuid::from_u128(3),
            revision_id: Uuid::from_u128(4),
        },
        mode: ReadingMode::Fused,
        include_personal: true,
        include_originals: true,
        resource_versions: vec![],
        source_segments: vec![],
    }
}

#[test]
fn snapshot_decoder_accepts_versioned_exact_request_and_preserves_options() {
    let value = json!({"version":1,"kind":"snapshot_export","actor_id":Uuid::from_u128(1),
        "space_id":Uuid::from_u128(2),"request":request()});
    let input = JobInput::from_value(value.clone()).expect("C3 snapshot job must decode");
    assert_eq!(input.to_value(), value);
    assert!(
        input.asset_block().is_none(),
        "snapshot must not enter the C2 asset processor"
    );
    let mut changed = value.clone();
    changed["request"]["include_originals"] = json!(false);
    assert_ne!(
        JobInput::from_value(changed).unwrap().business_key(),
        input.business_key()
    );
    for (key, invalid) in [
        ("version", json!(2)),
        ("actor_id", json!(Uuid::nil())),
        ("unknown", json!(true)),
    ] {
        let mut bad = value.clone();
        bad[key] = invalid;
        assert!(JobInput::from_value(bad).is_err());
    }
    let mut bad = value;
    bad["request"]["reading"]["extra"] = json!(true);
    assert!(JobInput::from_value(bad).is_err());
}

#[test]
fn snapshot_copy_request_is_valid_and_cannot_be_confused_with_exact() {
    let exact = json!({"version":1,"kind":"snapshot_export","actor_id":Uuid::from_u128(1),
        "space_id":Uuid::from_u128(2),"request":request()});
    let mut copy = exact.clone();
    copy["request"]["include_personal"] = json!(false);
    let decoded = JobInput::from_value(copy.clone()).unwrap();
    assert_eq!(decoded.to_value(), copy);
    assert_ne!(
        decoded.business_key(),
        JobInput::from_value(exact).unwrap().business_key()
    );
}

use learning_db::{AssetProcessOutcome, JobStore, SnapshotStore};
use std::time::Duration;

async fn queued() -> (
    support::TestRig,
    learning_core::Principal,
    Uuid,
    SnapshotStore,
    JobStore,
    Uuid,
    SnapshotRequest,
) {
    let (rig, actor, space, _, saved) = support::reading::fixture().await;
    let store = SnapshotStore::new(rig.runtime_pool.clone());
    let jobs = JobStore::new(rig.runtime_pool.clone());
    let mut input = request();
    input.reading = saved.view;
    let id = store
        .enqueue_export(actor, Uuid::new_v4(), input.clone())
        .await
        .unwrap();
    jobs.dispatch_pending(256).await.unwrap();
    (rig, actor, space, store, jobs, id, input)
}

#[tokio::test]
async fn old_worker_cannot_consume_snapshot_attempt_new_worker_can() {
    let (_, _, _, _, jobs, id, _) = queued().await;
    assert!(!jobs.runnable_ids(256).await.unwrap().contains(&id));
    assert!(jobs.runnable_snapshot_ids(256).await.unwrap().contains(&id));
    assert!(
        jobs.claim(id, Duration::from_secs(30))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(jobs.get(id).await.unwrap().unwrap().attempt_count, 0);
    let lease = jobs
        .claim_snapshot(id, Duration::from_secs(30))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(lease.attempt_count, 1);
    assert!(
        jobs.claim(id, Duration::from_secs(30))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn request_key_replays_same_job_and_rejects_changed_options_atomically() {
    let (rig, actor, _, store, _, _, input) = queued().await;
    let key = Uuid::new_v4();
    let id = store
        .enqueue_export(actor, key, input.clone())
        .await
        .unwrap();
    assert_eq!(
        id,
        store
            .enqueue_export(actor, key, input.clone())
            .await
            .unwrap()
    );
    let mut changed = input;
    changed.include_originals = false;
    assert!(matches!(
        store.enqueue_export(actor, key, changed).await,
        Err(learning_core::ContentError::IdempotencyConflict)
    ));
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM request_key WHERE actor_id=$1 AND request_id=$2")
            .bind(actor.actor_id)
            .bind(key)
            .fetch_one(&rig.admin_pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn database_and_rust_reject_same_malformed_snapshot_payloads_and_keep_asset_branch() {
    let (rig, actor, space, _, _, _, _) = queued().await;
    let input = JobInput::snapshot_export(actor.actor_id, space, request()).unwrap();
    let value = input.to_value();
    let good: bool =
        sqlx::query_scalar("SELECT p0c3_valid_job_payload('snapshot_export_requested',1,$1,$2,$3)")
            .bind(&value)
            .bind(actor.actor_id)
            .bind(input.business_key())
            .fetch_one(&rig.runtime_pool)
            .await
            .unwrap();
    assert!(good);
    let mismatched: bool =
        sqlx::query_scalar("SELECT p0c3_valid_job_payload('snapshot_export_requested',1,$1,$2,$3)")
            .bind(&value)
            .bind(actor.actor_id)
            .bind(format!(
                "snapshot-export:v1:{}:{}:{}",
                actor.actor_id,
                space,
                "0".repeat(64)
            ))
            .fetch_one(&rig.runtime_pool)
            .await
            .unwrap();
    assert!(
        !mismatched,
        "valid payload must still reject a mismatched key"
    );
    let asset = JobInput::asset_integrity(
        actor.actor_id,
        space,
        learning_core::BlockRef {
            block_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
        },
    )
    .unwrap();
    let old_good: bool =
        sqlx::query_scalar("SELECT p0c3_valid_job_payload('asset_integrity_requested',1,$1,$2,$3)")
            .bind(asset.to_value())
            .bind(actor.actor_id)
            .bind(asset.business_key())
            .fetch_one(&rig.runtime_pool)
            .await
            .unwrap();
    assert!(old_good);
    let old_rejects: bool =
        sqlx::query_scalar("SELECT p0c2_valid_job_payload('snapshot_export_requested',1,$1,$2,$3)")
            .bind(&value)
            .bind(actor.actor_id)
            .bind(input.business_key())
            .fetch_one(&rig.runtime_pool)
            .await
            .unwrap();
    assert!(!old_rejects);
    let mut malformed = vec![];
    for (key, bad) in [
        ("version", json!(2)),
        ("extra", json!(1)),
        ("actor_id", json!(Uuid::nil())),
        ("request", json!(null)),
    ] {
        let mut v = value.clone();
        v[key] = bad;
        malformed.push(v);
    }
    for (key, bad) in [
        ("mode", json!("unknown")),
        ("include_personal", json!(1)),
        ("resource_versions", json!([{"space_id":Uuid::new_v4()}])),
        ("source_segments", json!(null)),
    ] {
        let mut v = value.clone();
        v["request"][key] = bad;
        malformed.push(v);
    }
    let mut nested = value.clone();
    nested["request"]["reading"]["unknown"] = json!(true);
    malformed.push(nested);
    let mut changed = value.clone();
    changed["request"]["include_originals"] = json!(false);
    let changed_valid: bool =
        sqlx::query_scalar("SELECT p0c3_valid_job_payload('snapshot_export_requested',1,$1,$2,$3)")
            .bind(&changed)
            .bind(actor.actor_id)
            .bind(format!(
                "snapshot-export:v1:{}:{}:{}",
                actor.actor_id,
                space,
                learning_core::canonical_record_hash(&changed["request"])
            ))
            .fetch_one(&rig.runtime_pool)
            .await
            .unwrap();
    assert!(changed_valid);
    for v in malformed {
        assert!(JobInput::from_value(v.clone()).is_err());
        let valid: bool = sqlx::query_scalar(
            "SELECT p0c3_valid_job_payload('snapshot_export_requested',1,$1,$2,$3)",
        )
        .bind(&v)
        .bind(actor.actor_id)
        .bind(format!(
            "snapshot-export:v1:{}:{}:{}",
            actor.actor_id,
            space,
            learning_core::canonical_record_hash(&v["request"])
        ))
        .fetch_one(&rig.runtime_pool)
        .await
        .unwrap();
        assert!(!valid);
    }
}

#[tokio::test]
async fn legacy_dispatcher_random_id_is_normalized_only_for_c3() {
    let (rig, actor, _, _, saved) = support::reading::fixture().await;
    let mut input = request();
    input.reading = saved.view;
    let store = SnapshotStore::new(rig.runtime_pool.clone());
    let id = store
        .enqueue_export(actor, Uuid::new_v4(), input)
        .await
        .unwrap();
    let mut tx = rig.runtime_pool.begin().await.unwrap();
    let actual:Uuid=sqlx::query_scalar("INSERT INTO job(id,outbox_id,idempotency_key,status,attempt_count) SELECT $1,id,business_key,'queued',0 FROM job_outbox WHERE id=$2 RETURNING id")
        .bind(Uuid::new_v4()).bind(id).fetch_one(&mut *tx).await.unwrap();
    sqlx::query("UPDATE job_outbox SET dispatched_at=clock_timestamp() WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(id, actual);
    let jobs = JobStore::new(rig.runtime_pool.clone());
    assert!(
        jobs.claim(id, Duration::from_secs(30))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(jobs.get(id).await.unwrap().unwrap().attempt_count, 0);
    assert!(
        store
            .claim_export(id, Duration::from_secs(30))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(jobs.get(id).await.unwrap().unwrap().attempt_count, 0);
}

#[tokio::test]
async fn sixty_four_c3_jobs_cannot_starve_legacy_c2_scan_and_generic_calls_are_inert() {
    let rig = support::TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let jobs = JobStore::new(rig.runtime_pool.clone());
    let mut snapshots = vec![];
    for _ in 0..64 {
        let mut req = request();
        req.reading.view_id = Uuid::new_v4();
        let input = JobInput::snapshot_export(actor.actor_id, space, req).unwrap();
        let event = Uuid::new_v4();
        sqlx::query("INSERT INTO job_outbox(id,business_key,event_type,payload_version,payload,actor_id,processor_version) VALUES($1,$2,'snapshot_export_requested',1,$3,$4,1)")
            .bind(event).bind(input.business_key()).bind(input.to_value()).bind(actor.actor_id).execute(&rig.runtime_pool).await.unwrap();
        snapshots.push(event);
    }
    let asset = JobInput::asset_integrity(
        actor.actor_id,
        space,
        learning_core::BlockRef {
            block_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
        },
    )
    .unwrap();
    let event = Uuid::new_v4();
    sqlx::query("INSERT INTO job_outbox(id,business_key,event_type,payload_version,payload,actor_id,processor_version) VALUES($1,$2,'asset_integrity_requested',1,$3,$4,1)")
        .bind(event).bind(asset.business_key()).bind(asset.to_value()).bind(actor.actor_id).execute(&rig.runtime_pool).await.unwrap();
    let mut dispatch = rig.runtime_pool.begin().await.unwrap();
    for (position, id) in snapshots.iter().chain(std::iter::once(&event)).enumerate() {
        sqlx::query("INSERT INTO job(id,outbox_id,idempotency_key,status,attempt_count,created_at) SELECT $1,id,business_key,'queued',0,'1900-01-01'::timestamptz + $3 * interval '1 millisecond' FROM job_outbox WHERE id=$2")
            .bind(Uuid::new_v4()).bind(id).bind(position as i64).execute(&mut *dispatch).await.unwrap();
        sqlx::query("UPDATE job_outbox SET dispatched_at=clock_timestamp() WHERE id=$1")
            .bind(id)
            .execute(&mut *dispatch)
            .await
            .unwrap();
    }
    dispatch.commit().await.unwrap();
    let asset_id: Uuid = sqlx::query_scalar("SELECT id FROM job WHERE outbox_id=$1")
        .bind(event)
        .fetch_one(&rig.runtime_pool)
        .await
        .unwrap();
    // Exact query compiled into the deployed old binary, not a new helper.
    let legacy:Vec<Uuid>=sqlx::query_scalar("SELECT id FROM public.job WHERE status='queued' OR (status='retry_wait' AND next_attempt_at<=clock_timestamp()) OR (status='running' AND lease_expires_at<=clock_timestamp()) ORDER BY created_at,id LIMIT 64")
        .fetch_all(&rig.runtime_pool).await.unwrap();
    assert!(legacy.contains(&asset_id));
    assert!(legacy.iter().all(|id| !snapshots.contains(id)));
    assert!(jobs.runnable_ids(64).await.unwrap().contains(&asset_id));
    assert_eq!(jobs.runnable_snapshot_ids(64).await.unwrap().len(), 64);
    assert!(
        SnapshotStore::new(rig.runtime_pool.clone())
            .runnable_export_ids(64)
            .await
            .unwrap()
            .is_empty()
    );
    let c2_wrong=sqlx::query("UPDATE job SET status='snapshot_running',attempt_count=1,lease_token=gen_random_uuid(),lease_expires_at=clock_timestamp()+interval '30 seconds' WHERE id=$1").bind(asset_id).execute(&rig.admin_pool).await.unwrap_err();
    assert_eq!(support::sqlstate(&c2_wrong).as_deref(), Some("23514"));
    let id = snapshots[0];
    let lease = jobs
        .claim_snapshot(id, Duration::from_secs(30))
        .await
        .unwrap()
        .unwrap();
    let c3_wrong = sqlx::query("UPDATE job SET status='running' WHERE id=$1")
        .bind(id)
        .execute(&rig.admin_pool)
        .await
        .unwrap_err();
    assert_eq!(support::sqlstate(&c3_wrong).as_deref(), Some("23514"));
    for name in [
        "p0c2_renew_job",
        "p0c2_checkpoint_job",
        "p0c2_succeed_job",
        "p0c2_fail_job",
        "p0c2_cancel_job",
    ] {
        let sql = match name {
            "p0c2_renew_job" => "SELECT p0c2_renew_job($1,$2,30000)",
            "p0c2_checkpoint_job" => "SELECT p0c2_checkpoint_job($1,$2,'{}'::jsonb)",
            "p0c2_succeed_job" => "SELECT p0c2_succeed_job($1,$2,repeat('a',64))",
            "p0c2_fail_job" => "SELECT p0c2_fail_job($1,$2,'invalid_input',false)",
            _ => "SELECT p0c2_cancel_job($1) WHERE $2::uuid IS NOT NULL",
        };
        let accepted: bool = sqlx::query_scalar(sql)
            .bind(id)
            .bind(lease.token)
            .fetch_one(&rig.runtime_pool)
            .await
            .unwrap();
        assert!(!accepted, "{name}");
    }
    assert_eq!(
        jobs.get(id).await.unwrap().unwrap().status,
        learning_core::JobStatus::Running
    );
    assert!(
        jobs.renew(id, lease.token, Duration::from_secs(30))
            .await
            .unwrap()
    );
    assert!(
        jobs.checkpoint(id, lease.token, json!({"copy":1}))
            .await
            .unwrap()
    );
    assert!(
        jobs.fail(
            id,
            lease.token,
            learning_db::JobFailureClass::TransientStorage
        )
        .await
        .unwrap()
    );
    assert_eq!(
        jobs.get(id).await.unwrap().unwrap().status,
        learning_core::JobStatus::RetryWait
    );
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let second = jobs
        .claim_snapshot(id, Duration::from_secs(30))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second.attempt_count, 2);
    sqlx::query(
        "UPDATE job SET lease_expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(id)
    .execute(&rig.admin_pool)
    .await
    .unwrap();
    let third = jobs
        .claim_snapshot(id, Duration::from_secs(30))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(third.attempt_count, 3);
    sqlx::query(
        "UPDATE job SET lease_expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(id)
    .execute(&rig.admin_pool)
    .await
    .unwrap();
    assert!(
        jobs.claim(id, Duration::from_secs(30))
            .await
            .unwrap()
            .is_none()
    );
    let status: String = sqlx::query_scalar("SELECT status FROM job WHERE id=$1")
        .bind(id)
        .fetch_one(&rig.runtime_pool)
        .await
        .unwrap();
    assert_eq!(status, "snapshot_running");
    assert!(
        jobs.claim_snapshot(id, Duration::from_secs(30))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        jobs.get(id).await.unwrap().unwrap().status,
        learning_core::JobStatus::Failed
    );
    let cancel = snapshots[1];
    assert!(jobs.cancel(cancel).await.unwrap());
    assert_eq!(
        jobs.get(cancel).await.unwrap().unwrap().status,
        learning_core::JobStatus::Cancelled
    );
    let succeed = snapshots[2];
    let current = jobs
        .claim_snapshot(succeed, Duration::from_secs(30))
        .await
        .unwrap()
        .unwrap();
    assert!(
        jobs.succeed(succeed, current.token, &"b".repeat(64))
            .await
            .unwrap()
    );
    assert_eq!(
        jobs.get(succeed).await.unwrap().unwrap().status,
        learning_core::JobStatus::Succeeded
    );
}

mod integration {
    use super::*;
    use learning_assets::FsAssetStore;
    use std::fs;
    #[cfg(target_os = "linux")]
    use std::os::unix::fs::PermissionsExt;
    fn storage(store: SnapshotStore) -> (SnapshotStore, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("snapshot-jobs-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        #[cfg(target_os = "linux")]
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let assets = FsAssetStore::new(root.join("assets"), root.join("uploads")).unwrap();
        let stage = root.join("exports");
        fs::create_dir(&stage).unwrap();
        #[cfg(target_os = "linux")]
        fs::set_permissions(&stage, fs::Permissions::from_mode(0o700)).unwrap();
        (store.with_export_storage(stage, assets), root)
    }
    #[tokio::test]
    async fn revocation_after_plan_or_copy_prevents_result_and_delivery() {
        for revoke_before_copy in [true, false] {
            let (rig, actor, space, store, jobs, id, _) = queued().await;
            let (store, _root) = storage(store);
            let lease = jobs
                .claim_snapshot(id, Duration::from_secs(30))
                .await
                .unwrap()
                .unwrap();
            let prepared = store.prepare_export(&lease).await.unwrap();
            if revoke_before_copy {
                rig.revoke(actor, space).await;
            }
            let staged = store.stage_export(prepared).unwrap();
            if !revoke_before_copy {
                rig.revoke(actor, space).await;
            }
            assert_eq!(
                store.publish_export(staged).await.unwrap(),
                AssetProcessOutcome::Failed
            );
            assert!(store.deliver_export(actor, id).await.is_err());
            let count: i64 =
                sqlx::query_scalar("SELECT count(*) FROM snapshot_export_result WHERE job_id=$1")
                    .bind(id)
                    .fetch_one(&rig.runtime_pool)
                    .await
                    .unwrap();
            assert_eq!(count, 0);
        }
    }
    #[tokio::test]
    async fn old_token_cannot_publish_and_winning_package_survives_restart_until_revocation() {
        let (rig, actor, space, store, jobs, id, _) = queued().await;
        let (store, root) = storage(store);
        let old = jobs
            .claim_snapshot(id, Duration::from_secs(30))
            .await
            .unwrap()
            .unwrap();
        let stale = store
            .stage_export(store.prepare_export(&old).await.unwrap())
            .unwrap();
        sqlx::query("UPDATE public.job SET lease_expires_at=clock_timestamp()-interval '1 second' WHERE id=$1").bind(id).execute(&rig.admin_pool).await.unwrap();
        let current = jobs
            .claim_snapshot(id, Duration::from_secs(30))
            .await
            .unwrap()
            .unwrap();
        assert!(!jobs.succeed(id, old.token, &"a".repeat(64)).await.unwrap());
        assert_eq!(
            store.publish_export(stale).await.unwrap(),
            AssetProcessOutcome::LeaseLost
        );
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM snapshot_export_result WHERE job_id=$1")
                .bind(id)
                .fetch_one(&rig.runtime_pool)
                .await
                .unwrap();
        assert_eq!(count, 0);
        assert_eq!(
            store.process_snapshot_export(&current).await.unwrap(),
            AssetProcessOutcome::Succeeded
        );
        let restarted = SnapshotStore::new(rig.runtime_pool.clone()).with_export_storage(
            root.join("exports"),
            FsAssetStore::new(root.join("assets"), root.join("uploads")).unwrap(),
        );
        let delivery = restarted.deliver_export(actor, id).await.unwrap();
        assert!(
            delivery
                .files
                .iter()
                .any(|(name, _)| name == "manifest.json")
        );
        rig.revoke(actor, space).await;
        assert!(restarted.deliver_export(actor, id).await.is_err());
    }
    #[tokio::test]
    async fn copy_jobs_deliver_only_clipped_files_and_recheck_authorization() {
        let (rig, actor, space, store, jobs, _, mut input) = queued().await;
        let (store, _root) = storage(store);
        input.include_personal = false;
        let id = store
            .enqueue_export(actor, Uuid::new_v4(), input)
            .await
            .unwrap();
        jobs.dispatch_pending(256).await.unwrap();
        let lease = jobs
            .claim_snapshot(id, Duration::from_secs(30))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            store.process_snapshot_export(&lease).await.unwrap(),
            AssetProcessOutcome::Succeeded
        );
        let delivery = store.deliver_export(actor, id).await.unwrap();
        assert!(delivery.files.iter().all(|(name, _)| {
            [
                "reading.md",
                "reading.html",
                "manifest.json",
                "validation.json",
            ]
            .contains(&name.as_str())
        }));
        rig.revoke(actor, space).await;
        assert!(store.deliver_export(actor, id).await.is_err());
    }
    async fn wait_blocker(pool: &sqlx::PgPool, blocker: i32) {
        for _ in 0..200 {
            let found: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE $1=ANY(pg_blocking_pids(pid)))",
            )
            .bind(blocker)
            .fetch_one(pool)
            .await
            .unwrap();
            if found {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("expected a database lock waiter; race was not exercised");
    }
    #[tokio::test]
    async fn revocation_winning_grant_lock_denies_publication() {
        let (rig, actor, space, store, jobs, id, _) = queued().await;
        let (store, _root) = storage(store);
        let lease = jobs
            .claim_snapshot(id, Duration::from_secs(30))
            .await
            .unwrap()
            .unwrap();
        let staged = store
            .stage_export(store.prepare_export(&lease).await.unwrap())
            .unwrap();
        let mut revoke = rig.admin_pool.begin().await.unwrap();
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *revoke)
            .await
            .unwrap();
        sqlx::query("DELETE FROM space_grant WHERE actor_id=$1 AND space_id=$2")
            .bind(actor.actor_id)
            .bind(space)
            .execute(&mut *revoke)
            .await
            .unwrap();
        let publisher = store.clone();
        let task = tokio::spawn(async move { publisher.publish_export(staged).await });
        wait_blocker(&rig.admin_pool, pid).await;
        revoke.commit().await.unwrap();
        assert_eq!(task.await.unwrap().unwrap(), AssetProcessOutcome::Failed);
        assert!(store.deliver_export(actor, id).await.is_err());
    }
    #[tokio::test]
    async fn publication_winning_grant_lock_finishes_then_revocation_denies_delivery() {
        let (rig, actor, space, store, jobs, id, _) = queued().await;
        let (store, _root) = storage(store);
        let lease = jobs
            .claim_snapshot(id, Duration::from_secs(30))
            .await
            .unwrap()
            .unwrap();
        let staged = store
            .stage_export(store.prepare_export(&lease).await.unwrap())
            .unwrap();
        // Hold the job row so the publisher pauses only after taking grant locks.
        let mut blocker = rig.admin_pool.begin().await.unwrap();
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *blocker)
            .await
            .unwrap();
        sqlx::query("SELECT id FROM job WHERE id=$1 FOR UPDATE")
            .bind(id)
            .execute(&mut *blocker)
            .await
            .unwrap();
        let publisher = store.clone();
        let task = tokio::spawn(async move { publisher.publish_export(staged).await });
        wait_blocker(&rig.admin_pool, pid).await;
        let publisher_pid: i32 = sqlx::query_scalar(
            "SELECT pid FROM pg_stat_activity WHERE $1=ANY(pg_blocking_pids(pid)) LIMIT 1",
        )
        .bind(pid)
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap();
        let pool = rig.admin_pool.clone();
        let revoke = tokio::spawn(async move {
            sqlx::query("DELETE FROM space_grant WHERE actor_id=$1 AND space_id=$2")
                .bind(actor.actor_id)
                .bind(space)
                .execute(&pool)
                .await
                .unwrap()
        });
        wait_blocker(&rig.admin_pool, publisher_pid).await;
        blocker.commit().await.unwrap();
        assert_eq!(task.await.unwrap().unwrap(), AssetProcessOutcome::Succeeded);
        assert_eq!(revoke.await.unwrap().rows_affected(), 1);
        assert!(store.deliver_export(actor, id).await.is_err());
    }
    #[tokio::test]
    async fn concurrent_staging_and_revocation_never_produce_delivery() {
        let (rig, actor, space, store, jobs, id, _) = queued().await;
        let (store, _root) = storage(store);
        let lease = jobs
            .claim_snapshot(id, Duration::from_secs(30))
            .await
            .unwrap()
            .unwrap();
        let prepared = store.prepare_export(&lease).await.unwrap();
        let writer = store.clone();
        let (staged, ()) = tokio::join!(
            tokio::task::spawn_blocking(move || writer.stage_export(prepared)),
            rig.revoke(actor, space)
        );
        let staged = staged.unwrap().unwrap();
        assert_eq!(
            store.publish_export(staged).await.unwrap(),
            AssetProcessOutcome::Failed
        );
        assert!(store.deliver_export(actor, id).await.is_err());
    }
    #[tokio::test]
    async fn selected_v2_reading_enqueues_processes_and_delivers_safe_copy() {
        use learning_core::*;
        use learning_db::RelationStore;
        let (rig, actor, space, _, saved) = support::reading::fixture().await;
        let from = rig
            .store
            .create(actor, space, support::command("Evidence source"))
            .await
            .unwrap();
        let to = rig
            .store
            .create(actor, space, support::command("Evidence target"))
            .await
            .unwrap();
        let rel = RelationStore::new(rig.runtime_pool.clone())
            .save(
                actor,
                relations::save(space, relations::exact(&from), relations::exact(&to)),
            )
            .await
            .unwrap();
        let review = RelationStore::new(rig.runtime_pool.clone())
            .review(actor, relations::review(&rel))
            .await
            .unwrap();
        let reading = support::reading::store(&rig);
        let selected = reading
            .select_relations(
                actor,
                saved.overlay.overlay_id,
                SelectRelations {
                    request_id: Uuid::new_v4(),
                    expected_overlay_revision: saved.overlay.revision_id,
                    expected_reading_view_revision: saved.view.revision_id,
                    selections: vec![RelationSelection {
                        relation: rel.reference.clone(),
                        review: Some(review.reference.clone()),
                    }],
                    epistemic_reviews: vec![],
                    reason: "v2 copy regression".into(),
                },
            )
            .await
            .unwrap();
        let projection = reading
            .read_versioned(actor, selected.view.clone(), ReadingMode::Fused)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(projection.contract_version, 2);
        let (store, _root) = storage(SnapshotStore::new(rig.runtime_pool.clone()));
        let jobs = JobStore::new(rig.runtime_pool.clone());
        let mut req = request();
        req.reading = selected.view;
        req.include_personal = false;
        let id = store
            .enqueue_export(actor, Uuid::new_v4(), req)
            .await
            .unwrap();
        jobs.dispatch_pending(256).await.unwrap();
        let lease = jobs
            .claim_snapshot(id, Duration::from_secs(30))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            store.process_snapshot_export(&lease).await.unwrap(),
            AssetProcessOutcome::Succeeded
        );
        let delivery = store.deliver_export(actor, id).await.unwrap();
        for (name, mut file) in delivery.files {
            use std::io::Read;
            let mut body = String::new();
            file.read_to_string(&mut body).unwrap();
            assert!(!body.contains(&rel.reference.relation_id.to_string()));
            assert!(!body.contains("checked within stated conditions"));
            if name == "reading.md" {
                assert!(body.contains('K') && body.contains('M'));
            }
            if name == "manifest.json" {
                let manifest: SnapshotManifest = serde_json::from_str(&body).unwrap();
                assert!(matches!(manifest, SnapshotManifest::ReadingCopyV1(_)));
            }
        }
    }
}

// Requires a separately provisioned EMPTY database. It never resets an existing
// database and exercises the actual baseline constraints before applying 0014.
#[tokio::test]
async fn fresh_0013_database_upgrades_actual_constraints_without_changing_c2_state() {
    use learning_db::MIGRATOR;
    let admin_url = std::env::var("TEST_C3_UPGRADE_ADMIN_DATABASE_URL")
        .expect("dedicated fresh C3 upgrade admin DSN required");
    let runtime_url = std::env::var("TEST_C3_UPGRADE_DATABASE_URL")
        .expect("dedicated fresh C3 upgrade runtime DSN required");
    let admin = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&admin_url)
        .await
        .unwrap();
    let empty:bool=sqlx::query_scalar("SELECT to_regclass('public._sqlx_migrations') IS NULL AND to_regclass('public.job') IS NULL").fetch_one(&admin).await.unwrap();
    assert!(
        empty,
        "upgrade fixture must start empty; no existing schema is reset"
    );
    let baseline = sqlx::migrate::Migrator {
        migrations: std::borrow::Cow::Owned(
            MIGRATOR
                .iter()
                .filter(|m| m.version <= 13)
                .cloned()
                .collect(),
        ),
        ..sqlx::migrate::Migrator::DEFAULT
    };
    baseline.run(&admin).await.unwrap();
    let names:Vec<String>=sqlx::query_scalar("SELECT conname::text FROM pg_constraint WHERE conrelid='public.job'::regclass AND conname=ANY($1) ORDER BY conname")
        .bind(vec!["job_check","job_check1","job_check2","job_status_check"]).fetch_all(&admin).await.unwrap();
    assert_eq!(
        names,
        vec!["job_check", "job_check1", "job_check2", "job_status_check"]
    );
    let checksums: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&admin)
            .await
            .unwrap();
    assert_eq!(checksums.len(), 13);
    let runtime = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&runtime_url)
        .await
        .unwrap();
    let actor = Uuid::new_v4();
    let space = Uuid::new_v4();
    sqlx::query("INSERT INTO app_user(id) VALUES($1)")
        .bind(actor)
        .execute(&admin)
        .await
        .unwrap();
    let input = JobInput::asset_integrity(
        actor,
        space,
        learning_core::BlockRef {
            block_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
        },
    )
    .unwrap();
    let event = Uuid::new_v4();
    sqlx::query("INSERT INTO job_outbox(id,business_key,event_type,payload_version,payload,actor_id,processor_version) VALUES($1,$2,'asset_integrity_requested',1,$3,$4,1)")
        .bind(event).bind(input.business_key()).bind(input.to_value()).bind(actor).execute(&runtime).await.unwrap();
    let jobs = JobStore::new(runtime.clone());
    jobs.dispatch_pending(1).await.unwrap();
    let job: Uuid = sqlx::query_scalar("SELECT id FROM job WHERE outbox_id=$1")
        .bind(event)
        .fetch_one(&runtime)
        .await
        .unwrap();
    let lease = jobs
        .claim(job, Duration::from_secs(300))
        .await
        .unwrap()
        .unwrap();
    let snapshot = JobInput::snapshot_export(actor, space, request()).unwrap();
    let rejected=sqlx::query("INSERT INTO job_outbox(id,business_key,event_type,payload_version,payload,actor_id,processor_version) VALUES($1,$2,'snapshot_export_requested',1,$3,$4,1)")
        .bind(Uuid::new_v4()).bind(snapshot.business_key()).bind(snapshot.to_value()).bind(actor).execute(&runtime).await.unwrap_err();
    assert_eq!(support::sqlstate(&rejected).as_deref(), Some("23514"));
    MIGRATOR.run(&admin).await.unwrap();
    let after: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations WHERE version<=13 ORDER BY version",
    )
    .fetch_all(&admin)
    .await
    .unwrap();
    assert_eq!(after, checksums);
    let saved:(serde_json::Value,String,String)=sqlx::query_as("SELECT e.payload,e.business_key,j.status FROM job_outbox e JOIN job j ON j.outbox_id=e.id WHERE e.id=$1").bind(event).fetch_one(&runtime).await.unwrap();
    assert_eq!(
        saved,
        (input.to_value(), input.business_key(), "running".into())
    );
    assert!(
        jobs.renew(job, lease.token, Duration::from_secs(30))
            .await
            .unwrap()
    );
    assert!(
        jobs.succeed(job, lease.token, &"a".repeat(64))
            .await
            .unwrap()
    );
    let c3 = Uuid::new_v4();
    sqlx::query("INSERT INTO job_outbox(id,business_key,event_type,payload_version,payload,actor_id,processor_version) VALUES($1,$2,'snapshot_export_requested',1,$3,$4,1)")
        .bind(c3).bind(snapshot.business_key()).bind(snapshot.to_value()).bind(actor).execute(&runtime).await.unwrap();
    jobs.dispatch_pending(1).await.unwrap();
    let state: String = sqlx::query_scalar("SELECT status FROM job WHERE id=$1")
        .bind(c3)
        .fetch_one(&runtime)
        .await
        .unwrap();
    assert_eq!(state, "snapshot_queued");
    assert!(
        jobs.claim(c3, Duration::from_secs(30))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        jobs.claim_snapshot(c3, Duration::from_secs(30))
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn c2_and_c3_failure_cancellation_and_stale_token_semantics_match() {
    use learning_core::JobStatus;
    use learning_db::JobFailureClass;
    let rig = support::TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let jobs = JobStore::new(rig.runtime_pool.clone());
    for snapshot in [false, true] {
        for cancel_retry in [false, true] {
            let input = if snapshot {
                let mut req = request();
                req.reading.view_id = Uuid::new_v4();
                JobInput::snapshot_export(actor.actor_id, space, req).unwrap()
            } else {
                JobInput::asset_integrity(
                    actor.actor_id,
                    space,
                    learning_core::BlockRef {
                        block_id: Uuid::new_v4(),
                        revision_id: Uuid::new_v4(),
                    },
                )
                .unwrap()
            };
            let event = Uuid::new_v4();
            let kind = if snapshot {
                "snapshot_export_requested"
            } else {
                "asset_integrity_requested"
            };
            let mut tx = rig.runtime_pool.begin().await.unwrap();
            sqlx::query("INSERT INTO job_outbox(id,business_key,event_type,payload_version,payload,actor_id,processor_version) VALUES($1,$2,$3,1,$4,$5,1)")
                .bind(event).bind(input.business_key()).bind(kind).bind(input.to_value()).bind(actor.actor_id).execute(&mut *tx).await.unwrap();
            sqlx::query("INSERT INTO job(id,outbox_id,idempotency_key,status,attempt_count) VALUES($1,$1,$2,'queued',0)")
                .bind(event).bind(input.business_key()).execute(&mut *tx).await.unwrap();
            sqlx::query("UPDATE job_outbox SET dispatched_at=clock_timestamp() WHERE id=$1")
                .bind(event)
                .execute(&mut *tx)
                .await
                .unwrap();
            tx.commit().await.unwrap();
            let lease = if snapshot {
                jobs.claim_snapshot(event, Duration::from_secs(30)).await
            } else {
                jobs.claim(event, Duration::from_secs(30)).await
            }
            .unwrap()
            .unwrap();
            let stale = Uuid::new_v4();
            assert!(
                !jobs
                    .renew(event, stale, Duration::from_secs(30))
                    .await
                    .unwrap()
            );
            assert!(!jobs.checkpoint(event, stale, json!({})).await.unwrap());
            assert!(!jobs.succeed(event, stale, &"a".repeat(64)).await.unwrap());
            assert!(
                !jobs
                    .fail(event, stale, JobFailureClass::InvalidInput)
                    .await
                    .unwrap()
            );
            let class = if cancel_retry {
                JobFailureClass::TransientStorage
            } else {
                JobFailureClass::InvalidInput
            };
            assert!(jobs.fail(event, lease.token, class).await.unwrap());
            assert_eq!(
                jobs.get(event).await.unwrap().unwrap().status,
                if cancel_retry {
                    JobStatus::RetryWait
                } else {
                    JobStatus::Failed
                }
            );
            let row:(Option<Uuid>,Option<String>,Option<String>,bool)=sqlx::query_as("SELECT lease_token,last_error_class,output_digest,next_attempt_at IS NOT NULL FROM job WHERE id=$1")
                .bind(event).fetch_one(&rig.runtime_pool).await.unwrap();
            assert_eq!(
                row,
                (
                    None,
                    Some(
                        if cancel_retry {
                            "transient_storage"
                        } else {
                            "invalid_input"
                        }
                        .into()
                    ),
                    None,
                    cancel_retry
                )
            );
            assert_eq!(jobs.cancel(event).await.unwrap(), cancel_retry);
            assert_eq!(
                jobs.get(event).await.unwrap().unwrap().status,
                if cancel_retry {
                    JobStatus::Cancelled
                } else {
                    JobStatus::Failed
                }
            );
            assert!(!jobs.cancel(event).await.unwrap());
            assert!(
                !jobs
                    .fail(event, lease.token, JobFailureClass::InvalidInput)
                    .await
                    .unwrap()
            );
            assert!(
                !jobs
                    .succeed(event, lease.token, &"a".repeat(64))
                    .await
                    .unwrap()
            );
            let pending:bool=sqlx::query_scalar("SELECT next_attempt_at IS NOT NULL OR lease_token IS NOT NULL OR lease_expires_at IS NOT NULL FROM job WHERE id=$1")
                .bind(event).fetch_one(&rig.runtime_pool).await.unwrap();
            assert!(!pending);
        }
    }
}

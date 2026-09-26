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
    assert!(jobs.runnable_ids(256).await.unwrap().contains(&id));
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
    for v in malformed {
        assert!(JobInput::from_value(v.clone()).is_err());
        let valid: bool = sqlx::query_scalar(
            "SELECT p0c3_valid_job_payload('snapshot_export_requested',1,$1,$2,$3)",
        )
        .bind(v)
        .bind(actor.actor_id)
        .bind(input.business_key())
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
}

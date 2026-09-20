mod support;
use learning_core::{ContentError, CreateCommand};
use learning_db::ContentStore;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{sync::Arc, time::Duration};
use support::*;
use tokio::sync::Barrier;

async fn connection() -> (PgPool, ContentStore, i32) {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let pid = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&pool)
        .await
        .unwrap();
    (pool.clone(), ContentStore::new(pool), pid)
}
#[tokio::test]
async fn competing_revisions_have_exactly_one_commit_and_one_current_head_conflict() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let first = rig
        .store
        .create(actor, space, command("base"))
        .await
        .unwrap();
    let (pool_a, a, pid_a) = connection().await;
    let (pool_b, b, pid_b) = connection().await;
    assert_ne!(pid_a, pid_b);
    let barrier = Arc::new(Barrier::new(2));
    let left = change(&first, "left");
    let right = change(&first, "right");
    let block = first.block_id;
    let gate = barrier.clone();
    let work_a = async move {
        gate.wait().await;
        a.revise(actor, block, left).await
    };
    let work_b = async move {
        barrier.wait().await;
        b.revise(actor, block, right).await
    };
    let (left, right) = tokio::time::timeout(Duration::from_secs(15), async {
        tokio::join!(work_a, work_b)
    })
    .await
    .unwrap();
    let results = [left, right];
    let success = results
        .iter()
        .find_map(|r| r.as_ref().ok())
        .expect("one success");
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|r|matches!(r,Err(ContentError::Conflict {current_revision_id}) if *current_revision_id==success.revision_id)).count(),1);
    assert_eq!(rig.head(block).await, success.revision_id);
    assert_eq!(rig.counts(actor, space).await, (1, 2, 2));
    pool_a.close().await;
    pool_b.close().await;
}
#[tokio::test]
async fn simultaneous_duplicate_create_requests_share_one_receipt() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let (pa, a, pid_a) = connection().await;
    let (pb, b, pid_b) = connection().await;
    assert_ne!(pid_a, pid_b);
    let cmd = command("once");
    let second = cmd.clone();
    let barrier = Barrier::new(2);
    let (left, right) = tokio::join!(
        async {
            barrier.wait().await;
            a.create(actor, space, cmd).await
        },
        async {
            barrier.wait().await;
            b.create(actor, space, second).await
        }
    );
    assert_eq!(left.unwrap(), right.unwrap());
    assert_eq!(rig.counts(actor, space).await, (1, 1, 1));
    pa.close().await;
    pb.close().await;
}
#[tokio::test]
async fn simultaneous_different_payloads_with_the_same_key_cannot_both_commit() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let (pa, a, _) = connection().await;
    let (pb, b, _) = connection().await;
    let cmd = command("one");
    let other = CreateCommand {
        draft: command("two").draft,
        ..cmd.clone()
    };
    let barrier = Barrier::new(2);
    let (left, right) = tokio::join!(
        async {
            barrier.wait().await;
            a.create(actor, space, cmd).await
        },
        async {
            barrier.wait().await;
            b.create(actor, space, other).await
        }
    );
    let results = [left, right];
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Err(ContentError::IdempotencyConflict)))
            .count(),
        1
    );
    assert_eq!(rig.counts(actor, space).await, (1, 1, 1));
    pa.close().await;
    pb.close().await;
}
#[tokio::test]
async fn independent_blocks_can_both_advance() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let first = rig
        .store
        .create(actor, space, command("one"))
        .await
        .unwrap();
    let second = rig
        .store
        .create(actor, space, command("two"))
        .await
        .unwrap();
    let (pa, a, _) = connection().await;
    let (pb, b, _) = connection().await;
    let (left, right) = tokio::join!(
        a.revise(actor, first.block_id, change(&first, "a")),
        b.revise(actor, second.block_id, change(&second, "b"))
    );
    assert_eq!(rig.head(first.block_id).await, left.unwrap().revision_id);
    assert_eq!(rig.head(second.block_id).await, right.unwrap().revision_id);
    assert_eq!(rig.counts(actor, space).await, (2, 4, 4));
    pa.close().await;
    pb.close().await;
}
async fn wait_blocked(admin: &PgPool, waiter: i32, blocker: i32) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT $1 = ANY(pg_blocking_pids($2))")
                .bind(blocker)
                .bind(waiter)
                .fetch_one(admin)
                .await
                .unwrap();
            if waiting {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("expected database lock wait was not observed");
}
#[tokio::test]
async fn revocation_and_downgrade_wait_for_an_authorized_save_to_finish() {
    let rig = TestRig::from_env().await;
    for revoke in [true, false] {
        let (actor, space) = rig.seed_actor_space(true).await;
        let first = rig
            .store
            .create(actor, space, command("base"))
            .await
            .unwrap();
        let (pool, writer, writer_pid) = connection().await;
        let mut blocker = rig.admin_pool.begin().await.unwrap();
        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *blocker)
            .await
            .unwrap();
        sqlx::query("SELECT id FROM public.block WHERE id=$1 FOR UPDATE")
            .bind(first.block_id)
            .execute(&mut *blocker)
            .await
            .unwrap();
        let cmd = change(&first, "authorized before revoke");
        let retry = cmd.clone();
        let block = first.block_id;
        let save = tokio::spawn(async move { writer.revise(actor, block, cmd).await });
        wait_blocked(&rig.admin_pool, writer_pid, blocker_pid).await;
        let mut revoker = rig.admin_pool.begin().await.unwrap();
        let revoker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *revoker)
            .await
            .unwrap();
        let revoke_task = tokio::spawn(async move {
            let sql = if revoke {
                "DELETE FROM public.space_grant WHERE actor_id=$1 AND space_id=$2"
            } else {
                "UPDATE public.space_grant SET can_write=false WHERE actor_id=$1 AND space_id=$2"
            };
            sqlx::query(sql)
                .bind(actor.actor_id)
                .bind(space)
                .execute(&mut *revoker)
                .await
                .unwrap();
            revoker.commit().await.unwrap();
        });
        wait_blocked(&rig.admin_pool, revoker_pid, writer_pid).await;
        blocker.commit().await.unwrap();
        let saved = save.await.unwrap().unwrap();
        revoke_task.await.unwrap();
        assert_eq!(rig.head(block).await, saved.revision_id);
        assert!(matches!(
            rig.store.revise(actor, block, retry).await,
            Err(ContentError::NotFound)
        ));
        pool.close().await;
    }
}

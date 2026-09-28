mod support;
use learning_core::*;
use learning_db::{CompositionStore, ContentStore, ReleaseStore};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::time::Duration;
use support::{assembly::*, *};
use tokio::sync::Barrier;

#[tokio::test]
async fn cross_space_revocation_commits_before_save_and_recheck_refuses_write() {
    let rig = TestRig::from_env().await;
    let (a, s) = rig.seed_actor_space(true).await;
    let (o, d) = rig.seed_actor_space(true).await;
    rig.grant(a, d, false).await;
    let b = rig.store.create(o, d, command("dependency")).await.unwrap();
    let before = rig.assembly_counts(a).await;
    let mut revoke = rig.admin_pool.begin().await.unwrap();
    let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *revoke)
        .await
        .unwrap();
    sqlx::query("DELETE FROM space_grant WHERE actor_id=$1 AND space_id=$2")
        .bind(a.actor_id)
        .bind(d)
        .execute(&mut *revoke)
        .await
        .unwrap();
    let (pool, pid) = connection().await;
    let cs = CompositionStore::new(pool);
    let cmd = doc(vec![block(&b)]);
    let save = tokio::spawn(async move { cs.save(a, s, cmd).await });
    wait_blocked(&rig.admin_pool, pid, blocker).await;
    revoke.commit().await.unwrap();
    assert!(matches!(save.await.unwrap(), Err(ContentError::NotFound)));
    assert_eq!(rig.assembly_counts(a).await, before);
    assert_eq!(rig.structure_counts(s).await, (0, 0, 0));
}
#[tokio::test]
async fn unordered_dependencies_lock_grants_in_uuid_order() {
    let rig = TestRig::from_env().await;
    let (a, s) = rig.seed_actor_space(true).await;
    let (_, d1) = rig.seed_actor_space(true).await;
    let (_, d2) = rig.seed_actor_space(true).await;
    let mut spaces = [s, d1, d2];
    spaces.sort();
    for space in spaces {
        rig.grant(a, space, true).await;
    }
    let b1 = rig
        .store
        .create(a, spaces[1], command("mid"))
        .await
        .unwrap();
    let b2 = rig
        .store
        .create(a, spaces[2], command("high"))
        .await
        .unwrap();
    rig.grant(a, spaces[1], false).await;
    rig.grant(a, spaces[2], false).await;
    let mut high = rig.admin_pool.begin().await.unwrap();
    let high_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *high)
        .await
        .unwrap();
    sqlx::query("SELECT actor_id FROM space_grant WHERE actor_id=$1 AND space_id=$2 FOR UPDATE")
        .bind(a.actor_id)
        .bind(spaces[2])
        .execute(&mut *high)
        .await
        .unwrap();
    let (pool, pid) = connection().await;
    let cs = CompositionStore::new(pool);
    let cmd = doc(vec![block(&b2), block(&b1)]);
    let save = tokio::spawn(async move { cs.save(a, spaces[0], cmd).await });
    wait_blocked(&rig.admin_pool, pid, high_pid).await;
    let mut low = rig.admin_pool.begin().await.unwrap();
    let low_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *low)
        .await
        .unwrap();
    let low_lock = tokio::spawn(async move {
        sqlx::query(
            "SELECT actor_id FROM space_grant WHERE actor_id=$1 AND space_id=$2 FOR UPDATE",
        )
        .bind(a.actor_id)
        .bind(spaces[0])
        .execute(&mut *low)
        .await
        .unwrap();
        low.rollback().await.unwrap();
    });
    wait_blocked(&rig.admin_pool, low_pid, pid).await;
    high.rollback().await.unwrap();
    assert!(save.await.unwrap().is_ok());
    low_lock.await.unwrap();
}

async fn connection() -> (PgPool, i32) {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let pid = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&pool)
        .await
        .unwrap();
    (pool, pid)
}
async fn wait_blocked(admin: &PgPool, waiter: i32, blocker: i32) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let b: bool = sqlx::query_scalar("SELECT $1=ANY(pg_blocking_pids($2))")
                .bind(blocker)
                .bind(waiter)
                .fetch_one(admin)
                .await
                .unwrap();
            if b {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("expected database blocker");
}

#[tokio::test]
async fn competing_composition_edits_conflict_without_losing_a_revision() {
    let rig = TestRig::from_env().await;
    let (a, s) = rig.seed_actor_space(true).await;
    let first = rig.compositions().save(a, s, doc(vec![])).await.unwrap();
    let (pa, ia) = connection().await;
    let (pb, ib) = connection().await;
    assert_ne!(ia, ib);
    let ca = CompositionStore::new(pa.clone());
    let cb = CompositionStore::new(pb.clone());
    let gate = Barrier::new(2);
    let x = edit(&first);
    let y = edit(&first);
    let (left, right) = tokio::join!(
        async {
            gate.wait().await;
            ca.save(a, s, x).await
        },
        async {
            gate.wait().await;
            cb.save(a, s, y).await
        }
    );
    let result = [left, right];
    assert_eq!(result.iter().filter(|r| r.is_ok()).count(), 1);
    let saved = result.iter().find_map(|r| r.as_ref().ok()).unwrap();
    assert_eq!(result.iter().filter(|r|matches!(r,Err(ContentError::Conflict{current_revision_id}) if *current_revision_id==saved.reference.revision_id)).count(),1);
    assert_eq!(rig.assembly_counts(a).await, (2, 0, 0, 2, 2));
}
#[tokio::test]
async fn duplicate_save_and_publish_requests_have_one_durable_result() {
    let rig = TestRig::from_env().await;
    let (a, s) = rig.seed_actor_space(true).await;
    let (pa, ia) = connection().await;
    let (pb, ib) = connection().await;
    assert_ne!(ia, ib);
    let ca = CompositionStore::new(pa.clone());
    let cb = CompositionStore::new(pb.clone());
    let gate = Barrier::new(2);
    let cmd = doc(vec![]);
    let (x, y) = tokio::join!(
        async {
            gate.wait().await;
            ca.save(a, s, cmd.clone()).await
        },
        async {
            gate.wait().await;
            cb.save(a, s, cmd.clone()).await
        }
    );
    let c = x.unwrap();
    assert_eq!(c, y.unwrap());
    let ra = ReleaseStore::new(pa);
    let rb = ReleaseStore::new(pb);
    let p = publish(vec![root(&c, None)]);
    let gate = Barrier::new(2);
    let (x, y) = tokio::join!(
        async {
            gate.wait().await;
            ra.publish(a, s, p.clone()).await
        },
        async {
            gate.wait().await;
            rb.publish(a, s, p.clone()).await
        }
    );
    assert_eq!(x.unwrap(), y.unwrap());
    assert_eq!(rig.assembly_counts(a).await, (1, 1, 1, 2, 2));
}
#[tokio::test]
async fn overlapping_releases_serialize_and_disjoint_releases_both_commit() {
    let rig = TestRig::from_env().await;
    let (a, s) = rig.seed_actor_space(true).await;
    let cs = rig.compositions();
    let x = cs.save(a, s, doc(vec![])).await.unwrap();
    let y = cs.save(a, s, doc(vec![])).await.unwrap();
    let z = cs.save(a, s, doc(vec![])).await.unwrap();
    let (pa, ia) = connection().await;
    let (pb, ib) = connection().await;
    assert_ne!(ia, ib);
    let ra = ReleaseStore::new(pa);
    let rb = ReleaseStore::new(pb);
    let gate = Barrier::new(2);
    let (left, right) = tokio::join!(
        async {
            gate.wait().await;
            ra.publish(a, s, publish(vec![root(&y, None), root(&x, None)]))
                .await
        },
        async {
            gate.wait().await;
            rb.publish(a, s, publish(vec![root(&z, None), root(&y, None)]))
                .await
        }
    );
    let results = [left, right];
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Err(ContentError::PublicationConflict { .. })))
            .count(),
        1
    );
    assert_eq!(rig.assembly_counts(a).await.1, 1);
    let u = cs.save(a, s, doc(vec![])).await.unwrap();
    let v = cs.save(a, s, doc(vec![])).await.unwrap();
    let (x, y) = tokio::join!(
        ra.publish(a, s, publish(vec![root(&u, None)])),
        rb.publish(a, s, publish(vec![root(&v, None)]))
    );
    assert!(x.is_ok() && y.is_ok());
    assert_eq!(rig.assembly_counts(a).await.1, 3);
}
#[tokio::test]
async fn content_and_composition_cannot_claim_the_same_request_key() {
    let rig = TestRig::from_env().await;
    let (a, s) = rig.seed_actor_space(true).await;
    let (pa, ia) = connection().await;
    let (pb, ib) = connection().await;
    assert_ne!(ia, ib);
    let cs = CompositionStore::new(pa);
    let bs = ContentStore::new(pb);
    let c = doc(vec![]);
    let mut b = command("same request");
    b.request_id = c.request_id;
    let gate = Barrier::new(2);
    let (x, y) = tokio::join!(
        async {
            gate.wait().await;
            cs.save(a, s, c).await.map(|_| ())
        },
        async {
            gate.wait().await;
            bs.create(a, s, b).await.map(|_| ())
        }
    );
    let results = [x, y];
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Err(ContentError::IdempotencyConflict)))
            .count(),
        1
    );
    assert_eq!(rig.assembly_counts(a).await.3, 1);
}
#[tokio::test]
async fn opposite_pinned_old_references_are_valid_but_followup_identity_cycle_is_not() {
    let rig = TestRig::from_env().await;
    let (a, s) = rig.seed_actor_space(true).await;
    let cs = rig.compositions();
    let x = cs.save(a, s, doc(vec![])).await.unwrap();
    let y = cs.save(a, s, doc(vec![])).await.unwrap();
    let mut cx = edit(&x);
    cx.nodes = vec![NodeDraft {
        occurrence_id: None,
        target: child(&y),
    }];
    let mut cy = edit(&y);
    cy.nodes = vec![NodeDraft {
        occurrence_id: None,
        target: child(&x),
    }];
    let (pa, ia) = connection().await;
    let (pb, ib) = connection().await;
    assert_ne!(ia, ib);
    let ca = CompositionStore::new(pa);
    let cb = CompositionStore::new(pb);
    let gate = Barrier::new(2);
    let (x2, y2) = tokio::join!(
        async {
            gate.wait().await;
            ca.save(a, s, cx).await
        },
        async {
            gate.wait().await;
            cb.save(a, s, cy).await
        }
    );
    let x2 = x2.unwrap();
    let y2 = y2.unwrap();
    let mut c = edit(&x2);
    c.nodes[0].target = child(&y2);
    assert!(
        matches!(cs.save(a,s,c).await,Err(ContentError::Invalid(code)) if code=="composition_cycle")
    );
}
#[tokio::test]
async fn grant_revocation_waits_for_locked_composition_save_then_blocks_replay() {
    let rig = TestRig::from_env().await;
    let (a, s) = rig.seed_actor_space(true).await;
    let first = rig.compositions().save(a, s, doc(vec![])).await.unwrap();
    let (pool, pid) = connection().await;
    let store = CompositionStore::new(pool);
    let mut blocker = rig.admin_pool.begin().await.unwrap();
    let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    sqlx::query("SELECT id FROM composition WHERE id=$1 FOR UPDATE")
        .bind(first.reference.composition_id)
        .execute(&mut *blocker)
        .await
        .unwrap();
    let cmd = edit(&first);
    let retry = cmd.clone();
    let save = tokio::spawn(async move { store.save(a, s, cmd).await });
    wait_blocked(&rig.admin_pool, pid, blocker_pid).await;
    let mut revoker = rig.admin_pool.begin().await.unwrap();
    let revoke_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *revoker)
        .await
        .unwrap();
    let revoke = tokio::spawn(async move {
        sqlx::query("DELETE FROM space_grant WHERE actor_id=$1 AND space_id=$2")
            .bind(a.actor_id)
            .bind(s)
            .execute(&mut *revoker)
            .await
            .unwrap();
        revoker.commit().await.unwrap();
    });
    wait_blocked(&rig.admin_pool, revoke_pid, pid).await;
    blocker.commit().await.unwrap();
    assert!(save.await.unwrap().is_ok());
    revoke.await.unwrap();
    assert!(matches!(
        rig.compositions().save(a, s, retry).await,
        Err(ContentError::NotFound)
    ));
}

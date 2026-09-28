mod support;
use learning_core::*;
use learning_db::{ContentStore, MigrationStore, ReadingStore};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::time::Duration;
use support::{assembly as a, reading as h, *};
use tokio::sync::Barrier;
use uuid::Uuid;

#[tokio::test]
async fn p0a_and_personal_revision_compete_on_same_body_head() {
    let (r, actor, _space, doc, zero) = h::fixture().await;
    let store = h::store(&r);
    let one = store
        .edit(
            actor,
            zero.overlay.overlay_id,
            h::edit(&zero, h::add(h::gap(&doc, 1), "N")),
        )
        .await
        .unwrap();
    let placement = store
        .state(actor, one.overlay.overlay_id)
        .await
        .unwrap()
        .unwrap()
        .editable
        .unwrap()
        .groups[0]
        .placements[0]
        .clone();
    let (pool1, pid1) = connection().await;
    let (pool2, pid2) = connection().await;
    assert_ne!(pid1, pid2);
    let rs = ReadingStore::new(pool1);
    let cs = ContentStore::new(pool2);
    let c = h::edit(
        &one,
        ReadingEdit::ReviseSelected {
            changes: vec![TextChange {
                block_id: placement.block.block_id,
                base_revision_id: placement.block.revision_id,
                draft: command("B2").draft,
                selected_placements: vec![placement.placement_id],
            }],
        },
    );
    let cc = ReviseCommand {
        request_id: Uuid::new_v4(),
        base_revision_id: placement.block.revision_id,
        draft: command("P0A").draft,
        reason: "compete".into(),
    };
    let b = Barrier::new(2);
    let (x, y) = tokio::join!(
        async {
            b.wait().await;
            rs.edit(actor, one.overlay.overlay_id, c).await
        },
        async {
            b.wait().await;
            cs.revise(actor, placement.block.block_id, cc).await
        }
    );
    assert_ne!(x.is_ok(), y.is_ok());
    assert!(
        matches!(x, Err(ContentError::Conflict { .. }))
            || matches!(y, Err(ContentError::Conflict { .. }))
    );
    let revisions: i64 =
        sqlx::query_scalar("SELECT count(*) FROM block_revision WHERE block_id=$1")
            .bind(placement.block.block_id)
            .fetch_one(&r.admin_pool)
            .await
            .unwrap();
    assert_eq!(revisions, 2);
}

#[tokio::test]
async fn reverse_input_still_locks_body_ids_in_sorted_order() {
    let (r, actor, _space, doc, zero) = h::fixture().await;
    let store = h::store(&r);
    let one = store
        .edit(
            actor,
            zero.overlay.overlay_id,
            h::edit(
                &zero,
                ReadingEdit::InsertNew {
                    drafts: vec![command("N").draft, command("I").draft],
                    target: InsertTarget::NewGroup {
                        anchor: h::gap(&doc, 1),
                    },
                },
            ),
        )
        .await
        .unwrap();
    let mut placements = store
        .state(actor, one.overlay.overlay_id)
        .await
        .unwrap()
        .unwrap()
        .editable
        .unwrap()
        .groups
        .remove(0)
        .placements;
    placements.sort_by_key(|p| p.block.block_id);
    let low = placements[0].block.block_id;
    let high = placements[1].block.block_id;
    let c = h::edit(
        &one,
        ReadingEdit::ReviseSelected {
            changes: placements
                .iter()
                .rev()
                .map(|p| TextChange {
                    block_id: p.block.block_id,
                    base_revision_id: p.block.revision_id,
                    draft: command("changed").draft,
                    selected_placements: vec![p.placement_id],
                })
                .collect(),
        },
    );
    let mut gate = r.admin_pool.begin().await.unwrap();
    let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .unwrap();
    sqlx::query("SELECT id FROM block WHERE id=$1 FOR UPDATE")
        .bind(high)
        .execute(&mut *gate)
        .await
        .unwrap();
    let (pool, pid) = connection().await;
    let store = ReadingStore::new(pool);
    let task = tokio::spawn(async move { store.edit(actor, one.overlay.overlay_id, c).await });
    blocked(&r.admin_pool, pid, blocker).await;
    let mut probe = r.admin_pool.begin().await.unwrap();
    let probe_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *probe)
        .await
        .unwrap();
    let probe_task = tokio::spawn(async move {
        sqlx::query("SELECT id FROM block WHERE id=$1 FOR UPDATE")
            .bind(low)
            .execute(&mut *probe)
            .await
            .unwrap();
        probe.rollback().await.unwrap();
    });
    blocked(&r.admin_pool, probe_pid, pid).await;
    gate.rollback().await.unwrap();
    assert!(task.await.unwrap().is_ok());
    probe_task.await.unwrap();
}
async fn connection() -> (PgPool, i32) {
    let p = PgPoolOptions::new()
        .max_connections(1)
        .connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let pid = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&p)
        .await
        .unwrap();
    (p, pid)
}
async fn blocked(admin: &PgPool, waiter: i32, blocker: i32) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let yes: bool = sqlx::query_scalar("SELECT $1=ANY(pg_blocking_pids($2))")
                .bind(blocker)
                .bind(waiter)
                .fetch_one(admin)
                .await
                .unwrap();
            if yes {
                break;
            }
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    })
    .await
    .expect("expected actual PostgreSQL blocking edge");
}

#[tokio::test]
async fn same_and_different_gaps_conflict_but_identical_request_replays() {
    for case in 0..3 {
        let (r, actor, _space, doc, zero) = h::fixture().await;
        let (pool1, pid1) = connection().await;
        let (pool2, pid2) = connection().await;
        assert_ne!(pid1, pid2);
        let s1 = ReadingStore::new(pool1);
        let s2 = ReadingStore::new(pool2);
        let c1 = h::edit(&zero, h::add(h::gap(&doc, 0), "N"));
        let c2 = if case == 2 {
            c1.clone()
        } else {
            h::edit(
                &zero,
                h::add(h::gap(&doc, if case == 0 { 0 } else { 1 }), "I"),
            )
        };
        let id = zero.overlay.overlay_id;
        let barrier = Barrier::new(2);
        let (x, y) = tokio::join!(
            async {
                barrier.wait().await;
                s1.edit(actor, id, c1).await
            },
            async {
                barrier.wait().await;
                s2.edit(actor, id, c2).await
            }
        );
        if case == 2 {
            assert_eq!(x.unwrap(), y.unwrap());
        } else {
            assert_ne!(x.is_ok(), y.is_ok());
            let error = if let Err(e) = x { e } else { y.unwrap_err() };
            assert!(matches!(error, ContentError::ReadingConflict { .. }));
        }
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM block_revision WHERE author_id=$1")
                .bind(actor.actor_id)
                .fetch_one(&r.admin_pool)
                .await
                .unwrap();
        assert_eq!(count, 3);
        let st = h::store(&r).state(actor, id).await.unwrap().unwrap();
        assert_eq!(st.editable.unwrap().groups.len(), 1);
    }
}

#[tokio::test]
async fn content_and_reading_share_global_request_namespace() {
    let (r, actor, space, doc, zero) = h::fixture().await;
    let (pool1, pid1) = connection().await;
    let (pool2, pid2) = connection().await;
    assert_ne!(pid1, pid2);
    let store = ReadingStore::new(pool1);
    let content = ContentStore::new(pool2);
    let c = h::edit(&zero, h::add(h::gap(&doc, 0), "N"));
    let mut cc = command("standalone");
    cc.request_id = c.request_id;
    let b = Barrier::new(2);
    let (x, y) = tokio::join!(
        async {
            b.wait().await;
            store.edit(actor, zero.overlay.overlay_id, c).await
        },
        async {
            b.wait().await;
            content.create(actor, space, cc).await
        }
    );
    assert_ne!(x.is_ok(), y.is_ok());
    assert!(
        matches!(x, Err(ContentError::IdempotencyConflict))
            || matches!(y, Err(ContentError::IdempotencyConflict))
    );
    let keys: i64 = sqlx::query_scalar("SELECT count(*) FROM block_revision WHERE author_id=$1")
        .bind(actor.actor_id)
        .fetch_one(&r.admin_pool)
        .await
        .unwrap();
    assert_eq!(keys, 3);
}

#[tokio::test]
async fn revocation_before_edit_refuses_and_edit_before_revocation_holds_grant() {
    for revoke_first in [true, false] {
        let (r, actor, space, doc, zero) = h::fixture().await;
        let (pool, pid) = connection().await;
        let store = ReadingStore::new(pool);
        let c = h::edit(&zero, h::add(h::gap(&doc, 1), "N"));
        let before = h::counts(&r, actor).await;
        let mut gate = r.admin_pool.begin().await.unwrap();
        let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *gate)
            .await
            .unwrap();
        if revoke_first {
            sqlx::query("DELETE FROM space_grant WHERE actor_id=$1 AND space_id=$2")
                .bind(actor.actor_id)
                .bind(space)
                .execute(&mut *gate)
                .await
                .unwrap();
            let op =
                tokio::spawn(async move { store.edit(actor, zero.overlay.overlay_id, c).await });
            blocked(&r.admin_pool, pid, blocker).await;
            gate.commit().await.unwrap();
            assert!(matches!(op.await.unwrap(), Err(ContentError::NotFound)));
            assert_eq!(h::counts(&r, actor).await, before);
        } else {
            sqlx::query("SELECT id FROM overlay WHERE id=$1 FOR UPDATE")
                .bind(zero.overlay.overlay_id)
                .execute(&mut *gate)
                .await
                .unwrap();
            let op =
                tokio::spawn(async move { store.edit(actor, zero.overlay.overlay_id, c).await });
            blocked(&r.admin_pool, pid, blocker).await;
            let mut revoke = r.admin_pool.begin().await.unwrap();
            let revoker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                .fetch_one(&mut *revoke)
                .await
                .unwrap();
            let revocation = tokio::spawn(async move {
                sqlx::query("DELETE FROM space_grant WHERE actor_id=$1 AND space_id=$2")
                    .bind(actor.actor_id)
                    .bind(space)
                    .execute(&mut *revoke)
                    .await
                    .unwrap();
                revoke.commit().await.unwrap();
            });
            blocked(&r.admin_pool, revoker, pid).await;
            gate.rollback().await.unwrap();
            assert!(op.await.unwrap().is_ok());
            revocation.await.unwrap();
        }
    }
}

#[tokio::test]
async fn migration_and_edit_or_other_decision_serialize_on_layer() {
    for competing_decision in [false, true] {
        let (r, actor, space, doc, zero) = h::fixture().await;
        let mut dc = a::edit(&doc);
        dc.title = "v2".into();
        let next = r.compositions().save(actor, space, dc).await.unwrap();
        let m = MigrationStore::new(r.runtime_pool.clone());
        let p = m
            .propose(
                actor,
                zero.overlay.overlay_id,
                ProposeMigration {
                    request_id: Uuid::new_v4(),
                    expected_overlay_revision: zero.overlay.revision_id,
                    expected_reading_view_revision: zero.view.revision_id,
                    target: next.reference,
                    reason: "upgrade".into(),
                },
            )
            .await
            .unwrap();
        let (pool1, pid1) = connection().await;
        let (pool2, pid2) = connection().await;
        assert_ne!(pid1, pid2);
        let m1 = MigrationStore::new(pool1);
        let m2 = MigrationStore::new(pool2.clone());
        let s2 = ReadingStore::new(pool2);
        let d = DecideMigration {
            request_id: Uuid::new_v4(),
            proposal_id: p.proposal_id,
            expected_overlay_revision: zero.overlay.revision_id,
            expected_reading_view_revision: zero.view.revision_id,
            action: MigrationAction::Adopt {
                groups: vec![],
                merges: vec![],
            },
            reason: "review".into(),
        };
        let mut d2 = d.clone();
        d2.request_id = Uuid::new_v4();
        let b = Barrier::new(2);
        let id = zero.overlay.overlay_id;
        let (x, y) = tokio::join!(
            async {
                b.wait().await;
                m1.decide(actor, id, d).await.map(|_| ())
            },
            async {
                b.wait().await;
                if competing_decision {
                    m2.decide(actor, id, d2).await.map(|_| ())
                } else {
                    s2.edit(actor, id, h::edit(&zero, h::add(h::gap(&doc, 0), "N")))
                        .await
                        .map(|_| ())
                }
            }
        );
        assert_ne!(x.is_ok(), y.is_ok());
        let e = if let Err(e) = x { e } else { y.unwrap_err() };
        assert!(matches!(e, ContentError::ReadingConflict { .. }));
    }
}

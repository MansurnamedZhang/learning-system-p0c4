#[path = "support/relation_store.rs"]
mod h;
mod support;
use h::*;
use learning_core::*;
use learning_db::RelationStore;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::time::Duration;
use tokio::sync::Barrier;
use uuid::Uuid;

async fn connection() -> (PgPool, RelationStore, i32) {
    let p = PgPoolOptions::new()
        .max_connections(1)
        .connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let pid = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&p)
        .await
        .unwrap();
    (p.clone(), RelationStore::new(p), pid)
}
async fn blocked(admin: &PgPool, waiter: i32, blocker: i32) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT $1=ANY(pg_blocking_pids($2))")
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
    .expect("database lock wait not observed");
}
#[tokio::test]
async fn reversed_related_to_race_commits_one_identity_revision_and_receipt() {
    let (rig, _, actor, space, e, h) = fixture().await;
    let (pa, a, pida) = connection().await;
    let (pb, b, pidb) = connection().await;
    assert_ne!(pida, pidb);
    let mut forward = save(space, exact(&e), exact(&h));
    forward.relation_type = RelationType::RelatedTo;
    let mut backward = forward.clone();
    backward.request_id = Uuid::new_v4();
    std::mem::swap(&mut backward.from, &mut backward.to);
    let gate = Barrier::new(2);
    let (left, right) = tokio::join!(
        async {
            gate.wait().await;
            a.save(actor, forward).await
        },
        async {
            gate.wait().await;
            b.save(actor, backward).await
        }
    );
    let results = [left, right];
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r,Err(ContentError::Invalid(s)) if s=="relation_exists"))
            .count(),
        1
    );
    assert_eq!(counts(&rig, space).await, (1, 1, 0, 1, 0));
    pa.close().await;
    pb.close().await;
}
#[tokio::test]
async fn competing_relation_revisions_have_one_cas_winner() {
    let (rig, store, actor, space, e, h) = fixture().await;
    let first = store
        .save(actor, save(space, exact(&e), exact(&h)))
        .await
        .unwrap();
    let (pa, a, _) = connection().await;
    let (pb, b, _) = connection().await;
    let mut left = edit(&first);
    left.rationale = "left".into();
    let mut right = edit(&first);
    right.rationale = "right".into();
    let gate = Barrier::new(2);
    let (left, right) = tokio::join!(
        async {
            gate.wait().await;
            a.save(actor, left).await
        },
        async {
            gate.wait().await;
            b.save(actor, right).await
        }
    );
    let results = [left, right];
    let winner = results.iter().find_map(|r| r.as_ref().ok()).unwrap();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|r|matches!(r,Err(ContentError::Conflict{current_revision_id}) if *current_revision_id==winner.reference.revision_id)).count(),1);
    assert_eq!(counts(&rig, space).await, (1, 2, 0, 2, 0));
    pa.close().await;
    pb.close().await;
}
#[tokio::test]
async fn competing_first_reviews_and_updates_each_have_one_cas_winner() {
    let (rig, store, actor, space, e, h) = fixture().await;
    let r = store
        .save(actor, save(space, exact(&e), exact(&h)))
        .await
        .unwrap();
    let (pa, a, _) = connection().await;
    let (pb, b, _) = connection().await;
    let mut previous = None;
    for _ in 0..2 {
        let mut left = review(&r);
        left.expected_previous = previous;
        let mut right = review(&r);
        right.expected_previous = previous;
        right.state = RelationReviewState::NeedsRecheck;
        let gate = Barrier::new(2);
        let (left, right) = tokio::join!(
            async {
                gate.wait().await;
                a.review(actor, left).await
            },
            async {
                gate.wait().await;
                b.review(actor, right).await
            }
        );
        let results = [left, right];
        let winner = results.iter().find_map(|r| r.as_ref().ok()).unwrap();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|r|matches!(r,Err(ContentError::Conflict{current_revision_id}) if *current_revision_id==winner.reference.review_id)).count(),1);
        previous = Some(winner.reference.review_id);
    }
    assert_eq!(counts(&rig, space).await, (1, 1, 2, 1, 2));
    pa.close().await;
    pb.close().await;
}
#[tokio::test]
async fn identical_save_and_review_requests_share_receipts_under_race() {
    let (rig, _, actor, space, e, h) = fixture().await;
    let (pa, a, _) = connection().await;
    let (pb, b, _) = connection().await;
    let c = save(space, exact(&e), exact(&h));
    let gate = Barrier::new(2);
    let (x, y) = tokio::join!(
        async {
            gate.wait().await;
            a.save(actor, c.clone()).await
        },
        async {
            gate.wait().await;
            b.save(actor, c.clone()).await
        }
    );
    let r = x.unwrap();
    assert_eq!(r, y.unwrap());
    let c = review(&r);
    let gate = Barrier::new(2);
    let (x, y) = tokio::join!(
        async {
            gate.wait().await;
            a.review(actor, c.clone()).await
        },
        async {
            gate.wait().await;
            b.review(actor, c.clone()).await
        }
    );
    assert_eq!(x.unwrap(), y.unwrap());
    assert_eq!(counts(&rig, space).await, (1, 1, 1, 1, 1));
    pa.close().await;
    pb.close().await;
}
#[tokio::test]
async fn relation_save_and_review_hold_current_grants_until_commit() {
    let (rig, store, actor, space, e, h) = fixture().await;
    let first = store
        .save(actor, save(space, exact(&e), exact(&h)))
        .await
        .unwrap();
    for reviewing in [false, true] {
        rig.grant(actor, space, true).await;
        let (pa, writer, writer_pid) = connection().await;
        let mut blocker = rig.admin_pool.begin().await.unwrap();
        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *blocker)
            .await
            .unwrap();
        sqlx::query("SELECT id FROM relation WHERE id=$1 FOR UPDATE")
            .bind(first.reference.relation_id)
            .execute(&mut *blocker)
            .await
            .unwrap();
        let c = edit(&first);
        let rc = review(&first);
        let writing = tokio::spawn(async move {
            if reviewing {
                writer.review(actor, rc).await.map(|_| ())
            } else {
                writer.save(actor, c).await.map(|_| ())
            }
        });
        blocked(&rig.admin_pool, writer_pid, blocker_pid).await;
        let mut revoke = rig.admin_pool.begin().await.unwrap();
        let revoke_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *revoke)
            .await
            .unwrap();
        let revoking = tokio::spawn(async move {
            sqlx::query("DELETE FROM space_grant WHERE actor_id=$1 AND space_id=$2")
                .bind(actor.actor_id)
                .bind(space)
                .execute(&mut *revoke)
                .await
                .unwrap();
            revoke.commit().await.unwrap();
        });
        blocked(&rig.admin_pool, revoke_pid, writer_pid).await;
        blocker.commit().await.unwrap();
        writing.await.unwrap().unwrap();
        revoking.await.unwrap();
        assert!(
            store
                .read(actor, first.reference.clone())
                .await
                .unwrap()
                .is_none()
        );
        pa.close().await;
    }
}
#[tokio::test]
async fn receipt_failure_rolls_back_relation_and_review_objects_heads_and_indexes() {
    let (rig, store, actor, space, e, h) = fixture().await;
    let c = save(space, exact(&e), exact(&h));
    // Late failure after all object writes; scoped by actor and removed before retry.
    let function = format!("task4_fail_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE FUNCTION {function}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.actor_id='{}'::uuid THEN RAISE EXCEPTION 'injected receipt failure'; END IF; RETURN NEW; END $$",actor.actor_id)).execute(&rig.admin_pool).await.unwrap();
    for table in ["relation_receipt", "relation_review_receipt"] {
        sqlx::query(&format!("CREATE TRIGGER {function} BEFORE INSERT ON {table} FOR EACH ROW EXECUTE FUNCTION {function}()" )).execute(&rig.admin_pool).await.unwrap();
    }
    assert!(matches!(
        store.save(actor, c.clone()).await,
        Err(ContentError::Storage)
    ));
    assert_eq!(counts(&rig, space).await, (0, 0, 0, 0, 0));
    sqlx::query(&format!("DROP TRIGGER {function} ON relation_receipt"))
        .execute(&rig.admin_pool)
        .await
        .unwrap();
    let r = store.save(actor, c).await.unwrap();
    let rc = review(&r);
    assert!(matches!(
        store.review(actor, rc.clone()).await,
        Err(ContentError::Storage)
    ));
    assert_eq!(counts(&rig, space).await, (1, 1, 0, 1, 0));
    let heads: i64 =
        sqlx::query_scalar("SELECT count(*) FROM relation_review_head WHERE relation_id=$1")
            .bind(r.reference.relation_id)
            .fetch_one(&rig.admin_pool)
            .await
            .unwrap();
    assert_eq!(heads, 0);
    let objects: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM reference_object WHERE space_id=$1 AND kind='relation_review'",
    )
    .bind(space)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(objects, 0);
    sqlx::query(&format!(
        "DROP TRIGGER {function} ON relation_review_receipt"
    ))
    .execute(&rig.admin_pool)
    .await
    .unwrap();
    let reviewed = store.review(actor, rc).await.unwrap();
    assert_eq!(counts(&rig, space).await, (1, 1, 1, 1, 1));
    for table in ["relation_receipt", "relation_review_receipt"] {
        sqlx::query(&format!("CREATE TRIGGER {function} BEFORE INSERT ON {table} FOR EACH ROW EXECUTE FUNCTION {function}()" )).execute(&rig.admin_pool).await.unwrap();
    }
    let mut update = edit(&r);
    update.rationale = "new rationale".into();
    let mut recheck = review(&r);
    recheck.expected_previous = Some(reviewed.reference.review_id);
    recheck.state = RelationReviewState::NeedsRecheck;
    assert!(matches!(
        store.save(actor, update.clone()).await,
        Err(ContentError::Storage)
    ));
    assert!(matches!(
        store.review(actor, recheck.clone()).await,
        Err(ContentError::Storage)
    ));
    assert_eq!(counts(&rig, space).await, (1, 1, 1, 1, 1));
    let relation_head: Uuid =
        sqlx::query_scalar("SELECT head_revision_id FROM relation WHERE id=$1")
            .bind(r.reference.relation_id)
            .fetch_one(&rig.admin_pool)
            .await
            .unwrap();
    let review_head:Uuid=sqlx::query_scalar("SELECT head_review_id FROM relation_review_head WHERE relation_id=$1 AND relation_revision_id=$2").bind(r.reference.relation_id).bind(r.reference.revision_id).fetch_one(&rig.admin_pool).await.unwrap();
    assert_eq!(relation_head, r.reference.revision_id);
    assert_eq!(review_head, reviewed.reference.review_id);
    let registered:i64=sqlx::query_scalar("SELECT count(*) FROM reference_object WHERE space_id=$1 AND kind IN ('relation','relation_review')").bind(space).fetch_one(&rig.admin_pool).await.unwrap();
    assert_eq!(registered, 2);
    let dependencies:i64=sqlx::query_scalar("SELECT count(*) FROM reference_dependency d JOIN reference_object r ON (r.kind,r.object_id,r.revision_id)=(d.source_kind,d.source_object_id,d.source_revision_id) WHERE r.space_id=$1 AND r.kind IN ('relation','relation_review')").bind(space).fetch_one(&rig.admin_pool).await.unwrap();
    assert_eq!(dependencies, 3);
    let keys: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM request_key WHERE actor_id=$1 AND request_id IN ($2,$3)",
    )
    .bind(actor.actor_id)
    .bind(update.request_id)
    .bind(recheck.request_id)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(keys, 0);
    for table in ["relation_receipt", "relation_review_receipt"] {
        sqlx::query(&format!("DROP TRIGGER {function} ON {table}"))
            .execute(&rig.admin_pool)
            .await
            .unwrap();
    }
    sqlx::query(&format!("DROP FUNCTION {function}()"))
        .execute(&rig.admin_pool)
        .await
        .unwrap();
    store.save(actor, update).await.unwrap();
    store.review(actor, recheck).await.unwrap();
    assert_eq!(counts(&rig, space).await, (1, 2, 2, 2, 2));
}

#[tokio::test]
async fn concurrent_head_with_new_dependency_space_requires_complete_authorization_retry() {
    let (rig, store, actor, space, e, h) = fixture().await;
    let first = store
        .save(actor, save(space, exact(&e), exact(&h)))
        .await
        .unwrap();
    let (_, other_space) = rig.seed_actor_space(true).await;
    rig.grant(actor, other_space, true).await;
    let witness = rig
        .store
        .create(actor, other_space, support::command("new dependency"))
        .await
        .unwrap();
    let h2 = learning_db::VersionedContentStore::new(rig.runtime_pool.clone())
        .revise(
            actor,
            h.block_id,
            ReviseContent {
                request_id: Uuid::new_v4(),
                base_revision_id: h.revision_id,
                draft: support::references::v2(exact(&witness), false),
                reason: "new scope".into(),
            },
        )
        .await
        .unwrap();
    let (pool, writer, pid) = connection().await;
    let mut blocker = rig.admin_pool.begin().await.unwrap();
    let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    sqlx::query("SELECT id FROM relation WHERE id=$1 FOR UPDATE")
        .bind(first.reference.relation_id)
        .execute(&mut *blocker)
        .await
        .unwrap();
    let command = edit(&first);
    let retry = command.clone();
    let saving = tokio::spawn(async move { writer.save(actor, command).await });
    blocked(&rig.admin_pool, pid, blocker_pid).await;
    let new_revision = Uuid::new_v4();
    // Model another trusted writer committing after discovery but before the
    // identity lock. This fixture uses real immutable rows and checked indexes.
    sqlx::query("INSERT INTO relation_revision(id,space_id,relation_id,parent_revision_id,from_space_id,from_block_id,from_revision_id,to_space_id,to_block_id,to_revision_id,rationale,conditions,content_sha256,author_id) SELECT $1,space_id,relation_id,id,from_space_id,from_block_id,from_revision_id,to_space_id,to_block_id,$2,rationale,conditions,content_sha256,author_id FROM relation_revision WHERE id=$3")
        .bind(new_revision).bind(h2.revision_id).bind(first.reference.revision_id).execute(&mut *blocker).await.unwrap();
    support::typed_references::dependency(
        &mut blocker,
        ("relation", first.reference.relation_id, new_revision),
        0,
        "target",
        ("block", e.block_id, e.revision_id),
    )
    .await;
    support::typed_references::dependency(
        &mut blocker,
        ("relation", first.reference.relation_id, new_revision),
        1,
        "target",
        ("block", h2.block_id, h2.revision_id),
    )
    .await;
    sqlx::query("UPDATE relation SET head_revision_id=$1 WHERE id=$2")
        .bind(new_revision)
        .bind(first.reference.relation_id)
        .execute(&mut *blocker)
        .await
        .unwrap();
    blocker.commit().await.unwrap();
    assert!(
        matches!(saving.await.unwrap(),Err(ContentError::Invalid(s)) if s=="reference_authorization_changed")
    );
    assert!(
        matches!(store.save(actor,retry).await,Err(ContentError::Conflict{current_revision_id}) if current_revision_id==new_revision)
    );
    assert_eq!(counts(&rig, space).await, (1, 2, 0, 1, 0));
    pool.close().await;
}

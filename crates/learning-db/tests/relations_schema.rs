mod support;

use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use support::{TestRig, sqlstate};
use uuid::Uuid;

type Tx<'a> = Transaction<'a, Postgres>;

fn integrity(error: sqlx::Error) {
    let code = sqlstate(&error).unwrap_or_default();
    assert!(
        code.starts_with("23"),
        "expected integrity error, got {error:?}"
    );
}

async fn dependency(
    tx: &mut Tx<'_>,
    source: (&str, Uuid, Uuid),
    position: i32,
    role: &str,
    target: (&str, Uuid, Uuid),
) {
    sqlx::query("INSERT INTO reference_dependency(source_kind,source_object_id,source_revision_id,position,role,target_kind,target_object_id,target_revision_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
        .bind(source.0).bind(source.1).bind(source.2).bind(position).bind(role)
        .bind(target.0).bind(target.1).bind(target.2).execute(&mut **tx).await.unwrap();
}

async fn relation(
    tx: &mut Tx<'_>,
    actor: Uuid,
    space: Uuid,
    from: (Uuid, Uuid),
    to: (Uuid, Uuid),
    indexed: bool,
) -> (Uuid, Uuid) {
    let id = Uuid::new_v4();
    let revision = Uuid::new_v4();
    sqlx::query("INSERT INTO relation(id,space_id,type,from_block_id,to_block_id,head_revision_id) VALUES($1,$2,'supports',$3,$4,$5)")
        .bind(id).bind(space).bind(from.0).bind(to.0).bind(revision).execute(&mut **tx).await.unwrap();
    sqlx::query("INSERT INTO relation_revision(id,space_id,relation_id,from_space_id,from_block_id,from_revision_id,to_space_id,to_block_id,to_revision_id,rationale,conditions,content_sha256,author_id) VALUES($1,$2,$3,$2,$4,$5,$2,$6,$7,'observation','within scope',$8,$9)")
        .bind(revision).bind(space).bind(id).bind(from.0).bind(from.1).bind(to.0).bind(to.1).bind("a".repeat(64)).bind(actor).execute(&mut **tx).await.unwrap();
    if indexed {
        dependency(
            tx,
            ("relation", id, revision),
            0,
            "target",
            ("block", from.0, from.1),
        )
        .await;
        dependency(
            tx,
            ("relation", id, revision),
            1,
            "target",
            ("block", to.0, to.1),
        )
        .await;
    }
    (id, revision)
}

async fn review(
    tx: &mut Tx<'_>,
    actor: Uuid,
    space: Uuid,
    rel: (Uuid, Uuid),
    previous: Option<Uuid>,
    indexed: bool,
) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO relation_review_head(relation_id,relation_revision_id,head_review_id) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
        .bind(rel.0).bind(rel.1).bind(id).execute(&mut **tx).await.unwrap();
    sqlx::query("INSERT INTO relation_review(id,space_id,relation_id,relation_revision_id,previous_review_id,state,explanation,reviewer_id) VALUES($1,$2,$3,$4,$5,'reviewed','checked',$6)")
        .bind(id).bind(space).bind(rel.0).bind(rel.1).bind(previous).bind(actor).execute(&mut **tx).await.unwrap();
    if indexed {
        dependency(
            tx,
            ("relation_review", rel.1, id),
            0,
            "target",
            ("relation", rel.0, rel.1),
        )
        .await;
    }
    id
}

async fn epistemic(
    tx: &mut Tx<'_>,
    actor: Uuid,
    space: Uuid,
    target: (Uuid, Uuid),
    selected: Value,
    evidence: Value,
    indexed: bool,
) -> (Uuid, Uuid) {
    let stream = Uuid::new_v4();
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO epistemic_stream(id,space_id,target_space_id,target_block_id,target_revision_id,actor_id,head_review_id) VALUES($1,$2,$2,$3,$4,$5,$6)")
        .bind(stream).bind(space).bind(target.0).bind(target.1).bind(actor).bind(id).execute(&mut **tx).await.unwrap();
    sqlx::query("INSERT INTO epistemic_review(id,space_id,stream_id,state,relations,evidence,conditions,explanation,reviewer_id) VALUES($1,$2,$3,'inconclusive',$4,$5,'bounded','pending',$6)")
        .bind(id).bind(space).bind(stream).bind(selected).bind(evidence).bind(actor).execute(&mut **tx).await.unwrap();
    if indexed {
        dependency(
            tx,
            ("epistemic_review", stream, id),
            0,
            "target",
            ("block", target.0, target.1),
        )
        .await;
    }
    (stream, id)
}

async fn v2(tx: &mut Tx<'_>, actor: Uuid, space: Uuid, draft: Value) -> (Uuid, Uuid) {
    let block = Uuid::new_v4();
    let revision = Uuid::new_v4();
    sqlx::query("INSERT INTO block(id,space_id,head_revision_id) VALUES($1,$2,$3)")
        .bind(block)
        .bind(space)
        .bind(revision)
        .execute(&mut **tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO block_revision(id,space_id,block_id,contract_version,content,content_sha256,author_id,reason) VALUES($1,$2,$3,2,$4,$5,$6,'v2')")
        .bind(revision).bind(space).bind(block).bind(draft).bind("b".repeat(64)).bind(actor).execute(&mut **tx).await.unwrap();
    (block, revision)
}

fn text() -> Value {
    json!({"intent":"note","language":"en","title":"v2","body":{"kind":"text","payload":{"format":"markdown","text":"body"}},"basis_refs":[],"requires_context":[],"source_run":null})
}

#[tokio::test]
async fn pre_b3_revisions_are_backfilled_without_changing_old_checksums_or_content() {
    let admin_pool = sqlx::PgPool::connect(
        &std::env::var("TEST_B3_SCHEMA_UPGRADE_ADMIN_DATABASE_URL")
            .expect("dedicated fresh B3 schema upgrade admin DSN required"),
    )
    .await
    .unwrap();
    let runtime_pool = sqlx::PgPool::connect(
        &std::env::var("TEST_B3_SCHEMA_UPGRADE_DATABASE_URL")
            .expect("dedicated B3 schema upgrade runtime DSN required"),
    )
    .await
    .unwrap();
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT to_regclass('public._sqlx_migrations') IS NULL")
            .fetch_one(&admin_pool)
            .await
            .unwrap()
    );
    let mut old = sqlx::migrate::Migrator::DEFAULT;
    old.migrations = std::borrow::Cow::Owned(
        learning_db::MIGRATOR
            .iter()
            .filter(|m| m.version <= 3)
            .cloned()
            .collect(),
    );
    old.run(&admin_pool).await.unwrap();
    let r = TestRig {
        store: learning_db::ContentStore::new(runtime_pool.clone()),
        admin_pool,
        runtime_pool,
    };
    let (a, s) = r.seed_actor_space(true).await;
    let cmd = support::command("historical v1 bytes");
    let before = r.store.create(a, s, cmd.clone()).await.unwrap();
    let checksums: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&r.admin_pool)
            .await
            .unwrap();
    learning_db::MIGRATOR.run(&r.admin_pool).await.unwrap();
    let after: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations WHERE version<=3 ORDER BY version",
    )
    .fetch_all(&r.admin_pool)
    .await
    .unwrap();
    assert_eq!(checksums, after);
    assert_eq!(
        r.store.read(a, before.revision_id).await.unwrap(),
        Some(before.clone())
    );
    assert_eq!(r.store.create(a, s, cmd).await.unwrap(), before);
    let registered:(Uuid,i64)=sqlx::query_as("SELECT space_id,(SELECT count(*) FROM reference_dependency WHERE source_kind='block' AND source_revision_id=$2) FROM reference_object WHERE kind='block' AND object_id=$1 AND revision_id=$2")
        .bind(before.block_id).bind(before.revision_id).fetch_one(&r.runtime_pool).await.unwrap();
    assert_eq!(registered, (s, 0));
}

#[tokio::test]
async fn runtime_cannot_mutate_history_or_schema() {
    let r = TestRig::from_env().await;
    for table in [
        "reference_object",
        "reference_dependency",
        "relation_revision",
        "relation_review",
        "epistemic_review",
        "relation_receipt",
        "relation_review_receipt",
        "epistemic_review_receipt",
    ] {
        for query in [
            format!("DELETE FROM {table} WHERE false"),
            format!(
                "UPDATE {table} SET {}={} WHERE false",
                if table == "reference_dependency" {
                    "position"
                } else if table == "reference_object" {
                    "kind"
                } else if table.ends_with("receipt") {
                    "request_id"
                } else {
                    "id"
                },
                if table == "reference_dependency" {
                    "position"
                } else if table == "reference_object" {
                    "kind"
                } else if table.ends_with("receipt") {
                    "request_id"
                } else {
                    "id"
                }
            ),
        ] {
            assert_eq!(
                sqlstate(
                    &sqlx::query(&query)
                        .execute(&r.runtime_pool)
                        .await
                        .unwrap_err()
                )
                .as_deref(),
                Some("42501"),
                "{query}"
            );
        }
    }
    for table in ["relation", "epistemic_stream"] {
        assert_eq!(
            sqlstate(
                &sqlx::query(&format!("UPDATE {table} SET space_id=space_id WHERE false"))
                    .execute(&r.runtime_pool)
                    .await
                    .unwrap_err()
            )
            .as_deref(),
            Some("42501")
        );
    }
    assert_eq!(
        sqlstate(
            &sqlx::query("CREATE TABLE public.runtime_schema_escape(id int)")
                .execute(&r.runtime_pool)
                .await
                .unwrap_err()
        )
        .as_deref(),
        Some("42501")
    );
}

#[tokio::test]
async fn legacy_store_still_writes_and_registry_is_exact() {
    let r = TestRig::from_env().await;
    let (actor, space) = r.seed_actor_space(true).await;
    let first = r
        .store
        .create(actor, space, support::command("v1 preserved"))
        .await
        .unwrap();
    let second = r
        .store
        .revise(actor, first.block_id, support::change(&first, "next"))
        .await
        .unwrap();
    for row in [&first, &second] {
        assert_eq!(
            r.store.read(actor, row.revision_id).await.unwrap().as_ref(),
            Some(row)
        );
        let stored:(Uuid,i32,i64)=sqlx::query_as("SELECT o.space_id,r.contract_version,(SELECT count(*) FROM reference_dependency d WHERE d.source_kind='block' AND d.source_object_id=r.block_id AND d.source_revision_id=r.id) FROM reference_object o JOIN block_revision r ON r.id=o.revision_id WHERE o.kind='block' AND o.object_id=$1 AND o.revision_id=$2")
            .bind(row.block_id).bind(row.revision_id).fetch_one(&r.runtime_pool).await.unwrap();
        assert_eq!(stored, (space, 1, 0));
    }
    for migration in learning_db::MIGRATOR.iter().filter(|m| m.version <= 3) {
        let checksum: Vec<u8> =
            sqlx::query_scalar("SELECT checksum FROM _sqlx_migrations WHERE version=$1")
                .bind(migration.version)
                .fetch_one(&r.admin_pool)
                .await
                .unwrap();
        assert_eq!(checksum, migration.checksum.as_ref());
    }
}

#[tokio::test]
async fn forged_registry_and_missing_dependency_targets_fail_at_commit() {
    let r = TestRig::from_env().await;
    let (a, s) = r.seed_actor_space(true).await;
    for kind in ["block", "relation", "relation_review", "epistemic_review"] {
        let mut tx = r.runtime_pool.begin().await.unwrap();
        sqlx::query(
            "INSERT INTO reference_object(kind,object_id,revision_id,space_id) VALUES($1,$2,$3,$4)",
        )
        .bind(kind)
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(s)
        .execute(&mut *tx)
        .await
        .unwrap();
        integrity(tx.commit().await.unwrap_err());
    }
    let mut tx = r.runtime_pool.begin().await.unwrap();
    let absent = (Uuid::new_v4(), Uuid::new_v4());
    let mut draft = text();
    draft["body"] =
        json!({"kind":"reference","target":{"block_id":absent.0,"revision_id":absent.1}});
    let b = v2(&mut tx, a.actor_id, s, draft).await;
    dependency(
        &mut tx,
        ("block", b.0, b.1),
        0,
        "target",
        ("block", absent.0, absent.1),
    )
    .await;
    // The index matches the payload, so only the target FK can reject this.
    assert_eq!(
        sqlstate(&tx.commit().await.unwrap_err()).as_deref(),
        Some("23503")
    );
}

#[tokio::test]
async fn relations_require_exact_endpoints_and_complete_dependency_indexes() {
    let r = TestRig::from_env().await;
    let (a, s) = r.seed_actor_space(true).await;
    let b = r.seed_block(a, s).await;
    let c = r.seed_block(a, s).await;
    let mut tx = r.runtime_pool.begin().await.unwrap();
    let rel = relation(&mut tx, a.actor_id, s, b, c, true).await;
    tx.commit().await.unwrap();
    assert_eq!(sqlx::query_scalar::<_,Uuid>("SELECT space_id FROM reference_object WHERE kind='relation' AND object_id=$1 AND revision_id=$2").bind(rel.0).bind(rel.1).fetch_one(&r.runtime_pool).await.unwrap(),s);
    let d = r.seed_block(a, s).await;
    let mut tx = r.runtime_pool.begin().await.unwrap();
    relation(&mut tx, a.actor_id, s, b, d, false).await;
    integrity(tx.commit().await.unwrap_err());
    let mut tx = r.runtime_pool.begin().await.unwrap();
    dependency(
        &mut tx,
        ("relation", rel.0, rel.1),
        2,
        "basis",
        ("block", d.0, d.1),
    )
    .await;
    integrity(tx.commit().await.unwrap_err());
    // A revision in a different space cannot be disguised with the claimed endpoint space.
    let (_, other) = r.seed_actor_space(true).await;
    let e = r.seed_block(a, other).await;
    let mut tx = r.runtime_pool.begin().await.unwrap();
    let id = Uuid::new_v4();
    let revision = Uuid::new_v4();
    sqlx::query("INSERT INTO relation(id,space_id,type,from_block_id,to_block_id,head_revision_id) VALUES($1,$2,'tests',$3,$4,$5)").bind(id).bind(s).bind(b.0).bind(e.0).bind(revision).execute(&mut *tx).await.unwrap();
    let result=sqlx::query("INSERT INTO relation_revision(id,space_id,relation_id,from_space_id,from_block_id,from_revision_id,to_space_id,to_block_id,to_revision_id,rationale,conditions,content_sha256,author_id) VALUES($1,$2,$3,$2,$4,$5,$2,$6,$7,'','',$8,$9)")
        .bind(revision).bind(s).bind(id).bind(b.0).bind(b.1).bind(e.0).bind(e.1).bind("a".repeat(64)).bind(a.actor_id).execute(&mut *tx).await;
    match result {
        Err(e) => integrity(e),
        Ok(_) => integrity(tx.commit().await.unwrap_err()),
    }
}

#[tokio::test]
async fn relation_identity_prevents_duplicate_reverse_self_and_wrong_overlay() {
    let r = TestRig::from_env().await;
    let (a, s) = r.seed_actor_space(true).await;
    let b = r.seed_block(a, s).await;
    let c = r.seed_block(a, s).await;
    let (lo, hi) = if b.0 < c.0 { (b.0, c.0) } else { (c.0, b.0) };
    let mut tx = r.runtime_pool.begin().await.unwrap();
    sqlx::query("INSERT INTO relation(id,space_id,type,from_block_id,to_block_id,head_revision_id) VALUES($1,$2,'related_to',$3,$4,$5)").bind(Uuid::new_v4()).bind(s).bind(lo).bind(hi).bind(Uuid::new_v4()).execute(&mut *tx).await.unwrap();
    let duplicate=sqlx::query("INSERT INTO relation(id,space_id,type,from_block_id,to_block_id,head_revision_id) VALUES($1,$2,'related_to',$3,$4,$5)").bind(Uuid::new_v4()).bind(s).bind(lo).bind(hi).bind(Uuid::new_v4()).execute(&mut *tx).await.unwrap_err();
    integrity(duplicate);
    tx.rollback().await.unwrap();
    for (from, to, kind) in [(hi, lo, "related_to"), (lo, lo, "supports")] {
        let mut tx = r.runtime_pool.begin().await.unwrap();
        integrity(sqlx::query("INSERT INTO relation(id,space_id,type,from_block_id,to_block_id,head_revision_id) VALUES($1,$2,$3,$4,$5,$6)").bind(Uuid::new_v4()).bind(s).bind(kind).bind(from).bind(to).bind(Uuid::new_v4()).execute(&mut *tx).await.unwrap_err());
    }
    let (_, _, other_space, _, saved) = support::reading::fixture().await;
    assert_ne!(s, other_space);
    let (from, to) = if b.0 == lo { (b, c) } else { (c, b) };
    for overlay in [Uuid::new_v4(), saved.overlay.overlay_id] {
        let mut tx = r.runtime_pool.begin().await.unwrap();
        let id = Uuid::new_v4();
        let revision = Uuid::new_v4();
        sqlx::query("INSERT INTO relation(id,space_id,overlay_id,type,from_block_id,to_block_id,head_revision_id) VALUES($1,$2,$3,'questions',$4,$5,$6)").bind(id).bind(s).bind(overlay).bind(lo).bind(hi).bind(revision).execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO relation_revision(id,space_id,relation_id,from_space_id,from_block_id,from_revision_id,to_space_id,to_block_id,to_revision_id,rationale,conditions,content_sha256,author_id) VALUES($1,$2,$3,$2,$4,$5,$2,$6,$7,'','',$8,$9)")
            .bind(revision).bind(s).bind(id).bind(from.0).bind(from.1).bind(to.0).bind(to.1).bind("a".repeat(64)).bind(a.actor_id).execute(&mut *tx).await.unwrap();
        dependency(
            &mut tx,
            ("relation", id, revision),
            0,
            "target",
            ("block", from.0, from.1),
        )
        .await;
        dependency(
            &mut tx,
            ("relation", id, revision),
            1,
            "target",
            ("block", to.0, to.1),
        )
        .await;
        // All head, endpoint and dependency constraints are valid; the overlay FK rejects this.
        assert_eq!(
            sqlstate(&tx.commit().await.unwrap_err()).as_deref(),
            Some("23503")
        );
    }
}

#[tokio::test]
async fn all_fixed_reference_variants_and_context_source_target_roles_commit() {
    let r = TestRig::from_env().await;
    let (a, s) = r.seed_actor_space(true).await;
    let b = r.seed_block(a, s).await;
    let c = r.seed_block(a, s).await;
    let mut tx = r.runtime_pool.begin().await.unwrap();
    let rel = relation(&mut tx, a.actor_id, s, b, c, true).await;
    let review_id = review(&mut tx, a.actor_id, s, rel, None, true).await;
    let selected = json!([{"relation":{"relation_id":rel.0,"revision_id":rel.1},"review":{"relation":{"relation_id":rel.0,"revision_id":rel.1},"review_id":review_id}}]);
    let ep = epistemic(
        &mut tx,
        a.actor_id,
        s,
        c,
        selected,
        json!([{"block_id":b.0,"revision_id":b.1}]),
        true,
    )
    .await;
    dependency(
        &mut tx,
        ("epistemic_review", ep.0, ep.1),
        1,
        "selected_relation",
        ("relation", rel.0, rel.1),
    )
    .await;
    dependency(
        &mut tx,
        ("epistemic_review", ep.0, ep.1),
        2,
        "selected_review",
        ("relation_review", rel.1, review_id),
    )
    .await;
    dependency(
        &mut tx,
        ("epistemic_review", ep.0, ep.1),
        3,
        "basis",
        ("block", b.0, b.1),
    )
    .await;
    let mut draft = text();
    draft["basis_refs"] = json!([
        {"type":"block","block_id":b.0,"revision_id":b.1},
        {"type":"relation","relation_id":rel.0,"revision_id":rel.1},
        {"type":"relation_review","relation":{"relation_id":rel.0,"revision_id":rel.1},"review_id":review_id},
        {"type":"epistemic_review","stream_id":ep.0,"review_id":ep.1}
    ]);
    draft["requires_context"] = json!([{"block_id":b.0,"revision_id":b.1}]);
    draft["source_run"] = json!({"block_id":b.0,"revision_id":b.1});
    draft["body"] = json!({"kind":"reference","target":{"block_id":c.0,"revision_id":c.1}});
    let block = v2(&mut tx, a.actor_id, s, draft.clone()).await;
    let edges = [
        ("basis", ("block", b.0, b.1)),
        ("basis", ("relation", rel.0, rel.1)),
        ("basis", ("relation_review", rel.1, review_id)),
        ("basis", ("epistemic_review", ep.0, ep.1)),
        ("requires_context", ("block", b.0, b.1)),
        ("source_run", ("block", b.0, b.1)),
        ("target", ("block", c.0, c.1)),
    ];
    for (position, (role, target)) in edges.iter().enumerate() {
        dependency(
            &mut tx,
            ("block", block.0, block.1),
            position as i32,
            role,
            *target,
        )
        .await;
    }
    tx.commit().await.unwrap();
    // Positions and roles are part of the immutable index contract, not just a target set.
    for wrong in [4, 5, 6] {
        let mut tx = r.runtime_pool.begin().await.unwrap();
        let block = v2(&mut tx, a.actor_id, s, draft.clone()).await;
        for (position, (role, target)) in edges.iter().enumerate() {
            dependency(
                &mut tx,
                ("block", block.0, block.1),
                position as i32,
                if position == wrong { "basis" } else { role },
                *target,
            )
            .await;
        }
        integrity(tx.commit().await.unwrap_err());
    }
}

#[tokio::test]
async fn reviews_register_correct_identity_and_reject_cross_stream_predecessors() {
    let r = TestRig::from_env().await;
    let (a, s) = r.seed_actor_space(true).await;
    let b = r.seed_block(a, s).await;
    let c = r.seed_block(a, s).await;
    let mut tx = r.runtime_pool.begin().await.unwrap();
    let rel = relation(&mut tx, a.actor_id, s, b, c, true).await;
    let first = review(&mut tx, a.actor_id, s, rel, None, true).await;
    let stream = epistemic(&mut tx, a.actor_id, s, b, json!([]), json!([]), true).await;
    tx.commit().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, Uuid>(
            "SELECT object_id FROM reference_object WHERE kind='relation_review' AND revision_id=$1"
        )
        .bind(first)
        .fetch_one(&r.runtime_pool)
        .await
        .unwrap(),
        rel.1
    );
    let mut tx = r.runtime_pool.begin().await.unwrap();
    let rel2 = relation(&mut tx, a.actor_id, s, c, b, true).await;
    review(&mut tx, a.actor_id, s, rel2, Some(first), true).await;
    integrity(tx.commit().await.unwrap_err());
    let mut tx = r.runtime_pool.begin().await.unwrap();
    let other = epistemic(&mut tx, a.actor_id, s, c, json!([]), json!([]), true).await;
    let result=sqlx::query("INSERT INTO epistemic_review(id,space_id,stream_id,previous_review_id,state,relations,evidence,conditions,explanation,reviewer_id) VALUES($1,$2,$3,$4,'untested','[]','[]','','',$5)")
        .bind(Uuid::new_v4()).bind(s).bind(other.0).bind(stream.1).bind(a.actor_id).execute(&mut *tx).await;
    match result {
        Err(e) => integrity(e),
        Ok(_) => integrity(tx.commit().await.unwrap_err()),
    }
}

#[tokio::test]
async fn review_dependency_indexes_cannot_omit_or_add_evidence() {
    let r = TestRig::from_env().await;
    let (a, s) = r.seed_actor_space(true).await;
    let b = r.seed_block(a, s).await;
    let c = r.seed_block(a, s).await;
    let mut tx = r.runtime_pool.begin().await.unwrap();
    let rel = relation(&mut tx, a.actor_id, s, b, c, true).await;
    review(&mut tx, a.actor_id, s, rel, None, false).await;
    integrity(tx.commit().await.unwrap_err());
    let mut tx = r.runtime_pool.begin().await.unwrap();
    epistemic(
        &mut tx,
        a.actor_id,
        s,
        b,
        json!([]),
        json!([{"block_id":c.0,"revision_id":c.1}]),
        true,
    )
    .await;
    integrity(tx.commit().await.unwrap_err());
    let mut tx = r.runtime_pool.begin().await.unwrap();
    let ep = epistemic(&mut tx, a.actor_id, s, b, json!([]), json!([]), true).await;
    dependency(
        &mut tx,
        ("epistemic_review", ep.0, ep.1),
        1,
        "basis",
        ("block", c.0, c.1),
    )
    .await;
    integrity(tx.commit().await.unwrap_err());
}

#[tokio::test]
async fn v2_payload_dependency_order_and_exact_review_owner_are_enforced() {
    let r = TestRig::from_env().await;
    let (a, s) = r.seed_actor_space(true).await;
    let b = r.seed_block(a, s).await;
    let c = r.seed_block(a, s).await;
    let mut tx = r.runtime_pool.begin().await.unwrap();
    let rel = relation(&mut tx, a.actor_id, s, b, c, true).await;
    let rev = review(&mut tx, a.actor_id, s, rel, None, true).await;
    tx.commit().await.unwrap();
    let mut draft = text();
    draft["body"] = json!({"kind":"relation_view","selections":[{"relation":{"relation_id":rel.0,"revision_id":rel.1},"review":{"relation":{"relation_id":rel.0,"revision_id":rel.1},"review_id":rev}}]});
    let mut tx = r.runtime_pool.begin().await.unwrap();
    let block = v2(&mut tx, a.actor_id, s, draft.clone()).await;
    dependency(
        &mut tx,
        ("block", block.0, block.1),
        0,
        "selected_relation",
        ("relation", rel.0, rel.1),
    )
    .await;
    dependency(
        &mut tx,
        ("block", block.0, block.1),
        1,
        "selected_review",
        ("relation_review", rel.1, rev),
    )
    .await;
    tx.commit().await.unwrap();
    let mut tx = r.runtime_pool.begin().await.unwrap();
    v2(&mut tx, a.actor_id, s, draft.clone()).await;
    integrity(tx.commit().await.unwrap_err());
    draft["body"]["selections"][0]["review"]["relation"]["relation_id"] = json!(Uuid::new_v4());
    let mut tx = r.runtime_pool.begin().await.unwrap();
    let bad = v2(&mut tx, a.actor_id, s, draft).await;
    dependency(
        &mut tx,
        ("block", bad.0, bad.1),
        0,
        "selected_relation",
        ("relation", rel.0, rel.1),
    )
    .await;
    dependency(
        &mut tx,
        ("block", bad.0, bad.1),
        1,
        "selected_review",
        ("relation_review", rel.1, rev),
    )
    .await;
    integrity(tx.commit().await.unwrap_err());
}

#[tokio::test]
async fn content_contract_version_rejects_wrong_shapes_and_extra_indexes() {
    let r = TestRig::from_env().await;
    let (a, s) = r.seed_actor_space(true).await;
    let b = r.seed_block(a, s).await;
    for draft in [
        serde_json::to_value(support::command("v1").draft).unwrap(),
        json!({}),
        {
            let mut d = text();
            d["body"]["extra"] = json!(1);
            d
        },
    ] {
        let mut tx = r.runtime_pool.begin().await.unwrap();
        v2(&mut tx, a.actor_id, s, draft).await;
        integrity(tx.commit().await.unwrap_err());
    }
    let mut tx = r.runtime_pool.begin().await.unwrap();
    let block = v2(&mut tx, a.actor_id, s, text()).await;
    dependency(
        &mut tx,
        ("block", block.0, block.1),
        0,
        "basis",
        ("block", b.0, b.1),
    )
    .await;
    integrity(tx.commit().await.unwrap_err());
    let mut tx = r.runtime_pool.begin().await.unwrap();
    v2(&mut tx, a.actor_id, s, text()).await;
    tx.commit().await.unwrap();
}

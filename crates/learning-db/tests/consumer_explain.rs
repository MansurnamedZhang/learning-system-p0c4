//! Disposable synthetic plan measurement for the Task 2 reverse SELECTs.
//! Run only with explicit isolated test DSNs and `--ignored --test-threads=1`.
mod support;
use learning_core::*;
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction, types::Json};
use std::collections::BTreeSet;
use support::{TestRig, assembly as a, references as refs};
use uuid::Uuid;

// These SELECTs intentionally match queries/consumers.rs. Do not simplify the
// predicates for EXPLAIN: the purpose is to measure actual prepared queries.
const STRUCTURAL: &str = "SELECT o.composition_id,o.composition_revision_id,o.occurrence_id \
             FROM public.composition_occurrence o \
             JOIN public.space_grant g ON g.space_id=o.space_id AND g.actor_id=$1 \
             WHERE o.block_revision_id=$2 AND o.block_id=$3 \
               AND ($4::uuid IS NULL OR (o.composition_revision_id,o.occurrence_id)>($4,$5)) \
             ORDER BY o.composition_revision_id,o.occurrence_id LIMIT $6";
const COMPOSITION: &str = "SELECT o.composition_id,o.composition_revision_id,o.occurrence_id \
             FROM public.composition_occurrence o \
             JOIN public.space_grant g ON g.space_id=o.space_id AND g.actor_id=$1 \
             WHERE o.child_revision_id=$2 AND o.child_composition_id=$3 \
               AND ($4::uuid IS NULL OR (o.composition_revision_id,o.occurrence_id)>($4,$5)) \
             ORDER BY o.composition_revision_id,o.occurrence_id LIMIT $6";
const NECESSARY: &str = "SELECT d.source_kind,d.source_object_id,d.source_revision_id,d.position,d.role,rr.relation_id \
             FROM public.reference_dependency d \
             JOIN public.reference_object s ON (s.kind,s.object_id,s.revision_id)=(d.source_kind,d.source_object_id,d.source_revision_id) \
             JOIN public.space_grant g ON g.space_id=s.space_id AND g.actor_id=$1 \
             LEFT JOIN public.relation_review rr ON d.source_kind='relation_review' AND rr.relation_revision_id=d.source_object_id AND rr.id=d.source_revision_id \
             WHERE (d.target_kind,d.target_object_id,d.target_revision_id)=($2,$3,$4) \
               AND ($5::text IS NULL OR (d.source_kind,d.source_object_id,d.source_revision_id,d.position)>($5,$6,$7,$8)) \
               AND (d.source_kind='block' OR \
                    (d.source_kind='relation' AND EXISTS(SELECT 1 FROM public.relation r LEFT JOIN public.overlay o ON o.id=r.overlay_id WHERE r.id=d.source_object_id AND (r.overlay_id IS NULL OR o.owner_id=$1))) OR \
                    (d.source_kind='relation_review' AND EXISTS(SELECT 1 FROM public.relation r LEFT JOIN public.overlay o ON o.id=r.overlay_id WHERE r.id=rr.relation_id AND (r.overlay_id IS NULL OR o.owner_id=$1))) OR \
                    (d.source_kind='epistemic_review' AND EXISTS(SELECT 1 FROM public.epistemic_review er JOIN public.epistemic_stream es ON es.id=er.stream_id LEFT JOIN public.overlay o ON o.id=es.overlay_id WHERE er.stream_id=d.source_object_id AND er.id=d.source_revision_id AND (es.overlay_id IS NULL OR o.owner_id=$1)))) \
             ORDER BY d.source_kind,d.source_object_id,d.source_revision_id,d.position LIMIT $9";

fn fixture_error_class(error: &ContentError) -> &'static str {
    match error {
        ContentError::Invalid(code) => match code.as_str() {
            "composition_occurrences_limit" => "composition_occurrences_limit",
            "composition_objects_limit" => "composition_objects_limit",
            "composition_depth_limit" => "composition_depth_limit",
            "composition_body_limit" => "composition_body_limit",
            "composition_cycle" => "composition_cycle",
            _ => "invalid",
        },
        ContentError::NotFound => "not_found",
        ContentError::Conflict { .. } => "conflict",
        ContentError::PublicationConflict { .. } => "publication_conflict",
        ContentError::ReadingConflict { .. } => "reading_conflict",
        ContentError::IdempotencyConflict => "idempotency_conflict",
        ContentError::IdentityConflict => "identity_conflict",
        ContentError::Storage => "storage",
    }
}

fn plan_summary(label: &str, mode: &str, page: u8, plan: &Value) -> Value {
    let top = &plan[0];
    let mut indexes = BTreeSet::new();
    let mut nodes = BTreeSet::new();
    let mut sorts = BTreeSet::new();
    fn visit(
        node: &Value,
        indexes: &mut BTreeSet<String>,
        nodes: &mut BTreeSet<String>,
        sorts: &mut BTreeSet<String>,
    ) {
        if let Some(v) = node.get("Index Name").and_then(Value::as_str) {
            indexes.insert(v.to_owned());
        }
        if let Some(v) = node.get("Node Type").and_then(Value::as_str) {
            nodes.insert(v.to_owned());
        }
        if let Some(v) = node.get("Sort Method").and_then(Value::as_str) {
            sorts.insert(v.to_owned());
        }
        if let Some(children) = node.get("Plans").and_then(Value::as_array) {
            for child in children {
                visit(child, indexes, nodes, sorts);
            }
        }
    }
    visit(&top["Plan"], &mut indexes, &mut nodes, &mut sorts);
    json!({
        "query": label, "plan_mode": mode, "page": page,
        "indexes": indexes, "node_types": nodes, "sort_methods": sorts,
        "actual_rows": top["Plan"]["Actual Rows"],
        "shared_hit_blocks": top["Plan"]["Shared Hit Blocks"],
        "shared_read_blocks": top["Plan"]["Shared Read Blocks"],
        "execution_ms": top["Execution Time"],
    })
}

async fn structural_rows(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    target: &BlockRef,
    after: Option<(Uuid, Uuid)>,
) -> Vec<(Uuid, Uuid, Uuid)> {
    sqlx::query_as(STRUCTURAL)
        .bind(actor.actor_id)
        .bind(target.revision_id)
        .bind(target.block_id)
        .bind(after.map(|v| v.0))
        .bind(after.map(|v| v.1))
        .bind(128_i64)
        .fetch_all(&mut **tx)
        .await
        .unwrap()
}
async fn composition_rows(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    target: &CompositionRef,
    after: Option<(Uuid, Uuid)>,
) -> Vec<(Uuid, Uuid, Uuid)> {
    sqlx::query_as(COMPOSITION)
        .bind(actor.actor_id)
        .bind(target.revision_id)
        .bind(target.composition_id)
        .bind(after.map(|v| v.0))
        .bind(after.map(|v| v.1))
        .bind(128_i64)
        .fetch_all(&mut **tx)
        .await
        .unwrap()
}
async fn necessary_rows(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    target: &BlockRef,
    after: Option<(String, Uuid, Uuid, i32)>,
) -> Vec<(String, Uuid, Uuid, i32, String, Option<Uuid>)> {
    sqlx::query_as(NECESSARY)
        .bind(actor.actor_id)
        .bind("block")
        .bind(target.block_id)
        .bind(target.revision_id)
        .bind(after.as_ref().map(|v| v.0.as_str()))
        .bind(after.as_ref().map(|v| v.1))
        .bind(after.as_ref().map(|v| v.2))
        .bind(after.as_ref().map(|v| v.3))
        .bind(128_i64)
        .fetch_all(&mut **tx)
        .await
        .unwrap()
}
async fn explain_structural(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    target: &BlockRef,
    after: Option<(Uuid, Uuid)>,
) -> Value {
    let Json(value): Json<Value> = sqlx::query_scalar(&format!(
        "EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {STRUCTURAL}"
    ))
    .bind(actor.actor_id)
    .bind(target.revision_id)
    .bind(target.block_id)
    .bind(after.map(|v| v.0))
    .bind(after.map(|v| v.1))
    .bind(128_i64)
    .fetch_one(&mut **tx)
    .await
    .unwrap();
    value
}
async fn explain_composition(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    target: &CompositionRef,
    after: Option<(Uuid, Uuid)>,
) -> Value {
    let Json(value): Json<Value> = sqlx::query_scalar(&format!(
        "EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {COMPOSITION}"
    ))
    .bind(actor.actor_id)
    .bind(target.revision_id)
    .bind(target.composition_id)
    .bind(after.map(|v| v.0))
    .bind(after.map(|v| v.1))
    .bind(128_i64)
    .fetch_one(&mut **tx)
    .await
    .unwrap();
    value
}
async fn explain_necessary(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    target: &BlockRef,
    after: Option<(String, Uuid, Uuid, i32)>,
) -> Value {
    let Json(value): Json<Value> = sqlx::query_scalar(&format!(
        "EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {NECESSARY}"
    ))
    .bind(actor.actor_id)
    .bind("block")
    .bind(target.block_id)
    .bind(target.revision_id)
    .bind(after.as_ref().map(|v| v.0.as_str()))
    .bind(after.as_ref().map(|v| v.1))
    .bind(after.as_ref().map(|v| v.2))
    .bind(after.as_ref().map(|v| v.3))
    .bind(128_i64)
    .fetch_one(&mut **tx)
    .await
    .unwrap();
    value
}

#[tokio::test]
#[ignore = "requires fresh isolated PostgreSQL and explicit B4_EXPLAIN_SCALE=1..8"]
async fn measure_target_first_reverse_plans_on_valid_synthetic_rows() {
    // Panics from fixture internals must not echo SQL parameter values.
    std::panic::set_hook(Box::new(|_| eprintln!("synthetic EXPLAIN harness failed")));
    let scale: usize = std::env::var("B4_EXPLAIN_SCALE")
        .unwrap_or_else(|_| "1".into())
        .parse()
        .unwrap();
    assert!((1..=8).contains(&scale));
    eprintln!("phase=seed_actor_space");
    let rig = TestRig::from_env().await;
    let (actor, visible_space) = rig.seed_actor_space(true).await;
    let (_, hidden_space) = rig.seed_actor_space(true).await;
    rig.grant(actor, hidden_space, true).await;
    let target_revision = rig
        .store
        .create(actor, visible_space, support::command("synthetic target"))
        .await
        .unwrap();
    let other_revision = rig
        .store
        .create(
            actor,
            visible_space,
            support::command("synthetic distractor"),
        )
        .await
        .unwrap();
    let target = BlockRef {
        block_id: target_revision.block_id,
        revision_id: target_revision.revision_id,
    };
    let other = BlockRef {
        block_id: other_revision.block_id,
        revision_id: other_revision.revision_id,
    };
    eprintln!("phase=seed_child");
    let child = rig
        .compositions()
        .save(
            actor,
            visible_space,
            a::doc(vec![a::block(&target_revision)]),
        )
        .await
        .unwrap_or_else(|error| {
            eprintln!(
                "failure=seed_child category={}",
                fixture_error_class(&error)
            );
            panic!("synthetic child seed failed")
        });
    let other_child = rig
        .compositions()
        .save(
            actor,
            visible_space,
            a::doc(vec![a::block(&other_revision)]),
        )
        .await
        .unwrap_or_else(|error| {
            eprintln!(
                "failure=seed_other_child category={}",
                fixture_error_class(&error)
            );
            panic!("synthetic distractor child seed failed")
        });

    eprintln!("phase=block_docs");
    for (space, block, documents) in [
        (visible_space, &target_revision, 1_usize),
        (hidden_space, &target_revision, scale),
        (hidden_space, &other_revision, scale),
    ] {
        for _ in 0..documents {
            let targets = (0..256).map(|_| a::block(block)).collect();
            rig.compositions()
                .save(actor, space, a::doc(targets))
                .await
                .unwrap_or_else(|error| {
                    eprintln!(
                        "failure=block_docs category={}",
                        fixture_error_class(&error)
                    );
                    panic!("synthetic block parent save failed")
                });
        }
    }
    eprintln!("phase=child_docs");
    for (space, child_ref, documents) in [
        (visible_space, &child, 1_usize),
        (hidden_space, &child, scale),
        (hidden_space, &other_child, scale),
    ] {
        for _ in 0..documents {
            let targets = (0..256).map(|_| a::child(child_ref)).collect();
            rig.compositions()
                .save(actor, space, a::doc(targets))
                .await
                .unwrap_or_else(|error| {
                    eprintln!(
                        "failure=child_docs category={}",
                        fixture_error_class(&error)
                    );
                    panic!("synthetic child parent save failed")
                });
        }
    }
    eprintln!("phase=dependency");
    for (space, basis, count) in [
        (visible_space, &target, 256_usize),
        (hidden_space, &target, 256 * scale),
        (hidden_space, &other, 256 * scale),
    ] {
        for _ in 0..count / 128 {
            let mut tx = rig.runtime_pool.begin().await.unwrap();
            for _ in 0..128 {
                refs::seed_content(&mut tx, actor, space, refs::v2(basis.clone(), false), None)
                    .await;
            }
            tx.commit().await.unwrap();
        }
    }
    eprintln!("phase=revoke");
    rig.revoke(actor, hidden_space).await;
    eprintln!("phase=analyze");
    for statement in [
        "ANALYZE public.composition_occurrence",
        "ANALYZE public.reference_dependency",
        "ANALYZE public.reference_object",
        "ANALYZE public.space_grant",
    ] {
        sqlx::query(statement)
            .execute(&rig.admin_pool)
            .await
            .unwrap();
    }

    eprintln!("phase=counts");
    let structural_all: i64 = sqlx::query_scalar("SELECT count(*) FROM public.composition_occurrence WHERE block_revision_id=$1 AND block_id=$2")
        .bind(target.revision_id).bind(target.block_id).fetch_one(&rig.admin_pool).await.unwrap();
    let necessary_all: i64 = sqlx::query_scalar("SELECT count(*) FROM public.reference_dependency WHERE target_kind='block' AND target_object_id=$1 AND target_revision_id=$2")
        .bind(target.block_id).bind(target.revision_id).fetch_one(&rig.admin_pool).await.unwrap();
    let composition_all: i64 = sqlx::query_scalar("SELECT count(*) FROM public.composition_occurrence WHERE child_revision_id=$1 AND child_composition_id=$2")
        .bind(child.reference.revision_id).bind(child.reference.composition_id).fetch_one(&rig.admin_pool).await.unwrap();
    let expected = (256 * (scale + 1)) as i64;
    // The visible target child itself contains one target block occurrence.
    let structural_expected = expected + 1;
    println!(
        "{}",
        json!({"fixture_counts":true, "scale":scale, "structural_all":structural_all, "composition_all":composition_all, "necessary_all":necessary_all, "structural_expected":structural_expected, "composition_expected":expected, "necessary_expected":expected})
    );
    assert_eq!(structural_all, structural_expected);
    assert_eq!(necessary_all, expected);
    assert_eq!(composition_all, expected);
    println!(
        "{}",
        json!({"fixture":"synthetic", "scale":scale, "structural_target_rows":structural_all, "composition_target_rows":composition_all, "necessary_target_rows":necessary_all, "structural_visible_rows":257, "composition_visible_rows":256, "necessary_visible_rows":256})
    );

    eprintln!("phase=plan");
    for mode in ["force_custom_plan", "force_generic_plan"] {
        eprintln!("phase=plan mode={mode}");
        let mut tx = rig.runtime_pool.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("SET LOCAL statement_timeout='15s'")
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query(if mode == "force_custom_plan" {
            "SET LOCAL plan_cache_mode=force_custom_plan"
        } else {
            "SET LOCAL plan_cache_mode=force_generic_plan"
        })
        .execute(&mut *tx)
        .await
        .unwrap();
        let first_s = structural_rows(&mut tx, actor, &target, None).await;
        let first_c = composition_rows(&mut tx, actor, &child.reference, None).await;
        let first_n = necessary_rows(&mut tx, actor, &target, None).await;
        assert_eq!(first_s.len(), 128);
        assert_eq!(first_c.len(), 128);
        assert_eq!(first_n.len(), 128);
        let last_s = first_s.last().map(|v| (v.1, v.2));
        let last_c = first_c.last().map(|v| (v.1, v.2));
        let last_n = first_n.last().map(|v| (v.0.clone(), v.1, v.2, v.3));
        assert_eq!(
            structural_rows(&mut tx, actor, &target, last_s).await.len(),
            128
        );
        assert_eq!(
            composition_rows(&mut tx, actor, &child.reference, last_c)
                .await
                .len(),
            128
        );
        assert_eq!(
            necessary_rows(&mut tx, actor, &target, last_n.clone())
                .await
                .len(),
            128
        );
        for (page, s_after, c_after, n_after) in
            [(1, None, None, None), (2, last_s, last_c, last_n)]
        {
            for _ in 0..3 {
                let s = explain_structural(&mut tx, actor, &target, s_after).await;
                println!("{}", plan_summary("structural", mode, page, &s));
                let c = explain_composition(&mut tx, actor, &child.reference, c_after).await;
                println!("{}", plan_summary("composition", mode, page, &c));
                let n = explain_necessary(&mut tx, actor, &target, n_after.clone()).await;
                println!("{}", plan_summary("necessary", mode, page, &n));
            }
        }
        tx.commit().await.unwrap();
    }
}

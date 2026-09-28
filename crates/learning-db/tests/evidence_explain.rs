//! Disposable B4 Task 3 plan evidence. Run on fresh isolated PG only.
#[path = "support/relation_store.rs"]
mod relations;
mod support;
use learning_core::*;
use learning_db::RelationStore;
use serde_json::{Value, json};
use sqlx::types::Json;
use support::{TestRig, references as refs};
use uuid::Uuid;

// Deliberately identical to queries/evidence.rs::dynamic_heads.
const DYNAMIC: &str = "SELECT d.source_object_id,d.source_revision_id,d.position \
 FROM public.reference_dependency d \
 JOIN public.relation r ON r.id=d.source_object_id AND r.head_revision_id=d.source_revision_id \
 JOIN public.space_grant g ON g.space_id=r.space_id AND g.actor_id=$1 \
 WHERE d.target_kind='block' AND d.target_object_id=$2 AND d.target_revision_id=$3 \
 AND d.source_kind='relation' AND d.role='target' \
 AND (r.overlay_id IS NULL OR EXISTS(SELECT 1 FROM public.overlay o \
 WHERE o.id=r.overlay_id AND o.owner_id=$1 AND o.space_id=r.space_id)) \
 AND ($4::uuid IS NULL OR (d.source_object_id,d.source_revision_id,d.position)>($4,$5,$6)) \
 ORDER BY d.source_object_id,d.source_revision_id,d.position LIMIT $7";

async fn rows(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: Principal,
    target: &BlockRef,
    after: Option<(Uuid, Uuid, i32)>,
) -> Vec<(Uuid, Uuid, i32)> {
    sqlx::query_as(DYNAMIC)
        .bind(actor.actor_id)
        .bind(target.block_id)
        .bind(target.revision_id)
        .bind(after.map(|v| v.0))
        .bind(after.map(|v| v.1))
        .bind(after.map(|v| v.2))
        .bind(128_i64)
        .fetch_all(&mut **tx)
        .await
        .unwrap()
}

async fn explain(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: Principal,
    target: &BlockRef,
    after: Option<(Uuid, Uuid, i32)>,
) -> Value {
    let Json(value): Json<Value> = sqlx::query_scalar(&format!(
        "EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {DYNAMIC}"
    ))
    .bind(actor.actor_id)
    .bind(target.block_id)
    .bind(target.revision_id)
    .bind(after.map(|v| v.0))
    .bind(after.map(|v| v.1))
    .bind(after.map(|v| v.2))
    .bind(128_i64)
    .fetch_one(&mut **tx)
    .await
    .unwrap();
    value
}

#[tokio::test]
#[ignore = "requires fresh isolated PostgreSQL; emits raw EXPLAIN JSON"]
async fn measure_exact_dynamic_target_first_and_current_head_in_visible_and_revoked_scopes() {
    let rig = TestRig::from_env().await;
    let (actor, visible_space) = rig.seed_actor_space(true).await;
    let (_, hidden_space) = rig.seed_actor_space(true).await;
    rig.grant(actor, hidden_space, true).await;
    let target = rig
        .store
        .create(actor, visible_space, support::command("plan target"))
        .await
        .unwrap();
    let target = BlockRef {
        block_id: target.block_id,
        revision_id: target.revision_id,
    };
    let store = RelationStore::new(rig.runtime_pool.clone());
    for (space, label) in [(visible_space, "visible"), (hidden_space, "later-revoked")] {
        for index in 0..130 {
            let e = rig
                .store
                .create(actor, space, support::command(&format!("{label}-{index}")))
                .await
                .unwrap();
            let relation = store
                .save(
                    actor,
                    relations::save(space, relations::exact(&e), target.clone()),
                )
                .await
                .unwrap();
            // A historical revision is present but the query must only return
            // the exact current head for each relation identity.
            if index == 0 {
                let mut edit = relations::edit(&relation);
                edit.rationale = "new head".into();
                store.save(actor, edit).await.unwrap();
            }
        }
    }
    // Raise target selectivity without inventing a second relationship table.
    // These registered, valid v2 dependencies point to an unrelated target;
    // the production dynamic predicate must still seek the exact H revision.
    let distractor = rig
        .store
        .create(actor, visible_space, support::command("unrelated target"))
        .await
        .unwrap();
    let distractor = BlockRef {
        block_id: distractor.block_id,
        revision_id: distractor.revision_id,
    };
    for _ in 0..32 {
        let mut tx = rig.runtime_pool.begin().await.unwrap();
        for _ in 0..128 {
            refs::seed_content(
                &mut tx,
                actor,
                visible_space,
                refs::v2(distractor.clone(), false),
                None,
            )
            .await;
        }
        tx.commit().await.unwrap();
    }
    // Same source kind and role as the product query, but a different exact
    // target. Keep source identities distinct to satisfy relation uniqueness.
    for _ in 0..4 {
        let mut tx = rig.runtime_pool.begin().await.unwrap();
        let mut sources = Vec::new();
        for _ in 0..128 {
            sources.push(
                refs::seed_content(
                    &mut tx,
                    actor,
                    visible_space,
                    ContentDraft::V1(support::command("unrelated relation source").draft),
                    None,
                )
                .await,
            );
        }
        tx.commit().await.unwrap();
        for source in sources {
            store
                .save(
                    actor,
                    relations::save(visible_space, source, distractor.clone()),
                )
                .await
                .unwrap();
        }
    }
    println!(
        "{}",
        json!({"fixture":"dynamic_target_selectivity","exact_relation_identities":260,"unrelated_relation_identities":512,"unrelated_block_dependencies":4096})
    );
    for revoked in [false, true] {
        if revoked {
            rig.revoke(actor, hidden_space).await;
        }
        for statement in [
            "ANALYZE public.reference_dependency",
            "ANALYZE public.relation",
            "ANALYZE public.space_grant",
        ] {
            sqlx::query(statement)
                .execute(&rig.admin_pool)
                .await
                .unwrap();
        }
        for mode in ["force_custom_plan", "force_generic_plan"] {
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
            let first = rows(&mut tx, actor, &target, None).await;
            assert_eq!(first.len(), 128);
            let after = first.last().copied();
            let second = rows(&mut tx, actor, &target, after).await;
            assert_eq!(second.len(), if revoked { 2 } else { 128 });
            for (page, cursor) in [(1, None), (2, after)] {
                let plan = explain(&mut tx, actor, &target, cursor).await;
                println!(
                    "{}",
                    json!({"query":"dynamic_exact_head", "revoked":revoked, "mode":mode, "page":page, "plan":plan})
                );
            }
            tx.commit().await.unwrap();
        }
    }
}

mod support;

use learning_core::*;
use learning_db::{LineageStore, QueryStore};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction, types::Json};
use support::{TestRig, assembly as a};
use uuid::Uuid;

fn lineage_command(
    operation: LineageOperation,
    inputs: Vec<BlockRef>,
    outputs: usize,
) -> LineageCommand {
    LineageCommand {
        request_id: Uuid::new_v4(),
        operation,
        inputs,
        outputs: (0..outputs)
            .map(|i| ContentDraft::V1(support::command(&format!("derived {i}")).draft))
            .collect(),
        reason: "explicit transformation".into(),
    }
}

async fn release_pair(
    rig: &TestRig,
    actor: Principal,
    space: Uuid,
    members: Vec<BlockRef>,
) -> Uuid {
    let root = rig
        .compositions()
        .save(
            actor,
            space,
            a::doc(members.into_iter().map(NodeTarget::Block).collect()),
        )
        .await
        .unwrap();
    rig.releases()
        .publish(actor, space, a::publish(vec![a::root(&root, None)]))
        .await
        .unwrap()
        .release_id
}

fn query(start: BlockRef, release_id: Uuid) -> ImpactQuery {
    ImpactQuery {
        start: ImpactStart::Block(start),
        scope: ImpactScope::Release { release_id },
        families: vec![ImpactFamily::Lineage],
        max_depth: 2,
        limit: 50,
        work_limit: 4096,
        after: None,
    }
}

#[tokio::test]
async fn split_keeps_one_operation_and_two_distinct_output_explanations() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let (block_id, revision_id) = rig.seed_block(actor, space).await;
    let source = BlockRef {
        block_id,
        revision_id,
    };
    let saved = LineageStore::new(rig.runtime_pool.clone())
        .apply(
            actor,
            space,
            lineage_command(LineageOperation::Split, vec![source.clone()], 2),
        )
        .await
        .unwrap();
    let release_id = release_pair(
        &rig,
        actor,
        space,
        vec![
            source.clone(),
            saved.outputs[0].clone(),
            saved.outputs[1].clone(),
        ],
    )
    .await;
    let result = QueryStore::new(rig.runtime_pool.clone())
        .traverse(actor, query(source.clone(), release_id))
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result.status(), PageStatus::Complete));
    assert_eq!(result.consumers().len(), 3);
    let operation = ImpactNode::Lineage {
        operation_id: saved.operation_id,
    };
    let first = result
        .consumers()
        .iter()
        .find(|g| g.consumer == operation)
        .unwrap();
    assert_eq!(first.explanations.len(), 1);
    assert_eq!(first.explanations[0].steps.len(), 1);
    let entry = &first.explanations[0].steps[0];
    assert_eq!(entry.from, ImpactNode::Block(source.clone()));
    assert_eq!(entry.to, operation);
    assert_eq!(entry.family, ImpactFamily::Lineage);
    assert_eq!(entry.reason, ImpactReason::DerivedFrom);
    assert_eq!(entry.provenance, ImpactProvenance::Stored);
    assert_eq!(entry.direction, Some(TraversalDirection::SavedReverse));
    assert_eq!(entry.lineage_type, Some(SystemLineageType::SplitFrom));
    assert_eq!(entry.relation_type, None);
    for output in &saved.outputs {
        let group = result
            .consumers()
            .iter()
            .find(|g| g.consumer == ImpactNode::Block(output.clone()))
            .unwrap();
        assert_eq!(group.explanations.len(), 1);
        let steps = &group.explanations[0].steps;
        assert_eq!(steps.len(), 2);
        assert_eq!(&steps[0], entry);
        assert_eq!(steps[1].from, operation);
        assert_eq!(steps[1].to, ImpactNode::Block(output.clone()));
        assert_eq!(steps[1].family, ImpactFamily::Lineage);
        assert_eq!(steps[1].lineage_type, Some(SystemLineageType::SplitFrom));
        assert_eq!(steps[1].direction, Some(TraversalDirection::SavedReverse));
        assert_eq!(steps[1].provenance, ImpactProvenance::Stored);
        assert_eq!(steps[1].reason, ImpactReason::DerivedFrom);
    }
}

// Keep this byte-for-byte equivalent to the reverse SELECT in
// queries/lineage_impact.rs. It is deliberately an ignored, isolated-DB plan
// fixture and not part of the routine workspace test suite.
const REVERSE_INPUT: &str = "SELECT i.operation_id,i.position \
    FROM public.lineage_input i \
    JOIN public.lineage_operation o ON o.id=i.operation_id \
    JOIN public.space_grant g ON g.space_id=o.space_id AND g.actor_id=$1 \
    WHERE i.block_id=$2 AND i.revision_id=$3 \
      AND ($4::uuid IS NULL OR (i.operation_id,i.position)>($4,$5)) \
    ORDER BY i.operation_id,i.position LIMIT $6";

async fn reverse_rows(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    target: &BlockRef,
    after: Option<(Uuid, i32)>,
) -> Vec<(Uuid, i32)> {
    sqlx::query_as(REVERSE_INPUT)
        .bind(actor.actor_id)
        .bind(target.block_id)
        .bind(target.revision_id)
        .bind(after.map(|v| v.0))
        .bind(after.map(|v| v.1))
        .bind(128_i64)
        .fetch_all(&mut **tx)
        .await
        .unwrap()
}

async fn explain_reverse(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    target: &BlockRef,
    after: Option<(Uuid, i32)>,
) -> Value {
    let Json(plan): Json<Value> = sqlx::query_scalar(&format!(
        "EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {REVERSE_INPUT}"
    ))
    .bind(actor.actor_id)
    .bind(target.block_id)
    .bind(target.revision_id)
    .bind(after.map(|v| v.0))
    .bind(after.map(|v| v.1))
    .bind(128_i64)
    .fetch_one(&mut **tx)
    .await
    .unwrap();
    plan
}

#[tokio::test]
#[ignore = "fresh isolated PostgreSQL; emits eight raw EXPLAIN JSON plans"]
async fn explain_dense_lineage_reverse_target_before_and_after_revoke() {
    let rig = TestRig::from_env().await;
    let (actor, visible_space) = rig.seed_actor_space(true).await;
    let (_, revoked_space) = rig.seed_actor_space(true).await;
    rig.grant(actor, revoked_space, true).await;
    let (target_id, target_revision) = rig.seed_block(actor, visible_space).await;
    let target = BlockRef {
        block_id: target_id,
        revision_id: target_revision,
    };
    let (other_id, other_revision) = rig.seed_block(actor, visible_space).await;
    let other = BlockRef {
        block_id: other_id,
        revision_id: other_revision,
    };
    let store = LineageStore::new(rig.runtime_pool.clone());
    for (space, source, count) in [
        (visible_space, target.clone(), 130),
        (revoked_space, target.clone(), 130),
        (visible_space, other, 512),
    ] {
        for _ in 0..count {
            store
                .apply(
                    actor,
                    space,
                    lineage_command(LineageOperation::Derive, vec![source.clone()], 1),
                )
                .await
                .unwrap();
        }
    }
    println!(
        "{}",
        json!({"fixture":"lineage_reverse_target_selectivity","target_operations":260,"unrelated_operations":512})
    );
    for revoked in [false, true] {
        if revoked {
            rig.revoke(actor, revoked_space).await;
        }
        for statement in [
            "ANALYZE public.lineage_input",
            "ANALYZE public.lineage_operation",
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
            let first = reverse_rows(&mut tx, actor, &target, None).await;
            assert_eq!(first.len(), 128);
            let cursor = first.last().copied();
            let second = reverse_rows(&mut tx, actor, &target, cursor).await;
            assert_eq!(second.len(), if revoked { 2 } else { 128 });
            for (page, after) in [(1, None), (2, cursor)] {
                let plan = explain_reverse(&mut tx, actor, &target, after).await;
                println!(
                    "{}",
                    json!({"query":"lineage_reverse_input","revoked":revoked,"mode":mode,"page":page,"plan":plan})
                );
            }
            tx.commit().await.unwrap();
        }
    }
}

#[tokio::test]
async fn hidden_merge_input_severs_lineage_bridge_between_visible_blocks() {
    let rig = TestRig::from_env().await;
    let (actor, visible_space) = rig.seed_actor_space(true).await;
    let (_, hidden_space) = rig.seed_actor_space(true).await;
    rig.grant(actor, hidden_space, false).await;
    let (first_id, first_revision) = rig.seed_block(actor, visible_space).await;
    let (second_id, second_revision) = rig.seed_block(actor, hidden_space).await;
    let first = BlockRef {
        block_id: first_id,
        revision_id: first_revision,
    };
    let second = BlockRef {
        block_id: second_id,
        revision_id: second_revision,
    };
    let saved = LineageStore::new(rig.runtime_pool.clone())
        .apply(
            actor,
            visible_space,
            lineage_command(LineageOperation::Merge, vec![first.clone(), second], 1),
        )
        .await
        .unwrap();
    let release_id = release_pair(
        &rig,
        actor,
        visible_space,
        vec![first.clone(), saved.outputs[0].clone()],
    )
    .await;
    let store = QueryStore::new(rig.runtime_pool.clone());
    let before = store
        .traverse(actor, query(first.clone(), release_id))
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(before.status(), PageStatus::Complete));
    assert!(before.consumers().iter().any(|g| g.consumer
        == ImpactNode::Lineage {
            operation_id: saved.operation_id
        }));
    assert!(
        before
            .consumers()
            .iter()
            .any(|g| g.consumer == ImpactNode::Block(saved.outputs[0].clone()))
    );

    rig.revoke(actor, hidden_space).await;
    let after = store
        .traverse(actor, query(first, release_id))
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(after.status(), PageStatus::Complete));
    assert!(after.consumers().is_empty());
}

#[tokio::test]
async fn lineage_operation_with_no_exact_output_in_fixed_scope_is_absent() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let (block_id, revision_id) = rig.seed_block(actor, space).await;
    let source = BlockRef {
        block_id,
        revision_id,
    };
    let release_id = release_pair(&rig, actor, space, vec![source.clone()]).await;
    // This operation is visible to the actor, but it happened after the
    // release; no exact output revision belongs to the frozen scope.
    let saved = LineageStore::new(rig.runtime_pool.clone())
        .apply(
            actor,
            space,
            lineage_command(LineageOperation::Derive, vec![source.clone()], 1),
        )
        .await
        .unwrap();
    assert!(
        LineageStore::new(rig.runtime_pool.clone())
            .read(actor, saved.operation_id)
            .await
            .unwrap()
            .is_some()
    );
    let result = QueryStore::new(rig.runtime_pool.clone())
        .traverse(actor, query(source, release_id))
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result.status(), PageStatus::Complete));
    assert!(result.consumers().is_empty());
}

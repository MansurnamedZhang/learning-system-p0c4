mod support;
use learning_core::*;
use learning_db::{LineageStore, VersionedContentStore};
use support::TestRig;
use uuid::Uuid;

fn command(operation: LineageOperation, inputs: Vec<BlockRef>, count: usize) -> LineageCommand {
    LineageCommand {
        request_id: Uuid::new_v4(),
        operation,
        inputs,
        outputs: (0..count)
            .map(|i| ContentDraft::V1(support::command(&format!("output {i}")).draft))
            .collect(),
        reason: "explicit transformation".into(),
    }
}

#[tokio::test]
async fn split_is_new_ordered_content_and_leaves_source_unchanged() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let original = rig
        .store
        .create(actor, space, support::command("original"))
        .await
        .unwrap();
    let input = BlockRef {
        block_id: original.block_id,
        revision_id: original.revision_id,
    };
    let store = LineageStore::new(rig.runtime_pool.clone());
    let cmd = command(LineageOperation::Split, vec![input.clone()], 2);
    let saved = store.apply(actor, space, cmd.clone()).await.unwrap();
    assert_eq!(saved.inputs, vec![input]);
    assert_eq!(saved.outputs.len(), 2);
    assert_ne!(saved.outputs[0].block_id, saved.outputs[1].block_id);
    assert!(
        saved
            .outputs
            .iter()
            .all(|r| r.block_id != original.block_id)
    );
    assert_eq!(saved.author_id, actor.actor_id);
    assert_eq!(store.apply(actor, space, cmd.clone()).await.unwrap(), saved);
    assert_eq!(
        store.read(actor, saved.operation_id).await.unwrap(),
        Some(saved.clone())
    );
    let content = VersionedContentStore::new(rig.runtime_pool.clone());
    for (i, reference) in saved.outputs.iter().enumerate() {
        let output = content
            .read(actor, reference.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(output.draft, cmd.outputs[i]);
        assert_eq!(output.parent_revision_id, None);
        assert!(output.draft.dependencies().is_empty());
    }
    assert_eq!(rig.head(original.block_id).await, original.revision_id);
    assert_eq!(
        rig.store.read(actor, original.revision_id).await.unwrap(),
        Some(original)
    );
    let relations: i64 = sqlx::query_scalar("SELECT count(*) FROM relation WHERE space_id=$1")
        .bind(space)
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap();
    assert_eq!(relations, 0);
}

#[test]
fn arities_duplicate_block_identities_and_system_types_are_closed() {
    let a = BlockRef {
        block_id: Uuid::new_v4(),
        revision_id: Uuid::new_v4(),
    };
    let b = BlockRef {
        block_id: Uuid::new_v4(),
        revision_id: Uuid::new_v4(),
    };
    for (op, inputs, outputs) in [
        (LineageOperation::Derive, vec![a.clone()], 1),
        (LineageOperation::Split, vec![a.clone()], 2),
        (LineageOperation::Split, vec![a.clone()], 32),
        (LineageOperation::Merge, vec![a.clone(), b.clone()], 1),
    ] {
        command(op, inputs, outputs).validate().unwrap();
    }
    for (op, inputs, outputs) in [
        (LineageOperation::Derive, vec![], 1),
        (LineageOperation::Derive, vec![a.clone()], 2),
        (LineageOperation::Split, vec![a.clone()], 1),
        (LineageOperation::Split, vec![a.clone()], 33),
        (LineageOperation::Merge, vec![a.clone()], 1),
        (LineageOperation::Merge, vec![a.clone(), b], 2),
        (LineageOperation::Merge, vec![a.clone(), a.clone()], 1),
        (
            LineageOperation::Merge,
            vec![
                a.clone(),
                BlockRef {
                    block_id: a.block_id,
                    revision_id: Uuid::new_v4(),
                },
            ],
            1,
        ),
    ] {
        assert!(matches!(
            command(op, inputs, outputs).validate(),
            Err(ContentError::Invalid(_))
        ));
    }
    for value in ["derived_from", "split_from", "merged_from"] {
        assert!(serde_json::from_value::<RelationType>(serde_json::json!(value)).is_err());
    }
}

#[tokio::test]
async fn hidden_provenance_blocks_lineage_and_replay_but_not_independent_output() {
    let rig = TestRig::from_env().await;
    let (actor, source_space) = rig.seed_actor_space(true).await;
    let (_, output_space) = rig.seed_actor_space(true).await;
    sqlx::query("INSERT INTO space_grant VALUES($1,$2,true)")
        .bind(actor.actor_id)
        .bind(output_space)
        .execute(&rig.admin_pool)
        .await
        .unwrap();
    let (block_id, revision_id) = rig.seed_block(actor, source_space).await;
    let store = LineageStore::new(rig.runtime_pool.clone());
    let cmd = command(
        LineageOperation::Derive,
        vec![BlockRef {
            block_id,
            revision_id,
        }],
        1,
    );
    let saved = store.apply(actor, output_space, cmd.clone()).await.unwrap();
    rig.revoke(actor, source_space).await;
    assert_eq!(store.read(actor, saved.operation_id).await.unwrap(), None);
    assert!(matches!(
        store.apply(actor, output_space, cmd).await,
        Err(ContentError::NotFound)
    ));
    assert!(
        VersionedContentStore::new(rig.runtime_pool.clone())
            .read(actor, saved.outputs[0].clone())
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn request_namespace_and_ordered_digest_are_enforced() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let mut inputs = vec![];
    for _ in 0..2 {
        let (block_id, revision_id) = rig.seed_block(actor, space).await;
        inputs.push(BlockRef {
            block_id,
            revision_id,
        });
    }
    let store = LineageStore::new(rig.runtime_pool.clone());
    let mut cmd = command(LineageOperation::Merge, inputs, 1);
    store.apply(actor, space, cmd.clone()).await.unwrap();
    cmd.inputs.reverse();
    assert!(matches!(
        store.apply(actor, space, cmd.clone()).await,
        Err(ContentError::IdempotencyConflict)
    ));
    let mut content_cmd = support::command("collision");
    content_cmd.request_id = cmd.request_id;
    assert!(matches!(
        rig.store.create(actor, space, content_cmd).await,
        Err(ContentError::IdempotencyConflict)
    ));
    let content_cmd = support::command("existing request");
    cmd.request_id = content_cmd.request_id;
    rig.store.create(actor, space, content_cmd).await.unwrap();
    assert!(matches!(
        store.apply(actor, space, cmd).await,
        Err(ContentError::IdempotencyConflict)
    ));
}

fn with_basis(basis: Vec<BlockRef>) -> ContentDraft {
    ContentDraft::V2(ContentV2 {
        intent: Intent::Note,
        language: "en".into(),
        title: "declared basis".into(),
        body: BodyV2::Text(support::command("conclusion").draft.payload),
        basis_refs: basis.into_iter().map(ExactRef::Block).collect(),
        requires_context: vec![],
        source_run: None,
    })
}
async fn counts(rig: &TestRig, actor: Principal) -> Vec<i64> {
    let mut result = vec![];
    for q in [
        "SELECT count(*) FROM block WHERE space_id IN (SELECT id FROM space WHERE owner_id=$1)",
        "SELECT count(*) FROM block_revision WHERE author_id=$1",
        "SELECT count(*) FROM reference_object WHERE space_id IN (SELECT id FROM space WHERE owner_id=$1)",
        "SELECT count(*) FROM reference_dependency d JOIN reference_object o ON o.kind=d.source_kind AND o.object_id=d.source_object_id AND o.revision_id=d.source_revision_id WHERE o.space_id IN (SELECT id FROM space WHERE owner_id=$1)",
        "SELECT count(*) FROM lineage_operation WHERE author_id=$1",
        "SELECT count(*) FROM lineage_input WHERE operation_id IN (SELECT id FROM lineage_operation WHERE author_id=$1)",
        "SELECT count(*) FROM lineage_output WHERE operation_id IN (SELECT id FROM lineage_operation WHERE author_id=$1)",
        "SELECT count(*) FROM lineage_receipt WHERE actor_id=$1",
        "SELECT count(*) FROM request_key WHERE actor_id=$1",
        "SELECT count(*) FROM mutation_receipt WHERE actor_id=$1",
    ] {
        result.push(
            sqlx::query_scalar(q)
                .bind(actor.actor_id)
                .fetch_one(&rig.admin_pool)
                .await
                .unwrap(),
        );
    }
    result
}

#[tokio::test]
async fn second_output_lineage_and_receipt_faults_rollback_every_table() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let (block_id, revision_id) = rig.seed_block(actor, space).await;
    let input = BlockRef {
        block_id,
        revision_id,
    };
    let store = LineageStore::new(rig.runtime_pool.clone());
    for table in ["block_revision", "lineage_output", "lineage_receipt"] {
        let function = format!("fail_lineage_{}", Uuid::new_v4().simple());
        let witness = format!("{function}_hit");
        // Sequence increments survive transaction rollback. This proves the
        // intended injection fired rather than an earlier unrelated SQL error.
        sqlx::query(&format!("CREATE SEQUENCE public.{witness}"))
            .execute(&rig.admin_pool)
            .await
            .unwrap();
        sqlx::query(&format!(
            "GRANT USAGE ON SEQUENCE public.{witness} TO learning_runtime"
        ))
        .execute(&rig.admin_pool)
        .await
        .unwrap();
        let condition = match table {
            "block_revision" => format!(
                "NEW.author_id='{}' AND NEW.reason='explicit transformation' AND (SELECT count(*) FROM block_revision WHERE author_id='{}' AND reason='explicit transformation')=1",
                actor.actor_id, actor.actor_id
            ),
            "lineage_output" => format!(
                "NEW.position=1 AND EXISTS(SELECT 1 FROM lineage_operation WHERE id=NEW.operation_id AND author_id='{}')",
                actor.actor_id
            ),
            _ => format!("NEW.actor_id='{}'", actor.actor_id),
        };
        sqlx::query(&format!("CREATE FUNCTION {function}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF {condition} THEN PERFORM nextval('public.{witness}'); RAISE EXCEPTION 'injected'; END IF; RETURN NEW; END $$")).execute(&rig.admin_pool).await.unwrap();
        sqlx::query(&format!("CREATE TRIGGER {function} BEFORE INSERT ON {table} FOR EACH ROW EXECUTE FUNCTION {function}()" )).execute(&rig.admin_pool).await.unwrap();
        let before = counts(&rig, actor).await;
        let mut cmd = command(LineageOperation::Split, vec![input.clone()], 2);
        cmd.outputs = vec![with_basis(vec![input.clone()]); 2];
        let result = store.apply(actor, space, cmd).await;
        let injected: bool = sqlx::query_scalar(&format!("SELECT is_called FROM public.{witness}"))
            .fetch_one(&rig.admin_pool)
            .await
            .unwrap();
        sqlx::query(&format!("DROP TRIGGER {function} ON {table}"))
            .execute(&rig.admin_pool)
            .await
            .unwrap();
        sqlx::query(&format!("DROP FUNCTION {function}()"))
            .execute(&rig.admin_pool)
            .await
            .unwrap();
        sqlx::query(&format!("DROP SEQUENCE public.{witness}"))
            .execute(&rig.admin_pool)
            .await
            .unwrap();
        assert!(
            injected,
            "{table}: injection was never reached; actual result {result:?}"
        );
        assert!(
            matches!(result, Err(ContentError::Storage)),
            "{table}: {result:?}"
        );
        assert_eq!(counts(&rig, actor).await, before, "{table}");
        assert_eq!(rig.head(block_id).await, revision_id);
    }
}

#[tokio::test]
async fn source_positions_reading_heads_and_old_body_stay_fixed() {
    use support::reading as h;
    let (rig, actor, space, doc, initial) = h::fixture().await;
    let reader = h::store(&rig);
    let first = reader
        .edit(
            actor,
            initial.overlay.overlay_id,
            h::edit(&initial, h::add(h::gap(&doc, 1), "personal source")),
        )
        .await
        .unwrap();
    let source = first.changed_blocks[0].clone();
    let second = reader
        .edit(
            actor,
            first.overlay.overlay_id,
            h::edit(
                &first,
                ReadingEdit::InsertExisting {
                    block: source.clone(),
                    target: InsertTarget::NewGroup {
                        anchor: h::gap(&doc, 2),
                    },
                },
            ),
        )
        .await
        .unwrap();
    let state = reader
        .state(actor, second.overlay.overlay_id)
        .await
        .unwrap()
        .unwrap();
    let positions: Vec<_> = state
        .editable
        .as_ref()
        .unwrap()
        .groups
        .iter()
        .flat_map(|g| &g.placements)
        .collect();
    assert_eq!(positions.len(), 2);
    assert!(positions.iter().all(|p| p.block == source));
    let before = serde_json::to_value(state).unwrap();
    let body = rig
        .store
        .read(actor, source.revision_id)
        .await
        .unwrap()
        .unwrap();
    let structures = rig.structure_counts(space).await;
    let store = LineageStore::new(rig.runtime_pool.clone());
    let split = store
        .apply(
            actor,
            space,
            command(LineageOperation::Split, vec![source.clone()], 2),
        )
        .await
        .unwrap();
    assert!(split.outputs.iter().all(|r| r.block_id != source.block_id));
    assert_eq!(
        serde_json::to_value(
            reader
                .state(actor, second.overlay.overlay_id)
                .await
                .unwrap()
                .unwrap()
        )
        .unwrap(),
        before
    );
    assert_eq!(rig.structure_counts(space).await, structures);
    assert_eq!(
        rig.store
            .read(actor, source.revision_id)
            .await
            .unwrap()
            .unwrap(),
        body
    );
    assert_eq!(rig.head(source.block_id).await, source.revision_id);
}

#[tokio::test]
async fn declared_output_dependencies_require_access_and_output_space_requires_write() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let (_, dependency_space) = rig.seed_actor_space(true).await;
    rig.grant(actor, dependency_space, true).await;
    let (block_id, revision_id) = rig.seed_block(actor, space).await;
    let input = BlockRef {
        block_id,
        revision_id,
    };
    let (block_id, revision_id) = rig.seed_block(actor, dependency_space).await;
    let dependency = BlockRef {
        block_id,
        revision_id,
    };
    let store = LineageStore::new(rig.runtime_pool.clone());
    let mut cmd = command(LineageOperation::Derive, vec![input], 1);
    cmd.outputs = vec![with_basis(vec![dependency])];
    let saved = store.apply(actor, space, cmd.clone()).await.unwrap();
    rig.grant(actor, space, false).await;
    assert!(matches!(
        store.apply(actor, space, cmd.clone()).await,
        Err(ContentError::NotFound)
    ));
    assert!(
        store
            .read(actor, saved.operation_id)
            .await
            .unwrap()
            .is_some()
    );
    rig.grant(actor, space, true).await;
    rig.revoke(actor, dependency_space).await;
    assert_eq!(store.read(actor, saved.operation_id).await.unwrap(), None);
    let content = VersionedContentStore::new(rig.runtime_pool.clone());
    assert_eq!(
        content.read(actor, saved.outputs[0].clone()).await.unwrap(),
        None
    );
    assert!(matches!(
        store.apply(actor, space, cmd.clone()).await,
        Err(ContentError::NotFound)
    ));
    cmd.request_id = Uuid::new_v4();
    let before = counts(&rig, actor).await;
    assert!(matches!(
        store.apply(actor, space, cmd).await,
        Err(ContentError::NotFound)
    ));
    assert_eq!(counts(&rig, actor).await, before);
    assert_eq!(store.read(actor, Uuid::new_v4()).await.unwrap(), None);
}

#[tokio::test]
async fn concurrent_request_replay_creates_one_operation_and_ordered_output_set() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let (block_id, revision_id) = rig.seed_block(actor, space).await;
    let store = LineageStore::new(rig.runtime_pool.clone());
    let cmd = command(
        LineageOperation::Split,
        vec![BlockRef {
            block_id,
            revision_id,
        }],
        2,
    );
    let (a, b) = tokio::join!(
        store.apply(actor, space, cmd.clone()),
        store.apply(actor, space, cmd)
    );
    assert_eq!(a.unwrap(), b.unwrap());
    assert_eq!(
        counts(&rig, actor).await,
        vec![3, 3, 3, 0, 1, 1, 2, 1, 1, 0]
    );
}

#[tokio::test]
async fn one_merged_payload_budget_includes_all_new_output_bodies() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let text = "a".repeat(199_000);
    let mut inputs = vec![];
    let mut tx = rig.admin_pool.begin().await.unwrap();
    for _ in 0..20 {
        inputs.push(
            support::references::seed_content(
                &mut tx,
                actor,
                space,
                ContentDraft::V1(support::command(&text).draft),
                None,
            )
            .await,
        );
    }
    let root =
        support::references::seed_content(&mut tx, actor, space, with_basis(inputs.clone()), None)
            .await;
    tx.commit().await.unwrap();
    let store = LineageStore::new(rig.runtime_pool.clone());
    let mut cmd = command(LineageOperation::Split, vec![root.clone()], 32);
    cmd.outputs = vec![ContentDraft::V1(support::command(&text).draft); 32];
    let before = counts(&rig, actor).await;
    let result = store.apply(actor, space, cmd).await;
    assert!(
        matches!(&result, Err(ContentError::Invalid(s)) if s=="reference_budget_exceeded"),
        "{result:?}"
    );
    assert_eq!(counts(&rig, actor).await, before);
    // The shared 4 MB dependency closure is counted once, not once per output.
    let mut shared = command(LineageOperation::Split, vec![root], 32);
    shared.outputs = vec![with_basis(inputs); 32];
    let saved = store.apply(actor, space, shared).await.unwrap();
    assert_eq!(saved.outputs.len(), 32);
    assert!(
        store
            .read(actor, saved.operation_id)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn disjoint_output_dependencies_and_combined_edges_cannot_reset_budgets() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let (block_id, revision_id) = rig.seed_block(actor, space).await;
    let input = BlockRef {
        block_id,
        revision_id,
    };
    let mut dependencies = vec![];
    let mut tx = rig.admin_pool.begin().await.unwrap();
    for i in 0..256 {
        let body = if i < 44 {
            "a".repeat(199_000)
        } else {
            "small".into()
        };
        dependencies.push(
            support::references::seed_content(
                &mut tx,
                actor,
                space,
                ContentDraft::V1(support::command(&body).draft),
                None,
            )
            .await,
        );
    }
    tx.commit().await.unwrap();
    let store = LineageStore::new(rig.runtime_pool.clone());
    let mut cmd = command(LineageOperation::Split, vec![input.clone()], 2);
    cmd.outputs = vec![
        with_basis(dependencies[..22].to_vec()),
        with_basis(dependencies[22..44].to_vec()),
    ];
    let before = counts(&rig, actor).await;
    let result = store.apply(actor, space, cmd).await;
    assert!(
        matches!(&result, Err(ContentError::Invalid(s)) if s=="reference_budget_exceeded"),
        "{result:?}"
    );
    assert_eq!(counts(&rig, actor).await, before);
    // 20 outputs * 212 small dependencies = 4240 edges, individually valid.
    let mut edges = command(LineageOperation::Split, vec![input], 20);
    edges.outputs = vec![with_basis(dependencies[44..].to_vec()); 20];
    let result = store.apply(actor, space, edges).await;
    assert!(
        matches!(&result, Err(ContentError::Invalid(s)) if s=="reference_budget_exceeded"),
        "{result:?}"
    );
    assert_eq!(counts(&rig, actor).await, before);
}

#[tokio::test]
async fn system_projection_is_owned_by_operations_and_runtime_history_is_immutable() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let mut inputs = vec![];
    for _ in 0..2 {
        let (block_id, revision_id) = rig.seed_block(actor, space).await;
        inputs.push(BlockRef {
            block_id,
            revision_id,
        });
    }
    let store = LineageStore::new(rig.runtime_pool.clone());
    for (op, kind, sources, n) in [
        (
            LineageOperation::Derive,
            "derived_from",
            inputs[..1].to_vec(),
            1,
        ),
        (
            LineageOperation::Split,
            "split_from",
            inputs[..1].to_vec(),
            2,
        ),
        (LineageOperation::Merge, "merged_from", inputs.clone(), 1),
    ] {
        let saved = store
            .apply(actor, space, command(op, sources.clone(), n))
            .await
            .unwrap();
        let rows: Vec<(String, Uuid, Uuid, Uuid, Uuid)> = sqlx::query_as("SELECT type,from_block_id,from_revision_id,to_block_id,to_revision_id FROM system_lineage WHERE operation_id=$1 ORDER BY output_position,input_position").bind(saved.operation_id).fetch_all(&rig.runtime_pool).await.unwrap();
        assert_eq!(rows.len(), n * sources.len());
        for (index, row) in rows.iter().enumerate() {
            let output = &saved.outputs[index / sources.len()];
            let input = &sources[index % sources.len()];
            assert_eq!(
                row,
                &(
                    kind.into(),
                    output.block_id,
                    output.revision_id,
                    input.block_id,
                    input.revision_id
                )
            );
        }
        let projected = saved.system_relations();
        assert_eq!(projected.len(), rows.len());
        assert_eq!(projected[0].operation_id, saved.operation_id);
        assert_eq!(
            serde_json::to_value(projected[0].relation_type).unwrap(),
            serde_json::json!(kind)
        );
        for query in [
            "UPDATE lineage_operation SET reason='tamper' WHERE id=$1",
            "DELETE FROM lineage_operation WHERE id=$1",
            "UPDATE lineage_input SET position=31 WHERE operation_id=$1",
            "DELETE FROM lineage_input WHERE operation_id=$1",
            "UPDATE lineage_output SET position=31 WHERE operation_id=$1",
            "DELETE FROM lineage_output WHERE operation_id=$1",
            "UPDATE lineage_receipt SET request_sha256=repeat('b',64) WHERE operation_id=$1",
            "DELETE FROM lineage_receipt WHERE operation_id=$1",
        ] {
            let error = sqlx::query(query)
                .bind(saved.operation_id)
                .execute(&rig.runtime_pool)
                .await
                .unwrap_err();
            assert_eq!(
                support::sqlstate(&error).as_deref(),
                Some("42501"),
                "{query}"
            );
        }
        let editable: bool = sqlx::query_scalar(
            "SELECT has_table_privilege(current_user,'system_lineage','INSERT,UPDATE,DELETE')",
        )
        .fetch_one(&rig.runtime_pool)
        .await
        .unwrap();
        assert!(!editable);
        assert!(
            sqlx::query("DELETE FROM system_lineage WHERE operation_id=$1")
                .bind(saved.operation_id)
                .execute(&rig.runtime_pool)
                .await
                .is_err()
        );
        // Even INSERT cannot extend an already committed operation.
        let error = sqlx::query("INSERT INTO lineage_input(operation_id,position,space_id,block_id,revision_id) VALUES($1,31,$2,$3,$4)")
            .bind(saved.operation_id).bind(space).bind(inputs[1].block_id).bind(inputs[1].revision_id).execute(&rig.runtime_pool).await.unwrap_err();
        assert_eq!(support::sqlstate(&error).as_deref(), Some("23514"));
    }
    let relation_count: i64 = sqlx::query_scalar("SELECT count(*) FROM relation WHERE space_id=$1")
        .bind(space)
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap();
    assert_eq!(relation_count, 0);
}

async fn raw_operation(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: Principal,
    space: Uuid,
) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO lineage_operation(id,space_id,operation,author_id,reason,input_count,output_count) VALUES($1,$2,'derive',$3,'fixture',1,1)")
        .bind(id).bind(space).bind(actor.actor_id).execute(&mut **tx).await.unwrap();
    id
}

#[tokio::test]
async fn database_rejects_orphans_existing_outputs_wrong_ownership_and_bad_receipts() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let (other, other_space) = rig.seed_actor_space(true).await;
    let (block_id, revision_id) = rig.seed_block(actor, space).await;
    let input = BlockRef {
        block_id,
        revision_id,
    };
    let before = counts(&rig, actor).await;
    let mut tx = rig.runtime_pool.begin().await.unwrap();
    raw_operation(&mut tx, actor, space).await;
    assert_eq!(
        support::sqlstate(&tx.commit().await.unwrap_err()).as_deref(),
        Some("23514")
    );
    for variant in [
        "old_output",
        "wrong_author",
        "wrong_space",
        "bad_revision",
        "reused_identity_new_root",
    ] {
        let mut tx = rig.runtime_pool.begin().await.unwrap();
        let op = raw_operation(&mut tx, actor, space).await;
        let output_space = if variant == "wrong_space" {
            other_space
        } else {
            space
        };
        let output_actor = if variant == "wrong_author" {
            other
        } else {
            actor
        };
        let output = if variant == "reused_identity_new_root" {
            let new_revision = Uuid::new_v4();
            sqlx::query("INSERT INTO block_revision(id,space_id,block_id,content,content_sha256,author_id,reason) SELECT $1,space_id,block_id,content,content_sha256,author_id,'fixture' FROM block_revision WHERE id=$2").bind(new_revision).bind(input.revision_id).execute(&mut *tx).await.unwrap();
            sqlx::query("UPDATE block SET head_revision_id=$1 WHERE id=$2")
                .bind(new_revision)
                .bind(input.block_id)
                .execute(&mut *tx)
                .await
                .unwrap();
            BlockRef {
                block_id: input.block_id,
                revision_id: new_revision,
            }
        } else if variant == "old_output" {
            input.clone()
        } else {
            support::references::seed_content(
                &mut tx,
                output_actor,
                output_space,
                ContentDraft::V1(support::command("output").draft),
                None,
            )
            .await
        };
        let revision = if variant == "bad_revision" {
            input.revision_id
        } else {
            output.revision_id
        };
        let error = sqlx::query("INSERT INTO lineage_output(operation_id,position,space_id,block_id,revision_id) VALUES($1,0,$2,$3,$4)")
            .bind(op).bind(output_space).bind(output.block_id).bind(revision).execute(&mut *tx).await.unwrap_err();
        assert!(
            matches!(
                support::sqlstate(&error).as_deref(),
                Some("23514" | "23503")
            ),
            "{variant}: {error}"
        );
        tx.rollback().await.unwrap();
        assert_eq!(counts(&rig, actor).await, before);
    }
    for variant in [
        "missing_receipt",
        "wrong_family",
        "wrong_hash",
        "wrong_position",
    ] {
        let mut tx = rig.runtime_pool.begin().await.unwrap();
        let op = raw_operation(&mut tx, actor, space).await;
        sqlx::query("INSERT INTO lineage_input(operation_id,position,space_id,block_id,revision_id) VALUES($1,0,$2,$3,$4)").bind(op).bind(space).bind(input.block_id).bind(input.revision_id).execute(&mut *tx).await.unwrap();
        let output = support::references::seed_content(
            &mut tx,
            actor,
            space,
            ContentDraft::V1(support::command("output").draft),
            None,
        )
        .await;
        sqlx::query("INSERT INTO lineage_output(operation_id,position,space_id,block_id,revision_id) VALUES($1,$2,$3,$4,$5)").bind(op).bind(if variant=="wrong_position" { 1 } else { 0 }).bind(space).bind(output.block_id).bind(output.revision_id).execute(&mut *tx).await.unwrap();
        if variant != "missing_receipt" {
            let request_id = Uuid::new_v4();
            let family = if variant == "wrong_family" {
                "content_v1"
            } else {
                "lineage_apply"
            };
            sqlx::query("INSERT INTO request_key VALUES($1,$2,repeat('a',64),$3)")
                .bind(actor.actor_id)
                .bind(request_id)
                .bind(family)
                .execute(&mut *tx)
                .await
                .unwrap();
            sqlx::query("INSERT INTO lineage_receipt(actor_id,request_id,operation_id,request_sha256) VALUES($1,$2,$3,$4)").bind(actor.actor_id).bind(request_id).bind(op).bind(if variant=="wrong_hash" { "b".repeat(64) } else { "a".repeat(64) }).execute(&mut *tx).await.unwrap();
        }
        assert_eq!(
            support::sqlstate(&tx.commit().await.unwrap_err()).as_deref(),
            Some("23514"),
            "{variant}"
        );
        assert_eq!(counts(&rig, actor).await, before);
    }
}

#[tokio::test]
async fn revocation_committed_while_waiting_on_grant_prevents_new_outputs() {
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration;
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let (block_id, revision_id) = rig.seed_block(actor, space).await;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let waiter: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&pool)
        .await
        .unwrap();
    let store = LineageStore::new(pool.clone());
    let mut lock = rig.admin_pool.begin().await.unwrap();
    let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *lock)
        .await
        .unwrap();
    sqlx::query("SELECT 1 FROM space_grant WHERE actor_id=$1 AND space_id=$2 FOR UPDATE")
        .bind(actor.actor_id)
        .bind(space)
        .execute(&mut *lock)
        .await
        .unwrap();
    let before = counts(&rig, actor).await;
    let (result, ()) = tokio::join!(
        store.apply(
            actor,
            space,
            command(
                LineageOperation::Derive,
                vec![BlockRef {
                    block_id,
                    revision_id
                }],
                1
            )
        ),
        async {
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let blocked: bool = sqlx::query_scalar("SELECT $1=ANY(pg_blocking_pids($2))")
                        .bind(blocker)
                        .bind(waiter)
                        .fetch_one(&rig.admin_pool)
                        .await
                        .unwrap();
                    if blocked {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("lineage grant lock wait not observed");
            sqlx::query("DELETE FROM space_grant WHERE actor_id=$1 AND space_id=$2")
                .bind(actor.actor_id)
                .bind(space)
                .execute(&mut *lock)
                .await
                .unwrap();
            lock.commit().await.unwrap();
        }
    );
    assert!(matches!(result, Err(ContentError::NotFound)));
    assert_eq!(counts(&rig, actor).await, before);
    pool.close().await;
}

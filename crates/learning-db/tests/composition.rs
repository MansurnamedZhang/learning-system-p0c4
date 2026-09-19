mod support;
use learning_core::*;
use support::{assembly::*, *};
use uuid::Uuid;

#[tokio::test]
async fn cross_pairing_two_existing_objects_never_resolves_by_revision_alone() {
    let rig = TestRig::from_env().await;
    let (a, s) = rig.seed_actor_space(true).await;
    let (o, d) = rig.seed_actor_space(true).await;
    rig.grant(a, d, false).await;
    let k = rig.store.create(a, s, command("a")).await.unwrap();
    let foreign = rig.store.create(o, d, command("b")).await.unwrap();
    let cs = rig.compositions();
    let x = cs.save(a, s, doc(vec![])).await.unwrap();
    let y = cs.save(o, d, doc(vec![])).await.unwrap();
    for target in [
        NodeTarget::Block(BlockRef {
            block_id: k.block_id,
            revision_id: foreign.revision_id,
        }),
        NodeTarget::Composition(CompositionRef {
            composition_id: x.reference.composition_id,
            revision_id: y.reference.revision_id,
        }),
    ] {
        assert!(matches!(
            cs.save(a, s, doc(vec![target])).await,
            Err(ContentError::NotFound)
        ));
    }
    assert!(
        cs.read(
            a,
            CompositionRef {
                composition_id: x.reference.composition_id,
                revision_id: y.reference.revision_id
            }
        )
        .await
        .unwrap()
        .is_none()
    );
}

#[tokio::test]
async fn distinct_object_budget_accepts_2048_and_rejects_2049() {
    let rig = TestRig::from_env().await;
    let (a, s) = rig.seed_actor_space(true).await;
    let cs = rig.compositions();
    let mut tx = rig.admin_pool.begin().await.unwrap();
    let rows:Vec<(Uuid,Uuid)>=sqlx::query_as("WITH source AS MATERIALIZED (SELECT gen_random_uuid() id,gen_random_uuid() revision_id FROM generate_series(1,2044)), inserted AS (INSERT INTO block(id,space_id,head_revision_id) SELECT id,$1,revision_id FROM source RETURNING id) INSERT INTO block_revision(id,space_id,block_id,content,content_sha256,author_id,reason) SELECT source.revision_id,$1,source.id,$2,$3,$4,'fixture' FROM source JOIN inserted USING(id) RETURNING block_id,id").bind(s).bind(sqlx::types::Json(command("").draft)).bind(command("").draft.digest()).bind(a.actor_id).fetch_all(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let targets: Vec<_> = rows
        .into_iter()
        .map(|(block_id, revision_id)| {
            NodeTarget::Block(BlockRef {
                block_id,
                revision_id,
            })
        })
        .collect();
    let mut children = vec![];
    for chunk in targets[..2043].chunks(512) {
        children.push(child(&cs.save(a, s, doc(chunk.to_vec())).await.unwrap()));
    }
    let allowed = cs.save(a, s, doc(children.clone())).await.unwrap();
    assert_eq!(
        cs.read(a, allowed.reference)
            .await
            .unwrap()
            .unwrap()
            .blocks
            .len(),
        2043
    );
    children.push(targets[2043].clone());
    assert!(
        matches!(cs.save(a,s,doc(children)).await,Err(ContentError::Invalid(code)) if code=="composition_objects_limit")
    );
}

#[tokio::test]
async fn body_budget_is_checked_before_loading_beyond_eight_mib() {
    let rig = TestRig::from_env().await;
    let (a, s) = rig.seed_actor_space(true).await;
    let cs = rig.compositions();
    let overhead: i32 = sqlx::query_scalar("SELECT octet_length($1::jsonb::text)")
        .bind(sqlx::types::Json(command("").draft))
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap();
    let mut targets = vec![];
    for _ in 0..41 {
        let r = rig
            .store
            .create(a, s, command(&"x".repeat(200000)))
            .await
            .unwrap();
        targets.push(block(&r));
    }
    let remaining = 8 * 1024 * 1024 - 41 * (200000 + overhead as usize) - overhead as usize;
    assert!(remaining < 200000);
    let last = rig
        .store
        .create(a, s, command(&"x".repeat(remaining)))
        .await
        .unwrap();
    targets.push(block(&last));
    let allowed = cs.save(a, s, doc(targets.clone())).await.unwrap();
    assert_eq!(
        cs.read(a, allowed.reference)
            .await
            .unwrap()
            .unwrap()
            .blocks
            .len(),
        42
    );
    let extra = rig
        .store
        .revise(a, last.block_id, change(&last, &"x".repeat(remaining + 1)))
        .await
        .unwrap();
    targets[41] = block(&extra);
    assert!(
        matches!(cs.save(a,s,doc(targets)).await,Err(ContentError::Invalid(code)) if code=="composition_body_limit")
    );
}

#[tokio::test]
async fn composition_receipt_failure_leaves_no_orphans_or_advanced_head() {
    let rig = TestRig::from_env().await;
    let (a, s) = rig.seed_actor_space(true).await;
    let cs = rig.compositions();
    let k = rig.store.create(a, s, command("note")).await.unwrap();
    let original = cs.save(a, s, doc(vec![block(&k)])).await.unwrap();
    for cmd in [doc(vec![block(&k)]), edit(&original)] {
        let before = rig.assembly_counts(a).await;
        let structure_before = rig.structure_counts(s).await;
        let suffix = a.actor_id.simple();
        sqlx::raw_sql(&format!("CREATE FUNCTION fail_comp_{suffix}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected'; END; $$; CREATE TRIGGER fail_{suffix} BEFORE INSERT ON composition_receipt FOR EACH ROW WHEN (NEW.actor_id='{}'::uuid) EXECUTE FUNCTION fail_comp_{suffix}()",a.actor_id)).execute(&rig.admin_pool).await.unwrap();
        let result = cs.save(a, s, cmd.clone()).await;
        sqlx::raw_sql(&format!(
            "DROP TRIGGER fail_{suffix} ON composition_receipt; DROP FUNCTION fail_comp_{suffix}()"
        ))
        .execute(&rig.admin_pool)
        .await
        .unwrap();
        assert!(matches!(result, Err(ContentError::Storage)));
        assert_eq!(rig.assembly_counts(a).await, before);
        assert_eq!(rig.structure_counts(s).await, structure_before);
        let head: Uuid = sqlx::query_scalar("SELECT head_revision_id FROM composition WHERE id=$1")
            .bind(original.reference.composition_id)
            .fetch_one(&rig.admin_pool)
            .await
            .unwrap();
        assert_eq!(head, original.reference.revision_id);
        let saved = cs.save(a, s, cmd.clone()).await.unwrap();
        assert_eq!(cs.save(a, s, cmd).await.unwrap(), saved);
    }
}

#[tokio::test]
async fn shared_blocks_and_historical_compositions_do_not_follow_current_heads() {
    let rig = TestRig::from_env().await;
    let (a, s) = rig.seed_actor_space(true).await;
    let store = rig.compositions();
    let k = rig.store.create(a, s, command("original")).await.unwrap();
    let request = doc(vec![block(&k), block(&k)]);
    let x = store.save(a, s, request.clone()).await.unwrap();
    let y = store.save(a, s, doc(vec![block(&k)])).await.unwrap();
    assert_ne!(x.nodes[0].occurrence_id, x.nodes[1].occurrence_id);
    assert_ne!(x.nodes[0].occurrence_id, y.nodes[0].occurrence_id);
    let newer = rig
        .store
        .revise(a, k.block_id, change(&k, "new"))
        .await
        .unwrap();
    for r in [&x, &y] {
        let snap = store.read(a, r.reference.clone()).await.unwrap().unwrap();
        assert_eq!(snap.blocks, vec![k.clone()]);
    }
    let mut c = edit(&x);
    c.nodes[0].target = block(&newer);
    let v2 = store.save(a, s, c).await.unwrap();
    assert_eq!(v2.nodes[0].occurrence_id, x.nodes[0].occurrence_id);
    assert_eq!(store.save(a, s, request).await.unwrap(), x);
    assert_eq!(
        store.read(a, y.reference).await.unwrap().unwrap().blocks,
        vec![k]
    );
}
#[tokio::test]
async fn occurrence_reorder_copy_remove_and_wrong_identity_are_distinct() {
    let rig = TestRig::from_env().await;
    let (a, s) = rig.seed_actor_space(true).await;
    let store = rig.compositions();
    let k = rig.store.create(a, s, command("a")).await.unwrap();
    let other = rig.store.create(a, s, command("b")).await.unwrap();
    let one = store
        .save(a, s, doc(vec![block(&k), block(&other)]))
        .await
        .unwrap();
    let mut c = edit(&one);
    c.nodes.reverse();
    let two = store.save(a, s, c).await.unwrap();
    assert_eq!(two.nodes[0], one.nodes[1]);
    let mut wrong = edit(&two);
    wrong.nodes[0].target = block(&k);
    assert!(matches!(
        store.save(a, s, wrong).await,
        Err(ContentError::Invalid(_))
    ));
    let mut copy = edit(&two);
    copy.nodes.push(NodeDraft {
        occurrence_id: None,
        target: block(&k),
    });
    let three = store.save(a, s, copy).await.unwrap();
    assert_ne!(three.nodes[2].occurrence_id, three.nodes[1].occurrence_id);
    let mut remove = edit(&three);
    remove.nodes.remove(1);
    let four = store.save(a, s, remove).await.unwrap();
    assert_eq!(four.nodes.len(), 2);
    let mut revive = edit(&four);
    revive.nodes.push(NodeDraft {
        occurrence_id: Some(three.nodes[1].occurrence_id),
        target: block(&k),
    });
    assert!(matches!(
        store.save(a, s, revive).await,
        Err(ContentError::Invalid(_))
    ));
    assert_eq!(rig.store.read(a, k.revision_id).await.unwrap().unwrap(), k);
}
#[tokio::test]
async fn exact_revision_ownership_kind_and_stale_base_are_checked() {
    let rig = TestRig::from_env().await;
    let (a, s) = rig.seed_actor_space(true).await;
    let store = rig.compositions();
    let k = rig.store.create(a, s, command("a")).await.unwrap();
    let wrong = NodeTarget::Block(BlockRef {
        block_id: Uuid::new_v4(),
        revision_id: k.revision_id,
    });
    assert!(matches!(
        store.save(a, s, doc(vec![wrong])).await,
        Err(ContentError::NotFound)
    ));
    let first = store.save(a, s, doc(vec![])).await.unwrap();
    let mut bad = edit(&first);
    bad.kind = CompositionKind::Section;
    assert!(matches!(
        store.save(a, s, bad).await,
        Err(ContentError::Invalid(_))
    ));
    let stale = edit(&first);
    let second = store.save(a, s, edit(&first)).await.unwrap();
    assert!(
        matches!(store.save(a,s,stale).await,Err(ContentError::Conflict{current_revision_id}) if current_revision_id==second.reference.revision_id)
    );
    let mut wrong_ref = first.reference;
    wrong_ref.composition_id = Uuid::new_v4();
    assert!(store.read(a, wrong_ref).await.unwrap().is_none());
    let (_, other_space) = rig.seed_actor_space(true).await;
    rig.grant(a, other_space, true).await;
    assert!(matches!(
        store.save(a, other_space, edit(&second)).await,
        Err(ContentError::NotFound)
    ));
}
#[tokio::test]
async fn fixed_nested_dag_allows_shared_children_but_rejects_identity_cycles() {
    let rig = TestRig::from_env().await;
    let (a, s) = rig.seed_actor_space(true).await;
    let store = rig.compositions();
    let b = store.save(a, s, doc(vec![])).await.unwrap();
    let c = store.save(a, s, doc(vec![child(&b)])).await.unwrap();
    let root = store
        .save(a, s, doc(vec![child(&b), child(&c), child(&b)]))
        .await
        .unwrap();
    let snap = store.read(a, root.reference).await.unwrap().unwrap();
    assert_eq!(snap.compositions.len(), 3);
    let mut back = edit(&b);
    back.nodes = vec![NodeDraft {
        occurrence_id: None,
        target: child(&c),
    }];
    assert!(
        matches!(store.save(a,s,back).await,Err(ContentError::Invalid(code)) if code=="composition_cycle")
    );
    let mut self_ref = edit(&b);
    self_ref.nodes = vec![NodeDraft {
        occurrence_id: None,
        target: child(&b),
    }];
    assert!(matches!(
        store.save(a, s, self_ref).await,
        Err(ContentError::Invalid(_))
    ));
}
#[tokio::test]
async fn depth_and_expanded_occurrence_budgets_count_paths_not_only_objects() {
    let rig = TestRig::from_env().await;
    let (a, s) = rig.seed_actor_space(true).await;
    let store = rig.compositions();
    let mut current = store.save(a, s, doc(vec![])).await.unwrap();
    for _ in 1..16 {
        current = store.save(a, s, doc(vec![child(&current)])).await.unwrap();
    }
    assert!(
        store
            .read(a, current.reference.clone())
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        matches!(store.save(a,s,doc(vec![child(&current)])).await,Err(ContentError::Invalid(code)) if code=="composition_depth_limit")
    );
    let k = rig.store.create(a, s, command("budget")).await.unwrap();
    let leaf = store.save(a, s, doc(vec![block(&k); 511])).await.unwrap();
    let allowed = store.save(a, s, doc(vec![child(&leaf); 8])).await.unwrap();
    assert!(store.read(a, allowed.reference).await.unwrap().is_some());
    let mut too_many = vec![child(&leaf); 8];
    too_many.push(block(&k));
    assert!(
        matches!(store.save(a,s,doc(too_many)).await,Err(ContentError::Invalid(code)) if code=="composition_occurrences_limit")
    );
}
#[tokio::test]
async fn invalid_direct_commands_and_duplicate_keys_leave_no_rows() {
    let rig = TestRig::from_env().await;
    let (a, s) = rig.seed_actor_space(true).await;
    let store = rig.compositions();
    let mut invalid = doc(vec![]);
    invalid.title = "x".repeat(301);
    assert!(matches!(
        store.save(a, s, invalid).await,
        Err(ContentError::Invalid(_))
    ));
    assert_eq!(rig.assembly_counts(a).await, (0, 0, 0, 0, 0));
    let c = doc(vec![]);
    let r = store.save(a, s, c.clone()).await.unwrap();
    let mut collision = c;
    collision.title.push('x');
    assert!(matches!(
        store.save(a, s, collision).await,
        Err(ContentError::IdempotencyConflict)
    ));
    assert_eq!(rig.assembly_counts(a).await, (1, 0, 0, 1, 1));
    assert!(store.read(a, r.reference).await.unwrap().is_some());
}

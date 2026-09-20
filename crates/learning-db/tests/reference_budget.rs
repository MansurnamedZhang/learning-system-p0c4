mod support;
use learning_core::*;
use learning_db::VersionedContentStore;
use support::{TestRig, references::seed_content};
use uuid::Uuid;

fn budget<T: std::fmt::Debug>(result: Result<T, ContentError>) {
    assert!(
        matches!(result,Err(ContentError::Invalid(ref code)) if code=="reference_budget_exceeded"),
        "{result:?}"
    );
}
fn with_basis(refs: Vec<BlockRef>) -> ContentDraft {
    ContentDraft::V2(ContentV2 {
        intent: Intent::Note,
        language: "en".into(),
        title: "".into(),
        body: BodyV2::Text(support::command("").draft.payload),
        basis_refs: refs.into_iter().map(ExactRef::Block).collect(),
        requires_context: vec![],
        source_run: None,
    })
}
#[tokio::test]
async fn exact_depth_boundary_and_diamond_paths_are_bounded() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let store = VersionedContentStore::new(rig.runtime_pool.clone());
    let mut tx = rig.runtime_pool.begin().await.unwrap();
    let mut prior = vec![
        seed_content(
            &mut tx,
            actor,
            space,
            ContentDraft::V1(support::command("leaf").draft),
            None,
        )
        .await,
    ];
    for _ in 1..32 {
        let left = seed_content(&mut tx, actor, space, with_basis(prior.clone()), None).await;
        let right = seed_content(&mut tx, actor, space, with_basis(prior), None).await;
        prior = vec![left, right];
    }
    let valid = prior[0].clone();
    let invalid = seed_content(&mut tx, actor, space, with_basis(prior), None).await;
    tx.commit().await.unwrap();
    assert!(store.read(actor, valid.clone()).await.unwrap().is_some());
    assert_eq!(
        store
            .read_many(actor, vec![valid; 200])
            .await
            .unwrap()
            .len(),
        1
    );
    budget(store.read(actor, invalid).await);
}

#[tokio::test]
async fn exact_deduplicated_edge_boundary_is_shared_across_roots() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let store = VersionedContentStore::new(rig.runtime_pool.clone());
    let mut tx = rig.runtime_pool.begin().await.unwrap();
    let mut leaves = vec![];
    for _ in 0..256 {
        leaves.push(
            seed_content(
                &mut tx,
                actor,
                space,
                ContentDraft::V1(support::command("").draft),
                None,
            )
            .await,
        );
    }
    let mut branches = vec![];
    for _ in 0..16 {
        branches.push(
            seed_content(
                &mut tx,
                actor,
                space,
                with_basis(leaves[..255].to_vec()),
                None,
            )
            .await,
        );
    }
    let valid = seed_content(&mut tx, actor, space, with_basis(branches.clone()), None).await;
    branches[0] = seed_content(&mut tx, actor, space, with_basis(leaves), None).await;
    let invalid = seed_content(&mut tx, actor, space, with_basis(branches), None).await;
    tx.commit().await.unwrap();
    assert!(store.read(actor, valid).await.unwrap().is_some());
    budget(store.read(actor, invalid).await);
}

#[tokio::test]
async fn exact_object_boundary_and_two_hundred_inputs_use_one_budget() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let store = VersionedContentStore::new(rig.runtime_pool.clone());
    let mut tx = rig.runtime_pool.begin().await.unwrap();
    let mut leaves = vec![];
    for _ in 0..2040 {
        leaves.push(
            seed_content(
                &mut tx,
                actor,
                space,
                ContentDraft::V1(support::command("").draft),
                None,
            )
            .await,
        );
    }
    let mut branches = vec![];
    for group in leaves[..2039].chunks(256) {
        branches.push(seed_content(&mut tx, actor, space, with_basis(group.to_vec()), None).await);
    }
    let valid = seed_content(&mut tx, actor, space, with_basis(branches.clone()), None).await;
    let replacement = seed_content(
        &mut tx,
        actor,
        space,
        with_basis(leaves[1792..].to_vec()),
        None,
    )
    .await;
    *branches.last_mut().unwrap() = replacement;
    let invalid = seed_content(&mut tx, actor, space, with_basis(branches), None).await;
    tx.commit().await.unwrap();
    assert!(store.read(actor, valid.clone()).await.unwrap().is_some());
    budget(store.read(actor, invalid).await);
    let mut batch = vec![valid; 199];
    batch.push(leaves[2039].clone());
    budget(store.read_many(actor, batch).await);
}

#[tokio::test]
async fn postgres_payload_bytes_accept_exact_limit_and_reject_one_extra_byte() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let store = VersionedContentStore::new(rig.runtime_pool.clone());
    let overhead: i32 = sqlx::query_scalar("SELECT octet_length($1::jsonb::text)")
        .bind(sqlx::types::Json(support::command("").draft))
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap();
    let mut tx = rig.runtime_pool.begin().await.unwrap();
    let mut remaining = 8 * 1024 * 1024usize;
    let mut roots = vec![];
    while remaining > 0 {
        let size = (remaining - overhead as usize).min(200000);
        roots.push(
            seed_content(
                &mut tx,
                actor,
                space,
                ContentDraft::V1(support::command(&"x".repeat(size)).draft),
                None,
            )
            .await,
        );
        remaining -= size + overhead as usize;
    }
    let last = roots.last().unwrap();
    let value: serde_json::Value =
        sqlx::query_scalar("SELECT content FROM block_revision WHERE id=$1")
            .bind(last.revision_id)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    let mut longer: TextDraft = serde_json::from_value(value).unwrap();
    longer.payload.text.push('x');
    let extra = seed_content(&mut tx, actor, space, ContentDraft::V1(longer), None).await;
    tx.commit().await.unwrap();
    assert_eq!(
        store.read_many(actor, roots.clone()).await.unwrap().len(),
        42
    );
    *roots.last_mut().unwrap() = extra;
    budget(store.read_many(actor, roots).await);
}

#[tokio::test]
async fn cycles_terminate_without_authorizing_only_the_visible_prefix() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let store = VersionedContentStore::new(rig.runtime_pool.clone());
    let ids: Vec<_> = (0..10)
        .map(|_| BlockRef {
            block_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
        })
        .collect();
    let mut tx = rig.runtime_pool.begin().await.unwrap();
    for (i, r) in ids.iter().enumerate() {
        sqlx::query("INSERT INTO block(id,space_id,head_revision_id) VALUES($1,$2,$3)")
            .bind(r.block_id)
            .bind(space)
            .bind(r.revision_id)
            .execute(&mut *tx)
            .await
            .unwrap();
        // Ten fully connected nodes have short simple paths but explosive path
        // enumeration, so the independent work cap must fail closed.
        let targets: Vec<_> = ids
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, r)| r.clone())
            .collect();
        let draft = with_basis(targets.clone());
        sqlx::query("INSERT INTO block_revision(id,space_id,block_id,contract_version,content,content_sha256,author_id,reason) VALUES($1,$2,$3,2,$4,$5,$6,'cycle')")
            .bind(r.revision_id).bind(space).bind(r.block_id).bind(serde_json::to_value(draft).unwrap()).bind("a".repeat(64)).bind(actor.actor_id).execute(&mut *tx).await.unwrap();
        for (position, t) in targets.iter().enumerate() {
            sqlx::query(
                "INSERT INTO reference_dependency VALUES('block',$1,$2,$3,'basis','block',$4,$5)",
            )
            .bind(r.block_id)
            .bind(r.revision_id)
            .bind(position as i32)
            .bind(t.block_id)
            .bind(t.revision_id)
            .execute(&mut *tx)
            .await
            .unwrap();
        }
    }
    tx.commit().await.unwrap();
    budget(store.read(actor, ids[0].clone()).await);
}

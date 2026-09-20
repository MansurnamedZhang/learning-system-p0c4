mod support;
use learning_core::*;
use support::{TestRig, references::*};

// A direct JSON-to-v1 decode before closure authorization turns a hidden result
// into a distinguishable storage/version error. All legacy routes must filter first.
#[tokio::test]
async fn legacy_basis_reads_hide_revoked_dependencies_before_version_adaptation() {
    hidden_legacy(false).await;
}
#[tokio::test]
async fn legacy_target_reads_hide_revoked_dependencies_before_version_adaptation() {
    hidden_legacy(true).await;
}
async fn hidden_legacy(target: bool) {
    let rig = TestRig::from_env().await;
    let (actor, space, evidence_space, _, conclusion) = fixture(&rig, target).await;
    rig.revoke(actor, evidence_space).await;
    assert!(
        rig.store
            .read(actor, conclusion.revision_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        rig.store
            .read_many(actor, &[conclusion.revision_id])
            .await
            .unwrap()
            .is_empty()
    );
    let history = rig
        .store
        .history(actor, conclusion.block_id, None)
        .await
        .unwrap();
    assert!(history.items.is_empty());
    assert!(history.next_cursor.is_none());
    let page = rig.store.list(actor, space, None).await.unwrap();
    assert!(page.items.is_empty());
    assert!(page.next_cursor.is_none());
    rig.grant(actor, evidence_space, false).await;
    assert!(
        matches!(rig.store.read(actor, conclusion.revision_id).await, Err(ContentError::Invalid(code)) if code == "unsupported_content_version")
    );
}

#[tokio::test]
async fn readable_revision_redacts_parent_with_hidden_structured_evidence() {
    let rig = TestRig::from_env().await;
    let (actor, space, evidence_space, _, previous) = fixture(&rig, false).await;
    let mut tx = rig.runtime_pool.begin().await.unwrap();
    let current = seed_content(
        &mut tx,
        actor,
        space,
        ContentDraft::V1(support::command("independent current").draft),
        Some(previous.clone()),
    )
    .await;
    tx.commit().await.unwrap();
    rig.revoke(actor, evidence_space).await;
    let result = rig
        .store
        .read(actor, current.revision_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.parent_revision_id, None);
    assert_eq!(result.draft.payload.text, "independent current");
    rig.grant(actor, evidence_space, false).await;
    assert_eq!(
        rig.store
            .read(actor, current.revision_id)
            .await
            .unwrap()
            .unwrap()
            .parent_revision_id,
        Some(previous.revision_id)
    );
}

#[tokio::test]
async fn stale_legacy_write_does_not_disclose_hidden_current_revision() {
    let rig = TestRig::from_env().await;
    let (actor, _, evidence_space, _, current) = fixture(&rig, true).await;
    rig.revoke(actor, evidence_space).await;
    let result = rig
        .store
        .revise(
            actor,
            current.block_id,
            ReviseCommand {
                request_id: uuid::Uuid::new_v4(),
                base_revision_id: uuid::Uuid::new_v4(),
                draft: support::command("edit").draft,
                reason: "test".into(),
            },
        )
        .await;
    assert!(matches!(result, Err(ContentError::NotFound)));
}

#[tokio::test]
async fn assembly_basis_dependency_is_hidden_before_legacy_adaptation() {
    hidden_assembly(false).await;
}
#[tokio::test]
async fn assembly_target_dependency_is_hidden_before_legacy_adaptation() {
    hidden_assembly(true).await;
}
async fn hidden_assembly(target: bool) {
    let rig = TestRig::from_env().await;
    let (actor, space, evidence_space, evidence, conclusion) = fixture(&rig, target).await;
    let root = seed_composition(&rig, actor, space, &conclusion).await;
    let view = seed_reading(&rig, actor, space, &root).await;
    rig.revoke(actor, evidence_space).await;
    assert!(
        rig.compositions()
            .read(actor, root.clone())
            .await
            .unwrap()
            .is_none()
    );
    let projection = support::reading::store(&rig)
        .read(actor, view, ReadingMode::Fused)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(projection.source, SourceProjection::Unavailable));
    assert!(projection.items.is_empty());
    let json = serde_json::to_string(&projection).unwrap();
    for id in [
        evidence.block_id,
        evidence.revision_id,
        conclusion.block_id,
        conclusion.revision_id,
    ] {
        assert!(!json.contains(&id.to_string()));
    }
}

#[tokio::test]
async fn legacy_successful_revision_redacts_hidden_parent_metadata() {
    let rig = TestRig::from_env().await;
    let (actor, _, evidence_space, _, current) = fixture(&rig, false).await;
    rig.revoke(actor, evidence_space).await;
    let result = rig
        .store
        .revise(
            actor,
            current.block_id,
            ReviseCommand {
                request_id: uuid::Uuid::new_v4(),
                base_revision_id: current.revision_id,
                draft: support::command("independent new text").draft,
                reason: "new".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(result.parent_revision_id, None);
}

#[tokio::test]
async fn personal_stale_revision_does_not_reveal_hidden_versioned_head() {
    use support::reading as h;
    let (rig, actor, space, doc, zero) = h::fixture().await;
    let reading = h::store(&rig);
    let saved = reading
        .edit(
            actor,
            zero.overlay.overlay_id,
            h::edit(&zero, h::add(h::gap(&doc, 1), "personal")),
        )
        .await
        .unwrap();
    let placement = reading
        .state(actor, saved.overlay.overlay_id)
        .await
        .unwrap()
        .unwrap()
        .editable
        .unwrap()
        .groups[0]
        .placements[0]
        .clone();
    let (_, evidence_space) = rig.seed_actor_space(true).await;
    rig.grant(actor, evidence_space, true).await;
    let (eid, erid) = rig.seed_block(actor, evidence_space).await;
    let draft = v2(
        BlockRef {
            block_id: eid,
            revision_id: erid,
        },
        true,
    );
    learning_db::VersionedContentStore::new(rig.runtime_pool.clone())
        .revise(
            actor,
            placement.block.block_id,
            ReviseContent {
                request_id: uuid::Uuid::new_v4(),
                base_revision_id: placement.block.revision_id,
                draft,
                reason: "v2 head".into(),
            },
        )
        .await
        .unwrap();
    rig.revoke(actor, evidence_space).await;
    let result = reading
        .edit(
            actor,
            saved.overlay.overlay_id,
            h::edit(
                &saved,
                ReadingEdit::ReviseSelected {
                    changes: vec![TextChange {
                        block_id: placement.block.block_id,
                        base_revision_id: placement.block.revision_id,
                        draft: support::command("try stale edit").draft,
                        selected_placements: vec![placement.placement_id],
                    }],
                },
            ),
        )
        .await;
    assert!(matches!(result, Err(ContentError::NotFound)), "{result:?}");
    assert!(rig.store.list(actor, space, None).await.is_ok());
}

#[tokio::test]
async fn personal_insert_locks_transitive_evidence_grant_before_writing() {
    use support::{assembly as a, reading as h};
    let rig = TestRig::from_env().await;
    let (actor, space, evidence_space, _, conclusion) = fixture(&rig, false).await;
    let original = rig
        .store
        .create(actor, space, support::command("original"))
        .await
        .unwrap();
    let doc = rig
        .compositions()
        .save(actor, space, a::doc(vec![a::block(&original)]))
        .await
        .unwrap();
    let saved = h::store(&rig)
        .create(actor, space, h::create(doc.reference.clone()))
        .await
        .unwrap();
    let before = h::counts(&rig, actor).await;
    let mut revoke = rig.admin_pool.begin().await.unwrap();
    let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *revoke)
        .await
        .unwrap();
    sqlx::query("DELETE FROM space_grant WHERE actor_id=$1 AND space_id=$2")
        .bind(actor.actor_id)
        .bind(evidence_space)
        .execute(&mut *revoke)
        .await
        .unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let waiter: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&pool)
        .await
        .unwrap();
    let reading = learning_db::ReadingStore::new(pool);
    let command = h::edit(
        &saved,
        ReadingEdit::InsertExisting {
            block: conclusion,
            target: InsertTarget::NewGroup {
                anchor: h::gap(&doc, 1),
            },
        },
    );
    let work =
        tokio::spawn(async move { reading.edit(actor, saved.overlay.overlay_id, command).await });
    let blocked = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let blocked: bool = sqlx::query_scalar("SELECT $1=ANY(pg_blocking_pids($2))")
                .bind(blocker)
                .bind(waiter)
                .fetch_one(&rig.admin_pool)
                .await
                .unwrap();
            if blocked {
                return true;
            }
            if work.is_finished() {
                return false;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    revoke.commit().await.unwrap();
    let result = work.await.unwrap();
    assert!(
        blocked,
        "write failed to lock transitive evidence grant: {result:?}"
    );
    assert!(matches!(result, Err(ContentError::NotFound)), "{result:?}");
    assert_eq!(h::counts(&rig, actor).await, before);
}

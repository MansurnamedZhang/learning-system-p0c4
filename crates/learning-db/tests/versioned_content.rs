mod support;
use learning_core::*;
use learning_db::VersionedContentStore;
use support::{TestRig, references::*};
use uuid::Uuid;

fn create(draft: ContentDraft) -> CreateContent {
    CreateContent {
        request_id: Uuid::new_v4(),
        draft,
        reason: "versioned".into(),
    }
}
#[tokio::test]
async fn versioned_writes_preserve_exact_revisions_and_global_request_keys() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let old = rig
        .store
        .create(actor, space, support::command("evidence"))
        .await
        .unwrap();
    let exact = BlockRef {
        block_id: old.block_id,
        revision_id: old.revision_id,
    };
    let store = VersionedContentStore::new(rig.runtime_pool.clone());
    let command = create(v2(exact, false));
    let first = store.create(actor, space, command.clone()).await.unwrap();
    assert_eq!(
        store.create(actor, space, command.clone()).await.unwrap(),
        first
    );
    assert_eq!(first.author_id, actor.actor_id);
    assert_eq!(first.parent_revision_id, None);
    assert!(
        matches!(rig.store.read(actor, first.revision_id).await, Err(ContentError::Invalid(code)) if code=="unsupported_content_version")
    );
    let mut legacy = support::command("different family");
    legacy.request_id = command.request_id;
    assert!(matches!(
        rig.store.create(actor, space, legacy).await,
        Err(ContentError::IdempotencyConflict)
    ));
    let revised = store
        .revise(
            actor,
            first.block_id,
            ReviseContent {
                request_id: Uuid::new_v4(),
                base_revision_id: first.revision_id,
                draft: ContentDraft::V1(support::command("independent").draft),
                reason: "new".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(revised.parent_revision_id, Some(first.revision_id));
    assert_eq!(
        store
            .read(
                actor,
                BlockRef {
                    block_id: first.block_id,
                    revision_id: first.revision_id
                }
            )
            .await
            .unwrap(),
        Some(first)
    );
    assert_eq!(
        rig.store
            .read(actor, revised.revision_id)
            .await
            .unwrap()
            .unwrap()
            .draft
            .payload
            .text,
        "independent"
    );
}

#[tokio::test]
async fn complete_closure_is_required_before_preview_or_batch_projection() {
    let rig = TestRig::from_env().await;
    let (actor, space, evidence_space, evidence, middle) = fixture(&rig, true).await;
    let store = VersionedContentStore::new(rig.runtime_pool.clone());
    let root = store
        .create(actor, space, create(v2(middle.clone(), true)))
        .await
        .unwrap();
    let reference = BlockRef {
        block_id: root.block_id,
        revision_id: root.revision_id,
    };
    let preview = serde_json::to_value(
        store
            .preview(actor, reference.clone())
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let wire = preview.to_string();
    assert!(wire.contains(&evidence.revision_id.to_string()));
    assert!(!wire.contains("保留空格"), "third level must be link only");
    rig.revoke(actor, evidence_space).await;
    assert!(
        store
            .read(actor, reference.clone())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .preview(actor, reference.clone())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .read_many(actor, vec![reference.clone(), middle])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        store
            .create(actor, space, create(v2(reference.clone(), false)))
            .await,
        Err(ContentError::NotFound)
    ));
    rig.grant(actor, evidence_space, false).await;
    assert_eq!(
        store.read(actor, reference.clone()).await.unwrap(),
        Some(root)
    );
    assert!(
        store
            .read(
                actor,
                BlockRef {
                    block_id: Uuid::new_v4(),
                    revision_id: reference.revision_id
                }
            )
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn versioned_input_limit_precedes_work_and_duplicate_inputs_keep_first_order() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let store = VersionedContentStore::new(rig.runtime_pool.clone());
    let a = store
        .create(
            actor,
            space,
            create(ContentDraft::V1(support::command("a").draft)),
        )
        .await
        .unwrap();
    let b = store
        .create(
            actor,
            space,
            create(ContentDraft::V1(support::command("b").draft)),
        )
        .await
        .unwrap();
    let ar = BlockRef {
        block_id: a.block_id,
        revision_id: a.revision_id,
    };
    let br = BlockRef {
        block_id: b.block_id,
        revision_id: b.revision_id,
    };
    assert_eq!(
        store
            .read_many(actor, vec![br.clone(), ar.clone(), br])
            .await
            .unwrap(),
        vec![b, a]
    );
    assert!(store.read_many(actor, vec![ar.clone(); 200]).await.is_ok());
    assert!(matches!(
        store.read_many(actor, vec![ar; 201]).await,
        Err(ContentError::Invalid(_))
    ));
}

#[tokio::test]
async fn versioned_composition_reading_and_legacy_publish_guard_are_consistent() {
    use support::{assembly as a, reading as h};
    let rig = TestRig::from_env().await;
    let (actor, space, evidence_space, _, conclusion) = fixture(&rig, true).await;
    let doc = rig
        .compositions()
        .save(
            actor,
            space,
            a::doc(vec![NodeTarget::Block(conclusion.clone())]),
        )
        .await
        .unwrap();
    let snapshot = rig
        .compositions()
        .read_versioned(actor, doc.reference.clone())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(snapshot.blocks[0].draft, ContentDraft::V2(_)));
    assert!(
        matches!(rig.compositions().read(actor,doc.reference.clone()).await,Err(ContentError::Invalid(code)) if code=="unsupported_content_version")
    );
    let reading = h::store(&rig);
    let saved = reading
        .create(actor, space, h::create(doc.reference.clone()))
        .await
        .unwrap();
    let projected = reading
        .read_versioned(actor, saved.view.clone(), ReadingMode::Fused)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(projected.items.len(), 1);
    assert!(
        matches!(reading.read(actor,saved.view.clone(),ReadingMode::Fused).await,Err(ContentError::Invalid(code)) if code=="unsupported_content_version")
    );
    let before = rig.assembly_counts(actor).await;
    assert!(
        matches!(rig.releases().publish(actor,space,a::publish(vec![a::root(&doc,None)])).await,Err(ContentError::Invalid(code)) if code=="unsupported_content_version")
    );
    assert_eq!(rig.assembly_counts(actor).await, before);
    rig.revoke(actor, evidence_space).await;
    assert!(
        rig.compositions()
            .read_versioned(actor, doc.reference.clone())
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        reading
            .read_versioned(actor, saved.view, ReadingMode::Fused)
            .await
            .unwrap()
            .unwrap()
            .source,
        SourceProjection::Unavailable
    ));
    assert!(matches!(
        rig.releases()
            .publish(actor, space, a::publish(vec![a::root(&doc, None)]))
            .await,
        Err(ContentError::NotFound)
    ));
}
#[tokio::test]
async fn hidden_personal_v2_is_omitted_before_legacy_adaptation() {
    use support::{assembly as a, reading as h};
    let rig = TestRig::from_env().await;
    let (actor, space, evidence_space, _, conclusion) = fixture(&rig, false).await;
    let visible = rig
        .store
        .create(actor, space, support::command("visible original"))
        .await
        .unwrap();
    let doc = rig
        .compositions()
        .save(actor, space, a::doc(vec![a::block(&visible)]))
        .await
        .unwrap();
    let reading = h::store(&rig);
    let saved = reading
        .create(actor, space, h::create(doc.reference.clone()))
        .await
        .unwrap();
    let saved = reading
        .edit(
            actor,
            saved.overlay.overlay_id,
            h::edit(
                &saved,
                ReadingEdit::InsertExisting {
                    block: conclusion.clone(),
                    target: InsertTarget::NewGroup {
                        anchor: h::gap(&doc, 1),
                    },
                },
            ),
        )
        .await
        .unwrap();
    assert!(
        matches!(reading.read(actor,saved.view.clone(),ReadingMode::Fused).await,Err(ContentError::Invalid(code)) if code=="unsupported_content_version")
    );
    rig.revoke(actor, evidence_space).await;
    let projected = reading
        .read(actor, saved.view, ReadingMode::Fused)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(h::text(&projected), ["visible original"]);
    assert!(
        !serde_json::to_string(&projected)
            .unwrap()
            .contains(&conclusion.revision_id.to_string())
    );
}

#[tokio::test]
async fn versioned_pages_use_only_visible_rows_for_cursors_and_history() {
    let rig = TestRig::from_env().await;
    let (actor, space, evidence_space, _, conclusion) = fixture(&rig, false).await;
    let store = VersionedContentStore::new(rig.runtime_pool.clone());
    assert_eq!(store.list(actor, space, None).await.unwrap().items.len(), 1);
    assert_eq!(
        store
            .history(actor, conclusion.block_id, None)
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    rig.revoke(actor, evidence_space).await;
    let page = store.list(actor, space, None).await.unwrap();
    assert!(page.items.is_empty());
    assert!(page.next_cursor.is_none());
    let page = store
        .history(actor, conclusion.block_id, None)
        .await
        .unwrap();
    assert!(page.items.is_empty());
    assert!(page.next_cursor.is_none());
    let mut visible = vec![];
    for _ in 0..102 {
        visible.push(rig.seed_block(actor, space).await.0);
    }
    sqlx::query("UPDATE block SET created_at='2026-01-01T00:00:00Z' WHERE space_id=$1")
        .bind(space)
        .execute(&rig.admin_pool)
        .await
        .unwrap();
    visible.sort();
    visible.reverse();
    let page = store.list(actor, space, None).await.unwrap();
    assert_eq!(
        page.items.iter().map(|r| r.block_id).collect::<Vec<_>>(),
        visible[..100]
    );
    let cursor = page.next_cursor.unwrap();
    assert_eq!(cursor.id, visible[99]);
    let tail = store.list(actor, space, Some(cursor)).await.unwrap();
    assert_eq!(
        tail.items.iter().map(|r| r.block_id).collect::<Vec<_>>(),
        visible[100..]
    );
    assert!(tail.next_cursor.is_none());
}

#[tokio::test]
async fn all_reference_kinds_inherit_private_overlay_ownership_and_restore_grants() {
    use support::{assembly as a, reading as h, typed_references as t};
    let rig = TestRig::from_env().await;
    let (owner, space) = rig.seed_actor_space(true).await;
    let (reader, _) = rig.seed_actor_space(true).await;
    rig.grant(reader, space, false).await;
    let left = rig
        .store
        .create(owner, space, support::command("left"))
        .await
        .unwrap();
    let right = rig
        .store
        .create(owner, space, support::command("right"))
        .await
        .unwrap();
    let doc = rig
        .compositions()
        .save(owner, space, a::doc(vec![a::block(&left)]))
        .await
        .unwrap();
    let layer = h::store(&rig)
        .create(owner, space, h::create(doc.reference))
        .await
        .unwrap();
    let store = VersionedContentStore::new(rig.runtime_pool.clone());
    for scope in [Some(layer.overlay.overlay_id), None] {
        let mut tx = rig.runtime_pool.begin().await.unwrap();
        let relation = t::relation(
            &mut tx,
            owner.actor_id,
            space,
            scope,
            (left.block_id, left.revision_id),
            (right.block_id, right.revision_id),
            true,
        )
        .await;
        let review = t::review(&mut tx, owner.actor_id, space, relation, None, true).await;
        let epistemic = t::epistemic(
            &mut tx,
            owner.actor_id,
            space,
            scope,
            (left.block_id, left.revision_id),
            serde_json::json!([]),
            serde_json::json!([]),
            true,
        )
        .await;
        tx.commit().await.unwrap();
        let rr = RelationRef {
            relation_id: relation.0,
            revision_id: relation.1,
        };
        for reference in [
            ExactRef::Relation(rr.clone()),
            ExactRef::RelationReview(RelationReviewRef {
                relation: rr,
                review_id: review,
            }),
            ExactRef::EpistemicReview(EpistemicReviewRef {
                stream_id: epistemic.0,
                review_id: epistemic.1,
            }),
        ] {
            let draft = ContentDraft::V2(ContentV2 {
                intent: Intent::Note,
                language: "en".into(),
                title: "typed".into(),
                body: BodyV2::Text(support::command("body").draft.payload),
                basis_refs: vec![reference],
                requires_context: vec![],
                source_run: None,
            });
            let saved = store.create(owner, space, create(draft)).await.unwrap();
            let root = BlockRef {
                block_id: saved.block_id,
                revision_id: saved.revision_id,
            };
            assert_eq!(
                store.read(owner, root.clone()).await.unwrap(),
                Some(saved.clone())
            );
            assert_eq!(
                store.read(reader, root.clone()).await.unwrap().is_some(),
                scope.is_none()
            );
            rig.revoke(owner, space).await;
            assert!(store.read(owner, root.clone()).await.unwrap().is_none());
            rig.grant(owner, space, true).await;
            assert_eq!(store.read(owner, root).await.unwrap(), Some(saved));
        }
    }
}

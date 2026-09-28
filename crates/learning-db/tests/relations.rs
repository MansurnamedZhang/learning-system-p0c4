#[path = "support/relation_store.rs"]
mod h;
mod support;
use h::*;
use learning_core::*;
use uuid::Uuid;

#[tokio::test]
async fn relation_versions_pin_endpoints_and_reviews_survive_withdrawal() {
    let (rig, store, actor, space, e, h) = fixture().await;
    let command = save(space, exact(&e), exact(&h));
    let first = store.save(actor, command.clone()).await.unwrap();
    assert_eq!(first.author_id, actor.actor_id);
    assert_eq!(first.origin, RelationOrigin::UserAsserted);
    let checked = store.review(actor, review(&first)).await.unwrap();
    assert_eq!(checked.reviewer_id, actor.actor_id);
    let h2 = rig
        .store
        .revise(actor, h.block_id, support::change(&h, "H@2"))
        .await
        .unwrap();
    assert_ne!(h.content_sha256, h2.content_sha256);
    assert_eq!(
        store
            .read(actor, first.reference.clone())
            .await
            .unwrap()
            .unwrap()
            .to,
        exact(&h)
    );
    let mut next = edit(&first);
    next.to = exact(&h2);
    let second = store.save(actor, next.clone()).await.unwrap();
    assert_ne!(first.content_sha256, second.content_sha256);
    assert_eq!(second.parent_revision_id, Some(first.reference.revision_id));
    assert_eq!(store.save(actor, command).await.unwrap(), first);
    assert_eq!(store.save(actor, next).await.unwrap(), second);
    assert_eq!(
        rig.store
            .read(actor, e.revision_id)
            .await
            .unwrap()
            .unwrap()
            .content_sha256,
        e.content_sha256
    );
    assert_eq!(
        rig.store
            .read(actor, h2.revision_id)
            .await
            .unwrap()
            .unwrap()
            .content_sha256,
        h2.content_sha256
    );
    let second_review = store.review(actor, review(&second)).await.unwrap();
    assert_eq!(second_review.previous_review_id, None);
    let mut withdraw = review(&first);
    withdraw.expected_previous = Some(checked.reference.review_id);
    withdraw.state = RelationReviewState::Withdrawn;
    let withdrawn = store.review(actor, withdraw.clone()).await.unwrap();
    assert_eq!(
        withdrawn.previous_review_id,
        Some(checked.reference.review_id)
    );
    assert_eq!(store.review(actor, withdraw).await.unwrap(), withdrawn);
    assert_eq!(
        store
            .read_review(actor, checked.reference.clone())
            .await
            .unwrap(),
        ReviewProjection::Available(checked)
    );
    assert_eq!(
        store
            .read_review(actor, second_review.reference.clone())
            .await
            .unwrap(),
        ReviewProjection::Available(second_review)
    );
    assert_eq!(
        store
            .read_selected(
                actor,
                vec![first.reference.clone(), second.reference.clone()]
            )
            .await
            .unwrap(),
        vec![first, second]
    );
    assert_eq!(counts(&rig, space).await, (1, 2, 3, 2, 3));
}

#[tokio::test]
async fn normalized_symmetric_keys_and_immutable_identity_are_enforced() {
    let (rig, store, actor, space, e, h) = fixture().await;
    let mut c = save(space, exact(&e), exact(&h));
    c.relation_type = RelationType::RelatedTo;
    // Always exercise the swap branch despite server-generated UUIDs.
    if c.from.block_id < c.to.block_id {
        std::mem::swap(&mut c.from, &mut c.to);
    }
    let r = store.save(actor, c.clone()).await.unwrap();
    // Independent whole-pair normalization (including the matching revisions).
    // This is added coverage of already-correct behavior, not a claimed RED.
    let (expected_from, expected_to) = if e.block_id < h.block_id {
        (exact(&e), exact(&h))
    } else {
        (exact(&h), exact(&e))
    };
    assert_eq!((&r.from, &r.to), (&expected_from, &expected_to));
    let canonical = serde_json::json!({"domain":"relation-content-v1","scope":{"kind":"space","space_id":space},"type":"related_to","from":expected_from,"to":expected_to,"rationale":"observation","conditions":"within scope"});
    assert_eq!(
        r.content_sha256,
        hex_digest(canonical_json(&canonical).as_bytes())
    );
    std::mem::swap(&mut c.from, &mut c.to);
    assert!(matches!(
        store.save(actor, c.clone()).await,
        Err(ContentError::IdempotencyConflict)
    ));
    c.request_id = Uuid::new_v4();
    assert!(
        matches!(store.save(actor,c).await,Err(ContentError::Invalid(s)) if s=="relation_exists")
    );
    let (_, other_space) = rig.seed_actor_space(true).await;
    rig.grant(actor, other_space, true).await;
    let extra = rig
        .store
        .create(actor, space, support::command("third"))
        .await
        .unwrap();
    for variant in 0..3 {
        let mut c = edit(&r);
        match variant {
            0 => c.relation_type = RelationType::Opposes,
            1 => c.to = exact(&extra),
            _ => {
                c.scope = RelationScope::Space {
                    space_id: other_space,
                }
            }
        }
        assert!(
            matches!(store.save(actor,c).await,Err(ContentError::Invalid(s)) if s=="relation_identity_mismatch")
        );
    }
    assert_eq!(counts(&rig, space).await, (1, 1, 0, 1, 0));
}

#[tokio::test]
async fn reviewed_supports_and_opposes_require_all_three_nonblank_fields() {
    let (rig, store, actor, space, e, h) = fixture().await;
    for kind in [RelationType::Supports, RelationType::Opposes] {
        let mut c = save(space, exact(&e), exact(&h));
        c.relation_type = kind;
        c.rationale = " \n".into();
        c.conditions = " ".into();
        let empty = store.save(actor, c).await.unwrap();
        let mut unreviewed = review(&empty);
        unreviewed.state = RelationReviewState::Unreviewed;
        unreviewed.explanation.clear();
        let initial = store.review(actor, unreviewed).await.unwrap();
        let mut reviewed = review(&empty);
        reviewed.expected_previous = Some(initial.reference.review_id);
        assert!(matches!(
            store.review(actor, reviewed).await,
            Err(ContentError::Invalid(_))
        ));
        let mut c = edit(&empty);
        c.rationale = "observed".into();
        let missing_conditions = store.save(actor, c).await.unwrap();
        assert!(matches!(
            store.review(actor, review(&missing_conditions)).await,
            Err(ContentError::Invalid(_))
        ));
        let mut c = edit(&missing_conditions);
        c.conditions = " bounded \n".into();
        let ready = store.save(actor, c).await.unwrap();
        assert_eq!(ready.conditions, " bounded \n");
        let mut checked = review(&ready);
        checked.explanation = "\t".into();
        assert!(matches!(
            store.review(actor, checked).await,
            Err(ContentError::Invalid(_))
        ));
        store.review(actor, review(&ready)).await.unwrap();
    }
    assert_eq!(counts(&rig, space).await, (2, 6, 4, 6, 4));
}

#[tokio::test]
async fn replay_rechecks_current_grants_before_global_key_conflicts() {
    let (rig, store, actor, space, e, h) = fixture().await;
    let c = save(space, exact(&e), exact(&h));
    let r = store.save(actor, c.clone()).await.unwrap();
    let mut rc = review(&r);
    let old = store.review(actor, rc.clone()).await.unwrap();
    let mut later = rc.clone();
    later.request_id = Uuid::new_v4();
    later.expected_previous = Some(old.reference.review_id);
    later.state = RelationReviewState::NeedsRecheck;
    store.review(actor, later).await.unwrap();
    assert_eq!(store.review(actor, rc.clone()).await.unwrap(), old);
    let mut changed = c.clone();
    changed.conditions = "changed".into();
    assert!(matches!(
        store.save(actor, changed.clone()).await,
        Err(ContentError::IdempotencyConflict)
    ));
    rc.request_id = c.request_id;
    assert!(matches!(
        store.review(actor, rc.clone()).await,
        Err(ContentError::IdempotencyConflict)
    ));
    let mut legacy = support::command("global namespace");
    legacy.request_id = c.request_id;
    assert!(matches!(
        rig.store.create(actor, space, legacy).await,
        Err(ContentError::IdempotencyConflict)
    ));
    rig.revoke(actor, space).await;
    assert!(matches!(
        store.save(actor, c).await,
        Err(ContentError::NotFound)
    ));
    assert!(matches!(
        store.save(actor, changed).await,
        Err(ContentError::NotFound)
    ));
    assert!(matches!(
        store.review(actor, rc).await,
        Err(ContentError::NotFound)
    ));
    assert_eq!(
        store.read_review(actor, old.reference).await.unwrap(),
        ReviewProjection::Unavailable
    );
}

#[tokio::test]
async fn hidden_dependencies_hide_relations_replays_and_conflicts_and_mark_reviews_incomplete() {
    let (rig, store, actor, space, e, h) = fixture().await;
    let (_, secret) = rig.seed_actor_space(true).await;
    rig.grant(actor, secret, true).await;
    let witness = rig
        .store
        .create(actor, secret, support::command("private"))
        .await
        .unwrap();
    let original_command = save(space, exact(&e), exact(&h));
    let first = store.save(actor, original_command.clone()).await.unwrap();
    let original_review = review(&first);
    let original_checked = store.review(actor, original_review.clone()).await.unwrap();
    let h2 = learning_db::VersionedContentStore::new(rig.runtime_pool.clone())
        .revise(
            actor,
            h.block_id,
            ReviseContent {
                request_id: Uuid::new_v4(),
                base_revision_id: h.revision_id,
                draft: support::references::v2(exact(&witness), false),
                reason: "basis".into(),
            },
        )
        .await
        .unwrap();
    let mut c = edit(&first);
    c.to = BlockRef {
        block_id: h2.block_id,
        revision_id: h2.revision_id,
    };
    let hidden = store.save(actor, c.clone()).await.unwrap();
    let checked = store.review(actor, review(&hidden)).await.unwrap();
    rig.revoke(actor, secret).await;
    assert_eq!(store.save(actor, original_command).await.unwrap(), first);
    assert_eq!(
        store.review(actor, original_review).await.unwrap(),
        original_checked
    );
    assert!(
        store
            .read(actor, hidden.reference.clone())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store
            .read_selected(
                actor,
                vec![first.reference.clone(), hidden.reference.clone()]
            )
            .await
            .unwrap(),
        vec![first.clone()]
    );
    assert_eq!(
        store
            .read_review(actor, checked.reference.clone())
            .await
            .unwrap(),
        ReviewProjection::Incomplete
    );
    assert!(matches!(
        store.save(actor, c).await,
        Err(ContentError::NotFound)
    ));
    assert!(matches!(
        store.save(actor, edit(&first)).await,
        Err(ContentError::NotFound)
    ));
    let duplicate = save(space, exact(&e), exact(&h));
    assert!(matches!(
        store.save(actor, duplicate).await,
        Err(ContentError::NotFound)
    ));
    let mut missing = checked.reference;
    missing.review_id = Uuid::new_v4();
    assert_eq!(
        store.read_review(actor, missing).await.unwrap(),
        ReviewProjection::Unavailable
    );
}

#[tokio::test]
async fn personal_scope_requires_overlay_owner_even_with_a_write_grant() {
    let (rig, store, actor, space, e, h) = fixture().await;
    let root = support::references::seed_composition(&rig, actor, space, &exact(&e)).await;
    let view = support::references::seed_reading(&rig, actor, space, &root).await;
    let overlay: Uuid = sqlx::query_scalar("SELECT overlay_id FROM reading_view WHERE id=$1")
        .bind(view.view_id)
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap();
    let mut c = save(space, exact(&e), exact(&h));
    c.scope = RelationScope::PersonalOverlay {
        space_id: space,
        overlay_id: overlay,
    };
    let r = store.save(actor, c.clone()).await.unwrap();
    let rr = store.review(actor, review(&r)).await.unwrap();
    let (other, _) = rig.seed_actor_space(true).await;
    rig.grant(other, space, true).await;
    c.request_id = Uuid::new_v4();
    assert!(matches!(
        store.save(other, c).await,
        Err(ContentError::NotFound)
    ));
    assert!(matches!(
        store.review(other, review(&r)).await,
        Err(ContentError::NotFound)
    ));
    assert!(store.read(other, r.reference).await.unwrap().is_none());
    assert_eq!(
        store.read_review(other, rr.reference).await.unwrap(),
        ReviewProjection::Unavailable
    );
}

#[tokio::test]
async fn selected_relations_reject_duplicates_and_oversized_selection() {
    let (_, store, actor, space, e, h) = fixture().await;
    let r = store
        .save(actor, save(space, exact(&e), exact(&h)))
        .await
        .unwrap();
    assert!(matches!(
        store
            .read_selected(actor, vec![r.reference.clone(); 2])
            .await,
        Err(ContentError::Invalid(_))
    ));
    assert!(matches!(
        store
            .read_selected(
                actor,
                (0..257)
                    .map(|_| RelationRef {
                        relation_id: Uuid::new_v4(),
                        revision_id: Uuid::new_v4()
                    })
                    .collect()
            )
            .await,
        Err(ContentError::Invalid(_))
    ));
}

#[tokio::test]
async fn absent_or_foreign_review_predecessor_never_starts_or_splices_a_stream() {
    let (_, store, actor, space, e, h) = fixture().await;
    let first = store
        .save(actor, save(space, exact(&e), exact(&h)))
        .await
        .unwrap();
    let mut c = review(&first);
    c.expected_previous = Some(Uuid::new_v4());
    assert!(
        matches!(store.review(actor,c).await,Err(ContentError::Invalid(s)) if s=="relation_review_predecessor_mismatch")
    );
    let first_review = store.review(actor, review(&first)).await.unwrap();
    let second = store.save(actor, edit(&first)).await.unwrap();
    let mut c = review(&second);
    c.expected_previous = Some(first_review.reference.review_id);
    assert!(
        matches!(store.review(actor,c).await,Err(ContentError::Invalid(s)) if s=="relation_review_predecessor_mismatch")
    );
    let second_review = store.review(actor, review(&second)).await.unwrap();
    let mut c = review(&first);
    c.expected_previous = Some(second_review.reference.review_id);
    assert!(
        matches!(store.review(actor,c).await,Err(ContentError::Conflict{current_revision_id}) if current_revision_id==first_review.reference.review_id)
    );
}

#[tokio::test]
async fn selected_relation_dependencies_share_one_payload_budget() {
    let (rig, store, actor, space, e, _) = fixture().await;
    let mut roots = vec![];
    for _ in 0..2 {
        let mut tx = rig.runtime_pool.begin().await.unwrap();
        let mut basis = vec![];
        for _ in 0..24 {
            let r = support::references::seed_content(
                &mut tx,
                actor,
                space,
                ContentDraft::V1(support::command(&"a".repeat(180000)).draft),
                None,
            )
            .await;
            basis.push(ExactRef::Block(r));
        }
        let hub = support::references::seed_content(
            &mut tx,
            actor,
            space,
            ContentDraft::V2(ContentV2 {
                intent: Intent::Note,
                language: "en".into(),
                title: "hub".into(),
                body: BodyV2::Text(support::command("hub").draft.payload),
                basis_refs: basis,
                requires_context: vec![],
                source_run: None,
            }),
            None,
        )
        .await;
        tx.commit().await.unwrap();
        let r = store
            .save(actor, save(space, exact(&e), hub))
            .await
            .unwrap();
        assert!(
            store
                .read(actor, r.reference.clone())
                .await
                .unwrap()
                .is_some()
        );
        roots.push(r.reference);
    }
    assert!(
        matches!(store.read_selected(actor,roots).await,Err(ContentError::Invalid(s)) if s=="reference_budget_exceeded")
    );
}

#[tokio::test]
async fn update_replay_redacts_inaccessible_audit_parent_without_requiring_it() {
    let (rig, store, actor, space, e, h) = fixture().await;
    let (_, secret) = rig.seed_actor_space(true).await;
    rig.grant(actor, secret, true).await;
    let witness = rig
        .store
        .create(actor, secret, support::command("former evidence"))
        .await
        .unwrap();
    let versioned = learning_db::VersionedContentStore::new(rig.runtime_pool.clone());
    let h2 = versioned
        .revise(
            actor,
            h.block_id,
            ReviseContent {
                request_id: Uuid::new_v4(),
                base_revision_id: h.revision_id,
                draft: support::references::v2(exact(&witness), false),
                reason: "temporary basis".into(),
            },
        )
        .await
        .unwrap();
    let first = store
        .save(
            actor,
            save(
                space,
                exact(&e),
                BlockRef {
                    block_id: h2.block_id,
                    revision_id: h2.revision_id,
                },
            ),
        )
        .await
        .unwrap();
    let mut command = edit(&first);
    command.to = exact(&h);
    let mut second = store.save(actor, command.clone()).await.unwrap();
    assert_eq!(second.parent_revision_id, Some(first.reference.revision_id));
    let rc = review(&second);
    let checked = store.review(actor, rc.clone()).await.unwrap();
    rig.revoke(actor, secret).await;
    second.parent_revision_id = None;
    assert_eq!(
        store.read(actor, second.reference.clone()).await.unwrap(),
        Some(second.clone())
    );
    assert_eq!(store.save(actor, command).await.unwrap(), second);
    assert_eq!(store.review(actor, rc).await.unwrap(), checked);
}

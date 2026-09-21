#[path = "support/relation_store.rs"]
mod relations;
mod support;
use learning_core::*;
use learning_db::{MigrationStore, RelationStore};
use support::{assembly as a, reading as h};
use uuid::Uuid;

fn select(saved: &ReadingSaved, relation: RelationRef) -> SelectRelations {
    SelectRelations {
        request_id: Uuid::new_v4(),
        expected_overlay_revision: saved.overlay.revision_id,
        expected_reading_view_revision: saved.view.revision_id,
        selections: vec![RelationSelection {
            relation,
            review: None,
        }],
        epistemic_reviews: vec![],
        reason: "select precise evidence".into(),
    }
}
async fn relation(r: &support::TestRig, actor: Principal, space: Uuid) -> RelationRevision {
    let e = r
        .store
        .create(actor, space, support::command("historical E"))
        .await
        .unwrap();
    let h = r
        .store
        .create(actor, space, support::command("historical H"))
        .await
        .unwrap();
    RelationStore::new(r.runtime_pool.clone())
        .save(
            actor,
            relations::save(space, relations::exact(&e), relations::exact(&h)),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn view_only_selection_survives_edit_migration_modes_and_keeps_legacy_wire() {
    let (r, actor, space, doc, zero) = h::fixture().await;
    let store = h::store(&r);
    let rel = relation(&r, actor, space).await;
    let before = h::counts(&r, actor).await;
    let cmd = select(&zero, rel.reference.clone());
    let one = store
        .select_relations(actor, zero.overlay.overlay_id, cmd.clone())
        .await
        .unwrap();
    assert_eq!(one.overlay, zero.overlay);
    assert_ne!(one.view, zero.view);
    assert!(one.changed_blocks.is_empty());
    let after = h::counts(&r, actor).await;
    assert_eq!(&before[..9], &after[..9]);
    assert_eq!(after[9], before[9] + 1);
    assert_eq!(
        store
            .select_relations(actor, zero.overlay.overlay_id, cmd)
            .await
            .unwrap(),
        one
    );
    assert!(
        matches!(store.select_relations(actor, one.overlay.overlay_id, select(&one,rel.reference.clone())).await, Err(ContentError::Invalid(s)) if s == "no_change")
    );
    let legacy = store
        .read(actor, zero.view.clone(), ReadingMode::Fused)
        .await
        .unwrap()
        .unwrap();
    assert!(
        serde_json::to_value(legacy)
            .unwrap()
            .get("evidence")
            .is_none()
    );
    assert!(
        matches!(store.read(actor,one.view.clone(),ReadingMode::Fused).await,Err(ContentError::Invalid(s)) if s == "unsupported_content_version")
    );
    let two = store
        .edit(
            actor,
            one.overlay.overlay_id,
            h::edit(&one, h::add(h::gap(&doc, 1), "personal")),
        )
        .await
        .unwrap();
    let state = store
        .state(actor, two.overlay.overlay_id)
        .await
        .unwrap()
        .unwrap()
        .editable
        .unwrap();
    let moved = store
        .edit(
            actor,
            two.overlay.overlay_id,
            h::edit(
                &two,
                ReadingEdit::Move {
                    placement_id: state.groups[0].placements[0].placement_id,
                    target: InsertTarget::NewGroup {
                        anchor: h::gap(&doc, 0),
                    },
                },
            ),
        )
        .await
        .unwrap();
    let two = moved;
    let mut revised = a::edit(&doc);
    revised.title = "changed source".into();
    let next = r.compositions().save(actor, space, revised).await.unwrap();
    let migrations = MigrationStore::new(r.runtime_pool.clone());
    let proposal = migrations
        .propose(
            actor,
            two.overlay.overlay_id,
            ProposeMigration {
                request_id: Uuid::new_v4(),
                expected_overlay_revision: two.overlay.revision_id,
                expected_reading_view_revision: two.view.revision_id,
                target: next.reference.clone(),
                reason: "upgrade".into(),
            },
        )
        .await
        .unwrap();
    let groups = proposal
        .groups
        .iter()
        .map(|g| GroupDecision::KeepUnplaced {
            group_id: g.group_id,
        })
        .collect();
    let three = migrations
        .decide(
            actor,
            two.overlay.overlay_id,
            DecideMigration {
                request_id: Uuid::new_v4(),
                proposal_id: proposal.proposal_id,
                expected_overlay_revision: two.overlay.revision_id,
                expected_reading_view_revision: two.view.revision_id,
                action: MigrationAction::Adopt {
                    groups,
                    merges: vec![],
                },
                reason: "adopt".into(),
            },
        )
        .await
        .unwrap()
        .adopted
        .unwrap();
    let mut new_rel = relations::edit(&rel);
    new_rel.rationale = "later relation head".into();
    RelationStore::new(r.runtime_pool.clone())
        .save(actor, new_rel)
        .await
        .unwrap();
    for saved in [&one, &two, &three] {
        for mode in [
            ReadingMode::Original,
            ReadingMode::Fused,
            ReadingMode::Personal,
        ] {
            let p = store
                .read_versioned(actor, saved.view.clone(), mode)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(p.view, saved.view);
            assert_eq!(p.evidence.selections[0].relation.reference, rel.reference);
        }
    }
    assert!(
        store
            .read_versioned(actor, zero.view, ReadingMode::Fused)
            .await
            .unwrap()
            .unwrap()
            .evidence
            .selections
            .is_empty()
    );
}

#[tokio::test]
async fn hidden_endpoint_omits_selection_and_replay_rechecks_current_authorization() {
    let (r, actor, space, _, zero) = h::fixture().await;
    let (other, foreign) = r.seed_actor_space(true).await;
    r.grant(actor, foreign, false).await;
    let e = r
        .store
        .create(other, foreign, support::command("SECRET endpoint"))
        .await
        .unwrap();
    let target = r
        .store
        .create(actor, space, support::command("visible"))
        .await
        .unwrap();
    let rel = RelationStore::new(r.runtime_pool.clone())
        .save(
            actor,
            relations::save(space, relations::exact(&e), relations::exact(&target)),
        )
        .await
        .unwrap();
    let cmd = select(&zero, rel.reference.clone());
    let store = h::store(&r);
    let saved = store
        .select_relations(actor, zero.overlay.overlay_id, cmd.clone())
        .await
        .unwrap();
    r.revoke(actor, foreign).await;
    let p = store
        .read_versioned(actor, saved.view, ReadingMode::Fused)
        .await
        .unwrap()
        .unwrap();
    assert!(p.evidence.selections.is_empty());
    let wire = serde_json::to_string(&p).unwrap();
    for secret in [
        e.block_id,
        e.revision_id,
        rel.reference.relation_id,
        rel.reference.revision_id,
    ] {
        assert!(!wire.contains(&secret.to_string()));
    }
    assert!(matches!(
        store
            .select_relations(actor, zero.overlay.overlay_id, cmd)
            .await,
        Err(ContentError::NotFound)
    ));
}

#[tokio::test]
async fn concurrent_selection_uses_view_cas_without_overlay_mutation() {
    let (r, actor, space, _, zero) = h::fixture().await;
    let rel = relation(&r, actor, space).await;
    let store = h::store(&r);
    let a = select(&zero, rel.reference.clone());
    let b = select(&zero, rel.reference);
    let (a, b) = tokio::join!(
        store.select_relations(actor, zero.overlay.overlay_id, a),
        store.select_relations(actor, zero.overlay.overlay_id, b)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let failed = if a.is_err() { a } else { b };
    assert!(matches!(failed, Err(ContentError::ReadingConflict { .. })));
    assert_eq!(
        store
            .state(actor, zero.overlay.overlay_id)
            .await
            .unwrap()
            .unwrap()
            .overlay,
        zero.overlay
    );
}

#[tokio::test]
async fn changed_selection_invalidates_proposal_and_does_not_retarget_old_receipts() {
    let (r, actor, space, doc, zero) = h::fixture().await;
    let store = h::store(&r);
    let original_edit = h::edit(&zero, h::add(h::gap(&doc, 1), "personal"));
    let one = store
        .edit(actor, zero.overlay.overlay_id, original_edit.clone())
        .await
        .unwrap();
    let mut edit = a::edit(&doc);
    edit.title = "next".into();
    let next = r.compositions().save(actor, space, edit).await.unwrap();
    let migrations = MigrationStore::new(r.runtime_pool.clone());
    let p = migrations
        .propose(
            actor,
            one.overlay.overlay_id,
            ProposeMigration {
                request_id: Uuid::new_v4(),
                expected_overlay_revision: one.overlay.revision_id,
                expected_reading_view_revision: one.view.revision_id,
                target: next.reference,
                reason: "proposal".into(),
            },
        )
        .await
        .unwrap();
    let rel = relation(&r, actor, space).await;
    let two = store
        .select_relations(actor, one.overlay.overlay_id, select(&one, rel.reference))
        .await
        .unwrap();
    assert_eq!(
        store
            .edit(actor, zero.overlay.overlay_id, original_edit)
            .await
            .unwrap(),
        one
    );
    let result = migrations
        .decide(
            actor,
            one.overlay.overlay_id,
            DecideMigration {
                request_id: Uuid::new_v4(),
                proposal_id: p.proposal_id,
                expected_overlay_revision: two.overlay.revision_id,
                expected_reading_view_revision: two.view.revision_id,
                action: MigrationAction::Adopt {
                    groups: p
                        .groups
                        .iter()
                        .map(|g| GroupDecision::KeepUnplaced {
                            group_id: g.group_id,
                        })
                        .collect(),
                    merges: vec![],
                },
                reason: "wrong snapshot".into(),
            },
        )
        .await;
    assert!(matches!(result,Err(ContentError::Invalid(s)) if s=="stale_proposal"));
    assert_eq!(
        store
            .state(actor, one.overlay.overlay_id)
            .await
            .unwrap()
            .unwrap()
            .view,
        two.view
    );
}

#[tokio::test]
async fn stale_selection_never_discloses_a_current_view_with_hidden_evidence() {
    let (r, actor, space, doc, zero) = h::fixture().await;
    let mut dc = a::edit(&doc);
    dc.title = "next source".into();
    let next = r.compositions().save(actor, space, dc).await.unwrap();
    let migrations = MigrationStore::new(r.runtime_pool.clone());
    let proposal = migrations
        .propose(
            actor,
            zero.overlay.overlay_id,
            ProposeMigration {
                request_id: Uuid::new_v4(),
                expected_overlay_revision: zero.overlay.revision_id,
                expected_reading_view_revision: zero.view.revision_id,
                target: next.reference.clone(),
                reason: "before selection".into(),
            },
        )
        .await
        .unwrap();
    let (other, foreign) = r.seed_actor_space(true).await;
    r.grant(actor, foreign, false).await;
    let local = relation(&r, actor, space).await;
    let external = relation(&r, other, foreign).await;
    let store = h::store(&r);
    store
        .select_relations(
            actor,
            zero.overlay.overlay_id,
            select(&zero, external.reference),
        )
        .await
        .unwrap();
    r.revoke(actor, foreign).await;
    let result = store
        .select_relations(
            actor,
            zero.overlay.overlay_id,
            select(&zero, local.reference),
        )
        .await;
    assert!(
        matches!(result, Err(ContentError::NotFound)),
        "hidden current snapshot must not be returned in a conflict: {result:?}"
    );
    assert!(matches!(
        store
            .edit(
                actor,
                zero.overlay.overlay_id,
                h::edit(&zero, h::add(h::gap(&doc, 1), "stale edit"))
            )
            .await,
        Err(ContentError::NotFound)
    ));
    assert!(matches!(
        migrations
            .propose(
                actor,
                zero.overlay.overlay_id,
                ProposeMigration {
                    request_id: Uuid::new_v4(),
                    expected_overlay_revision: zero.overlay.revision_id,
                    expected_reading_view_revision: zero.view.revision_id,
                    target: next.reference,
                    reason: "stale proposal".into()
                }
            )
            .await,
        Err(ContentError::NotFound)
    ));
    assert!(matches!(
        migrations
            .decide(
                actor,
                zero.overlay.overlay_id,
                DecideMigration {
                    request_id: Uuid::new_v4(),
                    expected_overlay_revision: zero.overlay.revision_id,
                    expected_reading_view_revision: zero.view.revision_id,
                    proposal_id: proposal.proposal_id,
                    action: MigrationAction::Adopt {
                        groups: vec![],
                        merges: vec![]
                    },
                    reason: "stale decision".into()
                }
            )
            .await,
        Err(ContentError::NotFound)
    ));
}

mod support;
use learning_core::*;
use learning_db::{MigrationStore, QueryStore};
use support::{TestRig, assembly as a, reading as r};
use uuid::Uuid;

fn query(start: BlockRef, scope: ImpactScope) -> ImpactQuery {
    ImpactQuery {
        start: ImpactStart::Block(start),
        scope,
        families: vec![ImpactFamily::Structural, ImpactFamily::Necessary],
        max_depth: 3,
        limit: 50,
        work_limit: 4096,
        after: None,
    }
}

#[tokio::test]
async fn unsupported_family_is_an_explicit_error_before_storage_access() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://nobody:unused@127.0.0.1:1/unused")
        .unwrap();
    let actor = Principal {
        actor_id: Uuid::new_v4(),
    };
    let mut request = query(
        BlockRef {
            block_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
        },
        ImpactScope::Release {
            release_id: Uuid::new_v4(),
        },
    );
    request.families = vec![ImpactFamily::Semantic];
    assert!(matches!(QueryStore::new(pool).direct(actor, request).await,
        Err(ContentError::Invalid(code)) if code == "impact_family_not_implemented"));
}

#[tokio::test]
async fn release_preserves_both_nested_occurrences_and_deduplicates_consumer() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let block = rig
        .store
        .create(actor, space, support::command("shared"))
        .await
        .unwrap();
    let reference = BlockRef {
        block_id: block.block_id,
        revision_id: block.revision_id,
    };
    let child = rig
        .compositions()
        .save(actor, space, a::doc(vec![a::block(&block)]))
        .await
        .unwrap();
    let root = rig
        .compositions()
        .save(
            actor,
            space,
            a::doc(vec![a::child(&child), a::child(&child)]),
        )
        .await
        .unwrap();
    let second_root = rig
        .compositions()
        .save(actor, space, a::doc(vec![a::child(&child)]))
        .await
        .unwrap();
    let released = rig
        .releases()
        .publish(
            actor,
            space,
            a::publish(vec![a::root(&root, None), a::root(&second_root, None)]),
        )
        .await
        .unwrap();
    let result = QueryStore::new(rig.runtime_pool.clone())
        .direct(
            actor,
            query(
                reference.clone(),
                ImpactScope::Release {
                    release_id: released.release_id,
                },
            ),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.start_membership(), ImpactMembership::Displayed);
    assert!(matches!(result.status(), PageStatus::Complete));
    let group = result
        .consumers()
        .iter()
        .find(|g| g.consumer == ImpactNode::Composition(child.reference.clone()))
        .unwrap();
    assert_eq!(group.locations.len(), 3);
    let mut paths: Vec<_> = group
        .locations
        .iter()
        .map(|l| match l {
            ImpactLocation::Occurrence { path } => path.clone(),
            _ => panic!("expected occurrence"),
        })
        .collect();
    let mut expected = vec![
        vec![root.nodes[0].occurrence_id, child.nodes[0].occurrence_id],
        vec![root.nodes[1].occurrence_id, child.nodes[0].occurrence_id],
        vec![
            second_root.nodes[0].occurrence_id,
            child.nodes[0].occurrence_id,
        ],
    ];
    paths.sort();
    expected.sort();
    assert_eq!(paths, expected);
    assert_eq!(group.explanations.len(), 3);
    assert!(result.consumers().iter().all(|g| g.consumer
        != ImpactNode::Release {
            release_id: released.release_id
        }));
    let child_start: ImpactQuery = serde_json::from_value(serde_json::json!({
        "start":{"type":"composition","composition_id":child.reference.composition_id,"revision_id":child.reference.revision_id},
        "scope":{"type":"release","release_id":released.release_id}
    })).unwrap();
    let parents = QueryStore::new(rig.runtime_pool.clone())
        .direct(actor, child_start)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(parents.start_membership(), ImpactMembership::Context);
    let mut actual = parents
        .consumers()
        .iter()
        .flat_map(|g| {
            g.locations
                .iter()
                .map(move |l| (g.consumer.clone(), l.clone()))
        })
        .collect::<Vec<_>>();
    actual.sort();
    let mut expected = vec![
        (
            ImpactNode::Composition(root.reference.clone()),
            ImpactLocation::Occurrence {
                path: vec![root.nodes[0].occurrence_id],
            },
        ),
        (
            ImpactNode::Composition(root.reference.clone()),
            ImpactLocation::Occurrence {
                path: vec![root.nodes[1].occurrence_id],
            },
        ),
        (
            ImpactNode::Composition(second_root.reference.clone()),
            ImpactLocation::Occurrence {
                path: vec![second_root.nodes[0].occurrence_id],
            },
        ),
    ];
    expected.sort();
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn old_anchor_manifest_members_are_context_without_fabricated_citation() {
    let (rig, actor, space, old_doc, zero) = r::fixture().await;
    let reading = r::store(&rig);
    let one = reading
        .edit(
            actor,
            zero.overlay.overlay_id,
            r::edit(&zero, r::add(r::gap(&old_doc, 1), "personal")),
        )
        .await
        .unwrap();
    let old_group = reading
        .state(actor, one.overlay.overlay_id)
        .await
        .unwrap()
        .unwrap()
        .editable
        .unwrap()
        .groups[0]
        .group_id;
    let mut edit = a::edit(&old_doc);
    edit.nodes.clear();
    let new_doc = rig.compositions().save(actor, space, edit).await.unwrap();
    let migration = MigrationStore::new(rig.runtime_pool.clone());
    let proposed = migration
        .propose(
            actor,
            one.overlay.overlay_id,
            ProposeMigration {
                request_id: Uuid::new_v4(),
                expected_overlay_revision: one.overlay.revision_id,
                expected_reading_view_revision: one.view.revision_id,
                target: new_doc.reference.clone(),
                reason: "move".into(),
            },
        )
        .await
        .unwrap();
    let adopted = migration
        .decide(
            actor,
            one.overlay.overlay_id,
            DecideMigration {
                request_id: Uuid::new_v4(),
                proposal_id: proposed.proposal_id,
                expected_overlay_revision: one.overlay.revision_id,
                expected_reading_view_revision: one.view.revision_id,
                action: MigrationAction::Adopt {
                    groups: vec![GroupDecision::KeepUnplaced {
                        group_id: old_group,
                    }],
                    merges: vec![],
                },
                reason: "reviewed".into(),
            },
        )
        .await
        .unwrap()
        .adopted
        .unwrap();
    let release = rig
        .releases()
        .publish_evidence(
            actor,
            space,
            PublishEvidence {
                request_id: Uuid::new_v4(),
                roots: vec![a::root(&new_doc, None)],
                readings: vec![adopted.view.clone()],
                reason: "fixed reading".into(),
            },
        )
        .await
        .unwrap();
    let old_block = match &old_doc.nodes[0].target {
        NodeTarget::Block(r) => r.clone(),
        _ => panic!("expected block"),
    };
    let store = QueryStore::new(rig.runtime_pool.clone());
    let block_result = store
        .direct(
            actor,
            query(
                old_block,
                ImpactScope::Release {
                    release_id: release.release_id,
                },
            ),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(block_result.start_membership(), ImpactMembership::Context);
    assert!(matches!(block_result.status(), PageStatus::Complete));
    assert!(block_result.consumers().is_empty());
    let old_composition: ImpactQuery = serde_json::from_value(serde_json::json!({
        "start":{"type":"composition","composition_id":old_doc.reference.composition_id,"revision_id":old_doc.reference.revision_id},
        "scope":{"type":"release","release_id":release.release_id}
    })).unwrap();
    let composition_result = store.direct(actor, old_composition).await.unwrap().unwrap();
    assert_eq!(
        composition_result.start_membership(),
        ImpactMembership::Context
    );
    assert!(composition_result.consumers().is_empty());
}

#[tokio::test]
async fn visible_consumer_pages_are_atomic_and_use_visible_cursors() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let block = rig
        .store
        .create(actor, space, support::command("shared"))
        .await
        .unwrap();
    let reference = BlockRef {
        block_id: block.block_id,
        revision_id: block.revision_id,
    };
    let left = rig
        .compositions()
        .save(actor, space, a::doc(vec![a::block(&block)]))
        .await
        .unwrap();
    let right = rig
        .compositions()
        .save(actor, space, a::doc(vec![a::block(&block)]))
        .await
        .unwrap();
    let release = rig
        .releases()
        .publish(
            actor,
            space,
            a::publish(vec![a::root(&left, None), a::root(&right, None)]),
        )
        .await
        .unwrap();
    let store = QueryStore::new(rig.runtime_pool.clone());
    let mut request = query(
        reference,
        ImpactScope::Release {
            release_id: release.release_id,
        },
    );
    request.limit = 1;
    let first = store.direct(actor, request.clone()).await.unwrap().unwrap();
    assert_eq!(first.consumers().len(), 1);
    let cursor = match first.status() {
        PageStatus::Truncated { after } => after.clone(),
        other => panic!("expected visible next cursor: {other:?}"),
    };
    assert_eq!(cursor.consumer, first.consumers()[0].consumer);
    request.after = Some(cursor);
    let second = store.direct(actor, request.clone()).await.unwrap().unwrap();
    assert!(matches!(second.status(), PageStatus::Complete));
    assert_eq!(second.consumers().len(), 1);
    let mut consumers = vec![
        first.consumers()[0].consumer.clone(),
        second.consumers()[0].consumer.clone(),
    ];
    consumers.sort();
    let mut expected = vec![
        ImpactNode::Composition(left.reference),
        ImpactNode::Composition(right.reference),
    ];
    expected.sort();
    assert_eq!(consumers, expected);
    let (reader, _) = rig.seed_actor_space(false).await;
    rig.grant(reader, space, false).await;
    assert_eq!(
        store
            .direct(reader, request.clone())
            .await
            .unwrap()
            .unwrap()
            .consumers()
            .len(),
        1
    );
    rig.revoke(reader, space).await;
    assert!(store.direct(reader, request).await.unwrap().is_none());
}

#[tokio::test]
async fn placed_and_unplaced_personal_items_have_distinct_locations() {
    let (rig, actor, space, doc, zero) = r::fixture().await;
    let reading = r::store(&rig);
    let one = reading
        .edit(
            actor,
            zero.overlay.overlay_id,
            r::edit(&zero, r::add(r::gap(&doc, 1), "unplaced")),
        )
        .await
        .unwrap();
    let two = reading
        .edit(
            actor,
            one.overlay.overlay_id,
            r::edit(&one, r::add(r::gap(&doc, 2), "placed")),
        )
        .await
        .unwrap();
    let before = reading
        .state(actor, two.overlay.overlay_id)
        .await
        .unwrap()
        .unwrap()
        .editable
        .unwrap();
    assert_eq!(before.groups.len(), 2);
    let mut next_command = a::edit(&doc);
    next_command.nodes.clear();
    let next = rig
        .compositions()
        .save(actor, space, next_command)
        .await
        .unwrap();
    let migration = MigrationStore::new(rig.runtime_pool.clone());
    let proposed = migration
        .propose(
            actor,
            two.overlay.overlay_id,
            ProposeMigration {
                request_id: Uuid::new_v4(),
                expected_overlay_revision: two.overlay.revision_id,
                expected_reading_view_revision: two.view.revision_id,
                target: next.reference.clone(),
                reason: "move".into(),
            },
        )
        .await
        .unwrap();
    let unplaced_group = before
        .groups
        .iter()
        .find(|g| g.placements[0].block == before.groups[0].placements[0].block)
        .unwrap();
    let placed_group = before
        .groups
        .iter()
        .find(|g| g.group_id != unplaced_group.group_id)
        .unwrap();
    assert_eq!(proposed.groups.len(), 2);
    let adopted = migration
        .decide(
            actor,
            two.overlay.overlay_id,
            DecideMigration {
                request_id: Uuid::new_v4(),
                proposal_id: proposed.proposal_id,
                expected_overlay_revision: two.overlay.revision_id,
                expected_reading_view_revision: two.view.revision_id,
                action: MigrationAction::Adopt {
                    groups: vec![
                        GroupDecision::KeepUnplaced {
                            group_id: unplaced_group.group_id,
                        },
                        GroupDecision::Manual {
                            group_id: placed_group.group_id,
                            anchor: r::gap(&next, 0),
                        },
                    ],
                    merges: vec![],
                },
                reason: "reviewed".into(),
            },
        )
        .await
        .unwrap()
        .adopted
        .unwrap();
    let store = QueryStore::new(rig.runtime_pool.clone());
    for (group, expected_unplaced) in [(unplaced_group, true), (placed_group, false)] {
        let block = group.placements[0].block.clone();
        let result = store
            .direct(
                actor,
                query(
                    block.clone(),
                    ImpactScope::Reading {
                        view: adopted.view.clone(),
                        mode: ReadingMode::Personal,
                    },
                ),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.start_membership(), ImpactMembership::Displayed);
        let reading_group = result
            .consumers()
            .iter()
            .find(|g| matches!(g.consumer, ImpactNode::Reading { .. }))
            .unwrap();
        assert_eq!(reading_group.locations.len(), 1);
        assert!(match (&reading_group.locations[0], expected_unplaced) {
            (ImpactLocation::Unplaced { placement_id, .. }, true)
            | (ImpactLocation::Placement { placement_id, .. }, false) =>
                *placement_id == group.placements[0].placement_id,
            _ => false,
        });
        assert!(
            store
                .direct(
                    actor,
                    query(
                        block,
                        ImpactScope::Reading {
                            view: adopted.view.clone(),
                            mode: ReadingMode::Original
                        }
                    )
                )
                .await
                .unwrap()
                .is_none()
        );
    }
}

#[tokio::test]
async fn necessary_source_is_exact_and_hidden_dense_candidates_do_not_charge_visible_work() {
    use support::references as refs;
    let rig = TestRig::from_env().await;
    let (actor, space, _, evidence, conclusion) = refs::fixture(&rig, false).await;
    let root = refs::seed_composition(&rig, actor, space, &conclusion).await;
    let root_revision = rig
        .compositions()
        .read_versioned(actor, root.clone())
        .await
        .unwrap()
        .unwrap()
        .compositions
        .into_iter()
        .find(|c| c.reference == root)
        .unwrap();
    let released = rig
        .releases()
        .publish_evidence(
            actor,
            space,
            PublishEvidence {
                request_id: Uuid::new_v4(),
                roots: vec![a::root(&root_revision, None)],
                readings: vec![],
                reason: "fixed evidence".into(),
            },
        )
        .await
        .unwrap();
    let store = QueryStore::new(rig.runtime_pool.clone());
    let mut request = query(
        evidence.clone(),
        ImpactScope::Release {
            release_id: released.release_id,
        },
    );
    request.families = vec![ImpactFamily::Necessary];
    // One start node, one consumer node, one edge, and one explanation path.
    // First prove the exact visible budget boundary without hidden candidates.
    request.work_limit = 3;
    let insufficient = store.direct(actor, request.clone()).await.unwrap().unwrap();
    assert!(matches!(insufficient.status(), PageStatus::BudgetExceeded));
    request.work_limit = 4;
    let baseline = store.direct(actor, request.clone()).await.unwrap().unwrap();
    assert!(matches!(baseline.status(), PageStatus::Complete));
    assert_eq!(baseline.consumers().len(), 1);
    // The dependency target is a scope context member, while the direct source
    // is a displayed block. Hidden sources exercise the reverse index filter.
    let (_, hidden_space) = rig.seed_actor_space(true).await;
    rig.grant(actor, hidden_space, true).await;
    let mut hidden_ids = vec![];
    for _ in 0..64 {
        let mut tx = rig.runtime_pool.begin().await.unwrap();
        let hidden = refs::seed_content(
            &mut tx,
            actor,
            hidden_space,
            refs::v2(evidence.clone(), false),
            None,
        )
        .await;
        tx.commit().await.unwrap();
        hidden_ids.push(hidden);
    }
    rig.revoke(actor, hidden_space).await;
    let result = store.direct(actor, request.clone()).await.unwrap().unwrap();
    assert!(matches!(result.status(), PageStatus::Complete));
    assert_eq!(
        serde_json::to_value(&result).unwrap(),
        serde_json::to_value(&baseline).unwrap()
    );
    request.work_limit = 3;
    let dense_insufficient = store.direct(actor, request).await.unwrap().unwrap();
    assert_eq!(
        serde_json::to_value(&dense_insufficient).unwrap(),
        serde_json::to_value(&insufficient).unwrap()
    );
    assert_eq!(result.start_membership(), ImpactMembership::Context);
    assert_eq!(result.consumers().len(), 1);
    let group = &result.consumers()[0];
    assert_eq!(group.consumer, ImpactNode::Block(conclusion));
    assert_eq!(group.explanations.len(), 1);
    assert_eq!(
        group.explanations[0].steps[0].dependency_role,
        Some(DependencyRole::Basis)
    );
    assert_eq!(group.explanations[0].steps[0].dependency_position, Some(0));
    let rendered = serde_json::to_string(&result).unwrap();
    assert!(
        hidden_ids
            .iter()
            .all(|r| !rendered.contains(&r.block_id.to_string()))
    );
}

#[tokio::test]
async fn reading_modes_and_revocation_gate_original_but_keep_personal() {
    let rig = TestRig::from_env().await;
    let (actor, personal_space) = rig.seed_actor_space(true).await;
    let (_, original_space) = rig.seed_actor_space(true).await;
    rig.grant(actor, original_space, true).await;
    let original = rig
        .store
        .create(actor, original_space, support::command("original"))
        .await
        .unwrap();
    let root = rig
        .compositions()
        .save(actor, original_space, a::doc(vec![a::block(&original)]))
        .await
        .unwrap();
    let saved = r::store(&rig)
        .create(actor, personal_space, r::create(root.reference.clone()))
        .await
        .unwrap();
    let saved = r::store(&rig)
        .edit(
            actor,
            saved.overlay.overlay_id,
            r::edit(&saved, r::add(r::gap(&root, 1), "mine")),
        )
        .await
        .unwrap();
    let placement = r::store(&rig)
        .state(actor, saved.overlay.overlay_id)
        .await
        .unwrap()
        .unwrap()
        .editable
        .unwrap()
        .groups[0]
        .placements[0]
        .clone();
    let store = QueryStore::new(rig.runtime_pool.clone());
    let original_ref = BlockRef {
        block_id: original.block_id,
        revision_id: original.revision_id,
    };
    let personal_query = || {
        query(
            placement.block.clone(),
            ImpactScope::Reading {
                view: saved.view.clone(),
                mode: ReadingMode::Personal,
            },
        )
    };
    let personal = store
        .direct(actor, personal_query())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(personal.start_membership(), ImpactMembership::Displayed);
    assert_eq!(
        personal
            .consumers()
            .iter()
            .filter(|g| matches!(g.consumer, ImpactNode::Reading { .. }))
            .count(),
        1
    );
    assert!(
        store
            .direct(
                actor,
                query(
                    original_ref.clone(),
                    ImpactScope::Reading {
                        view: saved.view.clone(),
                        mode: ReadingMode::Personal
                    }
                )
            )
            .await
            .unwrap()
            .is_none()
    );
    rig.revoke(actor, original_space).await;
    assert!(
        store
            .direct(
                actor,
                query(
                    original_ref,
                    ImpactScope::Reading {
                        view: saved.view.clone(),
                        mode: ReadingMode::Fused
                    }
                )
            )
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .direct(actor, personal_query())
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn readable_saved_base_composition_directly_contains_fixed_reading() {
    let rig = TestRig::from_env().await;
    let (actor, personal_space) = rig.seed_actor_space(true).await;
    let (_, source_space) = rig.seed_actor_space(true).await;
    rig.grant(actor, source_space, true).await;
    let original = rig
        .store
        .create(actor, source_space, support::command("original"))
        .await
        .unwrap();
    let base = rig
        .compositions()
        .save(actor, source_space, a::doc(vec![a::block(&original)]))
        .await
        .unwrap();
    let saved = r::store(&rig)
        .create(actor, personal_space, r::create(base.reference.clone()))
        .await
        .unwrap();
    let saved = r::store(&rig)
        .edit(
            actor,
            saved.overlay.overlay_id,
            r::edit(&saved, r::add(r::gap(&base, 1), "mine")),
        )
        .await
        .unwrap();
    let personal_block = r::store(&rig)
        .state(actor, saved.overlay.overlay_id)
        .await
        .unwrap()
        .unwrap()
        .editable
        .unwrap()
        .groups[0]
        .placements[0]
        .block
        .clone();
    let store = QueryStore::new(rig.runtime_pool.clone());
    let composition_query = |mode| ImpactQuery {
        start: ImpactStart::Composition(base.reference.clone()),
        scope: ImpactScope::Reading {
            view: saved.view.clone(),
            mode,
        },
        families: vec![ImpactFamily::Structural],
        max_depth: 3,
        limit: 50,
        work_limit: 4096,
        after: None,
    };
    for mode in [ReadingMode::Original, ReadingMode::Fused] {
        let result = store
            .direct(actor, composition_query(mode))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.start_membership(), ImpactMembership::Context);
        assert!(matches!(result.status(), PageStatus::Complete));
        assert_eq!(result.consumers().len(), 1);
        let group = &result.consumers()[0];
        assert_eq!(
            group.consumer,
            ImpactNode::Reading {
                view_id: saved.view.view_id,
                revision_id: saved.view.revision_id,
            }
        );
        assert!(group.locations.is_empty());
        assert_eq!(group.explanations.len(), 1);
        let step = &group.explanations[0].steps[0];
        assert_eq!(step.from, ImpactNode::Composition(base.reference.clone()));
        assert_eq!(step.to, group.consumer);
        assert_eq!(step.family, ImpactFamily::Structural);
        assert_eq!(step.provenance, ImpactProvenance::FixedReadingSelection);
        assert!(step.location.is_none());
    }
    assert!(
        store
            .direct(actor, composition_query(ReadingMode::Personal))
            .await
            .unwrap()
            .is_none()
    );
    rig.revoke(actor, source_space).await;
    for mode in [ReadingMode::Original, ReadingMode::Fused] {
        assert!(
            store
                .direct(actor, composition_query(mode))
                .await
                .unwrap()
                .is_none()
        );
    }
    assert!(
        store
            .direct(
                actor,
                query(
                    personal_block,
                    ImpactScope::Reading {
                        view: saved.view.clone(),
                        mode: ReadingMode::Personal,
                    },
                ),
            )
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn release_root_and_fixed_reading_share_one_identical_occurrence_path() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let block = rig
        .store
        .create(actor, space, support::command("shared"))
        .await
        .unwrap();
    let root = rig
        .compositions()
        .save(actor, space, a::doc(vec![a::block(&block)]))
        .await
        .unwrap();
    let reading = r::store(&rig)
        .create(actor, space, r::create(root.reference.clone()))
        .await
        .unwrap();
    let release = rig
        .releases()
        .publish_evidence(
            actor,
            space,
            PublishEvidence {
                request_id: Uuid::new_v4(),
                roots: vec![a::root(&root, None)],
                readings: vec![reading.view.clone()],
                reason: "root and reading".into(),
            },
        )
        .await
        .unwrap();
    let mut request = query(
        BlockRef {
            block_id: block.block_id,
            revision_id: block.revision_id,
        },
        ImpactScope::Release {
            release_id: release.release_id,
        },
    );
    request.families = vec![ImpactFamily::Structural];
    // Start, composition, reading; two edges and two explanation paths.
    request.work_limit = 7;
    let result = QueryStore::new(rig.runtime_pool.clone())
        .direct(actor, request)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result.status(), PageStatus::Complete));
    assert_eq!(result.consumers().len(), 2);
    let composition = result
        .consumers()
        .iter()
        .find(|g| g.consumer == ImpactNode::Composition(root.reference.clone()))
        .unwrap();
    assert_eq!(
        composition.locations,
        vec![ImpactLocation::Occurrence {
            path: vec![root.nodes[0].occurrence_id],
        }]
    );
    assert_eq!(composition.explanations.len(), 1);
    let reading = result
        .consumers()
        .iter()
        .find(|g| matches!(g.consumer, ImpactNode::Reading { .. }))
        .unwrap();
    assert_eq!(reading.explanations.len(), 1);
}

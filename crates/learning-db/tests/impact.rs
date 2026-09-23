#[path = "support/relation_store.rs"]
mod relations;
mod support;

use learning_core::*;
use learning_db::{QueryStore, RelationStore, ReviewStore};
use support::{TestRig, assembly as a, reading as r};
use uuid::Uuid;

#[tokio::test]
async fn nested_occurrences_keep_two_explanations_without_cross_product() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let block = rig
        .store
        .create(actor, space, support::command("H old"))
        .await
        .unwrap();
    let h = BlockRef {
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
    let release = rig
        .releases()
        .publish(actor, space, a::publish(vec![a::root(&root, None)]))
        .await
        .unwrap();
    let query = ImpactQuery {
        start: ImpactStart::Block(h.clone()),
        scope: ImpactScope::Release {
            release_id: release.release_id,
        },
        families: vec![ImpactFamily::Structural],
        max_depth: 3,
        limit: 50,
        work_limit: 4096,
        after: None,
    };
    let result = QueryStore::new(rig.runtime_pool.clone())
        .traverse(actor, query)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result.status(), PageStatus::Complete));
    let group = result
        .consumers()
        .iter()
        .find(|g| g.consumer == ImpactNode::Composition(root.reference.clone()))
        .unwrap();
    assert_eq!(group.explanations.len(), 2);
    let mut actual = group
        .explanations
        .iter()
        .map(|explanation| {
            assert_eq!(explanation.steps.len(), 2);
            assert_eq!(explanation.steps[0].from, ImpactNode::Block(h.clone()));
            assert_eq!(
                explanation.steps[0].to,
                ImpactNode::Composition(child.reference.clone())
            );
            assert_eq!(
                explanation.steps[1].to,
                ImpactNode::Composition(root.reference.clone())
            );
            assert_eq!(
                explanation.steps[0].reason,
                ImpactReason::ReferencesOldRevision
            );
            let first = match &explanation.steps[0].location {
                Some(ImpactLocation::Occurrence { path }) => path.clone(),
                _ => panic!("expected exact occurrence path"),
            };
            let second = match &explanation.steps[1].location {
                Some(ImpactLocation::Occurrence { path }) => path.clone(),
                _ => panic!("expected exact parent path"),
            };
            (first, second)
        })
        .collect::<Vec<_>>();
    actual.sort();
    let mut expected = vec![
        (
            vec![root.nodes[0].occurrence_id, child.nodes[0].occurrence_id],
            vec![root.nodes[0].occurrence_id],
        ),
        (
            vec![root.nodes[1].occurrence_id, child.nodes[0].occurrence_id],
            vec![root.nodes[1].occurrence_id],
        ),
    ];
    expected.sort();
    assert_eq!(actual, expected);
    let mut locations = group.locations.clone();
    locations.sort();
    let mut expected_locations = vec![
        ImpactLocation::Occurrence {
            path: vec![root.nodes[0].occurrence_id, child.nodes[0].occurrence_id],
        },
        ImpactLocation::Occurrence {
            path: vec![root.nodes[1].occurrence_id, child.nodes[0].occurrence_id],
        },
    ];
    expected_locations.sort();
    assert_eq!(locations, expected_locations);
    let release_group = result
        .consumers()
        .iter()
        .find(|g| {
            g.consumer
                == ImpactNode::Release {
                    release_id: release.release_id,
                }
        })
        .unwrap();
    assert_eq!(release_group.explanations.len(), 2);
    let mut released_paths = release_group
        .explanations
        .iter()
        .map(|e| {
            assert_eq!(e.steps.len(), 3);
            assert_eq!(e.steps[0].from, ImpactNode::Block(h.clone()));
            assert_eq!(
                e.steps[0].to,
                ImpactNode::Composition(child.reference.clone())
            );
            assert_eq!(
                e.steps[1].to,
                ImpactNode::Composition(root.reference.clone())
            );
            assert_eq!(
                e.steps[2].to,
                ImpactNode::Release {
                    release_id: release.release_id
                }
            );
            assert_eq!(e.steps[2].location, None);
            let first = match &e.steps[0].location {
                Some(ImpactLocation::Occurrence { path }) => path.clone(),
                _ => panic!("child path"),
            };
            let second = match &e.steps[1].location {
                Some(ImpactLocation::Occurrence { path }) => path.clone(),
                _ => panic!("root path"),
            };
            (first, second)
        })
        .collect::<Vec<_>>();
    released_paths.sort();
    assert_eq!(released_paths, expected);
    let mut release_locations = release_group.locations.clone();
    release_locations.sort();
    assert_eq!(release_locations, expected_locations);
}

#[tokio::test]
async fn released_personal_block_reaches_release_only_through_fixed_reading() {
    let (rig, actor, space, doc, zero) = r::fixture().await;
    let reading = r::store(&rig);
    let saved = reading
        .edit(
            actor,
            zero.overlay.overlay_id,
            r::edit(&zero, r::add(r::gap(&doc, 1), "personal H")),
        )
        .await
        .unwrap();
    let editable = reading
        .state(actor, saved.overlay.overlay_id)
        .await
        .unwrap()
        .unwrap()
        .editable
        .unwrap();
    let placement = &editable.groups[0].placements[0];
    let h = placement.block.clone();
    let location = ImpactLocation::Placement {
        view_id: saved.view.view_id,
        revision_id: saved.view.revision_id,
        placement_id: placement.placement_id,
    };
    let release = rig
        .releases()
        .publish_evidence(
            actor,
            space,
            PublishEvidence {
                request_id: Uuid::new_v4(),
                roots: vec![a::root(&doc, None)],
                readings: vec![saved.view.clone()],
                reason: "fixed personal reading".into(),
            },
        )
        .await
        .unwrap();
    let result = QueryStore::new(rig.runtime_pool.clone())
        .traverse(
            actor,
            ImpactQuery {
                start: ImpactStart::Block(h.clone()),
                scope: ImpactScope::Release {
                    release_id: release.release_id,
                },
                families: vec![ImpactFamily::Structural],
                max_depth: 2,
                limit: 50,
                work_limit: 4096,
                after: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result.status(), PageStatus::Complete));
    assert_eq!(result.start_membership(), ImpactMembership::Displayed);
    assert_eq!(result.consumers().len(), 2);
    let reading_node = ImpactNode::Reading {
        view_id: saved.view.view_id,
        revision_id: saved.view.revision_id,
    };
    let reading_group = result
        .consumers()
        .iter()
        .find(|g| g.consumer == reading_node)
        .unwrap();
    assert_eq!(reading_group.locations, vec![location.clone()]);
    assert_eq!(reading_group.explanations.len(), 1);
    assert_eq!(
        reading_group.explanations[0].steps[0].from,
        ImpactNode::Block(h)
    );
    assert_eq!(reading_group.explanations[0].steps[0].to, reading_node);
    let release_group = result
        .consumers()
        .iter()
        .find(|g| {
            g.consumer
                == ImpactNode::Release {
                    release_id: release.release_id,
                }
        })
        .unwrap();
    assert_eq!(release_group.locations, vec![location]);
    assert_eq!(release_group.explanations.len(), 1);
    let steps = &release_group.explanations[0].steps;
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].family, ImpactFamily::Structural);
    assert_eq!(steps[1].family, ImpactFamily::Structural);
    assert_eq!(steps[1].from, reading_node);
    assert_eq!(
        steps[1].to,
        ImpactNode::Release {
            release_id: release.release_id
        }
    );
    assert_eq!(steps[1].location, None);
}

#[tokio::test]
async fn necessary_diamond_preserves_both_visible_explanations_to_one_consumer() {
    use support::references as refs;
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let (e_id, e_revision) = rig.seed_block(actor, space).await;
    let e = BlockRef {
        block_id: e_id,
        revision_id: e_revision,
    };
    let mut tx = rig.runtime_pool.begin().await.unwrap();
    let left = refs::seed_content(&mut tx, actor, space, refs::v2(e.clone(), false), None).await;
    let right = refs::seed_content(&mut tx, actor, space, refs::v2(e.clone(), false), None).await;
    let conclusion = refs::seed_content(
        &mut tx,
        actor,
        space,
        ContentDraft::V2(ContentV2 {
            intent: Intent::Note,
            language: "en".into(),
            title: "two independent paths".into(),
            body: BodyV2::Text(support::command("C").draft.payload),
            basis_refs: vec![
                ExactRef::Block(left.clone()),
                ExactRef::Block(right.clone()),
            ],
            requires_context: vec![],
            source_run: None,
        }),
        None,
    )
    .await;
    tx.commit().await.unwrap();
    let doc = rig
        .compositions()
        .save(
            actor,
            space,
            a::doc(vec![
                NodeTarget::Block(e.clone()),
                NodeTarget::Block(left.clone()),
                NodeTarget::Block(right.clone()),
                NodeTarget::Block(conclusion.clone()),
            ]),
        )
        .await
        .unwrap();
    let release = rig
        .releases()
        .publish_evidence(
            actor,
            space,
            PublishEvidence {
                request_id: Uuid::new_v4(),
                roots: vec![a::root(&doc, None)],
                readings: vec![],
                reason: "fixed necessary diamond".into(),
            },
        )
        .await
        .unwrap();
    let result = QueryStore::new(rig.runtime_pool.clone())
        .traverse(
            actor,
            ImpactQuery {
                start: ImpactStart::Block(e.clone()),
                scope: ImpactScope::Release {
                    release_id: release.release_id,
                },
                families: vec![ImpactFamily::Necessary],
                max_depth: 2,
                limit: 50,
                work_limit: 4096,
                after: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result.status(), PageStatus::Complete));
    assert_eq!(result.consumers().len(), 3);
    let c = result
        .consumers()
        .iter()
        .find(|g| g.consumer == ImpactNode::Block(conclusion.clone()))
        .unwrap();
    assert_eq!(c.explanations.len(), 2);
    assert!(c.locations.is_empty());
    let mut middle = c
        .explanations
        .iter()
        .map(|path| {
            assert_eq!(path.steps.len(), 2);
            assert_eq!(path.steps[0].from, ImpactNode::Block(e.clone()));
            assert_eq!(path.steps[1].to, ImpactNode::Block(conclusion.clone()));
            assert!(
                path.steps
                    .iter()
                    .all(|step| step.family == ImpactFamily::Necessary
                        && step.reason == ImpactReason::RequiresExactRevision
                        && step.dependency_role == Some(DependencyRole::Basis))
            );
            assert_eq!(path.steps[0].dependency_position, Some(0));
            let second_position = if path.steps[0].to == ImpactNode::Block(left.clone()) {
                0
            } else if path.steps[0].to == ImpactNode::Block(right.clone()) {
                1
            } else {
                panic!("unexpected middle consumer")
            };
            assert_eq!(path.steps[1].dependency_position, Some(second_position));
            assert_eq!(path.steps[0].to, path.steps[1].from);
            path.steps[0].to.clone()
        })
        .collect::<Vec<_>>();
    middle.sort();
    let mut expected = vec![
        ImpactNode::Block(left.clone()),
        ImpactNode::Block(right.clone()),
    ];
    expected.sort();
    assert_eq!(middle, expected);

    // A page boundary is a whole exact consumer, including both paths to C.
    let store = QueryStore::new(rig.runtime_pool.clone());
    let mut after = None;
    let mut paged = vec![];
    for index in 0..3 {
        let page = store
            .traverse(
                actor,
                ImpactQuery {
                    start: ImpactStart::Block(e.clone()),
                    scope: ImpactScope::Release {
                        release_id: release.release_id,
                    },
                    families: vec![ImpactFamily::Necessary],
                    max_depth: 2,
                    limit: 1,
                    work_limit: 4096,
                    after,
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(page.consumers().len(), 1);
        assert!(serde_json::to_vec(&page).unwrap().len() <= MAX_PROJECTION_BYTES);
        let group = &page.consumers()[0];
        if group.consumer == ImpactNode::Block(conclusion.clone()) {
            assert_eq!(group.explanations.len(), 2);
        }
        paged.push(group.consumer.clone());
        after = match page.status() {
            PageStatus::Truncated { after } if index < 2 => Some(after.clone()),
            PageStatus::Complete if index == 2 => None,
            other => panic!("unexpected page status {other:?}"),
        };
    }
    paged.sort();
    let mut expected = vec![
        ImpactNode::Block(left),
        ImpactNode::Block(right),
        ImpactNode::Block(conclusion),
    ];
    expected.sort();
    assert_eq!(paged, expected);
    let exhausted = store
        .traverse(
            actor,
            ImpactQuery {
                start: ImpactStart::Block(e),
                scope: ImpactScope::Release {
                    release_id: release.release_id,
                },
                families: vec![ImpactFamily::Necessary],
                max_depth: 2,
                limit: 1,
                work_limit: 1,
                after: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(exhausted.status(), PageStatus::BudgetExceeded));
    assert!(exhausted.consumers().is_empty());
}

#[tokio::test]
async fn hidden_selected_relation_cannot_bridge_two_still_visible_blocks() {
    let (rig, actor, _space, doc, zero) = r::fixture().await;
    let e = match &doc.nodes[0].target {
        NodeTarget::Block(b) => b.clone(),
        _ => panic!("E"),
    };
    let h = match &doc.nodes[1].target {
        NodeTarget::Block(b) => b.clone(),
        _ => panic!("H"),
    };
    let (_, relation_space) = rig.seed_actor_space(true).await;
    rig.grant(actor, relation_space, true).await;
    let relation = RelationStore::new(rig.runtime_pool.clone())
        .save(actor, relations::save(relation_space, e.clone(), h.clone()))
        .await
        .unwrap();
    let selected = r::store(&rig)
        .select_relations(
            actor,
            zero.overlay.overlay_id,
            SelectRelations {
                request_id: Uuid::new_v4(),
                expected_overlay_revision: zero.overlay.revision_id,
                expected_reading_view_revision: zero.view.revision_id,
                selections: vec![RelationSelection {
                    relation: relation.reference.clone(),
                    review: None,
                }],
                epistemic_reviews: vec![],
                reason: "fixed third-space relation".into(),
            },
        )
        .await
        .unwrap();
    let query = ImpactQuery {
        start: ImpactStart::Block(e.clone()),
        scope: ImpactScope::Reading {
            view: selected.view.clone(),
            mode: ReadingMode::Fused,
        },
        families: vec![ImpactFamily::Semantic],
        max_depth: 2,
        limit: 50,
        work_limit: 4096,
        after: None,
    };
    let store = QueryStore::new(rig.runtime_pool.clone());
    let before = store.traverse(actor, query.clone()).await.unwrap().unwrap();
    assert!(matches!(before.status(), PageStatus::Complete));
    let relation_node = ImpactNode::Relation(relation.reference.clone());
    let relation_group = before
        .consumers()
        .iter()
        .find(|g| g.consumer == relation_node)
        .unwrap();
    assert!(
        relation_group
            .explanations
            .iter()
            .any(|path| matches!(&path.steps[..], [step]
        if step.from == ImpactNode::Block(e.clone()) && step.to == relation_node
           && step.family == ImpactFamily::Semantic && step.reason == ImpactReason::SuggestReview
           && step.relation_type == Some(RelationType::Supports)
           && step.direction == Some(TraversalDirection::SavedForward)
           && step.provenance == ImpactProvenance::FixedReadingSelection))
    );
    let h_group = before
        .consumers()
        .iter()
        .find(|g| g.consumer == ImpactNode::Block(h.clone()))
        .unwrap();
    assert!(
        h_group
            .explanations
            .iter()
            .any(|path| matches!(&path.steps[..], [first, second]
        if first.to == relation_node && second.from == relation_node
           && second.to == ImpactNode::Block(h.clone())
           && first.provenance == ImpactProvenance::FixedReadingSelection
           && second.family == ImpactFamily::Semantic
           && second.reason == ImpactReason::SuggestReview
           && second.relation_type == Some(RelationType::Supports)
           && second.direction == Some(TraversalDirection::SavedForward)
           && second.provenance == ImpactProvenance::FixedReadingSelection))
    );
    let mut provenances = h_group
        .explanations
        .iter()
        .map(|path| {
            assert_eq!(path.steps.len(), 2);
            assert_eq!(path.steps[0].provenance, path.steps[1].provenance);
            path.steps[0].provenance
        })
        .collect::<Vec<_>>();
    provenances.sort_by_key(|source| format!("{source:?}"));
    assert_eq!(
        provenances,
        vec![
            ImpactProvenance::DynamicWorking,
            ImpactProvenance::FixedReadingSelection,
        ]
    );
    let relation_start = store
        .traverse(
            actor,
            ImpactQuery {
                start: ImpactStart::Relation(relation.reference.clone()),
                max_depth: 1,
                ..query.clone()
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(relation_start.status(), PageStatus::Complete));
    assert_eq!(relation_start.consumers().len(), 2);
    for endpoint in [e.clone(), h.clone()] {
        let endpoint_group = relation_start
            .consumers()
            .iter()
            .find(|group| group.consumer == ImpactNode::Block(endpoint.clone()))
            .unwrap();
        assert_eq!(endpoint_group.explanations.len(), 2);
        let mut sources = endpoint_group
            .explanations
            .iter()
            .map(|path| {
                let [step] = &path.steps[..] else {
                    panic!("relation start must give one hop per source")
                };
                assert_eq!(step.from, relation_node);
                assert_eq!(step.to, ImpactNode::Block(endpoint.clone()));
                assert_eq!(step.family, ImpactFamily::Semantic);
                step.provenance
            })
            .collect::<Vec<_>>();
        sources.sort_by_key(|source| format!("{source:?}"));
        assert_eq!(
            sources,
            vec![
                ImpactProvenance::DynamicWorking,
                ImpactProvenance::FixedReadingSelection,
            ]
        );
    }
    let first_page = store
        .traverse(
            actor,
            ImpactQuery {
                limit: 1,
                ..query.clone()
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first_page.consumers().len(), 1);
    let PageStatus::Truncated { after: cursor } = first_page.status() else {
        panic!("expected pre-revocation cursor")
    };
    rig.revoke(actor, relation_space).await;
    let after = store.traverse(actor, query.clone()).await.unwrap().unwrap();
    assert!(matches!(after.status(), PageStatus::Complete));
    assert_eq!(after.start_membership(), ImpactMembership::Displayed);
    assert!(after.consumers().is_empty());
    let resumed = store
        .traverse(
            actor,
            ImpactQuery {
                limit: 1,
                after: Some(cursor.clone()),
                ..query.clone()
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(resumed.status(), PageStatus::Complete));
    assert!(resumed.consumers().is_empty());
    rig.grant(actor, relation_space, true).await;
    let restored = store
        .traverse(
            actor,
            ImpactQuery {
                limit: 1,
                after: Some(cursor.clone()),
                ..query
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(restored.status(), PageStatus::Complete));
    assert_eq!(restored.consumers().len(), 1);
    assert_eq!(restored.consumers()[0].consumer, ImpactNode::Block(h));
}

#[tokio::test]
async fn context_only_relation_is_not_a_semantic_bridge() {
    use support::references as refs;
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let e = rig
        .store
        .create(actor, space, support::command("E"))
        .await
        .unwrap();
    let h = rig
        .store
        .create(actor, space, support::command("H"))
        .await
        .unwrap();
    let e_ref = relations::exact(&e);
    let h_ref = relations::exact(&h);
    let relation = RelationStore::new(rig.runtime_pool.clone())
        .save(actor, relations::save(space, e_ref.clone(), h_ref.clone()))
        .await
        .unwrap();
    let mut tx = rig.runtime_pool.begin().await.unwrap();
    let c = refs::seed_content(
        &mut tx,
        actor,
        space,
        ContentDraft::V2(ContentV2 {
            intent: Intent::Note,
            language: "en".into(),
            title: "mentions relation but does not select it".into(),
            body: BodyV2::Text(support::command("C").draft.payload),
            basis_refs: vec![ExactRef::Relation(relation.reference.clone())],
            requires_context: vec![],
            source_run: None,
        }),
        None,
    )
    .await;
    tx.commit().await.unwrap();
    let doc = rig
        .compositions()
        .save(
            actor,
            space,
            a::doc(vec![
                NodeTarget::Block(e_ref.clone()),
                NodeTarget::Block(h_ref.clone()),
                NodeTarget::Block(c.clone()),
            ]),
        )
        .await
        .unwrap();
    let release = rig
        .releases()
        .publish_evidence(
            actor,
            space,
            PublishEvidence {
                request_id: Uuid::new_v4(),
                roots: vec![a::root(&doc, None)],
                readings: vec![],
                reason: "context only".into(),
            },
        )
        .await
        .unwrap();
    let store = QueryStore::new(rig.runtime_pool.clone());
    let scope = ImpactScope::Release {
        release_id: release.release_id,
    };
    let relation_only = store
        .traverse(
            actor,
            ImpactQuery {
                start: ImpactStart::Relation(relation.reference.clone()),
                scope: scope.clone(),
                families: vec![ImpactFamily::Semantic],
                max_depth: 2,
                limit: 50,
                work_limit: 4096,
                after: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(relation_only.start_membership(), ImpactMembership::Context);
    assert!(matches!(relation_only.status(), PageStatus::Complete));
    assert!(relation_only.consumers().is_empty());
    let not_selected = store
        .traverse(
            actor,
            ImpactQuery {
                start: ImpactStart::Relation(relation.reference.clone()),
                scope: scope.clone(),
                families: vec![ImpactFamily::ReviewSelection],
                max_depth: 1,
                limit: 50,
                work_limit: 4096,
                after: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(not_selected.status(), PageStatus::Complete));
    assert!(not_selected.consumers().is_empty());
    let mixed = store
        .traverse(
            actor,
            ImpactQuery {
                start: ImpactStart::Block(e_ref),
                scope,
                families: vec![ImpactFamily::Necessary, ImpactFamily::Semantic],
                max_depth: 2,
                limit: 50,
                work_limit: 4096,
                after: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(mixed.status(), PageStatus::Complete));
    let relation_group = mixed
        .consumers()
        .iter()
        .find(|g| g.consumer == ImpactNode::Relation(relation.reference.clone()))
        .unwrap();
    assert!(relation_group.explanations.iter().any(|p| matches!(&p.steps[..], [s]
        if s.family == ImpactFamily::Necessary && s.dependency_role == Some(DependencyRole::Target))));
    assert!(
        !mixed
            .consumers()
            .iter()
            .any(|g| g.consumer == ImpactNode::Block(h_ref.clone()))
    );
    assert!(mixed.consumers().iter().all(|g| {
        g.explanations
            .iter()
            .all(|p| p.steps.iter().all(|s| s.family != ImpactFamily::Semantic))
    }));

    let reading = r::store(&rig)
        .create(actor, space, r::create(doc.reference.clone()))
        .await
        .unwrap();
    let selected = r::store(&rig)
        .select_relations(
            actor,
            reading.overlay.overlay_id,
            SelectRelations {
                request_id: Uuid::new_v4(),
                expected_overlay_revision: reading.overlay.revision_id,
                expected_reading_view_revision: reading.view.revision_id,
                selections: vec![RelationSelection {
                    relation: relation.reference.clone(),
                    review: None,
                }],
                epistemic_reviews: vec![],
                reason: "select exact R".into(),
            },
        )
        .await
        .unwrap();
    let selected_release = rig
        .releases()
        .publish_evidence(
            actor,
            space,
            PublishEvidence {
                request_id: Uuid::new_v4(),
                roots: vec![a::root(&doc, Some(release.release_id))],
                readings: vec![selected.view],
                reason: "fixed selected R".into(),
            },
        )
        .await
        .unwrap();
    let positive = store
        .traverse(
            actor,
            ImpactQuery {
                start: ImpactStart::Relation(relation.reference.clone()),
                scope: ImpactScope::Release {
                    release_id: selected_release.release_id,
                },
                families: vec![ImpactFamily::Semantic],
                max_depth: 1,
                limit: 50,
                work_limit: 4096,
                after: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(positive.start_membership(), ImpactMembership::Selected);
    assert!(matches!(positive.status(), PageStatus::Complete));
    for endpoint in [relations::exact(&e), relations::exact(&h)] {
        let group = positive
            .consumers()
            .iter()
            .find(|g| g.consumer == ImpactNode::Block(endpoint.clone()))
            .unwrap();
        assert!(
            group
                .explanations
                .iter()
                .any(|path| matches!(&path.steps[..], [step]
            if step.from == ImpactNode::Relation(relation.reference.clone())
               && step.to == ImpactNode::Block(endpoint.clone())
               && step.family == ImpactFamily::Semantic
               && step.relation_type == Some(RelationType::Supports)
               && step.provenance == ImpactProvenance::FixedReleaseManifest))
        );
    }
}

#[tokio::test]
async fn readable_external_dynamic_endpoint_is_terminal_not_a_scope_member() {
    let (rig, actor, space, doc, zero) = r::fixture().await;
    let e = match &doc.nodes[0].target {
        NodeTarget::Block(b) => b.clone(),
        _ => panic!("E"),
    };
    let h = match &doc.nodes[1].target {
        NodeTarget::Block(b) => b.clone(),
        _ => panic!("H"),
    };
    let (owner, external_space) = rig.seed_actor_space(true).await;
    rig.grant(actor, external_space, false).await;
    let x = rig
        .store
        .create(owner, external_space, support::command("external X"))
        .await
        .unwrap();
    let x = relations::exact(&x);
    let relation_store = RelationStore::new(rig.runtime_pool.clone());
    let entry = relation_store
        .save(actor, relations::save(space, e.clone(), x.clone()))
        .await
        .unwrap();
    let bridge = relation_store
        .save(actor, relations::save(space, x.clone(), h.clone()))
        .await
        .unwrap();
    let scope = ImpactScope::Reading {
        view: zero.view,
        mode: ReadingMode::Fused,
    };
    let store = QueryStore::new(rig.runtime_pool.clone());
    let mut query = ImpactQuery {
        start: ImpactStart::Block(e),
        scope: scope.clone(),
        families: vec![ImpactFamily::Semantic],
        max_depth: 4,
        limit: 50,
        work_limit: 4096,
        after: None,
    };
    let result = store.traverse(actor, query.clone()).await.unwrap().unwrap();
    assert!(matches!(result.status(), PageStatus::Complete));
    assert_eq!(result.consumers().len(), 2);
    assert!(
        result
            .consumers()
            .iter()
            .any(|g| g.consumer == ImpactNode::Relation(entry.reference.clone()))
    );
    let x_group = result
        .consumers()
        .iter()
        .find(|g| g.consumer == ImpactNode::Block(x.clone()))
        .unwrap();
    assert!(x_group.locations.is_empty());
    assert!(
        x_group
            .explanations
            .iter()
            .any(|path| matches!(&path.steps[..], [first, second]
        if first.to == ImpactNode::Relation(entry.reference.clone())
           && second.to == ImpactNode::Block(x.clone())
           && first.provenance == ImpactProvenance::DynamicWorking
           && second.provenance == ImpactProvenance::DynamicWorking
           && [first, second].iter().all(|s| s.direction == Some(TraversalDirection::SavedForward)
               && s.relation_type == Some(RelationType::Supports)
               && s.reason == ImpactReason::SuggestReview)))
    );
    assert!(
        !result
            .consumers()
            .iter()
            .any(|g| g.consumer == ImpactNode::Relation(bridge.reference.clone()))
    );
    query.start = ImpactStart::Block(x);
    assert!(store.traverse(actor, query).await.unwrap().is_none());
}

#[tokio::test]
async fn fixed_old_relation_and_dynamic_new_head_keep_exact_versions_and_directions() {
    let (rig, actor, space, doc, zero) = r::fixture().await;
    let e = match &doc.nodes[0].target {
        NodeTarget::Block(b) => b.clone(),
        _ => panic!("E"),
    };
    let h = match &doc.nodes[1].target {
        NodeTarget::Block(b) => b.clone(),
        _ => panic!("H"),
    };
    let relations = RelationStore::new(rig.runtime_pool.clone());
    let old = relations
        .save(actor, self::relations::save(space, e.clone(), h.clone()))
        .await
        .unwrap();
    let selected = r::store(&rig)
        .select_relations(
            actor,
            zero.overlay.overlay_id,
            SelectRelations {
                request_id: Uuid::new_v4(),
                expected_overlay_revision: zero.overlay.revision_id,
                expected_reading_view_revision: zero.view.revision_id,
                selections: vec![RelationSelection {
                    relation: old.reference.clone(),
                    review: None,
                }],
                epistemic_reviews: vec![],
                reason: "pin R@1".into(),
            },
        )
        .await
        .unwrap();
    let release = rig
        .releases()
        .publish_evidence(
            actor,
            space,
            PublishEvidence {
                request_id: Uuid::new_v4(),
                roots: vec![a::root(&doc, None)],
                readings: vec![selected.view.clone()],
                reason: "freeze R@1".into(),
            },
        )
        .await
        .unwrap();
    let mut edit = self::relations::edit(&old);
    edit.rationale = "revised support rationale".into();
    let new = relations.save(actor, edit).await.unwrap();
    let store = QueryStore::new(rig.runtime_pool.clone());
    let mut request = ImpactQuery {
        start: ImpactStart::Block(e.clone()),
        scope: ImpactScope::Reading {
            view: selected.view.clone(),
            mode: ReadingMode::Fused,
        },
        families: vec![ImpactFamily::Semantic],
        max_depth: 2,
        limit: 50,
        work_limit: 4096,
        after: None,
    };
    let reading = store
        .traverse(actor, request.clone())
        .await
        .unwrap()
        .unwrap();
    let h_group = reading
        .consumers()
        .iter()
        .find(|g| g.consumer == ImpactNode::Block(h.clone()))
        .unwrap();
    assert_eq!(h_group.explanations.len(), 2);
    let mut found = h_group
        .explanations
        .iter()
        .map(|path| {
            assert_eq!(path.steps.len(), 2);
            assert_eq!(path.steps[0].from, ImpactNode::Block(e.clone()));
            assert_eq!(path.steps[1].to, ImpactNode::Block(h.clone()));
            assert_eq!(path.steps[0].to, path.steps[1].from);
            assert!(path.steps.iter().all(|s| s.family == ImpactFamily::Semantic
                && s.reason == ImpactReason::SuggestReview
                && s.direction == Some(TraversalDirection::SavedForward)
                && s.provenance == path.steps[0].provenance
                && s.relation_type == path.steps[0].relation_type));
            (
                path.steps[0].to.clone(),
                path.steps[0].provenance,
                path.steps[0].relation_type,
            )
        })
        .collect::<Vec<_>>();
    found.sort_by_key(|v| serde_json::to_string(v).unwrap());
    let mut expected = vec![
        (
            ImpactNode::Relation(old.reference.clone()),
            ImpactProvenance::FixedReadingSelection,
            Some(RelationType::Supports),
        ),
        (
            ImpactNode::Relation(new.reference.clone()),
            ImpactProvenance::DynamicWorking,
            Some(RelationType::Supports),
        ),
    ];
    expected.sort_by_key(|v| serde_json::to_string(v).unwrap());
    assert_eq!(found, expected);
    request.scope = ImpactScope::Release {
        release_id: release.release_id,
    };
    let frozen = store
        .traverse(actor, request.clone())
        .await
        .unwrap()
        .unwrap();
    let frozen_h = frozen
        .consumers()
        .iter()
        .find(|g| g.consumer == ImpactNode::Block(h.clone()))
        .unwrap();
    assert_eq!(frozen_h.explanations.len(), 1);
    assert!(
        matches!(&frozen_h.explanations[0].steps[..], [first, second]
        if first.to == ImpactNode::Relation(old.reference.clone())
           && first.provenance == ImpactProvenance::FixedReleaseManifest
           && second.provenance == ImpactProvenance::FixedReleaseManifest
           && first.relation_type == Some(RelationType::Supports)
           && second.relation_type == Some(RelationType::Supports))
    );
    assert!(
        !frozen
            .consumers()
            .iter()
            .any(|g| g.consumer == ImpactNode::Relation(new.reference.clone()))
    );
    request.start = ImpactStart::Block(h);
    request.scope = ImpactScope::Reading {
        view: selected.view,
        mode: ReadingMode::Fused,
    };
    let reverse = store.traverse(actor, request).await.unwrap().unwrap();
    let e_group = reverse
        .consumers()
        .iter()
        .find(|g| g.consumer == ImpactNode::Block(e.clone()))
        .unwrap();
    assert_eq!(e_group.explanations.len(), 2);
    assert!(e_group.explanations.iter().all(|path| {
        path.steps
            .iter()
            .all(|s| s.direction == Some(TraversalDirection::SavedReverse))
    }));
}

#[tokio::test]
async fn fixed_review_selection_connects_exact_relation_and_review_to_reading_then_release() {
    let (rig, actor, space, doc, zero) = r::fixture().await;
    let e = match &doc.nodes[0].target {
        NodeTarget::Block(b) => b.clone(),
        _ => panic!("E"),
    };
    let h = match &doc.nodes[1].target {
        NodeTarget::Block(b) => b.clone(),
        _ => panic!("H"),
    };
    let relation_store = RelationStore::new(rig.runtime_pool.clone());
    let relation = relation_store
        .save(actor, relations::save(space, e, h))
        .await
        .unwrap();
    let review = relation_store
        .review(actor, relations::review(&relation))
        .await
        .unwrap();
    let selected = r::store(&rig)
        .select_relations(
            actor,
            zero.overlay.overlay_id,
            SelectRelations {
                request_id: Uuid::new_v4(),
                expected_overlay_revision: zero.overlay.revision_id,
                expected_reading_view_revision: zero.view.revision_id,
                selections: vec![RelationSelection {
                    relation: relation.reference.clone(),
                    review: Some(review.reference.clone()),
                }],
                epistemic_reviews: vec![],
                reason: "select relation with review".into(),
            },
        )
        .await
        .unwrap();
    let reading_node = ImpactNode::Reading {
        view_id: selected.view.view_id,
        revision_id: selected.view.revision_id,
    };
    let store = QueryStore::new(rig.runtime_pool.clone());
    for start in [
        ImpactStart::Relation(relation.reference.clone()),
        ImpactStart::RelationReview(review.reference.clone()),
    ] {
        let result = store
            .traverse(
                actor,
                ImpactQuery {
                    start: start.clone(),
                    scope: ImpactScope::Reading {
                        view: selected.view.clone(),
                        mode: ReadingMode::Fused,
                    },
                    families: vec![ImpactFamily::ReviewSelection],
                    max_depth: 1,
                    limit: 50,
                    work_limit: 4096,
                    after: None,
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.start_membership(), ImpactMembership::Selected);
        assert!(matches!(result.status(), PageStatus::Complete));
        assert_eq!(result.consumers().len(), 1);
        let group = &result.consumers()[0];
        assert_eq!(group.consumer, reading_node);
        assert!(group.locations.is_empty());
        assert!(
            matches!(&group.explanations[..], [path] if matches!(&path.steps[..], [step]
            if step.from == match &start {
                ImpactStart::Relation(r) => ImpactNode::Relation(r.clone()),
                ImpactStart::RelationReview(r) => ImpactNode::RelationReview(r.clone()),
                _ => unreachable!(),
            } && step.to == reading_node && step.family == ImpactFamily::ReviewSelection
               && step.provenance == ImpactProvenance::FixedReadingSelection
               && step.reason == ImpactReason::SelectedEvidence))
        );
    }
    let release = rig
        .releases()
        .publish_evidence(
            actor,
            space,
            PublishEvidence {
                request_id: Uuid::new_v4(),
                roots: vec![a::root(&doc, None)],
                readings: vec![selected.view],
                reason: "freeze review selection".into(),
            },
        )
        .await
        .unwrap();
    let result = store
        .traverse(
            actor,
            ImpactQuery {
                start: ImpactStart::Relation(relation.reference),
                scope: ImpactScope::Release {
                    release_id: release.release_id,
                },
                families: vec![ImpactFamily::ReviewSelection, ImpactFamily::Structural],
                max_depth: 2,
                limit: 50,
                work_limit: 4096,
                after: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result.status(), PageStatus::Complete));
    let release_group = result
        .consumers()
        .iter()
        .find(|g| {
            g.consumer
                == ImpactNode::Release {
                    release_id: release.release_id,
                }
        })
        .unwrap();
    assert!(release_group.locations.is_empty());
    assert!(
        matches!(&release_group.explanations[..], [path] if matches!(&path.steps[..], [first, second]
        if first.to == reading_node && first.provenance == ImpactProvenance::FixedReleaseManifest
           && first.family == ImpactFamily::ReviewSelection && second.from == reading_node
           && second.to == ImpactNode::Release { release_id: release.release_id }
           && second.family == ImpactFamily::Structural))
    );
}

#[tokio::test]
async fn necessary_chain_stops_at_requested_depth_without_partial_next_consumer() {
    use support::references as refs;
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let (id, revision) = rig.seed_block(actor, space).await;
    let mut chain = vec![BlockRef {
        block_id: id,
        revision_id: revision,
    }];
    let mut tx = rig.runtime_pool.begin().await.unwrap();
    for _ in 1..10 {
        let next = refs::seed_content(
            &mut tx,
            actor,
            space,
            refs::v2(chain.last().unwrap().clone(), false),
            None,
        )
        .await;
        chain.push(next);
    }
    tx.commit().await.unwrap();
    let doc = rig
        .compositions()
        .save(
            actor,
            space,
            a::doc(chain.iter().cloned().map(NodeTarget::Block).collect()),
        )
        .await
        .unwrap();
    let release = rig
        .releases()
        .publish_evidence(
            actor,
            space,
            PublishEvidence {
                request_id: Uuid::new_v4(),
                roots: vec![a::root(&doc, None)],
                readings: vec![],
                reason: "fixed depth chain".into(),
            },
        )
        .await
        .unwrap();
    let result = QueryStore::new(rig.runtime_pool.clone())
        .traverse(
            actor,
            ImpactQuery {
                start: ImpactStart::Block(chain[0].clone()),
                scope: ImpactScope::Release {
                    release_id: release.release_id,
                },
                families: vec![ImpactFamily::Necessary],
                max_depth: 8,
                limit: 50,
                work_limit: 4096,
                after: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result.status(), PageStatus::Complete));
    assert_eq!(result.consumers().len(), 8);
    for (index, reference) in chain.iter().enumerate().skip(1).take(8) {
        let group = result
            .consumers()
            .iter()
            .find(|group| group.consumer == ImpactNode::Block(reference.clone()))
            .unwrap();
        assert_eq!(group.explanations.len(), 1);
        assert_eq!(group.explanations[0].steps.len(), index);
        assert!(group.explanations[0].steps.iter().all(|step| {
            step.family == ImpactFamily::Necessary
                && step.dependency_role == Some(DependencyRole::Basis)
                && step.dependency_position == Some(0)
                && step.reason == ImpactReason::RequiresExactRevision
        }));
    }
    assert!(
        !result
            .consumers()
            .iter()
            .any(|group| group.consumer == ImpactNode::Block(chain[9].clone()))
    );
}

#[tokio::test]
async fn semantic_cycle_is_bounded_by_path_identity_without_losing_other_consumers() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let a_block = rig
        .store
        .create(actor, space, support::command("A"))
        .await
        .unwrap();
    let b_block = rig
        .store
        .create(actor, space, support::command("B"))
        .await
        .unwrap();
    let c_block = rig
        .store
        .create(actor, space, support::command("C"))
        .await
        .unwrap();
    let doc = rig
        .compositions()
        .save(
            actor,
            space,
            a::doc(vec![
                a::block(&a_block),
                a::block(&b_block),
                a::block(&c_block),
            ]),
        )
        .await
        .unwrap();
    let zero = r::store(&rig)
        .create(actor, space, r::create(doc.reference))
        .await
        .unwrap();
    let relation_store = RelationStore::new(rig.runtime_pool.clone());
    let ab = relation_store
        .save(
            actor,
            relations::save(
                space,
                relations::exact(&a_block),
                relations::exact(&b_block),
            ),
        )
        .await
        .unwrap();
    let bc = relation_store
        .save(
            actor,
            relations::save(
                space,
                relations::exact(&b_block),
                relations::exact(&c_block),
            ),
        )
        .await
        .unwrap();
    let ca = relation_store
        .save(
            actor,
            relations::save(
                space,
                relations::exact(&c_block),
                relations::exact(&a_block),
            ),
        )
        .await
        .unwrap();
    let selected = r::store(&rig)
        .select_relations(
            actor,
            zero.overlay.overlay_id,
            SelectRelations {
                request_id: Uuid::new_v4(),
                expected_overlay_revision: zero.overlay.revision_id,
                expected_reading_view_revision: zero.view.revision_id,
                selections: [&ab, &bc, &ca]
                    .map(|saved| RelationSelection {
                        relation: saved.reference.clone(),
                        review: None,
                    })
                    .to_vec(),
                epistemic_reviews: vec![],
                reason: "fixed semantic cycle".into(),
            },
        )
        .await
        .unwrap();
    let result = QueryStore::new(rig.runtime_pool.clone())
        .traverse(
            actor,
            ImpactQuery {
                start: ImpactStart::Block(relations::exact(&a_block)),
                scope: ImpactScope::Reading {
                    view: selected.view,
                    mode: ReadingMode::Fused,
                },
                families: vec![ImpactFamily::Semantic],
                max_depth: 8,
                limit: 50,
                work_limit: 4096,
                after: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result.status(), PageStatus::Complete));
    let mut actual = result
        .consumers()
        .iter()
        .map(|group| group.consumer.clone())
        .collect::<Vec<_>>();
    actual.sort();
    let mut expected = vec![
        ImpactNode::Block(relations::exact(&b_block)),
        ImpactNode::Block(relations::exact(&c_block)),
        ImpactNode::Relation(ab.reference),
        ImpactNode::Relation(bc.reference),
        ImpactNode::Relation(ca.reference),
    ];
    expected.sort();
    assert_eq!(actual, expected);
    let start = ImpactNode::Block(relations::exact(&a_block));
    for group in result.consumers() {
        for explanation in &group.explanations {
            assert!(explanation.steps.len() <= 8);
            let mut seen = vec![start.clone()];
            for step in &explanation.steps {
                assert_eq!(seen.last(), Some(&step.from));
                assert!(!seen.contains(&step.to));
                assert_eq!(step.family, ImpactFamily::Semantic);
                assert_eq!(step.reason, ImpactReason::SuggestReview);
                seen.push(step.to.clone());
            }
        }
    }
}

#[tokio::test]
async fn incomplete_selected_judgment_remains_anonymous_across_traversal_and_cursor() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let mut conjecture = support::command("A conjecture");
    conjecture.draft.intent = Intent::Conjecture;
    let a_block = rig.store.create(actor, space, conjecture).await.unwrap();
    let b_block = rig
        .store
        .create(actor, space, support::command("B note"))
        .await
        .unwrap();
    let target = relations::exact(&a_block);
    let doc = rig
        .compositions()
        .save(
            actor,
            space,
            a::doc(vec![a::block(&a_block), a::block(&b_block)]),
        )
        .await
        .unwrap();
    let zero = r::store(&rig)
        .create(actor, space, r::create(doc.reference.clone()))
        .await
        .unwrap();
    let (basis_owner, basis_space) = rig.seed_actor_space(true).await;
    let basis = rig
        .store
        .create(basis_owner, basis_space, support::command("private basis"))
        .await
        .unwrap();
    rig.grant(actor, basis_space, false).await;
    let review = ReviewStore::new(rig.runtime_pool.clone())
        .append(
            actor,
            AppendEpistemicReview {
                request_id: Uuid::new_v4(),
                scope: RelationScope::Space { space_id: space },
                target: target.clone(),
                expected_previous: None,
                state: EpistemicState::Untested,
                relations: vec![],
                evidence: vec![relations::exact(&basis)],
                conditions: "".into(),
                explanation: "".into(),
            },
        )
        .await
        .unwrap();
    let selected = r::store(&rig)
        .select_relations(
            actor,
            zero.overlay.overlay_id,
            SelectRelations {
                request_id: Uuid::new_v4(),
                expected_overlay_revision: zero.overlay.revision_id,
                expected_reading_view_revision: zero.view.revision_id,
                selections: vec![],
                epistemic_reviews: vec![review.reference.clone()],
                reason: "pin judgment".into(),
            },
        )
        .await
        .unwrap();
    let scope = ImpactScope::Reading {
        view: selected.view,
        mode: ReadingMode::Fused,
    };
    let store = QueryStore::new(rig.runtime_pool.clone());
    rig.revoke(actor, basis_space).await;
    let evidence = store
        .evidence(
            actor,
            EvidenceQuery {
                endpoint: target.clone(),
                scope: scope.clone(),
                include_dynamic: false,
                limit: 50,
                work_limit: 4096,
                after: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(evidence.judgments(), &[EvidenceJudgment::Incomplete]);
    assert_eq!(
        serde_json::to_value(&evidence).unwrap()["judgments"],
        serde_json::json!([{"type":"incomplete"}])
    );
    let relation_only = store
        .traverse(
            actor,
            ImpactQuery {
                start: ImpactStart::Block(target.clone()),
                scope: scope.clone(),
                families: vec![ImpactFamily::Necessary, ImpactFamily::ReviewSelection],
                max_depth: 1,
                limit: 50,
                work_limit: 4096,
                after: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(relation_only.status(), PageStatus::Complete));
    assert!(relation_only.consumers().is_empty());
    assert!(
        store
            .traverse(
                actor,
                ImpactQuery {
                    start: ImpactStart::EpistemicReview(review.reference.clone()),
                    scope: scope.clone(),
                    families: vec![ImpactFamily::ReviewSelection],
                    max_depth: 1,
                    limit: 50,
                    work_limit: 4096,
                    after: None,
                },
            )
            .await
            .unwrap()
            .is_none()
    );
    let mut cursor = None;
    let mut consumers = vec![];
    for index in 0..2 {
        let page = store
            .traverse(
                actor,
                ImpactQuery {
                    start: ImpactStart::Block(target.clone()),
                    scope: scope.clone(),
                    families: vec![
                        ImpactFamily::Structural,
                        ImpactFamily::Necessary,
                        ImpactFamily::ReviewSelection,
                    ],
                    max_depth: 1,
                    limit: 1,
                    work_limit: 4096,
                    after: cursor,
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(page.consumers().len(), 1);
        let wire = serde_json::to_string(&page).unwrap();
        for hidden_id in [
            review.reference.stream_id,
            review.reference.review_id,
            basis.block_id,
            basis.revision_id,
        ] {
            assert!(!wire.contains(&hidden_id.to_string()));
        }
        consumers.push(page.consumers()[0].consumer.clone());
        cursor = match page.status() {
            PageStatus::Truncated { after } if index == 0 => Some(after.clone()),
            PageStatus::Complete if index == 1 => None,
            other => panic!("unexpected anonymous page status {other:?}"),
        };
    }
    consumers.sort();
    let mut expected = vec![
        ImpactNode::Composition(doc.reference),
        ImpactNode::Reading {
            view_id: match &scope {
                ImpactScope::Reading { view, .. } => view.view_id,
                _ => unreachable!(),
            },
            revision_id: match &scope {
                ImpactScope::Reading { view, .. } => view.revision_id,
                _ => unreachable!(),
            },
        },
    ];
    expected.sort();
    assert_eq!(consumers, expected);
}

#[tokio::test]
async fn out_of_scope_dense_necessary_candidates_do_not_change_visible_budget_or_result() {
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
        .find(|composition| composition.reference == root)
        .unwrap();
    let release = rig
        .releases()
        .publish_evidence(
            actor,
            space,
            PublishEvidence {
                request_id: Uuid::new_v4(),
                roots: vec![a::root(&root_revision, None)],
                readings: vec![],
                reason: "fixed necessary source".into(),
            },
        )
        .await
        .unwrap();
    let store = QueryStore::new(rig.runtime_pool.clone());
    let query = ImpactQuery {
        start: ImpactStart::Block(evidence.clone()),
        scope: ImpactScope::Release {
            release_id: release.release_id,
        },
        families: vec![ImpactFamily::Necessary],
        max_depth: 1,
        limit: 50,
        work_limit: 5,
        after: None,
    };
    let baseline = store.traverse(actor, query.clone()).await.unwrap().unwrap();
    assert_eq!(baseline.start_membership(), ImpactMembership::Context);
    assert!(matches!(baseline.status(), PageStatus::Complete));
    assert_eq!(baseline.consumers().len(), 1);
    assert_eq!(
        baseline.consumers()[0].consumer,
        ImpactNode::Block(conclusion)
    );
    let insufficient = store
        .traverse(
            actor,
            ImpactQuery {
                work_limit: 4,
                ..query.clone()
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(insufficient.status(), PageStatus::BudgetExceeded));
    assert!(insufficient.consumers().is_empty());
    let (_, candidate_space) = rig.seed_actor_space(true).await;
    rig.grant(actor, candidate_space, true).await;
    let (basis_owner, basis_space) = rig.seed_actor_space(true).await;
    let (basis_block, basis_revision) = rig.seed_block(basis_owner, basis_space).await;
    let hidden_basis = BlockRef {
        block_id: basis_block,
        revision_id: basis_revision,
    };
    rig.grant(actor, basis_space, false).await;
    let mut hidden = vec![];
    let mut tx = rig.runtime_pool.begin().await.unwrap();
    for _ in 0..260 {
        let mut draft = refs::v2(evidence.clone(), false);
        let ContentDraft::V2(content) = &mut draft else {
            unreachable!()
        };
        content
            .basis_refs
            .push(ExactRef::Block(hidden_basis.clone()));
        hidden.push(refs::seed_content(&mut tx, actor, candidate_space, draft, None).await);
    }
    tx.commit().await.unwrap();
    // Source grants remain, so 261 target rows cross three 128-row keyset
    // batches. They stay outside fixed scope; the second basis is now hidden.
    rig.revoke(actor, basis_space).await;
    let after = store.traverse(actor, query.clone()).await.unwrap().unwrap();
    assert_eq!(
        serde_json::to_vec(&after).unwrap(),
        serde_json::to_vec(&baseline).unwrap()
    );
    let after_insufficient = store
        .traverse(
            actor,
            ImpactQuery {
                work_limit: 4,
                ..query
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_vec(&after_insufficient).unwrap(),
        serde_json::to_vec(&insufficient).unwrap()
    );
    let wire = serde_json::to_string(&after).unwrap();
    assert!(!wire.contains(&hidden_basis.block_id.to_string()));
    assert!(!wire.contains(&hidden_basis.revision_id.to_string()));
    assert!(hidden.iter().all(|reference| {
        !wire.contains(&reference.block_id.to_string())
            && !wire.contains(&reference.revision_id.to_string())
    }));
}

#[tokio::test]
async fn each_fixed_reading_keeps_only_its_own_source_occurrence_paths() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let e = rig
        .store
        .create(actor, space, support::command("shared E"))
        .await
        .unwrap();
    let child = rig
        .compositions()
        .save(actor, space, a::doc(vec![a::block(&e)]))
        .await
        .unwrap();
    let parent = rig
        .compositions()
        .save(actor, space, a::doc(vec![a::child(&child)]))
        .await
        .unwrap();
    let child_occurrence = child.nodes[0].occurrence_id;
    let parent_occurrence = parent.nodes[0].occurrence_id;
    let child_view = r::store(&rig)
        .create(actor, space, r::create(child.reference.clone()))
        .await
        .unwrap();
    let parent_view = r::store(&rig)
        .create(actor, space, r::create(parent.reference.clone()))
        .await
        .unwrap();
    let release = rig
        .releases()
        .publish_evidence(
            actor,
            space,
            PublishEvidence {
                request_id: Uuid::new_v4(),
                roots: vec![a::root(&parent, None)],
                readings: vec![child_view.view.clone(), parent_view.view.clone()],
                reason: "two exact source roots".into(),
            },
        )
        .await
        .unwrap();
    let result = QueryStore::new(rig.runtime_pool.clone())
        .traverse(
            actor,
            ImpactQuery {
                start: ImpactStart::Block(relations::exact(&e)),
                scope: ImpactScope::Release {
                    release_id: release.release_id,
                },
                families: vec![ImpactFamily::Structural],
                max_depth: 3,
                limit: 50,
                work_limit: 4096,
                after: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result.status(), PageStatus::Complete));
    for (view, expected_path, expected_hops) in [
        (child_view.view, vec![child_occurrence], 2),
        (
            parent_view.view,
            vec![parent_occurrence, child_occurrence],
            3,
        ),
    ] {
        let reading = ImpactNode::Reading {
            view_id: view.view_id,
            revision_id: view.revision_id,
        };
        let group = result
            .consumers()
            .iter()
            .find(|group| group.consumer == reading)
            .unwrap();
        assert_eq!(
            group.locations,
            vec![ImpactLocation::Occurrence {
                path: expected_path.clone(),
            }]
        );
        assert_eq!(group.explanations.len(), 2);
        assert!(group.explanations.iter().any(|explanation| {
            matches!(&explanation.steps[..], [direct]
            if direct.to == reading
                && direct.location == Some(ImpactLocation::Occurrence {
                    path: expected_path.clone(),
                }))
        }));
        assert!(group.explanations.iter().any(|explanation| {
            explanation.steps.len() == expected_hops
                && explanation
                    .steps
                    .last()
                    .is_some_and(|step| step.to == reading)
                && explanation.steps[0].location
                    == Some(ImpactLocation::Occurrence {
                        path: expected_path.clone(),
                    })
        }));
    }
    let parent_group = result
        .consumers()
        .iter()
        .find(|group| group.consumer == ImpactNode::Composition(parent.reference.clone()))
        .unwrap();
    assert_eq!(
        parent_group.locations,
        vec![ImpactLocation::Occurrence {
            path: vec![parent_occurrence, child_occurrence],
        }]
    );
}

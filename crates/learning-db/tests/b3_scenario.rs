//! Synthetic acceptance data, not a scientific experiment or an automatic truth judgment.
#[path = "support/relation_store.rs"]
mod relation_helpers;
mod support;
use learning_core::*;
use learning_db::{LineageStore, QueryStore, RelationStore, ReviewStore, VersionedContentStore};
use serde_json::json;
use support::{assembly as a, reading as h};
use uuid::Uuid;

const ATTENTION: &str = "Synthetic Attention: compare attention under a fixed toy protocol.";
const H1: &str = "H@1: attention improves the toy score within protocol P.";
const E1: &str = "E1: run R reports toy score 0.7.";
const E2: &str = "E2: the same run R reports toy score 0.8.";
const X: &str = "X: protocol P includes a counterexample with toy score 0.2.";
const C1: &str = "C@1: mixed toy evidence is inconclusive within protocol P.";
const H2: &str = "H@2: restrict the toy claim to protocol P subset Q.";

fn draft(
    intent: Intent,
    text: &str,
    basis_refs: Vec<ExactRef>,
    source_run: Option<BlockRef>,
) -> ContentDraft {
    ContentDraft::V2(ContentV2 {
        intent,
        language: "en".into(),
        title: "Synthetic Attention".into(),
        body: BodyV2::Text(TextPayload {
            format: TextFormat::Markdown,
            text: text.into(),
        }),
        basis_refs,
        requires_context: vec![],
        source_run,
    })
}
fn create(draft: ContentDraft) -> CreateContent {
    CreateContent {
        request_id: Uuid::new_v4(),
        draft,
        reason: "synthetic acceptance".into(),
    }
}
fn exact(r: &ContentRevision) -> BlockRef {
    BlockRef {
        block_id: r.block_id,
        revision_id: r.revision_id,
    }
}
fn assert_body(
    r: &ContentRevision,
    intent: &str,
    text: &str,
    basis: Vec<ExactRef>,
    run: Option<BlockRef>,
) {
    let expected = json!({"intent":intent,"language":"en","title":"Synthetic Attention","body":{"kind":"text","payload":{"format":"markdown","text":text}},"basis_refs":basis,"requires_context":[],"source_run":run});
    assert_eq!(serde_json::to_value(&r.draft).unwrap(), expected);
    let canonical = json!({"domain":"content-v2","contract_version":2,"draft":expected});
    assert_eq!(
        r.content_sha256,
        hex_digest(canonical_json(&canonical).as_bytes())
    );
}
fn choice(
    saved: &ReadingSaved,
    selections: Vec<RelationSelection>,
    judgment: EpistemicReviewRef,
) -> SelectRelations {
    SelectRelations {
        request_id: Uuid::new_v4(),
        expected_overlay_revision: saved.overlay.revision_id,
        expected_reading_view_revision: saved.view.revision_id,
        selections,
        epistemic_reviews: vec![judgment],
        reason: "fixed reviewed choices".into(),
    }
}
fn bodies(p: &VersionedReadingProjection) -> Vec<(BlockRef, String)> {
    p.items
        .iter()
        .filter_map(|item| {
            let r = match item {
                VersionedReadingItem::Original { revision, .. } => revision,
                VersionedReadingItem::Personal { item } => &item.revision,
                _ => return None,
            };
            let text = match &r.draft {
                ContentDraft::V1(d) => d.payload.text.clone(),
                ContentDraft::V2(d) => match &d.body {
                    BodyV2::Text(p) => p.text.clone(),
                    _ => panic!("expected literal body"),
                },
            };
            Some((exact(r), text))
        })
        .collect()
}

fn impact_query(start: BlockRef, scope: ImpactScope) -> ImpactQuery {
    ImpactQuery {
        start: ImpactStart::Block(start),
        scope,
        families: vec![
            ImpactFamily::Structural,
            ImpactFamily::Necessary,
            ImpactFamily::Semantic,
            ImpactFamily::ReviewSelection,
            ImpactFamily::Lineage,
        ],
        max_depth: 4,
        limit: 50,
        work_limit: 4096,
        after: None,
    }
}

#[tokio::test]
async fn synthetic_attention_fixed_evidence_history_revocation_restoration_and_split() {
    let r = support::TestRig::from_env().await;
    let (actor, space) = r.seed_actor_space(true).await;
    let (source_actor, source_space) = r.seed_actor_space(true).await;
    r.grant(actor, source_space, false).await;
    let content = VersionedContentStore::new(r.runtime_pool.clone());
    let relations = RelationStore::new(r.runtime_pool.clone());
    let reviews = ReviewStore::new(r.runtime_pool.clone());
    let reading = h::store(&r);
    let original = r
        .store
        .create(actor, space, support::command(ATTENTION))
        .await
        .unwrap();
    let original_ref = relation_helpers::exact(&original);
    let doc = r
        .compositions()
        .save(actor, space, a::doc(vec![a::block(&original)]))
        .await
        .unwrap();
    assert_eq!(doc.title, "Attention");
    assert_eq!(doc.nodes[0].target, NodeTarget::Block(original_ref.clone()));
    let zero = reading
        .create(actor, space, h::create(doc.reference.clone()))
        .await
        .unwrap();
    let hypothesis = content
        .create(
            actor,
            space,
            create(draft(
                Intent::Conjecture,
                H1,
                vec![ExactRef::Block(original_ref.clone())],
                None,
            )),
        )
        .await
        .unwrap();
    let href = exact(&hypothesis);
    assert_body(
        &hypothesis,
        "conjecture",
        H1,
        vec![ExactRef::Block(original_ref.clone())],
        None,
    );
    let personal = reading
        .edit(
            actor,
            zero.overlay.overlay_id,
            h::edit(
                &zero,
                ReadingEdit::InsertExisting {
                    block: href.clone(),
                    target: InsertTarget::NewGroup {
                        anchor: h::gap(&doc, 1),
                    },
                },
            ),
        )
        .await
        .unwrap();
    let scope = RelationScope::PersonalOverlay {
        space_id: space,
        overlay_id: zero.overlay.overlay_id,
    };
    let run = content
        .create(
            source_actor,
            source_space,
            create(draft(
                Intent::Observation,
                "R: one synthetic source run",
                vec![],
                None,
            )),
        )
        .await
        .unwrap();
    let run_ref = exact(&run);
    let e1 = content
        .create(
            source_actor,
            source_space,
            create(draft(Intent::Evidence, E1, vec![], Some(run_ref.clone()))),
        )
        .await
        .unwrap();
    let e2 = content
        .create(
            source_actor,
            source_space,
            create(draft(Intent::Evidence, E2, vec![], Some(run_ref.clone()))),
        )
        .await
        .unwrap();
    let x = content
        .create(
            actor,
            space,
            create(draft(Intent::Evidence, X, vec![], None)),
        )
        .await
        .unwrap();
    let evidence = vec![exact(&e1), exact(&e2), exact(&x)];
    assert_eq!(
        reviews
            .group_sources(actor, evidence.clone())
            .await
            .unwrap(),
        vec![
            SourceGroup {
                source_run: Some(run_ref.clone()),
                evidence: evidence[..2].to_vec()
            },
            SourceGroup {
                source_run: None,
                evidence: vec![exact(&x)]
            }
        ]
    );
    let mut saved_relations = vec![];
    let mut checked = vec![];
    for (from, kind) in [
        (exact(&e1), RelationType::Supports),
        (exact(&e2), RelationType::Supports),
        (exact(&x), RelationType::Opposes),
    ] {
        let mut command = relation_helpers::save(space, from.clone(), href.clone());
        command.scope = scope.clone();
        command.relation_type = kind;
        command.rationale = "human records toy observation".into();
        command.conditions = "protocol P only".into();
        let saved = relations.save(actor, command).await.unwrap();
        assert_eq!(saved.from, from);
        assert_eq!(saved.to, href);
        assert_eq!(saved.scope, scope);
        assert_eq!(saved.relation_type, kind);
        assert_eq!(saved.origin, RelationOrigin::UserAsserted);
        let review = relations
            .review(
                actor,
                ReviewRelation {
                    request_id: Uuid::new_v4(),
                    relation: saved.reference.clone(),
                    expected_previous: None,
                    state: RelationReviewState::Reviewed,
                    explanation: "human checked synthetic record within P".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(review.state, RelationReviewState::Reviewed);
        assert_eq!(review.reference.relation, saved.reference);
        assert_eq!(review.reviewer_id, actor.actor_id);
        checked.push(review);
        saved_relations.push(saved);
    }
    let selections: Vec<_> = saved_relations
        .iter()
        .zip(&checked)
        .map(|(relation, review)| RelationSelection {
            relation: relation.reference.clone(),
            review: Some(review.reference.clone()),
        })
        .collect();
    let judgment_command = AppendEpistemicReview {
        request_id: Uuid::new_v4(),
        scope: scope.clone(),
        target: href.clone(),
        expected_previous: None,
        state: EpistemicState::Inconclusive,
        relations: selections.clone(),
        evidence: evidence.clone(),
        conditions: "protocol P only".into(),
        explanation: "E1 and E2 share R; X is counterevidence; no independence or truth inferred"
            .into(),
    };
    let judgment = reviews
        .append(actor, judgment_command.clone())
        .await
        .unwrap();
    assert_eq!(judgment.target, href);
    assert_eq!(judgment.state, EpistemicState::Inconclusive);
    assert_eq!(judgment.relations, selections);
    assert_eq!(judgment.evidence, evidence);
    assert_eq!(judgment.conditions, "protocol P only");
    let c_basis = vec![
        ExactRef::Block(href.clone()),
        ExactRef::EpistemicReview(judgment.reference.clone()),
    ];
    let conclusion = content
        .create(
            actor,
            space,
            create(draft(Intent::Conclusion, C1, c_basis.clone(), None)),
        )
        .await
        .unwrap();
    let cref = exact(&conclusion);
    assert_body(&conclusion, "conclusion", C1, c_basis.clone(), None);
    let state = reading
        .state(actor, zero.overlay.overlay_id)
        .await
        .unwrap()
        .unwrap();
    let group = &state.editable.unwrap().groups[0];
    let with_conclusion = reading
        .edit(
            actor,
            zero.overlay.overlay_id,
            h::edit(
                &personal,
                ReadingEdit::InsertExisting {
                    block: cref.clone(),
                    target: InsertTarget::ExistingGroup {
                        group_id: group.group_id,
                        order: PlacementOrder {
                            left_placement_id: Some(group.placements[0].placement_id),
                            right_placement_id: None,
                        },
                    },
                },
            ),
        )
        .await
        .unwrap();
    let selected = reading
        .select_relations(
            actor,
            zero.overlay.overlay_id,
            choice(
                &with_conclusion,
                selections.clone(),
                judgment.reference.clone(),
            ),
        )
        .await
        .unwrap();
    // Two exact views share one overlay revision; view identity must be honored.
    assert_eq!(selected.overlay, with_conclusion.overlay);
    assert_ne!(selected.view, with_conclusion.view);
    assert!(
        reading
            .read_versioned(actor, with_conclusion.view.clone(), ReadingMode::Fused)
            .await
            .unwrap()
            .unwrap()
            .evidence
            .selections
            .is_empty()
    );
    let expected_bodies = vec![
        (original_ref.clone(), ATTENTION.into()),
        (href.clone(), H1.into()),
        (cref.clone(), C1.into()),
    ];
    let assert_snapshot = |p: &VersionedReadingProjection| {
        assert_eq!(p.view, selected.view);
        assert_eq!(p.overlay, selected.overlay);
        assert_eq!(bodies(p), expected_bodies);
        assert!(p.unplaced.is_empty());
        assert_eq!(p.evidence.selections.len(), 3);
        for (i, item) in p.evidence.selections.iter().enumerate() {
            assert_eq!(item.relation.from, evidence[i]);
            assert_eq!(item.relation.to, href);
            assert_eq!(item.relation.reference, selections[i].relation);
            assert!(
                matches!(&item.review,Some(ReviewProjection::Available(value)) if value.reference==checked[i].reference && value.state==RelationReviewState::Reviewed)
            );
        }
        assert!(
            matches!(&p.evidence.epistemic_reviews[..],[ReviewProjection::Available(value)] if value.target==href && value.state==EpistemicState::Inconclusive && value.relations==selections && value.evidence==evidence)
        );
    };
    assert_snapshot(
        &reading
            .read_versioned(actor, selected.view.clone(), ReadingMode::Fused)
            .await
            .unwrap()
            .unwrap(),
    );
    let released = r
        .releases()
        .publish_evidence(
            actor,
            space,
            PublishEvidence {
                request_id: Uuid::new_v4(),
                roots: vec![a::root(&doc, None)],
                readings: vec![selected.view.clone()],
                reason: "synthetic historical publication".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(released.roots, std::slice::from_ref(&doc.reference));
    assert_eq!(released.readings, std::slice::from_ref(&selected.view));
    let mut objects: Vec<(String, Uuid, Uuid)> = [
        original_ref.clone(),
        href.clone(),
        run_ref.clone(),
        exact(&e1),
        exact(&e2),
        exact(&x),
        cref.clone(),
    ]
    .into_iter()
    .map(|b| ("block".into(), b.block_id, b.revision_id))
    .collect();
    objects.extend(saved_relations.iter().map(|v| {
        (
            "relation".into(),
            v.reference.relation_id,
            v.reference.revision_id,
        )
    }));
    objects.extend(checked.iter().map(|v| {
        (
            "relation_review".into(),
            v.reference.relation.revision_id,
            v.reference.review_id,
        )
    }));
    objects.push((
        "epistemic_review".into(),
        judgment.reference.stream_id,
        judgment.reference.review_id,
    ));
    objects.sort();
    let expected_objects: Vec<_> = objects.iter().map(|(kind,object_id,revision_id)|json!({"kind":kind,"object_id":object_id,"revision_id":revision_id})).collect();
    let expected_manifest = hex_digest(canonical_json(&json!({"domain":"release-manifest-v2","roots":[doc.reference],"compositions":[doc.reference],"objects":expected_objects,"readings":[selected.view]})).as_bytes());
    assert_eq!(released.manifest_sha256, expected_manifest);
    let actual_objects:Vec<(String,Uuid,Uuid)> = sqlx::query_as("SELECT kind,object_id,revision_id FROM release_manifest_object WHERE release_id=$1 ORDER BY kind,object_id,revision_id").bind(released.release_id).fetch_all(&r.admin_pool).await.unwrap();
    assert_eq!(actual_objects, objects);
    // B4 reads the immutable Attention release through the same exact B3
    // references. Expected edges and explanations are specified independently
    // of the query output, including the counterexample's saved relation type.
    let impact = QueryStore::new(r.runtime_pool.clone());
    let released_scope = ImpactScope::Release {
        release_id: released.release_id,
    };
    let selected_state = reading
        .state(actor, selected.overlay.overlay_id)
        .await
        .unwrap()
        .unwrap();
    let selected_editable = selected_state.editable.unwrap();
    let placements = &selected_editable.groups[0].placements;
    assert_eq!(placements.len(), 2);
    assert_eq!(placements[0].block, href);
    assert_eq!(placements[1].block, cref);
    let h_location = ImpactLocation::Placement {
        view_id: selected.view.view_id,
        revision_id: selected.view.revision_id,
        placement_id: placements[0].placement_id,
    };
    let reading_node = ImpactNode::Reading {
        view_id: selected.view.view_id,
        revision_id: selected.view.revision_id,
    };
    let release_node = ImpactNode::Release {
        release_id: released.release_id,
    };
    let to_reading = ImpactStep {
        from: ImpactNode::Block(href.clone()),
        to: reading_node.clone(),
        family: ImpactFamily::Structural,
        dependency_role: None,
        dependency_position: None,
        direction: None,
        relation_type: None,
        lineage_type: None,
        provenance: ImpactProvenance::FixedReadingSelection,
        location: Some(h_location.clone()),
        reason: ImpactReason::ReferencesOldRevision,
    };
    let to_release = ImpactStep {
        from: reading_node.clone(),
        to: release_node.clone(),
        family: ImpactFamily::Structural,
        dependency_role: None,
        dependency_position: None,
        direction: None,
        relation_type: None,
        lineage_type: None,
        provenance: ImpactProvenance::Stored,
        location: None,
        reason: ImpactReason::ReferencesOldRevision,
    };
    for (scope, released) in [
        (
            ImpactScope::Reading {
                view: selected.view.clone(),
                mode: ReadingMode::Fused,
            },
            false,
        ),
        (released_scope.clone(), true),
    ] {
        let mut request = impact_query(href.clone(), scope);
        request.families = vec![ImpactFamily::Structural];
        request.max_depth = 2;
        let result = impact.traverse(actor, request).await.unwrap().unwrap();
        assert!(matches!(result.status(), PageStatus::Complete));
        assert_eq!(result.start_membership(), ImpactMembership::Displayed);
        assert_eq!(result.consumers().len(), if released { 2 } else { 1 });
        let reading_group = result
            .consumers()
            .iter()
            .find(|group| group.consumer == reading_node)
            .unwrap();
        assert_eq!(reading_group.locations, vec![h_location.clone()]);
        assert_eq!(
            reading_group.explanations,
            vec![ImpactExplanation {
                steps: vec![to_reading.clone()],
            }]
        );
        if released {
            let release_group = result
                .consumers()
                .iter()
                .find(|group| group.consumer == release_node)
                .unwrap();
            assert_eq!(release_group.locations, vec![h_location.clone()]);
            assert_eq!(
                release_group.explanations,
                vec![ImpactExplanation {
                    steps: vec![to_reading.clone(), to_release.clone()],
                }]
            );
        }
    }
    let c_location = ImpactLocation::Placement {
        view_id: selected.view.view_id,
        revision_id: selected.view.revision_id,
        placement_id: placements[1].placement_id,
    };
    let c_reading_step = ImpactStep {
        from: ImpactNode::Block(cref.clone()),
        location: Some(c_location.clone()),
        ..to_reading.clone()
    };
    let mut c_structural = impact_query(cref.clone(), released_scope.clone());
    c_structural.families = vec![ImpactFamily::Structural];
    c_structural.max_depth = 2;
    let c_result = impact.traverse(actor, c_structural).await.unwrap().unwrap();
    assert!(matches!(c_result.status(), PageStatus::Complete));
    assert_eq!(c_result.consumers().len(), 2);
    let c_reading_group = c_result
        .consumers()
        .iter()
        .find(|group| group.consumer == reading_node)
        .unwrap();
    assert_eq!(c_reading_group.locations, vec![c_location.clone()]);
    assert_eq!(
        c_reading_group.explanations,
        vec![ImpactExplanation {
            steps: vec![c_reading_step.clone()],
        }]
    );
    let c_release_group = c_result
        .consumers()
        .iter()
        .find(|group| group.consumer == release_node)
        .unwrap();
    assert_eq!(c_release_group.locations, vec![c_location]);
    assert_eq!(
        c_release_group.explanations,
        vec![ImpactExplanation {
            steps: vec![c_reading_step, to_release.clone()],
        }]
    );
    let original_location = ImpactLocation::Occurrence {
        path: vec![doc.nodes[0].occurrence_id],
    };
    let original_to_doc = ImpactStep {
        from: ImpactNode::Block(original_ref.clone()),
        to: ImpactNode::Composition(doc.reference.clone()),
        provenance: ImpactProvenance::Stored,
        location: Some(original_location.clone()),
        ..to_reading.clone()
    };
    let original_to_reading = ImpactStep {
        from: ImpactNode::Block(original_ref.clone()),
        location: Some(original_location.clone()),
        ..to_reading.clone()
    };
    let doc_to_reading = ImpactStep {
        from: ImpactNode::Composition(doc.reference.clone()),
        location: None,
        ..to_reading.clone()
    };
    let doc_to_release = ImpactStep {
        from: ImpactNode::Composition(doc.reference.clone()),
        ..to_release.clone()
    };
    let mut original_structural = impact_query(original_ref.clone(), released_scope.clone());
    original_structural.families = vec![ImpactFamily::Structural];
    original_structural.max_depth = 2;
    let original_result = impact
        .traverse(actor, original_structural)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(original_result.status(), PageStatus::Complete));
    assert_eq!(original_result.consumers().len(), 3);
    let composition_group = original_result
        .consumers()
        .iter()
        .find(|group| group.consumer == ImpactNode::Composition(doc.reference.clone()))
        .unwrap();
    assert_eq!(composition_group.locations, vec![original_location.clone()]);
    assert_eq!(
        composition_group.explanations,
        vec![ImpactExplanation {
            steps: vec![original_to_doc.clone()],
        }]
    );
    let original_reading_group = original_result
        .consumers()
        .iter()
        .find(|group| group.consumer == reading_node)
        .unwrap();
    assert_eq!(
        original_reading_group.locations,
        vec![original_location.clone()]
    );
    assert_eq!(original_reading_group.explanations.len(), 2);
    assert!(
        original_reading_group
            .explanations
            .contains(&ImpactExplanation {
                steps: vec![original_to_reading.clone()],
            })
    );
    assert!(
        original_reading_group
            .explanations
            .contains(&ImpactExplanation {
                steps: vec![original_to_doc.clone(), doc_to_reading],
            })
    );
    let original_release_group = original_result
        .consumers()
        .iter()
        .find(|group| group.consumer == release_node)
        .unwrap();
    assert_eq!(original_release_group.locations, vec![original_location]);
    assert_eq!(original_release_group.explanations.len(), 2);
    assert!(
        original_release_group
            .explanations
            .contains(&ImpactExplanation {
                steps: vec![original_to_doc, doc_to_release],
            })
    );
    assert!(
        original_release_group
            .explanations
            .contains(&ImpactExplanation {
                steps: vec![original_to_reading, to_release.clone()],
            })
    );
    let local_evidence = impact
        .evidence(
            actor,
            EvidenceQuery {
                endpoint: href.clone(),
                scope: released_scope.clone(),
                include_dynamic: false,
                limit: 50,
                work_limit: 4096,
                after: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        local_evidence.status(),
        EvidencePageStatus::Complete
    ));
    assert_eq!(local_evidence.assertions().len(), 3);
    assert_eq!(local_evidence.judgments().len(), 1);
    for (from, relation, review, relation_type) in [
        (
            &e1,
            &saved_relations[0],
            &checked[0],
            RelationType::Supports,
        ),
        (
            &e2,
            &saved_relations[1],
            &checked[1],
            RelationType::Supports,
        ),
        (&x, &saved_relations[2], &checked[2], RelationType::Opposes),
    ] {
        let assertion = local_evidence
            .assertions()
            .iter()
            .find(|item| item.relation.reference == relation.reference)
            .unwrap();
        assert_eq!(assertion.relation.from, exact(from));
        assert_eq!(assertion.relation.to, href);
        assert_eq!(assertion.relation.relation_type, relation_type);
        assert_eq!(assertion.relation.conditions, "protocol P only");
        assert_eq!(
            assertion.relation.rationale,
            "human records toy observation"
        );
        assert_eq!(assertion.entry_direction, TraversalDirection::SavedReverse);
        assert!(matches!(&assertion.sources[..],
            [EvidenceSource::FixedReleaseSelection { view, review: Some(ReviewProjection::Available(found)) }]
            if *view == selected.view && found.reference == review.reference
                && found.explanation == "human checked synthetic record within P"));

        let mut semantic_query = impact_query(exact(from), released_scope.clone());
        semantic_query.families = vec![ImpactFamily::Semantic];
        semantic_query.max_depth = 2;
        let semantic_result = impact
            .traverse(actor, semantic_query)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(semantic_result.status(), PageStatus::Complete));
        assert_eq!(semantic_result.consumers().len(), 2);
        let relation_node = ImpactNode::Relation(relation.reference.clone());
        let entry = ImpactStep {
            from: ImpactNode::Block(exact(from)),
            to: relation_node.clone(),
            family: ImpactFamily::Semantic,
            dependency_role: None,
            dependency_position: None,
            direction: Some(TraversalDirection::SavedForward),
            relation_type: Some(relation_type),
            lineage_type: None,
            provenance: ImpactProvenance::FixedReleaseManifest,
            location: None,
            reason: ImpactReason::SuggestReview,
        };
        let counterpart = ImpactStep {
            from: relation_node.clone(),
            to: ImpactNode::Block(href.clone()),
            ..entry.clone()
        };
        let relation_group = semantic_result
            .consumers()
            .iter()
            .find(|group| group.consumer == relation_node)
            .unwrap();
        assert!(relation_group.locations.is_empty());
        assert_eq!(
            relation_group.explanations,
            vec![ImpactExplanation {
                steps: vec![entry.clone()],
            }]
        );
        let h_group = semantic_result
            .consumers()
            .iter()
            .find(|group| group.consumer == ImpactNode::Block(href.clone()))
            .unwrap();
        assert!(h_group.locations.is_empty());
        assert_eq!(
            h_group.explanations,
            vec![ImpactExplanation {
                steps: vec![entry, counterpart],
            }]
        );

        let result = impact
            .traverse(actor, impact_query(exact(from), released_scope.clone()))
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(result.status(), PageStatus::Complete));
        let conclusion_group = result
            .consumers()
            .iter()
            .find(|group| group.consumer == ImpactNode::Block(cref.clone()))
            .unwrap();
        assert!(
            conclusion_group
                .explanations
                .iter()
                .any(|path| matches!(&path.steps[..],
            [entry, counterpart, dependence]
            if entry.from == ImpactNode::Block(exact(from))
                && entry.to == ImpactNode::Relation(relation.reference.clone())
                && entry.family == ImpactFamily::Semantic
                && entry.direction == Some(TraversalDirection::SavedForward)
                && entry.relation_type == Some(relation_type)
                && entry.provenance == ImpactProvenance::FixedReleaseManifest
                && entry.reason == ImpactReason::SuggestReview
                && counterpart.from == ImpactNode::Relation(relation.reference.clone())
                && counterpart.to == ImpactNode::Block(href.clone())
                && counterpart.direction == Some(TraversalDirection::SavedForward)
                && counterpart.relation_type == Some(relation_type)
                && counterpart.provenance == ImpactProvenance::FixedReleaseManifest
                && dependence.from == ImpactNode::Block(href.clone())
                && dependence.to == ImpactNode::Block(cref.clone())
                && dependence.family == ImpactFamily::Necessary
                && dependence.dependency_role == Some(DependencyRole::Basis)
                && dependence.dependency_position == Some(0)
                && dependence.reason == ImpactReason::RequiresExactRevision))
        );
    }
    assert!(matches!(&local_evidence.judgments()[0],
        EvidenceJudgment::Available { views, review }
        if views == &vec![selected.view.clone()] && review.reference == judgment.reference
            && review.explanation == "E1 and E2 share R; X is counterevidence; no independence or truth inferred"));
    let mut necessary_query = impact_query(href.clone(), released_scope.clone());
    necessary_query.families = vec![ImpactFamily::Necessary];
    necessary_query.max_depth = 1;
    let necessary_result = impact
        .traverse(actor, necessary_query)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(necessary_result.status(), PageStatus::Complete));
    assert_eq!(necessary_result.consumers().len(), 5);
    for (consumer, role, position) in [
        (
            ImpactNode::Relation(saved_relations[0].reference.clone()),
            DependencyRole::Target,
            1,
        ),
        (
            ImpactNode::Relation(saved_relations[1].reference.clone()),
            DependencyRole::Target,
            1,
        ),
        (
            ImpactNode::Relation(saved_relations[2].reference.clone()),
            DependencyRole::Target,
            1,
        ),
        (
            ImpactNode::EpistemicReview(judgment.reference.clone()),
            DependencyRole::Target,
            0,
        ),
        (ImpactNode::Block(cref.clone()), DependencyRole::Basis, 0),
    ] {
        let group = necessary_result
            .consumers()
            .iter()
            .find(|group| group.consumer == consumer)
            .unwrap();
        assert!(group.locations.is_empty());
        assert_eq!(
            group.explanations,
            vec![ImpactExplanation {
                steps: vec![ImpactStep {
                    from: ImpactNode::Block(href.clone()),
                    to: consumer,
                    family: ImpactFamily::Necessary,
                    dependency_role: Some(role),
                    dependency_position: Some(position),
                    direction: None,
                    relation_type: None,
                    lineage_type: None,
                    provenance: ImpactProvenance::Stored,
                    location: None,
                    reason: ImpactReason::RequiresExactRevision,
                }],
            }]
        );
    }
    let mut selected_nodes = vec![];
    for (relation, review) in saved_relations.iter().zip(&checked) {
        selected_nodes.push((
            ImpactStart::Relation(relation.reference.clone()),
            ImpactNode::Relation(relation.reference.clone()),
        ));
        selected_nodes.push((
            ImpactStart::RelationReview(review.reference.clone()),
            ImpactNode::RelationReview(review.reference.clone()),
        ));
    }
    selected_nodes.push((
        ImpactStart::EpistemicReview(judgment.reference.clone()),
        ImpactNode::EpistemicReview(judgment.reference.clone()),
    ));
    for (start, node) in selected_nodes {
        let result = impact
            .traverse(
                actor,
                ImpactQuery {
                    start,
                    scope: released_scope.clone(),
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
        assert!(matches!(result.status(), PageStatus::Complete));
        assert_eq!(result.start_membership(), ImpactMembership::Selected);
        assert_eq!(result.consumers().len(), 1);
        let group = &result.consumers()[0];
        assert_eq!(group.consumer, reading_node);
        assert!(group.locations.is_empty());
        assert_eq!(
            group.explanations,
            vec![ImpactExplanation {
                steps: vec![ImpactStep {
                    from: node,
                    to: reading_node.clone(),
                    family: ImpactFamily::ReviewSelection,
                    dependency_role: None,
                    dependency_position: None,
                    direction: None,
                    relation_type: None,
                    lineage_type: None,
                    provenance: ImpactProvenance::FixedReleaseManifest,
                    location: None,
                    reason: ImpactReason::SelectedEvidence,
                }],
            }]
        );
    }
    let next = content
        .revise(
            actor,
            href.block_id,
            ReviseContent {
                request_id: Uuid::new_v4(),
                base_revision_id: href.revision_id,
                draft: draft(
                    Intent::Conjecture,
                    H2,
                    vec![ExactRef::Block(original_ref.clone())],
                    None,
                ),
                reason: "narrow conditions, not historical evidence".into(),
            },
        )
        .await
        .unwrap();
    assert_ne!(next.revision_id, href.revision_id);
    assert_body(
        &next,
        "conjecture",
        H2,
        vec![ExactRef::Block(original_ref.clone())],
        None,
    );
    assert_eq!(next.block_id, href.block_id);
    for scope in [
        ImpactScope::Reading {
            view: selected.view.clone(),
            mode: ReadingMode::Fused,
        },
        released_scope.clone(),
    ] {
        // The mutable head has advanced, but neither saved view nor release
        // acquired H@2. An exact H@1 impact query remains available.
        assert!(
            impact
                .traverse(actor, impact_query(exact(&next), scope.clone()))
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            impact
                .traverse(actor, impact_query(href.clone(), scope))
                .await
                .unwrap()
                .is_some()
        );
    }
    let next_judgment = reviews
        .append(
            actor,
            AppendEpistemicReview {
                request_id: Uuid::new_v4(),
                target: exact(&next),
                state: EpistemicState::Untested,
                relations: vec![],
                evidence: vec![],
                explanation: "new target has no transferred judgment".into(),
                ..judgment_command.clone()
            },
        )
        .await
        .unwrap();
    assert_ne!(
        next_judgment.reference.stream_id,
        judgment.reference.stream_id
    );
    assert_eq!(next_judgment.state, EpistemicState::Untested);
    let withdrawn = relations
        .review(
            actor,
            ReviewRelation {
                request_id: Uuid::new_v4(),
                relation: saved_relations[0].reference.clone(),
                expected_previous: Some(checked[0].reference.review_id),
                state: RelationReviewState::Withdrawn,
                explanation: "human withdraws current endorsement".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        withdrawn.previous_review_id,
        Some(checked[0].reference.review_id)
    );
    assert_eq!(withdrawn.state, RelationReviewState::Withdrawn);
    let mut unsupported = judgment_command.clone();
    unsupported.request_id = Uuid::new_v4();
    unsupported.expected_previous = Some(judgment.reference.clone());
    unsupported.state = EpistemicState::SupportedWithinScope;
    unsupported.relations = vec![selections[0].clone()];
    unsupported.evidence = vec![exact(&e1)];
    assert!(matches!(
        reviews.append(actor, unsupported).await,
        Err(ContentError::Invalid(_))
    ));
    assert_snapshot(
        &reading
            .read_versioned(actor, selected.view.clone(), ReadingMode::Fused)
            .await
            .unwrap()
            .unwrap(),
    );
    // A target card and the conclusion both depend on the hidden source.
    let card_draft = ContentDraft::V2(ContentV2 {
        intent: Intent::Note,
        language: "en".into(),
        title: "source card".into(),
        body: BodyV2::Reference { target: exact(&e1) },
        basis_refs: vec![],
        requires_context: vec![],
        source_run: None,
    });
    let card = content
        .create(actor, space, create(card_draft))
        .await
        .unwrap();
    r.revoke(actor, source_space).await;
    let hidden_scope = ImpactScope::Reading {
        view: selected.view.clone(),
        mode: ReadingMode::Fused,
    };
    let hidden_evidence = impact
        .evidence(
            actor,
            EvidenceQuery {
                endpoint: href.clone(),
                scope: hidden_scope.clone(),
                include_dynamic: false,
                limit: 50,
                work_limit: 4096,
                after: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        hidden_evidence.status(),
        EvidencePageStatus::Complete
    ));
    assert_eq!(hidden_evidence.assertions().len(), 1);
    assert_eq!(
        hidden_evidence.assertions()[0].relation.reference,
        saved_relations[2].reference
    );
    assert_eq!(hidden_evidence.judgments(), &[EvidenceJudgment::Incomplete]);
    let hidden_b4_wire = serde_json::to_string(&hidden_evidence).unwrap();
    for secret in [
        e1.block_id.to_string(),
        e2.block_id.to_string(),
        judgment.reference.review_id.to_string(),
        saved_relations[0].reference.relation_id.to_string(),
        source_space.to_string(),
    ] {
        assert!(!hidden_b4_wire.contains(&secret));
    }
    assert!(
        impact
            .traverse(actor, impact_query(exact(&e1), released_scope.clone()))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        impact
            .traverse(actor, impact_query(href.clone(), released_scope.clone()))
            .await
            .unwrap()
            .is_none()
    );
    let mut hidden_structural = impact_query(href.clone(), hidden_scope);
    hidden_structural.families = vec![ImpactFamily::Structural];
    hidden_structural.max_depth = 2;
    let visible_h = impact
        .traverse(actor, hidden_structural)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(visible_h.status(), PageStatus::Complete));
    assert_eq!(visible_h.consumers().len(), 1);
    assert_eq!(visible_h.consumers()[0].consumer, reading_node);
    assert_eq!(visible_h.consumers()[0].locations, vec![h_location.clone()]);
    for reference in [exact(&e1), exact(&e2), cref.clone(), exact(&card)] {
        assert_eq!(content.read(actor, reference).await.unwrap(), None);
    }
    let visible = content
        .read_many(
            actor,
            vec![
                href.clone(),
                exact(&e1),
                exact(&e2),
                cref.clone(),
                exact(&card),
            ],
        )
        .await
        .unwrap();
    assert_eq!(visible.len(), 1);
    assert_eq!(exact(&visible[0]), href);
    assert_eq!(
        reviews
            .read(actor, judgment.reference.clone())
            .await
            .unwrap(),
        ReviewProjection::Incomplete
    );
    assert_eq!(
        relations
            .read_review(actor, checked[0].reference.clone())
            .await
            .unwrap(),
        ReviewProjection::Incomplete
    );
    let visible_relations = relations
        .read_selected(
            actor,
            selections.iter().map(|s| s.relation.clone()).collect(),
        )
        .await
        .unwrap();
    assert_eq!(visible_relations.len(), 1);
    assert_eq!(visible_relations[0].from, exact(&x));
    assert_eq!(visible_relations[0].to, href);
    let hidden = reading
        .read_versioned(actor, selected.view.clone(), ReadingMode::Fused)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        bodies(&hidden),
        vec![
            (original_ref.clone(), ATTENTION.into()),
            (href.clone(), H1.into())
        ]
    );
    assert_eq!(hidden.evidence.selections.len(), 1);
    assert_eq!(
        hidden.evidence.epistemic_reviews,
        [ReviewProjection::Incomplete]
    );
    let hidden_wire = serde_json::to_string(&hidden).unwrap();
    for secret in [
        e1.block_id.to_string(),
        e1.revision_id.to_string(),
        e2.block_id.to_string(),
        conclusion.block_id.to_string(),
        judgment.reference.review_id.to_string(),
        saved_relations[0].reference.relation_id.to_string(),
        source_space.to_string(),
        E1.into(),
        E2.into(),
        C1.into(),
    ] {
        assert!(
            !hidden_wire.contains(&secret),
            "hidden identifier or content leaked"
        );
    }
    assert_eq!(
        r.releases()
            .read_evidence(actor, released.release_id)
            .await
            .unwrap(),
        None
    );
    r.grant(actor, source_space, false).await;
    let restored_evidence = impact
        .evidence(
            actor,
            EvidenceQuery {
                endpoint: href.clone(),
                scope: released_scope.clone(),
                include_dynamic: false,
                limit: 50,
                work_limit: 4096,
                after: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(restored_evidence, local_evidence);
    assert!(
        impact
            .traverse(actor, impact_query(exact(&e1), released_scope.clone()))
            .await
            .unwrap()
            .is_some()
    );
    assert_snapshot(
        &reading
            .read_versioned(actor, selected.view.clone(), ReadingMode::Fused)
            .await
            .unwrap()
            .unwrap(),
    );
    let restored = r
        .releases()
        .read_evidence(actor, released.release_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(restored.readings, std::slice::from_ref(&selected.view));
    assert_eq!(restored.manifest_sha256, expected_manifest);
    assert_body(
        &content.read(actor, cref.clone()).await.unwrap().unwrap(),
        "conclusion",
        C1,
        c_basis.clone(),
        None,
    );
    for (block, intent, text, run) in [
        (&e1, "evidence", E1, Some(run_ref.clone())),
        (&e2, "evidence", E2, Some(run_ref)),
        (&x, "evidence", X, None),
    ] {
        assert_body(
            &content.read(actor, exact(block)).await.unwrap().unwrap(),
            intent,
            text,
            vec![],
            run,
        );
    }
    assert_body(
        &content.read(actor, href.clone()).await.unwrap().unwrap(),
        "conjecture",
        H1,
        vec![ExactRef::Block(original_ref)],
        None,
    );
    let split = LineageStore::new(r.runtime_pool.clone())
        .apply(
            actor,
            space,
            LineageCommand {
                request_id: Uuid::new_v4(),
                operation: LineageOperation::Split,
                inputs: vec![cref.clone()],
                outputs: vec![
                    draft(Intent::Note, "C part A: evidence summary", vec![], None),
                    draft(Intent::Note, "C part B: open questions", vec![], None),
                ],
                reason: "explicit synthetic split".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(split.inputs, std::slice::from_ref(&cref));
    assert_eq!(split.outputs.len(), 2);
    assert_ne!(split.outputs[0].block_id, split.outputs[1].block_id);
    for (i, output) in split.outputs.iter().enumerate() {
        assert_ne!(output.block_id, cref.block_id);
        let value = content.read(actor, output.clone()).await.unwrap().unwrap();
        assert_body(
            &value,
            "note",
            ["C part A: evidence summary", "C part B: open questions"][i],
            vec![],
            None,
        );
    }
    let lineage = split.system_relations();
    assert_eq!(lineage.len(), 2);
    for (i, link) in lineage.iter().enumerate() {
        assert_eq!(link.from, split.outputs[i]);
        assert_eq!(link.to, cref);
        assert_eq!(json!(link.relation_type), json!("split_from"));
    }
    let mut historical_lineage = impact_query(cref.clone(), released_scope.clone());
    historical_lineage.families = vec![ImpactFamily::Lineage];
    historical_lineage.max_depth = 2;
    let historical_result = impact
        .traverse(actor, historical_lineage)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(historical_result.status(), PageStatus::Complete));
    assert!(historical_result.consumers().is_empty());

    let split_doc = r
        .compositions()
        .save(
            actor,
            space,
            a::doc(vec![
                NodeTarget::Block(cref.clone()),
                NodeTarget::Block(split.outputs[0].clone()),
                NodeTarget::Block(split.outputs[1].clone()),
            ]),
        )
        .await
        .unwrap();
    let split_reading = reading
        .create(actor, space, h::create(split_doc.reference))
        .await
        .unwrap();
    let mut current_lineage = impact_query(
        cref.clone(),
        ImpactScope::Reading {
            view: split_reading.view,
            mode: ReadingMode::Original,
        },
    );
    current_lineage.families = vec![ImpactFamily::Lineage];
    current_lineage.max_depth = 2;
    let current_result = impact
        .traverse(actor, current_lineage)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(current_result.status(), PageStatus::Complete));
    assert_eq!(current_result.consumers().len(), 3);
    let operation_node = ImpactNode::Lineage {
        operation_id: split.operation_id,
    };
    let input_step = ImpactStep {
        from: ImpactNode::Block(cref.clone()),
        to: operation_node.clone(),
        family: ImpactFamily::Lineage,
        dependency_role: None,
        dependency_position: None,
        direction: Some(TraversalDirection::SavedReverse),
        relation_type: None,
        lineage_type: Some(SystemLineageType::SplitFrom),
        provenance: ImpactProvenance::Stored,
        location: None,
        reason: ImpactReason::DerivedFrom,
    };
    let operation_group = current_result
        .consumers()
        .iter()
        .find(|group| group.consumer == operation_node)
        .unwrap();
    assert!(operation_group.locations.is_empty());
    assert_eq!(
        operation_group.explanations,
        vec![ImpactExplanation {
            steps: vec![input_step.clone()],
        }]
    );
    for output in &split.outputs {
        let output_group = current_result
            .consumers()
            .iter()
            .find(|group| group.consumer == ImpactNode::Block(output.clone()))
            .unwrap();
        assert!(output_group.locations.is_empty());
        assert_eq!(
            output_group.explanations,
            vec![ImpactExplanation {
                steps: vec![
                    input_step.clone(),
                    ImpactStep {
                        from: operation_node.clone(),
                        to: ImpactNode::Block(output.clone()),
                        ..input_step.clone()
                    },
                ],
            }]
        );
    }
    assert_body(
        &content.read(actor, cref).await.unwrap().unwrap(),
        "conclusion",
        C1,
        c_basis,
        None,
    );
    assert_snapshot(
        &reading
            .read_versioned(actor, selected.view.clone(), ReadingMode::Fused)
            .await
            .unwrap()
            .unwrap(),
    );
    assert_eq!(
        r.releases()
            .read_evidence(actor, released.release_id)
            .await
            .unwrap()
            .unwrap()
            .manifest_sha256,
        expected_manifest
    );
}

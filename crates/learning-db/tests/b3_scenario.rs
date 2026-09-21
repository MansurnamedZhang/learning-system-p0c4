//! Synthetic acceptance data, not a scientific experiment or an automatic truth judgment.
#[path = "support/relation_store.rs"]
mod relation_helpers;
mod support;
use learning_core::*;
use learning_db::{LineageStore, RelationStore, ReviewStore, VersionedContentStore};
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

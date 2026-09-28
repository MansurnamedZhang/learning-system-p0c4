#[path = "support/relation_store.rs"]
mod relations;
mod support;

use learning_core::*;
use learning_db::{QueryStore, RelationStore, ReviewStore};
use support::{assembly as a, reading as r};
use uuid::Uuid;

fn endpoint(doc: &CompositionRevision, index: usize) -> BlockRef {
    match &doc.nodes[index].target {
        NodeTarget::Block(b) => b.clone(),
        _ => panic!("block expected"),
    }
}

fn query(endpoint: BlockRef, scope: ImpactScope, include_dynamic: bool) -> EvidenceQuery {
    EvidenceQuery {
        endpoint,
        scope,
        include_dynamic,
        limit: 50,
        work_limit: 4096,
        after: None,
    }
}

#[tokio::test]
async fn fixed_evidence_preserves_saved_direction_conditions_and_review_pairing() {
    let (rig, actor, space, doc, zero) = r::fixture().await;
    let h = endpoint(&doc, 0);
    let e = endpoint(&doc, 1);
    let store = RelationStore::new(rig.runtime_pool.clone());
    let mut save = relations::save(space, e.clone(), h.clone());
    save.conditions = "protocol P only".into();
    save.rationale = "E supports H within P".into();
    let assertion = store.save(actor, save).await.unwrap();
    let review = store
        .review(actor, relations::review(&assertion))
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
                    relation: assertion.reference.clone(),
                    review: Some(review.reference.clone()),
                }],
                epistemic_reviews: vec![],
                reason: "fixed assertion".into(),
            },
        )
        .await
        .unwrap();
    let scope = ImpactScope::Reading {
        view: selected.view.clone(),
        mode: ReadingMode::Fused,
    };
    let result = QueryStore::new(rig.runtime_pool.clone())
        .evidence(actor, query(h, scope.clone(), false))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.assertions().len(), 1);
    let group = &result.assertions()[0];
    assert_eq!(group.relation.reference, assertion.reference);
    assert_eq!(group.relation.from, e);
    assert_eq!(group.relation.to, endpoint(&doc, 0));
    assert_eq!(group.relation.conditions, "protocol P only");
    assert_eq!(group.relation.rationale, "E supports H within P");
    assert_eq!(group.entry_direction, TraversalDirection::SavedReverse);
    assert!(
        matches!(&group.sources[..], [EvidenceSource::FixedReadingSelection { view, review: Some(ReviewProjection::Available(found)) }] if *view == selected.view && found.reference == review.reference)
    );
    assert!(matches!(result.status(), EvidencePageStatus::Complete));
    let reverse = QueryStore::new(rig.runtime_pool.clone())
        .evidence(actor, query(endpoint(&doc, 1), scope, false))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        reverse.assertions()[0].entry_direction,
        TraversalDirection::SavedForward
    );
}

#[tokio::test]
async fn release_rejects_dynamic_working_before_storage_access() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://nobody:unused@127.0.0.1:1/unused")
        .unwrap();
    let actor = Principal {
        actor_id: Uuid::new_v4(),
    };
    let request = query(
        BlockRef {
            block_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
        },
        ImpactScope::Release {
            release_id: Uuid::new_v4(),
        },
        true,
    );
    assert!(
        matches!(QueryStore::new(pool).evidence(actor, request).await, Err(ContentError::Invalid(code)) if code == "dynamic_requires_reading")
    );
}

#[tokio::test]
async fn fixed_history_and_current_dynamic_head_remain_distinct_and_withdrawal_only_hides_dynamic()
{
    let (rig, actor, space, doc, zero) = r::fixture().await;
    let h = endpoint(&doc, 0);
    let e = endpoint(&doc, 1);
    let store = RelationStore::new(rig.runtime_pool.clone());
    let one = store
        .save(actor, relations::save(space, e, h.clone()))
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
                    relation: one.reference.clone(),
                    review: None,
                }],
                epistemic_reviews: vec![],
                reason: "pin old relation".into(),
            },
        )
        .await
        .unwrap();
    let mut edit = relations::edit(&one);
    edit.rationale = "revised assertion".into();
    let two = store.save(actor, edit).await.unwrap();
    let scope = ImpactScope::Reading {
        view: selected.view,
        mode: ReadingMode::Fused,
    };
    let query = query(h, scope, true);
    let found = QueryStore::new(rig.runtime_pool.clone())
        .evidence(actor, query.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.assertions().len(), 2);
    let old = found
        .assertions()
        .iter()
        .find(|g| g.relation.reference == one.reference)
        .unwrap();
    assert!(matches!(
        &old.sources[..],
        [EvidenceSource::FixedReadingSelection { .. }]
    ));
    let current = found
        .assertions()
        .iter()
        .find(|g| g.relation.reference == two.reference)
        .unwrap();
    assert!(matches!(
        &current.sources[..],
        [EvidenceSource::DynamicWorking { .. }]
    ));
    let mut withdraw = relations::review(&two);
    withdraw.state = RelationReviewState::Withdrawn;
    store.review(actor, withdraw).await.unwrap();
    let after = QueryStore::new(rig.runtime_pool.clone())
        .evidence(actor, query)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.assertions().len(), 1);
    assert_eq!(after.assertions()[0].relation.reference, one.reference);
}

#[tokio::test]
async fn incomplete_selected_judgment_is_one_anonymous_marker_only_at_its_exact_target() {
    let rig = support::TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let mut a = support::command("A conjecture");
    a.draft.intent = Intent::Conjecture;
    let a = rig.store.create(actor, space, a).await.unwrap();
    let b = rig
        .store
        .create(actor, space, support::command("B note"))
        .await
        .unwrap();
    let doc = rig
        .compositions()
        .save(actor, space, a::doc(vec![a::block(&a), a::block(&b)]))
        .await
        .unwrap();
    let zero = r::store(&rig)
        .create(actor, space, r::create(doc.reference.clone()))
        .await
        .unwrap();
    let (owner, basis_space) = rig.seed_actor_space(true).await;
    let basis = rig
        .store
        .create(owner, basis_space, support::command("private basis"))
        .await
        .unwrap();
    rig.grant(actor, basis_space, false).await;
    let reviewed = ReviewStore::new(rig.runtime_pool.clone())
        .append(
            actor,
            AppendEpistemicReview {
                request_id: Uuid::new_v4(),
                scope: RelationScope::Space { space_id: space },
                target: endpoint(&doc, 0),
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
                epistemic_reviews: vec![reviewed.reference.clone()],
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
    let hidden_basis = store
        .evidence(actor, query(endpoint(&doc, 0), scope.clone(), false))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(hidden_basis.judgments(), &[EvidenceJudgment::Incomplete]);
    let wire = serde_json::to_value(hidden_basis).unwrap();
    assert_eq!(
        wire["judgments"],
        serde_json::json!([{"type":"incomplete"}])
    );
    let different_target = store
        .evidence(actor, query(endpoint(&doc, 1), scope.clone(), false))
        .await
        .unwrap()
        .unwrap();
    assert!(different_target.judgments().is_empty());
    rig.grant(actor, basis_space, false).await;
    let restored = store
        .evidence(actor, query(endpoint(&doc, 0), scope, false))
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(restored.judgments(), [EvidenceJudgment::Available { review, .. }] if review.reference == reviewed.reference)
    );
}

#[tokio::test]
async fn selected_visible_review_and_selection_edges_consume_atomic_group_budget() {
    let (rig, actor, space, doc, zero) = r::fixture().await;
    let h = endpoint(&doc, 0);
    let e = endpoint(&doc, 1);
    let store = RelationStore::new(rig.runtime_pool.clone());
    let relation = store
        .save(actor, relations::save(space, e, h.clone()))
        .await
        .unwrap();
    let select = |saved: &ReadingSaved, review: Option<RelationReviewRef>| SelectRelations {
        request_id: Uuid::new_v4(),
        expected_overlay_revision: saved.overlay.revision_id,
        expected_reading_view_revision: saved.view.revision_id,
        selections: vec![RelationSelection {
            relation: relation.reference.clone(),
            review,
        }],
        epistemic_reviews: vec![],
        reason: "pin evidence".into(),
    };
    let without_review = r::store(&rig)
        .select_relations(actor, zero.overlay.overlay_id, select(&zero, None))
        .await
        .unwrap();
    let review = store
        .review(actor, relations::review(&relation))
        .await
        .unwrap();
    let with_review = r::store(&rig)
        .select_relations(
            actor,
            zero.overlay.overlay_id,
            select(&without_review, Some(review.reference)),
        )
        .await
        .unwrap();
    let mut plain = query(
        h.clone(),
        ImpactScope::Reading {
            view: without_review.view,
            mode: ReadingMode::Fused,
        },
        false,
    );
    plain.work_limit = 7;
    let plain = QueryStore::new(rig.runtime_pool.clone())
        .evidence(actor, plain)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(plain.status(), EvidencePageStatus::Complete));
    assert_eq!(plain.assertions().len(), 1);
    let mut reviewed = query(
        h,
        ImpactScope::Reading {
            view: with_review.view,
            mode: ReadingMode::Fused,
        },
        false,
    );
    reviewed.work_limit = 7;
    let reviewed = QueryStore::new(rig.runtime_pool.clone())
        .evidence(actor, reviewed)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        reviewed.status(),
        EvidencePageStatus::BudgetExceeded
    ));
    assert!(reviewed.assertions().is_empty());
    assert!(reviewed.judgments().is_empty());
}

#[tokio::test]
async fn visible_dynamic_relations_exhaust_work_before_returning_a_partial_page() {
    let (rig, actor, space, doc, zero) = r::fixture().await;
    let h = endpoint(&doc, 0);
    let store = RelationStore::new(rig.runtime_pool.clone());
    for text in ["E1", "E2", "E3"] {
        let e = rig
            .store
            .create(actor, space, support::command(text))
            .await
            .unwrap();
        store
            .save(
                actor,
                relations::save(space, relations::exact(&e), h.clone()),
            )
            .await
            .unwrap();
    }
    let mut request = query(
        h,
        ImpactScope::Reading {
            view: zero.view,
            mode: ReadingMode::Fused,
        },
        true,
    );
    request.work_limit = 9; // start + two complete one-hop assertions, not the third
    let result = QueryStore::new(rig.runtime_pool.clone())
        .evidence(actor, request)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        result.status(),
        EvidencePageStatus::BudgetExceeded
    ));
    assert!(result.assertions().is_empty());
}

#[tokio::test]
async fn e1_e2_counterexample_and_two_hypothesis_revisions_keep_exact_endpoints() {
    let rig = support::TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let h1 = rig
        .store
        .create(actor, space, support::command("H@1"))
        .await
        .unwrap();
    let h2 = rig
        .store
        .revise(actor, h1.block_id, support::change(&h1, "H@2"))
        .await
        .unwrap();
    let e1 = rig
        .store
        .create(actor, space, support::command("E1"))
        .await
        .unwrap();
    let e2 = rig
        .store
        .create(actor, space, support::command("E2"))
        .await
        .unwrap();
    let x = rig
        .store
        .create(actor, space, support::command("X"))
        .await
        .unwrap();
    let doc = rig
        .compositions()
        .save(
            actor,
            space,
            a::doc(vec![
                a::block(&h1),
                a::block(&h2),
                a::block(&e1),
                a::block(&e2),
                a::block(&x),
            ]),
        )
        .await
        .unwrap();
    let zero = r::store(&rig)
        .create(actor, space, r::create(doc.reference))
        .await
        .unwrap();
    let relations = RelationStore::new(rig.runtime_pool.clone());
    let mut r1 = relations::save(space, relations::exact(&e1), relations::exact(&h1));
    r1.relation_type = RelationType::Supports;
    let r1 = relations.save(actor, r1).await.unwrap();
    let mut r2 = relations::save(space, relations::exact(&e2), relations::exact(&h1));
    r2.relation_type = RelationType::Tests;
    let r2 = relations.save(actor, r2).await.unwrap();
    let mut r3 = relations::save(space, relations::exact(&x), relations::exact(&h2));
    r3.relation_type = RelationType::Opposes;
    let r3 = relations.save(actor, r3).await.unwrap();
    let selected = r::store(&rig)
        .select_relations(
            actor,
            zero.overlay.overlay_id,
            SelectRelations {
                request_id: Uuid::new_v4(),
                expected_overlay_revision: zero.overlay.revision_id,
                expected_reading_view_revision: zero.view.revision_id,
                selections: [&r1, &r2, &r3]
                    .map(|rel| RelationSelection {
                        relation: rel.reference.clone(),
                        review: None,
                    })
                    .to_vec(),
                epistemic_reviews: vec![],
                reason: "pin explicit assertions".into(),
            },
        )
        .await
        .unwrap();
    let scope = ImpactScope::Reading {
        view: selected.view,
        mode: ReadingMode::Fused,
    };
    let store = QueryStore::new(rig.runtime_pool.clone());
    let one = store
        .evidence(actor, query(relations::exact(&h1), scope.clone(), false))
        .await
        .unwrap()
        .unwrap();
    let mut keys: Vec<_> = one
        .assertions()
        .iter()
        .map(|g| {
            (
                g.relation.reference.clone(),
                g.relation.relation_type,
                g.entry_direction,
            )
        })
        .collect();
    keys.sort_by_key(|(r, _, _)| r.clone());
    let mut expected = vec![
        (
            r1.reference.clone(),
            RelationType::Supports,
            TraversalDirection::SavedReverse,
        ),
        (
            r2.reference.clone(),
            RelationType::Tests,
            TraversalDirection::SavedReverse,
        ),
    ];
    expected.sort_by_key(|(r, _, _)| r.clone());
    assert_eq!(keys, expected);
    let two = store
        .evidence(actor, query(relations::exact(&h2), scope.clone(), false))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(two.assertions().len(), 1);
    assert_eq!(two.assertions()[0].relation.reference, r3.reference);
    assert_eq!(
        two.assertions()[0].entry_direction,
        TraversalDirection::SavedReverse
    );
    for (e, relation) in [(e1, r1), (e2, r2), (x, r3)] {
        let result = store
            .evidence(actor, query(relations::exact(&e), scope.clone(), false))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.assertions().len(), 1);
        assert_eq!(
            result.assertions()[0].relation.reference,
            relation.reference
        );
        assert_eq!(
            result.assertions()[0].entry_direction,
            TraversalDirection::SavedForward
        );
    }
}

#[tokio::test]
async fn page_path_budget_is_charged_only_for_the_returned_atomic_group() {
    let (rig, actor, space, doc, zero) = r::fixture().await;
    let h = endpoint(&doc, 0);
    let relations = RelationStore::new(rig.runtime_pool.clone());
    for text in ["page E1", "page E2"] {
        let e = rig
            .store
            .create(actor, space, support::command(text))
            .await
            .unwrap();
        relations
            .save(
                actor,
                relations::save(space, relations::exact(&e), h.clone()),
            )
            .await
            .unwrap();
    }
    let mut request = query(
        h,
        ImpactScope::Reading {
            view: zero.view,
            mode: ReadingMode::Fused,
        },
        true,
    );
    request.limit = 1;
    request.work_limit = 8; // start + two relation/endpoint/edge triples + one returned path
    let result = QueryStore::new(rig.runtime_pool.clone())
        .evidence(actor, request)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        result.status(),
        EvidencePageStatus::Truncated { .. }
    ));
    assert_eq!(result.assertions().len(), 1);
}

#[tokio::test]
async fn release_keeps_two_fixed_reading_review_pairs_in_one_exact_relation_group() {
    let (rig, actor, space, doc, first_zero) = r::fixture().await;
    let second_zero = r::store(&rig)
        .create(actor, space, r::create(doc.reference.clone()))
        .await
        .unwrap();
    let h = endpoint(&doc, 0);
    let e = endpoint(&doc, 1);
    let relations = RelationStore::new(rig.runtime_pool.clone());
    let relation = relations
        .save(actor, relations::save(space, e, h.clone()))
        .await
        .unwrap();
    let first_review = relations
        .review(actor, relations::review(&relation))
        .await
        .unwrap();
    let mut next_review = relations::review(&relation);
    next_review.expected_previous = Some(first_review.reference.review_id);
    next_review.explanation = "second fixed assessment".into();
    let second_review = relations.review(actor, next_review).await.unwrap();
    let select = |saved: &ReadingSaved, review: RelationReviewRef| SelectRelations {
        request_id: Uuid::new_v4(),
        expected_overlay_revision: saved.overlay.revision_id,
        expected_reading_view_revision: saved.view.revision_id,
        selections: vec![RelationSelection {
            relation: relation.reference.clone(),
            review: Some(review),
        }],
        epistemic_reviews: vec![],
        reason: "select historical review".into(),
    };
    let first = r::store(&rig)
        .select_relations(
            actor,
            first_zero.overlay.overlay_id,
            select(&first_zero, first_review.reference.clone()),
        )
        .await
        .unwrap();
    let second = r::store(&rig)
        .select_relations(
            actor,
            second_zero.overlay.overlay_id,
            select(&second_zero, second_review.reference.clone()),
        )
        .await
        .unwrap();
    let released = rig
        .releases()
        .publish_evidence(
            actor,
            space,
            PublishEvidence {
                request_id: Uuid::new_v4(),
                roots: vec![a::root(&doc, None)],
                readings: vec![first.view.clone(), second.view.clone()],
                reason: "two exact readings".into(),
            },
        )
        .await
        .unwrap();
    let mut withdrawn = relations::review(&relation);
    withdrawn.expected_previous = Some(second_review.reference.review_id);
    withdrawn.state = RelationReviewState::Withdrawn;
    relations.review(actor, withdrawn).await.unwrap();
    let result = QueryStore::new(rig.runtime_pool.clone())
        .evidence(
            actor,
            query(
                h,
                ImpactScope::Release {
                    release_id: released.release_id,
                },
                false,
            ),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.assertions().len(), 1);
    assert_eq!(
        result.assertions()[0].relation.reference,
        relation.reference
    );
    let sources = &result.assertions()[0].sources;
    assert_eq!(sources.len(), 2);
    let actual: Vec<_> = sources
        .iter()
        .map(|s| match s {
            EvidenceSource::FixedReleaseSelection {
                view,
                review: Some(ReviewProjection::Available(review)),
            } => (view.clone(), review.reference.clone()),
            _ => panic!("fixed release review expected"),
        })
        .collect();
    let mut expected = vec![
        (first.view, first_review.reference),
        (second.view, second_review.reference),
    ];
    expected.sort();
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn dynamic_relation_with_hidden_external_endpoint_matches_absence_and_restores() {
    let (rig, actor, space, doc, zero) = r::fixture().await;
    let h = endpoint(&doc, 0);
    let (owner, external_space) = rig.seed_actor_space(true).await;
    let e = rig
        .store
        .create(owner, external_space, support::command("external E"))
        .await
        .unwrap();
    rig.grant(actor, external_space, false).await;
    let request = query(
        h.clone(),
        ImpactScope::Reading {
            view: zero.view,
            mode: ReadingMode::Fused,
        },
        true,
    );
    let store = QueryStore::new(rig.runtime_pool.clone());
    let absent = store
        .evidence(actor, request.clone())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(absent.status(), EvidencePageStatus::Complete));
    assert!(absent.assertions().is_empty());
    let relations = RelationStore::new(rig.runtime_pool.clone());
    let relation = relations
        .save(
            actor,
            relations::save(space, relations::exact(&e), h.clone()),
        )
        .await
        .unwrap();
    let visible = store
        .evidence(actor, request.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(visible.assertions().len(), 1);
    assert_eq!(
        visible.assertions()[0].relation.reference,
        relation.reference
    );
    assert!(!visible.assertions()[0].other_in_scope);
    rig.revoke(actor, external_space).await;
    let hidden = store
        .evidence(actor, request.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(hidden, absent);
    assert!(matches!(hidden.status(), EvidencePageStatus::Complete));
    assert!(hidden.assertions().is_empty());
    assert!(hidden.judgments().is_empty());
    let wire = serde_json::to_value(&hidden).unwrap();
    assert_eq!(wire["assertions"], serde_json::json!([]));
    assert_eq!(wire["status"], serde_json::json!({"type":"complete"}));
    rig.grant(actor, external_space, false).await;
    let restored = store.evidence(actor, request).await.unwrap().unwrap();
    assert_eq!(restored.assertions().len(), 1);
    assert_eq!(
        restored.assertions()[0].relation.reference,
        relation.reference
    );
}

#[tokio::test]
async fn hidden_dynamic_candidates_do_not_change_visible_pages_or_public_budget() {
    let (rig, actor, space, doc, zero) = r::fixture().await;
    let h = endpoint(&doc, 0);
    let relations = RelationStore::new(rig.runtime_pool.clone());
    for text in ["visible E1", "visible E2"] {
        let e = rig
            .store
            .create(actor, space, support::command(text))
            .await
            .unwrap();
        relations
            .save(
                actor,
                relations::save(space, relations::exact(&e), h.clone()),
            )
            .await
            .unwrap();
    }
    let mut request = query(
        h.clone(),
        ImpactScope::Reading {
            view: zero.view,
            mode: ReadingMode::Fused,
        },
        true,
    );
    request.limit = 1;
    request.work_limit = 8;
    let store = QueryStore::new(rig.runtime_pool.clone());
    let first = store
        .evidence(actor, request.clone())
        .await
        .unwrap()
        .unwrap();
    let after = match first.status() {
        EvidencePageStatus::Truncated { after } => after.clone(),
        _ => panic!("first page must truncate"),
    };
    let mut next = request.clone();
    next.after = Some(after);
    let second = store.evidence(actor, next.clone()).await.unwrap().unwrap();
    assert!(matches!(second.status(), EvidencePageStatus::Complete));
    assert_eq!(second.assertions().len(), 1);
    let (owner, foreign) = rig.seed_actor_space(true).await;
    rig.grant(actor, foreign, false).await;
    for index in 0..12 {
        let e = rig
            .store
            .create(
                owner,
                foreign,
                support::command(&format!("hidden E{index}")),
            )
            .await
            .unwrap();
        relations
            .save(
                actor,
                relations::save(space, relations::exact(&e), h.clone()),
            )
            .await
            .unwrap();
    }
    rig.revoke(actor, foreign).await;
    assert_eq!(
        store.evidence(actor, request).await.unwrap().unwrap(),
        first
    );
    assert_eq!(store.evidence(actor, next).await.unwrap().unwrap(), second);
}

#[tokio::test]
async fn hidden_selected_judgment_root_is_omitted_without_an_incomplete_marker() {
    let rig = support::TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let mut conjecture = support::command("H");
    conjecture.draft.intent = Intent::Conjecture;
    let h = rig.store.create(actor, space, conjecture).await.unwrap();
    let doc = rig
        .compositions()
        .save(actor, space, a::doc(vec![a::block(&h)]))
        .await
        .unwrap();
    let zero = r::store(&rig)
        .create(actor, space, r::create(doc.reference.clone()))
        .await
        .unwrap();
    let (owner, review_space) = rig.seed_actor_space(true).await;
    rig.grant(actor, review_space, true).await;
    let reviewed = ReviewStore::new(rig.runtime_pool.clone())
        .append(
            actor,
            AppendEpistemicReview {
                request_id: Uuid::new_v4(),
                scope: RelationScope::Space {
                    space_id: review_space,
                },
                target: relations::exact(&h),
                expected_previous: None,
                state: EpistemicState::Untested,
                relations: vec![],
                evidence: vec![],
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
                epistemic_reviews: vec![reviewed.reference.clone()],
                reason: "pin external review".into(),
            },
        )
        .await
        .unwrap();
    let request = query(
        relations::exact(&h),
        ImpactScope::Reading {
            view: selected.view,
            mode: ReadingMode::Fused,
        },
        false,
    );
    let store = QueryStore::new(rig.runtime_pool.clone());
    assert!(matches!(
        store
            .evidence(actor, request.clone())
            .await
            .unwrap()
            .unwrap()
            .judgments(),
        [EvidenceJudgment::Available { .. }]
    ));
    rig.revoke(actor, review_space).await;
    let hidden = store
        .evidence(actor, request.clone())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(hidden.status(), EvidencePageStatus::Complete));
    assert!(hidden.judgments().is_empty());
    assert!(hidden.assertions().is_empty());
    rig.grant(actor, review_space, true).await;
    assert!(matches!(
        store
            .evidence(actor, request)
            .await
            .unwrap()
            .unwrap()
            .judgments(),
        [EvidenceJudgment::Available { .. }]
    ));
    let _ = owner;
}

#[tokio::test]
async fn relation_grant_revocation_hides_fixed_and_dynamic_sources_with_readable_endpoints() {
    let (rig, actor, _, doc, zero) = r::fixture().await;
    let h = endpoint(&doc, 0);
    let e = endpoint(&doc, 1);
    let (_, relation_space) = rig.seed_actor_space(true).await;
    rig.grant(actor, relation_space, true).await;
    let relations = RelationStore::new(rig.runtime_pool.clone());
    let relation = relations
        .save(actor, relations::save(relation_space, e, h.clone()))
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
                reason: "pin relation in separate space".into(),
            },
        )
        .await
        .unwrap();
    let request = query(
        h,
        ImpactScope::Reading {
            view: selected.view,
            mode: ReadingMode::Fused,
        },
        true,
    );
    let store = QueryStore::new(rig.runtime_pool.clone());
    let initial = store
        .evidence(actor, request.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(initial.assertions().len(), 1);
    assert_eq!(initial.assertions()[0].sources.len(), 2);
    rig.revoke(actor, relation_space).await;
    let hidden = store
        .evidence(actor, request.clone())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(hidden.status(), EvidencePageStatus::Complete));
    assert!(hidden.assertions().is_empty());
    rig.grant(actor, relation_space, true).await;
    let restored = store.evidence(actor, request).await.unwrap().unwrap();
    assert_eq!(restored.assertions().len(), 1);
    assert_eq!(restored.assertions()[0].sources.len(), 2);
}

#[tokio::test]
async fn old_visible_cursor_is_reauthorized_after_external_endpoint_revocation() {
    let (rig, actor, space, doc, zero) = r::fixture().await;
    let h = endpoint(&doc, 0);
    let relations = RelationStore::new(rig.runtime_pool.clone());
    let mut created = vec![];
    for text in ["cursor E1", "cursor E2"] {
        let (owner, external_space) = rig.seed_actor_space(true).await;
        let e = rig
            .store
            .create(owner, external_space, support::command(text))
            .await
            .unwrap();
        rig.grant(actor, external_space, false).await;
        let relation = relations
            .save(
                actor,
                relations::save(space, relations::exact(&e), h.clone()),
            )
            .await
            .unwrap();
        created.push((relation.reference, external_space));
    }
    created.sort_by_key(|(reference, _)| reference.clone());
    let mut request = query(
        h,
        ImpactScope::Reading {
            view: zero.view,
            mode: ReadingMode::Fused,
        },
        true,
    );
    request.limit = 1;
    let store = QueryStore::new(rig.runtime_pool.clone());
    let first = store
        .evidence(actor, request.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.assertions().len(), 1);
    assert_eq!(first.assertions()[0].relation.reference, created[0].0);
    let cursor = match first.status() {
        EvidencePageStatus::Truncated { after } => after.clone(),
        _ => panic!("must truncate"),
    };
    rig.revoke(actor, created[0].1).await;
    let fresh = store
        .evidence(actor, request.clone())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(fresh.status(), EvidencePageStatus::Complete));
    assert_eq!(fresh.assertions().len(), 1);
    assert_eq!(fresh.assertions()[0].relation.reference, created[1].0);
    request.after = Some(cursor);
    let continued = store.evidence(actor, request).await.unwrap().unwrap();
    assert!(matches!(continued.status(), EvidencePageStatus::Complete));
    assert_eq!(continued.assertions().len(), 1);
    assert_eq!(continued.assertions()[0].relation.reference, created[1].0);
}

#[tokio::test]
async fn release_manifest_context_relation_is_not_a_fixed_relation_selection() {
    let rig = support::TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let mut conjecture = support::command("H");
    conjecture.draft.intent = Intent::Conjecture;
    let h = rig.store.create(actor, space, conjecture).await.unwrap();
    let e = rig
        .store
        .create(actor, space, support::command("E"))
        .await
        .unwrap();
    let doc = rig
        .compositions()
        .save(actor, space, a::doc(vec![a::block(&h), a::block(&e)]))
        .await
        .unwrap();
    let zero = r::store(&rig)
        .create(actor, space, r::create(doc.reference.clone()))
        .await
        .unwrap();
    let relations = RelationStore::new(rig.runtime_pool.clone());
    let relation = relations
        .save(
            actor,
            relations::save(space, relations::exact(&e), relations::exact(&h)),
        )
        .await
        .unwrap();
    let checked = relations
        .review(actor, relations::review(&relation))
        .await
        .unwrap();
    let judgment = ReviewStore::new(rig.runtime_pool.clone())
        .append(
            actor,
            AppendEpistemicReview {
                request_id: Uuid::new_v4(),
                scope: RelationScope::Space { space_id: space },
                target: relations::exact(&h),
                expected_previous: None,
                state: EpistemicState::Untested,
                relations: vec![RelationSelection {
                    relation: relation.reference.clone(),
                    review: Some(checked.reference),
                }],
                evidence: vec![relations::exact(&e)],
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
                epistemic_reviews: vec![judgment.reference.clone()],
                reason: "select judgment only".into(),
            },
        )
        .await
        .unwrap();
    let released = rig
        .releases()
        .publish_evidence(
            actor,
            space,
            PublishEvidence {
                request_id: Uuid::new_v4(),
                roots: vec![a::root(&doc, None)],
                readings: vec![selected.view],
                reason: "review context".into(),
            },
        )
        .await
        .unwrap();
    let result = QueryStore::new(rig.runtime_pool.clone())
        .evidence(
            actor,
            query(
                relations::exact(&h),
                ImpactScope::Release {
                    release_id: released.release_id,
                },
                false,
            ),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(result.assertions().is_empty());
    assert!(
        matches!(result.judgments(), [EvidenceJudgment::Available { review, .. }] if review.reference == judgment.reference)
    );
}

#[tokio::test]
async fn local_cycle_and_related_to_keep_each_saved_assertion_once_without_transitivity() {
    let rig = support::TestRig::from_env().await;
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
    let relations = RelationStore::new(rig.runtime_pool.clone());
    let ab = relations
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
    let mut bc = relations::save(
        space,
        relations::exact(&c_block),
        relations::exact(&b_block),
    );
    bc.relation_type = RelationType::RelatedTo;
    let bc = relations.save(actor, bc).await.unwrap();
    let mut ca = relations::save(
        space,
        relations::exact(&c_block),
        relations::exact(&a_block),
    );
    ca.relation_type = RelationType::Opposes;
    let ca = relations.save(actor, ca).await.unwrap();
    let selected = r::store(&rig)
        .select_relations(
            actor,
            zero.overlay.overlay_id,
            SelectRelations {
                request_id: Uuid::new_v4(),
                expected_overlay_revision: zero.overlay.revision_id,
                expected_reading_view_revision: zero.view.revision_id,
                selections: [&ab, &bc, &ca]
                    .map(|v| RelationSelection {
                        relation: v.reference.clone(),
                        review: None,
                    })
                    .to_vec(),
                epistemic_reviews: vec![],
                reason: "fixed local cycle".into(),
            },
        )
        .await
        .unwrap();
    let scope = ImpactScope::Reading {
        view: selected.view,
        mode: ReadingMode::Fused,
    };
    let store = QueryStore::new(rig.runtime_pool.clone());
    for (endpoint, expected) in [
        (
            relations::exact(&a_block),
            vec![ab.reference.clone(), ca.reference.clone()],
        ),
        (
            relations::exact(&b_block),
            vec![ab.reference.clone(), bc.reference.clone()],
        ),
        (
            relations::exact(&c_block),
            vec![bc.reference.clone(), ca.reference.clone()],
        ),
    ] {
        let result = store
            .evidence(actor, query(endpoint, scope.clone(), false))
            .await
            .unwrap()
            .unwrap();
        let mut actual: Vec<_> = result
            .assertions()
            .iter()
            .map(|v| v.relation.reference.clone())
            .collect();
        actual.sort();
        let mut expected = expected;
        expected.sort();
        assert_eq!(actual, expected);
        assert!(matches!(result.status(), EvidencePageStatus::Complete));
    }
    assert!(bc.from.block_id < bc.to.block_id);
}

#[tokio::test]
async fn release_deduplicates_one_exact_judgment_selected_by_two_readings_before_paging() {
    let rig = support::TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let mut conjecture = support::command("H");
    conjecture.draft.intent = Intent::Conjecture;
    let h = rig.store.create(actor, space, conjecture).await.unwrap();
    let doc = rig
        .compositions()
        .save(actor, space, a::doc(vec![a::block(&h)]))
        .await
        .unwrap();
    let first_zero = r::store(&rig)
        .create(actor, space, r::create(doc.reference.clone()))
        .await
        .unwrap();
    let second_zero = r::store(&rig)
        .create(actor, space, r::create(doc.reference.clone()))
        .await
        .unwrap();
    let judgment = ReviewStore::new(rig.runtime_pool.clone())
        .append(
            actor,
            AppendEpistemicReview {
                request_id: Uuid::new_v4(),
                scope: RelationScope::Space { space_id: space },
                target: relations::exact(&h),
                expected_previous: None,
                state: EpistemicState::Untested,
                relations: vec![],
                evidence: vec![],
                conditions: "".into(),
                explanation: "".into(),
            },
        )
        .await
        .unwrap();
    let select = |saved: &ReadingSaved| SelectRelations {
        request_id: Uuid::new_v4(),
        expected_overlay_revision: saved.overlay.revision_id,
        expected_reading_view_revision: saved.view.revision_id,
        selections: vec![],
        epistemic_reviews: vec![judgment.reference.clone()],
        reason: "pin same judgment".into(),
    };
    let first = r::store(&rig)
        .select_relations(actor, first_zero.overlay.overlay_id, select(&first_zero))
        .await
        .unwrap();
    let second = r::store(&rig)
        .select_relations(actor, second_zero.overlay.overlay_id, select(&second_zero))
        .await
        .unwrap();
    let released = rig
        .releases()
        .publish_evidence(
            actor,
            space,
            PublishEvidence {
                request_id: Uuid::new_v4(),
                roots: vec![a::root(&doc, None)],
                readings: vec![first.view.clone(), second.view.clone()],
                reason: "two judgment sources".into(),
            },
        )
        .await
        .unwrap();
    let mut request = query(
        relations::exact(&h),
        ImpactScope::Release {
            release_id: released.release_id,
        },
        false,
    );
    request.limit = 1;
    let result = QueryStore::new(rig.runtime_pool.clone())
        .evidence(actor, request)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result.status(), EvidencePageStatus::Complete));
    assert_eq!(result.judgments().len(), 1);
    let mut expected_views = vec![first.view, second.view];
    expected_views.sort();
    assert!(matches!(
        &result.judgments()[0],
        EvidenceJudgment::Available { views, review }
            if review.reference == judgment.reference && views == &expected_views
    ));
}

#[tokio::test]
async fn large_reading_scope_does_not_exhaust_reference_session_across_small_dynamic_closures() {
    let rig = support::TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let mut tx = rig.runtime_pool.begin().await.unwrap();
    let mut displayed = Vec::with_capacity(1900);
    for _ in 0..1900 {
        displayed.push(
            support::references::seed_content(
                &mut tx,
                actor,
                space,
                ContentDraft::V1(support::command("displayed scope block").draft),
                None,
            )
            .await,
        );
    }
    tx.commit().await.unwrap();
    let h = displayed[0].clone();
    let mut children = Vec::new();
    for chunk in displayed.chunks(475) {
        children.push(
            rig.compositions()
                .save(
                    actor,
                    space,
                    a::doc(chunk.iter().cloned().map(NodeTarget::Block).collect()),
                )
                .await
                .unwrap(),
        );
    }
    let document = rig
        .compositions()
        .save(
            actor,
            space,
            a::doc(children.iter().map(a::child).collect()),
        )
        .await
        .unwrap();
    let reading = r::store(&rig)
        .create(actor, space, r::create(document.reference))
        .await
        .unwrap();
    let mut tx = rig.runtime_pool.begin().await.unwrap();
    let mut external = Vec::with_capacity(90);
    for _ in 0..90 {
        external.push(
            support::references::seed_content(
                &mut tx,
                actor,
                space,
                ContentDraft::V1(support::command("external evidence").draft),
                None,
            )
            .await,
        );
    }
    tx.commit().await.unwrap();
    let relations = RelationStore::new(rig.runtime_pool.clone());
    for e in external {
        relations
            .save(actor, relations::save(space, e, h.clone()))
            .await
            .unwrap();
    }
    let mut request = query(
        h,
        ImpactScope::Reading {
            view: reading.view,
            mode: ReadingMode::Fused,
        },
        true,
    );
    request.limit = 100;
    let result = QueryStore::new(rig.runtime_pool.clone())
        .evidence(actor, request)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result.status(), EvidencePageStatus::Complete));
    assert_eq!(result.assertions().len(), 90);
}

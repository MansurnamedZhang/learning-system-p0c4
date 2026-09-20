use learning_core::*;
use serde_json::json;
use uuid::Uuid;

fn block(n: u128) -> BlockRef {
    BlockRef {
        block_id: Uuid::from_u128(n),
        revision_id: Uuid::from_u128(n + 1000),
    }
}
fn relation(n: u128) -> RelationRef {
    RelationRef {
        relation_id: Uuid::from_u128(n),
        revision_id: Uuid::from_u128(n + 1000),
    }
}

#[test]
fn exact_references_require_both_identity_and_revision_and_are_strict() {
    assert!(serde_json::from_value::<BlockRef>(json!({"block_id":Uuid::new_v4()})).is_err());
    assert!(serde_json::from_value::<RelationRef>(json!({"relation_id":Uuid::new_v4()})).is_err());
    assert!(
        serde_json::from_value::<RelationRef>(json!({
            "relation_id":Uuid::new_v4(), "revision_id":Uuid::new_v4(), "extra":false
        }))
        .is_err()
    );
}

#[test]
fn dependencies_come_only_from_structured_fields_in_declared_order() {
    let b1 = block(1);
    let b2 = block(2);
    let run = block(3);
    let r = relation(4);
    let review = RelationReviewRef {
        relation: r.clone(),
        review_id: Uuid::from_u128(2004),
    };
    let draft = ContentDraft::V2(ContentV2 {
        intent: Intent::Note,
        language: "en".into(),
        title: String::new(),
        body: BodyV2::RelationView {
            selections: vec![RelationSelection {
                relation: r.clone(),
                review: Some(review.clone()),
            }],
        },
        basis_refs: vec![ExactRef::Block(b1.clone())],
        requires_context: vec![b2.clone()],
        source_run: Some(run.clone()),
    });
    assert_eq!(
        draft.dependencies(),
        vec![
            Dependency {
                role: DependencyRole::Basis,
                target: ExactRef::Block(b1)
            },
            Dependency {
                role: DependencyRole::RequiresContext,
                target: ExactRef::Block(b2)
            },
            Dependency {
                role: DependencyRole::SourceRun,
                target: ExactRef::Block(run)
            },
            Dependency {
                role: DependencyRole::SelectedRelation,
                target: ExactRef::Relation(r)
            },
            Dependency {
                role: DependencyRole::SelectedReview,
                target: ExactRef::RelationReview(review)
            },
        ]
    );
}

#[test]
fn duplicate_exact_references_and_direct_dependency_overflow_are_rejected() {
    let repeated = ExactRef::Block(block(1));
    let mut draft = ContentV2 {
        intent: Intent::Note,
        language: "en".into(),
        title: String::new(),
        body: BodyV2::Text(TextPayload {
            format: TextFormat::Markdown,
            text: String::new(),
        }),
        basis_refs: vec![repeated.clone(), repeated],
        requires_context: vec![],
        source_run: None,
    };
    assert!(draft.validate().is_err());
    draft.basis_refs = (0..257).map(|n| ExactRef::Block(block(n + 1))).collect();
    assert!(draft.validate().is_err());
}

#[test]
fn relation_selections_reject_duplicate_exact_relations_and_enforce_limit() {
    let selection = RelationSelection {
        relation: relation(1),
        review: None,
    };
    let mut draft = ContentV2 {
        intent: Intent::Note,
        language: "en".into(),
        title: String::new(),
        body: BodyV2::RelationView {
            selections: vec![selection.clone(), selection],
        },
        basis_refs: vec![],
        requires_context: vec![],
        source_run: None,
    };
    assert!(draft.validate().is_err());
    draft.body = BodyV2::RelationView {
        selections: (0..257)
            .map(|n| RelationSelection {
                relation: relation(n + 1),
                review: None,
            })
            .collect(),
    };
    assert!(draft.validate().is_err());
}

#[test]
fn selected_review_must_review_the_selected_relation_revision() {
    let draft = ContentV2 {
        intent: Intent::Note,
        language: "en".into(),
        title: String::new(),
        body: BodyV2::RelationView {
            selections: vec![RelationSelection {
                relation: relation(1),
                review: Some(RelationReviewRef {
                    relation: relation(2),
                    review_id: Uuid::from_u128(7),
                }),
            }],
        },
        basis_refs: vec![],
        requires_context: vec![],
        source_run: None,
    };
    assert!(draft.validate().is_err());
}

#[test]
fn relation_selection_digest_sorts_complete_references_as_a_set() {
    let selection_a = RelationSelection {
        relation: relation(1),
        review: None,
    };
    let selection_b = RelationSelection {
        relation: relation(2),
        review: Some(RelationReviewRef {
            relation: relation(2),
            review_id: Uuid::from_u128(8),
        }),
    };
    let build = |selections| {
        ContentDraft::V2(ContentV2 {
            intent: Intent::Note,
            language: "en".into(),
            title: String::new(),
            body: BodyV2::RelationView { selections },
            basis_refs: vec![],
            requires_context: vec![],
            source_run: None,
        })
    };
    assert_eq!(
        build(vec![selection_a.clone(), selection_b.clone()]).digest(),
        build(vec![selection_b, selection_a]).digest(),
    );
}

#[test]
fn v2_text_and_metadata_boundaries_are_enforced_without_normalizing_bytes() {
    let mut draft = ContentV2 {
        intent: Intent::Note,
        language: "en".into(),
        title: "字".repeat(300),
        body: BodyV2::Text(TextPayload {
            format: TextFormat::Markdown,
            text: "a".repeat(200_000),
        }),
        basis_refs: vec![],
        requires_context: vec![],
        source_run: None,
    };
    assert!(draft.validate().is_ok());
    draft.title.push('字');
    assert!(draft.validate().is_err());
    draft.title.clear();
    if let BodyV2::Text(payload) = &mut draft.body {
        payload.text.push('a');
    }
    assert!(draft.validate().is_err());
}

#[test]
fn versioned_commands_use_a_strict_flat_version_envelope() {
    let request_id = Uuid::from_u128(9);
    let raw = json!({
        "request_id":request_id, "contract_version":2, "reason":"because",
        "draft":{"intent":"note","language":"en","title":"x",
          "body":{"kind":"reference","target":{"block_id":Uuid::from_u128(1),"revision_id":Uuid::from_u128(2)}},
          "basis_refs":[],"requires_context":[],"source_run":null}
    });
    let create: CreateContent = serde_json::from_value(raw.clone()).unwrap();
    assert!(matches!(create.draft, ContentDraft::V2(_)));
    assert_eq!(serde_json::to_value(&create).unwrap(), raw);
    let mut unknown = raw;
    unknown["extra"] = json!(true);
    assert!(serde_json::from_value::<CreateContent>(unknown).is_err());
}

#[test]
fn reasons_require_trimmed_content_but_preserve_original_text() {
    let draft = ContentDraft::V1(serde_json::from_value(json!({"kind":"text","intent":"note","language":"en","title":"","payload":{"format":"markdown","text":""}})).unwrap());
    let mut command = CreateContent {
        request_id: Uuid::new_v4(),
        draft,
        reason: "  kept  ".into(),
    };
    assert!(command.validate().is_ok());
    assert_eq!(command.reason, "  kept  ");
    command.reason = "   ".into();
    assert!(command.validate().is_err());
}

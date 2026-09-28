use learning_core::*;
use uuid::Uuid;

fn command() -> SelectRelations {
    SelectRelations {
        request_id: Uuid::new_v4(),
        expected_overlay_revision: Uuid::new_v4(),
        expected_reading_view_revision: Uuid::new_v4(),
        selections: vec![],
        epistemic_reviews: vec![],
        reason: "selection".into(),
    }
}
#[test]
fn strict_commands_reject_extras_duplicates_mismatch_and_limits() {
    let c = command();
    let mut value = serde_json::to_value(&c).unwrap();
    value["author_id"] = serde_json::json!(Uuid::new_v4());
    assert!(serde_json::from_value::<SelectRelations>(value).is_err());
    let selection = RelationSelection {
        relation: RelationRef {
            relation_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
        },
        review: None,
    };
    let mut duplicate = c.clone();
    duplicate.selections = vec![selection.clone(), selection.clone()];
    assert!(duplicate.validate().is_err());
    let mut wrong = c.clone();
    wrong.selections = vec![selection.clone()];
    wrong.selections[0].review = Some(RelationReviewRef {
        relation: RelationRef {
            relation_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
        },
        review_id: Uuid::new_v4(),
    });
    assert!(wrong.validate().is_err());
    let mut limit = c.clone();
    limit.selections = (0..256)
        .map(|_| RelationSelection {
            relation: RelationRef {
                relation_id: Uuid::new_v4(),
                revision_id: Uuid::new_v4(),
            },
            review: None,
        })
        .collect();
    assert!(limit.validate().is_ok());
    limit.selections.push(selection);
    assert!(limit.validate().is_err());
    for reason in [" ".into(), "x\0y".into(), "x".repeat(1001)] {
        let mut bad = c.clone();
        bad.reason = reason;
        assert!(bad.validate().is_err());
    }
    let mut publishing = PublishEvidence {
        request_id: Uuid::new_v4(),
        roots: vec![PublishRoot {
            composition_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
            expected_head_revision_id: Uuid::new_v4(),
            expected_publication_token: "a".repeat(64),
        }],
        readings: vec![],
        reason: "release".into(),
    };
    let mut wire = serde_json::to_value(&publishing).unwrap();
    wire["manifest_sha256"] = serde_json::json!("client supplied");
    assert!(serde_json::from_value::<PublishEvidence>(wire).is_err());
    publishing.readings = (0..16)
        .map(|_| ReadingRef {
            view_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
        })
        .collect();
    assert!(publishing.validate().is_ok());
    publishing.readings.push(publishing.readings[0].clone());
    assert!(publishing.validate().is_err());
    publishing.readings.truncate(2);
    publishing.readings[1].view_id = publishing.readings[0].view_id;
    assert!(publishing.validate().is_err());
}

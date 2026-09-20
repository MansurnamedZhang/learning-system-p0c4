use learning_core::*;
use serde_json::{Value, json};
use uuid::Uuid;
fn command() -> Value {
    json!({"request_id":Uuid::new_v4(),"scope":{"kind":"space","space_id":Uuid::new_v4()},"relation_id":null,"expected_revision":null,"type":"supports","from":{"block_id":Uuid::new_v4(),"revision_id":Uuid::new_v4()},"to":{"block_id":Uuid::new_v4(),"revision_id":Uuid::new_v4()},"rationale":"","conditions":""})
}
#[test]
fn command_rejects_forged_metadata_unknown_nested_fields_and_system_types() {
    for field in ["author_id", "reviewer", "created_at", "origin"] {
        let mut v = command();
        v[field] = json!("system");
        assert!(serde_json::from_value::<SaveRelation>(v).is_err());
    }
    for kind in ["derived_from", "split_from", "merged_from", "unknown"] {
        let mut v = command();
        v["type"] = json!(kind);
        assert!(serde_json::from_value::<SaveRelation>(v).is_err());
    }
    for kind in [
        "annotates",
        "questions",
        "answers",
        "inspired_by",
        "supports",
        "opposes",
        "tests",
        "related_to",
    ] {
        let mut v = command();
        v["type"] = json!(kind);
        serde_json::from_value::<SaveRelation>(v)
            .unwrap()
            .validate()
            .unwrap();
    }
    for nested in ["scope", "from", "to"] {
        let mut v = command();
        v[nested]["extra"] = json!(true);
        assert!(serde_json::from_value::<SaveRelation>(v).is_err());
    }
    let base = json!({"request_id":Uuid::new_v4(),"relation":{"relation_id":Uuid::new_v4(),"revision_id":Uuid::new_v4()},"expected_previous":null,"state":"reviewed","explanation":"checked"});
    for field in ["reviewer", "reviewer_id", "created_at", "origin"] {
        let mut v = base.clone();
        v[field] = json!("system");
        assert!(serde_json::from_value::<ReviewRelation>(v).is_err());
    }
    for state in ["unreviewed", "reviewed", "needs_recheck", "withdrawn"] {
        let mut v = base.clone();
        v["state"] = json!(state);
        serde_json::from_value::<ReviewRelation>(v)
            .unwrap()
            .validate()
            .unwrap();
    }
    let mut bad = base;
    bad["state"] = json!("supported");
    assert!(serde_json::from_value::<ReviewRelation>(bad).is_err());
}
#[test]
fn save_validation_enforces_paired_cas_distinct_endpoints_and_unicode_limits() {
    let mut c: SaveRelation = serde_json::from_value(command()).unwrap();
    c.validate().unwrap();
    c.relation_id = Some(Uuid::new_v4());
    assert!(c.validate().is_err());
    c.expected_revision = Some(Uuid::new_v4());
    c.validate().unwrap();
    c.relation_id = None;
    assert!(c.validate().is_err());
    c.expected_revision = None;
    c.rationale = "界".repeat(1000);
    c.conditions = "界".repeat(3333) + "a";
    c.validate().unwrap();
    c.rationale.push('界');
    assert!(c.validate().is_err());
    c.rationale.clear();
    c.conditions.push('a');
    assert!(c.validate().is_err());
    c.conditions.clear();
    c.rationale = "\0".into();
    assert!(c.validate().is_err());
    c.rationale.clear();
    c.conditions = "\0".into();
    assert!(c.validate().is_err());
    c.conditions.clear();
    c.to.block_id = c.from.block_id;
    assert!(c.validate().is_err());
}
#[test]
fn review_validation_preserves_bytes_and_rejects_oversize_or_nul() {
    let mut c = ReviewRelation {
        request_id: Uuid::new_v4(),
        relation: RelationRef {
            relation_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
        },
        expected_previous: None,
        state: RelationReviewState::NeedsRecheck,
        explanation: " 界 \n".into(),
    };
    c.validate().unwrap();
    assert_eq!(c.explanation, " 界 \n");
    c.explanation = "a".repeat(10000);
    c.validate().unwrap();
    c.explanation.push('a');
    assert!(c.validate().is_err());
    c.explanation = "\0".into();
    assert!(c.validate().is_err());
}

use learning_core::*;
use uuid::Uuid;

fn request() -> serde_json::Value {
    serde_json::json!({
        "endpoint":{"block_id":Uuid::new_v4(),"revision_id":Uuid::new_v4()},
        "scope":{"type":"reading","view":{"view_id":Uuid::new_v4(),"revision_id":Uuid::new_v4()},"mode":"fused"}
    })
}

#[test]
fn wire_boundary_rejects_zero_limit_release_dynamic_and_unknown_nested_fields() {
    let mut value = request();
    value["limit"] = serde_json::json!(0);
    assert!(serde_json::from_value::<EvidenceQuery>(value).is_err());
    let mut value = request();
    value["endpoint"]["forged"] = serde_json::json!(true);
    assert!(serde_json::from_value::<EvidenceQuery>(value).is_err());
    let mut value = request();
    value["scope"] = serde_json::json!({"type":"release","release_id":Uuid::new_v4()});
    value["include_dynamic"] = serde_json::json!(true);
    assert!(serde_json::from_value::<EvidenceQuery>(value).is_err());
}

#[test]
fn public_constructor_rejects_impossible_cursor_and_unknown_wire_variant() {
    let mut query: EvidenceQuery = serde_json::from_value(request()).unwrap();
    query.after = Some(EvidenceCursor::Assertion {
        relation_id: Uuid::nil(),
        revision_id: Uuid::new_v4(),
    });
    assert!(query.validate().is_err());
    let mut value = request();
    value["after"] = serde_json::json!({"type":"judgment_incomplete","review_id":Uuid::new_v4()});
    assert!(serde_json::from_value::<EvidenceQuery>(value).is_err());
}

#[test]
fn every_cursor_variant_has_an_exact_shape_and_round_trips() {
    let cursors = [
        EvidenceCursor::Assertion {
            relation_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
        },
        EvidenceCursor::JudgmentAvailable {
            stream_id: Uuid::new_v4(),
            review_id: Uuid::new_v4(),
        },
        EvidenceCursor::JudgmentIncomplete,
    ];
    for cursor in cursors {
        let original = serde_json::to_value(&cursor).unwrap();
        assert_eq!(
            serde_json::from_value::<EvidenceCursor>(original.clone()).unwrap(),
            cursor
        );
        for extra in [serde_json::json!(null), serde_json::json!("forged")] {
            let mut malformed = original.clone();
            malformed["extra"] = extra;
            assert!(serde_json::from_value::<EvidenceCursor>(malformed).is_err());
        }
    }
    assert!(
        serde_json::from_value::<EvidenceCursor>(serde_json::json!({"type":"unknown"})).is_err()
    );
    assert!(serde_json::from_value::<EvidenceCursor>(serde_json::json!({"type":"judgment_available","stream_id":Uuid::new_v4(),"review_id":null})).is_err());
}

use learning_core::*;
use serde_json::json;
use uuid::Uuid;

#[test]
fn shipped_reading_examples_are_strict_contracts() {
    serde_json::from_str::<CreateReading>(include_str!(
        "../../../contracts/reading-create.example.json"
    ))
    .unwrap();
    serde_json::from_str::<EditReading>(include_str!(
        "../../../contracts/reading-edit.example.json"
    ))
    .unwrap();
    serde_json::from_str::<DecideMigration>(include_str!(
        "../../../contracts/migration-decide.example.json"
    ))
    .unwrap();
}

#[test]
fn reading_create_digest_matches_independent_python_utf8_golden() {
    // Python hashlib.sha256(json.dumps(sort_keys=True,ensure_ascii=False,separators=(',',':')).encode()).
    assert_eq!(
        reading_request_digest(
            "reading_create",
            Principal {
                actor_id: Uuid::from_u128(11)
            },
            Uuid::from_u128(12),
            &create()
        ),
        "43b9dbbf5b3b9a951bc5951072342297adb3cdfcf607d58b65403643410f3700"
    );
}

fn base() -> CompositionRef {
    CompositionRef {
        composition_id: Uuid::from_u128(1),
        revision_id: Uuid::from_u128(2),
    }
}
fn create() -> CreateReading {
    CreateReading {
        request_id: Uuid::from_u128(3),
        base: base(),
        title: "学习".into(),
        reason: "start".into(),
    }
}
fn draft() -> TextDraft {
    serde_json::from_value(json!({"kind":"text","intent":"note","language":"zh","title":"N","payload":{"format":"markdown","text":"body"}})).unwrap()
}
fn anchor() -> GapAnchor {
    GapAnchor {
        base: base(),
        parent_occurrence_path: vec![],
        left_occurrence_id: None,
        right_occurrence_id: None,
        affinity: Affinity::AfterLeft,
    }
}
fn edit(edit: ReadingEdit) -> EditReading {
    EditReading {
        request_id: Uuid::from_u128(4),
        expected_overlay_revision: Uuid::from_u128(5),
        expected_reading_view_revision: Uuid::from_u128(6),
        edit,
        reason: "edit".into(),
    }
}

#[test]
fn create_rejects_invalid_reason_title_and_unknown_fields() {
    assert!(create().validate().is_ok());
    for (title, reason) in [
        ("x".repeat(301), "ok".into()),
        ("x".into(), "".into()),
        ("x".into(), "a".repeat(1001)),
        ("x\0".into(), "ok".into()),
    ] {
        let mut c = create();
        c.title = title;
        c.reason = reason;
        assert!(c.validate().is_err());
        assert!(serde_json::from_value::<CreateReading>(serde_json::to_value(c).unwrap()).is_err());
    }
    let mut value = serde_json::to_value(create()).unwrap();
    value["author_id"] = json!(Uuid::new_v4());
    assert!(serde_json::from_value::<CreateReading>(value).is_err());
    let mut c = create();
    c.title = "知".repeat(300);
    c.reason = "r".repeat(1000);
    assert!(c.validate().is_ok());
}
#[test]
fn edit_enforces_batch_selection_path_and_nested_fields() {
    for n in [0, 33] {
        let c = edit(ReadingEdit::InsertNew {
            drafts: vec![draft(); n],
            target: InsertTarget::NewGroup { anchor: anchor() },
        });
        assert!(c.validate().is_err());
        assert!(serde_json::from_value::<EditReading>(serde_json::to_value(c).unwrap()).is_err());
    }
    let c = edit(ReadingEdit::InsertNew {
        drafts: vec![draft(); 32],
        target: InsertTarget::NewGroup { anchor: anchor() },
    });
    assert!(c.validate().is_ok());
    let mut a = anchor();
    a.parent_occurrence_path = vec![Uuid::new_v4(); 16];
    assert!(a.validate().is_err());
    a.parent_occurrence_path.truncate(15);
    assert!(a.validate().is_ok());
    for ids in [vec![], vec![Uuid::from_u128(8); 2]] {
        assert!(
            edit(ReadingEdit::AdoptExisting {
                block: BlockRef {
                    block_id: Uuid::new_v4(),
                    revision_id: Uuid::new_v4()
                },
                selected_placements: ids
            })
            .validate()
            .is_err()
        );
    }
    let mut value = serde_json::to_value(c).unwrap();
    value["edit"]["target"]["anchor"]["latest"] = json!(true);
    assert!(serde_json::from_value::<EditReading>(value).is_err());
}
#[test]
fn idempotency_sorts_selections_but_preserves_insert_order() {
    let a = edit(ReadingEdit::AdoptExisting {
        block: BlockRef {
            block_id: Uuid::from_u128(7),
            revision_id: Uuid::from_u128(8),
        },
        selected_placements: vec![Uuid::from_u128(9), Uuid::from_u128(10)],
    });
    let mut b = a.clone();
    if let ReadingEdit::AdoptExisting {
        selected_placements,
        ..
    } = &mut b.edit
    {
        selected_placements.reverse();
    }
    let actor = Principal {
        actor_id: Uuid::from_u128(11),
    };
    let target = Uuid::from_u128(12);
    assert_eq!(a.digest(actor, target), b.digest(actor, target));
    let mut second = draft();
    second.payload.text = "second".into();
    let a = edit(ReadingEdit::InsertNew {
        drafts: vec![draft(), second],
        target: InsertTarget::NewGroup { anchor: anchor() },
    });
    let mut b = a.clone();
    if let ReadingEdit::InsertNew { drafts, .. } = &mut b.edit {
        drafts.reverse();
    }
    assert_ne!(a.digest(actor, target), b.digest(actor, target));
}
#[test]
fn migration_rejects_duplicate_groups_and_invalid_merge() {
    let g = Uuid::new_v4();
    let c = DecideMigration {
        request_id: Uuid::new_v4(),
        proposal_id: Uuid::new_v4(),
        expected_overlay_revision: Uuid::new_v4(),
        expected_reading_view_revision: Uuid::new_v4(),
        action: MigrationAction::Adopt {
            groups: vec![
                GroupDecision::Exact { group_id: g },
                GroupDecision::KeepUnplaced { group_id: g },
            ],
            merges: vec![],
        },
        reason: "adopt".into(),
    };
    assert!(c.validate().is_err());
    assert!(serde_json::from_value::<DecideMigration>(serde_json::to_value(c).unwrap()).is_err());
    let c = edit(ReadingEdit::PlaceUnplaced {
        group_id: g,
        anchor: anchor(),
        merge_into: None,
        merged_order: vec![g],
    });
    assert!(c.validate().is_err());
}

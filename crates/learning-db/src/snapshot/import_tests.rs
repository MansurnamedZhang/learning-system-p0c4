use super::{import_rows::Package, rows};
use learning_core::*;
use serde_json::json;
use uuid::Uuid;

fn u(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

#[test]
fn asset_metadata_size_is_not_an_included_original_limit() {
    let (base, root, actor) = fixture();
    let sha = hex_digest(b"metadata declaration");
    let check = |size| {
        let mut records = base.clone();
        records.push(
            rows::record(
                SnapshotTable::Asset,
                json!({
                    "space_id":u(2),"id":u(100),"sha256":sha,"byte_size":size,
                    "storage_key":format!("sha256/{}/{}",&sha[..2],sha),
                    "media_type":"application/octet-stream","original_file_name":"large.bin",
                    "status":"ready","created_at":"2026-09-24T00:00:00.000000Z"
                }),
            )
            .unwrap(),
        );
        Package { rows: &records }.validate(&root, actor)
    };
    for size in [0, SNAPSHOT_MAX_ASSET_FILE_BYTES as i64 + 1, i64::MAX] {
        check(json!(size)).unwrap();
    }
    for size in [json!(-1), json!(u64::MAX), json!(1.5)] {
        assert!(check(size).is_err());
    }
}
pub(super) fn fixture() -> (Vec<SnapshotRow>, ReadingRef, Principal) {
    let time = "2026-09-24T00:00:00.000000Z";
    let content = json!({"kind":"text","intent":"note","language":"en","title":"Title","payload":{"format":"markdown","text":"Original"}});
    let draft = ContentDraft::decode(1, content.clone()).unwrap();
    let nodes = vec![Occurrence {
        occurrence_id: u(11),
        target: NodeTarget::Block(BlockRef {
            block_id: u(3),
            revision_id: u(4),
        }),
    }];
    let edit = EditableReading {
        base: CompositionRef {
            composition_id: u(5),
            revision_id: u(6),
        },
        title: "Personal".into(),
        groups: vec![],
    };
    let data = vec![
        (
            SnapshotTable::Block,
            json!({"id":u(3),"space_id":u(2),"created_at":time}),
        ),
        (
            SnapshotTable::BlockRevision,
            json!({"id":u(4),"space_id":u(2),"block_id":u(3),"parent_revision_id":null,"content":content,"content_sha256":draft.digest(),"author_id":u(1),"reason":"initial","created_at":time,"contract_version":1}),
        ),
        (
            SnapshotTable::ReferenceObject,
            json!({"kind":"block","object_id":u(3),"revision_id":u(4),"space_id":u(2)}),
        ),
        (
            SnapshotTable::Composition,
            json!({"id":u(5),"space_id":u(2),"kind":"document","created_at":time}),
        ),
        (
            SnapshotTable::CompositionRevision,
            json!({"id":u(6),"space_id":u(2),"composition_id":u(5),"parent_revision_id":null,"kind":"document","title":"Document","content_sha256":composition_digest(CompositionKind::Document,"Document",&nodes),"author_id":u(1),"reason":"initial","created_at":time}),
        ),
        (
            SnapshotTable::CompositionOccurrence,
            json!({"composition_revision_id":u(6),"space_id":u(2),"composition_id":u(5),"occurrence_id":u(11),"position":0,"block_space_id":u(2),"block_id":u(3),"block_revision_id":u(4),"child_space_id":null,"child_composition_id":null,"child_revision_id":null}),
        ),
        (
            SnapshotTable::Overlay,
            json!({"id":u(7),"space_id":u(2),"owner_id":u(1),"root_composition_id":u(5)}),
        ),
        (
            SnapshotTable::OverlayRevision,
            json!({"id":u(8),"overlay_id":u(7),"space_id":u(2),"root_composition_id":u(5),"source_space_id":u(2),"base_revision_id":u(6),"parent_revision_id":null,"title":"Personal","content_sha256":edit.digest(),"author_id":u(1),"reason":"initial","created_at":time}),
        ),
        (
            SnapshotTable::ReadingView,
            json!({"id":u(9),"overlay_id":u(7)}),
        ),
        (
            SnapshotTable::ReadingViewRevision,
            json!({"id":u(10),"view_id":u(9),"overlay_id":u(7),"overlay_revision_id":u(8),"parent_revision_id":null,"author_id":u(1),"created_at":time,"contract_version":1,"evidence":null}),
        ),
    ];
    (
        data.into_iter()
            .map(|(t, v)| rows::record(t, v).unwrap())
            .collect(),
        ReadingRef {
            view_id: u(9),
            revision_id: u(10),
        },
        Principal { actor_id: u(1) },
    )
}
#[test]
fn valid_preflight_rows_and_missing_required_closure() {
    let (rows, root, actor) = fixture();
    Package { rows: &rows }.validate(&root, actor).unwrap();
    for i in 0..rows.len() {
        let mut missing = rows.clone();
        missing.remove(i);
        assert!(
            Package { rows: &missing }.validate(&root, actor).is_err(),
            "missing {:?}",
            rows[i].table
        );
    }
}
#[test]
fn preflight_checks_canonical_fields_identity_and_business_digests() {
    let (rows, root, actor) = fixture();
    for (table, key, value) in [
        (
            SnapshotTable::BlockRevision,
            "parent_revision_id",
            json!(u(99)),
        ),
        (SnapshotTable::BlockRevision, "contract_version", json!(99)),
        (
            SnapshotTable::CompositionRevision,
            "content_sha256",
            json!("a".repeat(64)),
        ),
        (
            SnapshotTable::OverlayRevision,
            "title",
            json!("changed but old business digest"),
        ),
        (
            SnapshotTable::ReadingViewRevision,
            "contract_version",
            json!(3),
        ),
        (
            SnapshotTable::BlockRevision,
            "created_at",
            json!("2026-09-24T00:00:00Z"),
        ),
    ] {
        let mut bad = rows.clone();
        let row = bad.iter_mut().find(|r| r.table == table).unwrap();
        row.immutable_values[key] = value;
        row.sha256 = canonical_record_hash(&row.immutable_values);
        assert!(
            Package { rows: &bad }.validate(&root, actor).is_err(),
            "{table:?}/{key}"
        );
    }
    let mut bad = rows.clone();
    bad[0].immutable_values["extra"] = json!(true);
    bad[0].sha256 = canonical_record_hash(&bad[0].immutable_values);
    assert!(Package { rows: &bad }.validate(&root, actor).is_err());
    let mut bad = rows.clone();
    bad[0].identity = vec![SnapshotIdentityPart::Uuid(u(999))];
    assert!(Package { rows: &bad }.validate(&root, actor).is_err());
    assert!(matches!(
        Package { rows: &rows }.validate(&root, Principal { actor_id: u(99) }),
        Err(ContentError::NotFound)
    ));
}
fn relation_fixture() -> (Vec<SnapshotRow>, ReadingRef, Principal) {
    let (mut rows, root, actor) = fixture();
    let relation = u(20);
    let revision = u(21);
    let review = u(22);
    let from = BlockRef {
        block_id: u(3),
        revision_id: u(4),
    };
    // A second block is a separate endpoint, including its required registry.
    for table in [
        SnapshotTable::Block,
        SnapshotTable::BlockRevision,
        SnapshotTable::ReferenceObject,
    ] {
        let mut row = rows.iter().find(|r| r.table == table).unwrap().clone();
        for key in ["id", "block_id", "object_id", "revision_id"] {
            if let Some(v) = row.immutable_values.get_mut(key) {
                if *v == json!(u(3)) {
                    *v = json!(u(30));
                } else if *v == json!(u(4)) {
                    *v = json!(u(31));
                }
            }
        }
        rows.push(super::rows::record(table, row.immutable_values).unwrap());
    }
    let to = BlockRef {
        block_id: u(30),
        revision_id: u(31),
    };
    let scope = RelationScope::Space { space_id: u(2) };
    let hash = canonical_record_hash(
        &json!({"domain":"relation-content-v1","scope":scope,"type":"supports","from":from,"to":to,"rationale":"r","conditions":"c"}),
    );
    for (table, value) in [
        (
            SnapshotTable::Relation,
            json!({"id":relation,"space_id":u(2),"overlay_id":null,"type":"supports","origin":"user_asserted","from_block_id":u(3),"to_block_id":u(30),"created_at":"2026-09-24T00:00:00.000000Z"}),
        ),
        (
            SnapshotTable::RelationRevision,
            json!({"id":revision,"space_id":u(2),"relation_id":relation,"parent_revision_id":null,"from_space_id":u(2),"from_block_id":u(3),"from_revision_id":u(4),"to_space_id":u(2),"to_block_id":u(30),"to_revision_id":u(31),"rationale":"r","conditions":"c","content_sha256":hash,"author_id":u(1),"created_at":"2026-09-24T00:00:00.000000Z"}),
        ),
        (
            SnapshotTable::ReferenceObject,
            json!({"kind":"relation","object_id":relation,"revision_id":revision,"space_id":u(2)}),
        ),
        (
            SnapshotTable::ReferenceDependency,
            json!({"source_kind":"relation","source_object_id":relation,"source_revision_id":revision,"position":0,"role":"target","target_kind":"block","target_object_id":u(3),"target_revision_id":u(4)}),
        ),
        (
            SnapshotTable::ReferenceDependency,
            json!({"source_kind":"relation","source_object_id":relation,"source_revision_id":revision,"position":1,"role":"target","target_kind":"block","target_object_id":u(30),"target_revision_id":u(31)}),
        ),
        (
            SnapshotTable::RelationReviewHead,
            json!({"relation_id":relation,"relation_revision_id":revision}),
        ),
        (
            SnapshotTable::RelationReview,
            json!({"id":review,"space_id":u(2),"relation_id":relation,"relation_revision_id":revision,"previous_review_id":null,"state":"reviewed","explanation":"checked","reviewer_id":u(1),"created_at":"2026-09-24T00:00:00.000000Z"}),
        ),
        (
            SnapshotTable::ReferenceObject,
            json!({"kind":"relation_review","object_id":revision,"revision_id":review,"space_id":u(2)}),
        ),
        (
            SnapshotTable::ReferenceDependency,
            json!({"source_kind":"relation_review","source_object_id":revision,"source_revision_id":review,"position":0,"role":"target","target_kind":"relation","target_object_id":relation,"target_revision_id":revision}),
        ),
    ] {
        rows.push(super::rows::record(table, value).unwrap());
    }
    (rows, root, actor)
}

#[test]
fn preflight_requires_exact_reference_dependency_and_review_head() {
    let (rows, root, actor) = relation_fixture();
    Package { rows: &rows }.validate(&root, actor).unwrap();
    for table in [
        SnapshotTable::ReferenceDependency,
        SnapshotTable::RelationReviewHead,
    ] {
        let mut bad = rows.clone();
        let i = bad.iter().position(|r| r.table == table).unwrap();
        bad.remove(i);
        assert!(Package { rows: &bad }.validate(&root, actor).is_err());
    }
}

#[test]
fn preflight_preserves_bounded_necessary_reference_cycles() {
    let (mut rows, root, actor) = fixture();
    let mut second = rows
        .iter()
        .find(|r| r.table == SnapshotTable::Block)
        .unwrap()
        .clone();
    second.immutable_values["id"] = json!(u(30));
    rows.push(super::rows::record(second.table, second.immutable_values).unwrap());
    let mut revision = rows
        .iter()
        .find(|r| r.table == SnapshotTable::BlockRevision)
        .unwrap()
        .clone();
    revision.immutable_values["id"] = json!(u(31));
    revision.immutable_values["block_id"] = json!(u(30));
    rows.push(super::rows::record(revision.table, revision.immutable_values).unwrap());
    rows.push(
        super::rows::record(
            SnapshotTable::ReferenceObject,
            json!({"kind":"block","object_id":u(30),"revision_id":u(31),"space_id":u(2)}),
        )
        .unwrap(),
    );
    for (block, revision, target, target_revision) in
        [(u(3), u(4), u(30), u(31)), (u(30), u(31), u(3), u(4))]
    {
        let draft = ContentDraft::V2(ContentV2 {
            intent: Intent::Note,
            language: "en".into(),
            title: "cycle".into(),
            body: BodyV2::Text(TextPayload {
                format: TextFormat::Markdown,
                text: "cycle".into(),
            }),
            basis_refs: vec![ExactRef::Block(BlockRef {
                block_id: target,
                revision_id: target_revision,
            })],
            requires_context: vec![],
            source_run: None,
        });
        let row = rows
            .iter_mut()
            .find(|r| {
                r.table == SnapshotTable::BlockRevision
                    && r.immutable_values["id"] == json!(revision)
            })
            .unwrap();
        row.immutable_values["content"] = serde_json::to_value(match &draft {
            ContentDraft::V2(v) => v,
            _ => unreachable!(),
        })
        .unwrap();
        row.immutable_values["contract_version"] = json!(2);
        row.immutable_values["content_sha256"] = json!(draft.digest());
        *row = super::rows::record(row.table, row.immutable_values.clone()).unwrap();
        rows.push(super::rows::record(SnapshotTable::ReferenceDependency,json!({"source_kind":"block","source_object_id":block,"source_revision_id":revision,"position":0,"role":"basis","target_kind":"block","target_object_id":target,"target_revision_id":target_revision})).unwrap());
    }
    Package { rows: &rows }.validate(&root, actor).unwrap();
}

// The package contains immutable historical revisions and selections; no
// mutable review-head state is present or consulted by preflight.
pub(super) fn epistemic_fixture(
    intent: Intent,
    kind: RelationType,
    target_id: u128,
    evidence: bool,
) -> (Vec<SnapshotRow>, ReadingRef, Principal) {
    let (mut rows, root, actor) = relation_fixture();
    // A third target permits a relation unrelated to the judgment target.
    for table in [
        SnapshotTable::Block,
        SnapshotTable::BlockRevision,
        SnapshotTable::ReferenceObject,
    ] {
        let source = rows.iter().find(|r| r.table == table).unwrap();
        let mut v = source.immutable_values.clone();
        for key in ["id", "block_id", "object_id", "revision_id"] {
            if v.get(key) == Some(&json!(u(3))) {
                v[key] = json!(u(40));
            } else if v.get(key) == Some(&json!(u(4))) {
                v[key] = json!(u(41));
            }
        }
        rows.push(super::rows::record(table, v).unwrap());
    }
    for row in &mut rows {
        let v = &mut row.immutable_values;
        if row.table == SnapshotTable::BlockRevision && v["block_id"] == json!(u(target_id)) {
            v["content"]["intent"] = json!(intent);
            v["content_sha256"] = json!(
                ContentDraft::decode(1, v["content"].clone())
                    .unwrap()
                    .digest()
            );
        }
        if row.table == SnapshotTable::Relation {
            v["type"] = json!(kind);
        }
        if row.table == SnapshotTable::RelationRevision {
            v["content_sha256"] = json!(canonical_record_hash(&json!({
                "domain":"relation-content-v1", "scope":RelationScope::Space { space_id:u(2) },
                "type":kind,"from":BlockRef {block_id:u(3),revision_id:u(4)},
                "to":BlockRef {block_id:u(30),revision_id:u(31)},"rationale":"r","conditions":"c"
            })));
        }
        *row = super::rows::record(row.table, v.clone()).unwrap();
    }
    let target = BlockRef {
        block_id: u(target_id),
        revision_id: u(target_id + 1),
    };
    let relation = RelationRef {
        relation_id: u(20),
        revision_id: u(21),
    };
    let command = AppendEpistemicReview {
        request_id: u(60),
        scope: RelationScope::Space { space_id: u(2) },
        target: target.clone(),
        expected_previous: None,
        state: match kind {
            RelationType::Supports => EpistemicState::SupportedWithinScope,
            RelationType::Opposes => EpistemicState::RefutedWithinScope,
            _ => EpistemicState::Testing,
        },
        relations: vec![RelationSelection {
            relation: relation.clone(),
            review: Some(RelationReviewRef {
                relation,
                review_id: u(22),
            }),
        }],
        evidence: if evidence {
            vec![BlockRef {
                block_id: u(3),
                revision_id: u(4),
            }]
        } else {
            vec![]
        },
        conditions: "historical conditions".into(),
        explanation: "historical judgment".into(),
    };
    command.validate().unwrap();
    for (table, v) in [
        (
            SnapshotTable::EpistemicStream,
            json!({"id":u(60),"space_id":u(2),"overlay_id":null,"target_space_id":u(2),"target_block_id":target.block_id,"target_revision_id":target.revision_id,"actor_id":u(1)}),
        ),
        (
            SnapshotTable::EpistemicReview,
            json!({"id":u(61),"space_id":u(2),"stream_id":u(60),"previous_review_id":null,"state":command.state,"relations":command.relations,"evidence":command.evidence,"conditions":command.conditions,"explanation":command.explanation,"reviewer_id":u(1),"created_at":"2026-09-24T00:00:00.000000Z"}),
        ),
        (
            SnapshotTable::ReferenceObject,
            json!({"kind":"epistemic_review","object_id":u(60),"revision_id":u(61),"space_id":u(2)}),
        ),
    ] {
        rows.push(super::rows::record(table, v).unwrap());
    }
    for (position, dep) in command.dependencies().iter().enumerate() {
        let (kind, object, revision) = crate::references::key(&dep.target);
        rows.push(super::rows::record(SnapshotTable::ReferenceDependency,json!({"source_kind":"epistemic_review","source_object_id":u(60),"source_revision_id":u(61),"position":position,"role":dep.role,"target_kind":kind,"target_object_id":object,"target_revision_id":revision})).unwrap());
    }
    (rows, root, actor)
}

#[test]
fn preflight_rejects_non_judgment_target_intent() {
    let (rows, root, actor) = epistemic_fixture(Intent::Note, RelationType::Supports, 30, true);
    assert!(Package { rows: &rows }.validate(&root, actor).is_err());
}
#[test]
fn preflight_rejects_epistemic_support_direction_or_missing_evidence() {
    for kind in [RelationType::Supports, RelationType::Opposes] {
        for (target, evidence) in [(3, true), (30, false)] {
            let (rows, root, actor) = epistemic_fixture(Intent::Conjecture, kind, target, evidence);
            assert!(
                Package { rows: &rows }.validate(&root, actor).is_err(),
                "{kind:?}/{target}/{evidence}"
            );
        }
    }
}
#[test]
fn preflight_rejects_epistemic_relation_unrelated_to_target() {
    let (rows, root, actor) =
        epistemic_fixture(Intent::Conclusion, RelationType::RelatedTo, 40, false);
    assert!(Package { rows: &rows }.validate(&root, actor).is_err());
}
#[test]
fn preflight_accepts_valid_historical_epistemic_semantics() {
    for intent in [Intent::Conjecture, Intent::Conclusion] {
        for (kind, target, evidence) in [
            (RelationType::Supports, 30, true),
            (RelationType::Opposes, 30, true),
            (RelationType::RelatedTo, 3, false),
            (RelationType::RelatedTo, 30, false),
        ] {
            let (rows, root, actor) = epistemic_fixture(intent, kind, target, evidence);
            Package { rows: &rows }.validate(&root, actor).unwrap();
        }
    }
}

fn edit_judgment(rows: &mut Vec<SnapshotRow>, edit: impl FnOnce(&mut serde_json::Value)) {
    let row = rows
        .iter_mut()
        .find(|r| r.table == SnapshotTable::EpistemicReview)
        .unwrap();
    edit(&mut row.immutable_values);
    let v = &row.immutable_values;
    let command = AppendEpistemicReview {
        request_id: u(60),
        scope: RelationScope::Space { space_id: u(2) },
        target: BlockRef {
            block_id: u(30),
            revision_id: u(31),
        },
        expected_previous: None,
        state: serde_json::from_value(v["state"].clone()).unwrap(),
        relations: serde_json::from_value(v["relations"].clone()).unwrap(),
        evidence: serde_json::from_value(v["evidence"].clone()).unwrap(),
        conditions: v["conditions"].as_str().unwrap().into(),
        explanation: v["explanation"].as_str().unwrap().into(),
    };
    command.validate().unwrap();
    *row = super::rows::record(row.table, v.clone()).unwrap();
    // Keep hashes and exact ordered dependencies valid, so failures can only
    // exercise the historical judgment rule rather than stale fixture data.
    rows.retain(|r| {
        r.table != SnapshotTable::ReferenceDependency
            || r.immutable_values["source_kind"] != "epistemic_review"
    });
    for (position, dep) in command.dependencies().iter().enumerate() {
        let (kind, object, revision) = crate::references::key(&dep.target);
        rows.push(super::rows::record(SnapshotTable::ReferenceDependency,json!({"source_kind":"epistemic_review","source_object_id":u(60),"source_revision_id":u(61),"position":position,"role":dep.role,"target_kind":kind,"target_object_id":object,"target_revision_id":revision})).unwrap());
    }
}

#[test]
fn historical_basis_rejects_empty_selected_relations() {
    for kind in [RelationType::Supports, RelationType::Opposes] {
        let (mut rows, root, actor) = epistemic_fixture(Intent::Conjecture, kind, 30, true);
        edit_judgment(&mut rows, |v| v["relations"] = json!([]));
        assert!(
            Package { rows: &rows }.validate(&root, actor).is_err(),
            "{kind:?}"
        );
    }
}
#[test]
fn historical_basis_rejects_wrong_direction() {
    for kind in [RelationType::Supports, RelationType::Opposes] {
        let (mut rows, root, actor) = epistemic_fixture(Intent::Conjecture, kind, 30, true);
        edit_judgment(&mut rows, |v| {
            v["state"] = json!(if kind == RelationType::Supports {
                EpistemicState::RefutedWithinScope
            } else {
                EpistemicState::SupportedWithinScope
            })
        });
        assert!(
            Package { rows: &rows }.validate(&root, actor).is_err(),
            "{kind:?}"
        );
    }
}
#[test]
fn historical_basis_rejects_missing_selected_review() {
    for kind in [RelationType::Supports, RelationType::Opposes] {
        let (mut rows, root, actor) = epistemic_fixture(Intent::Conjecture, kind, 30, true);
        edit_judgment(&mut rows, |v| v["relations"][0]["review"] = json!(null));
        assert!(
            Package { rows: &rows }.validate(&root, actor).is_err(),
            "{kind:?}"
        );
    }
}
#[test]
fn historical_basis_rejects_non_reviewed_selected_review() {
    for kind in [RelationType::Supports, RelationType::Opposes] {
        for state in [
            RelationReviewState::Unreviewed,
            RelationReviewState::NeedsRecheck,
            RelationReviewState::Withdrawn,
        ] {
            let (mut rows, root, actor) = epistemic_fixture(Intent::Conjecture, kind, 30, true);
            let row = rows
                .iter_mut()
                .find(|r| r.table == SnapshotTable::RelationReview)
                .unwrap();
            row.immutable_values["state"] = json!(state);
            *row = super::rows::record(row.table, row.immutable_values.clone()).unwrap();
            assert!(
                Package { rows: &rows }.validate(&root, actor).is_err(),
                "{kind:?}/{state:?}"
            );
        }
    }
}
#[test]
fn historical_basis_accepts_selected_review_before_later_withdrawal() {
    for kind in [RelationType::Supports, RelationType::Opposes] {
        let (mut rows, root, actor) = epistemic_fixture(Intent::Conclusion, kind, 30, true);
        let mut later = rows
            .iter()
            .find(|r| r.table == SnapshotTable::RelationReview)
            .unwrap()
            .immutable_values
            .clone();
        later["id"] = json!(u(23));
        later["previous_review_id"] = json!(u(22));
        later["state"] = json!(RelationReviewState::Withdrawn);
        later["created_at"] = json!("2026-09-25T00:00:00.000000Z");
        rows.push(super::rows::record(SnapshotTable::RelationReview, later).unwrap());
        rows.push(super::rows::record(SnapshotTable::ReferenceObject,json!({"kind":"relation_review","object_id":u(21),"revision_id":u(23),"space_id":u(2)})).unwrap());
        rows.push(super::rows::record(SnapshotTable::ReferenceDependency,json!({"source_kind":"relation_review","source_object_id":u(21),"source_revision_id":u(23),"position":0,"role":"target","target_kind":"relation","target_object_id":u(20),"target_revision_id":u(21)})).unwrap());
        // The selected review is still immutable Reviewed @22. The newer
        // withdrawal @23 cannot rewrite that historical judgment's basis.
        Package { rows: &rows }.validate(&root, actor).unwrap();
    }
}

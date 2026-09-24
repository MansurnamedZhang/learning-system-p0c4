use learning_core::*;
use serde_json::{Value, json};
use uuid::Uuid;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

#[test]
fn canonical_hash_changes_for_author_parent_reason() {
    let original = json!({
        "id": id(1), "block_id": id(2), "author_id": id(3),
        "parent_revision_id": id(4), "reason": "first", "content": {"z": 2, "a": 1}
    });
    let reordered: Value = serde_json::from_str(&format!(
        "{{\"reason\":\"first\",\"parent_revision_id\":\"{}\",\"author_id\":\"{}\",\"block_id\":\"{}\",\"id\":\"{}\",\"content\":{{\"a\":1,\"z\":2}}}}",
        id(4), id(3), id(2), id(1)
    ))
    .unwrap();
    assert_eq!(
        canonical_record_hash(&original),
        canonical_record_hash(&reordered)
    );
    for changed in [
        json!({"id": id(1), "block_id": id(2), "author_id": id(9), "parent_revision_id": id(4), "reason": "first", "content": {"z": 2, "a": 1}}),
        json!({"id": id(1), "block_id": id(2), "author_id": id(3), "parent_revision_id": id(9), "reason": "first", "content": {"z": 2, "a": 1}}),
        json!({"id": id(1), "block_id": id(2), "author_id": id(3), "parent_revision_id": id(4), "reason": "edited", "content": {"z": 2, "a": 1}}),
    ] {
        assert_ne!(
            canonical_record_hash(&original),
            canonical_record_hash(&changed)
        );
    }
}

#[test]
fn unknown_version_or_capability_rejected() {
    let valid = json!({
        "capability": "exact_import_v1", "format_version": 1,
        "root": {"view_id": id(1), "revision_id": id(2)},
        "files": [], "requires_destination_assets": false
    });
    assert!(serde_json::from_value::<SnapshotManifest>(valid.clone()).is_ok());
    for (field, value) in [
        ("capability", json!("exact_import_v2")),
        ("format_version", json!(2)),
        ("requires_destination_assets", json!("false")),
    ] {
        let mut bad = valid.clone();
        bad[field] = value;
        assert!(
            serde_json::from_value::<SnapshotManifest>(bad).is_err(),
            "{field}"
        );
    }
    let mut extra = valid;
    extra["unrecognized"] = json!(true);
    assert!(serde_json::from_value::<SnapshotManifest>(extra).is_err());
}

#[test]
fn copy_has_no_exact_root() {
    let copy = SnapshotManifest::ReadingCopyV1(ReadingCopyManifest {
        format_version: 1,
        copy_id: id(3),
        files: vec![],
    });
    let value = serde_json::to_value(&copy).unwrap();
    assert_eq!(value["capability"], "reading_copy_v1");
    assert!(value.get("root").is_none());
    assert!(value.get("view_id").is_none());
    let mut forged = value;
    forged["root"] = json!({"view_id": id(1), "revision_id": id(2)});
    assert!(serde_json::from_value::<SnapshotManifest>(forged).is_err());
    let serialized = serde_json::to_value(ReadingCopy {
        items: vec![
            CopyItem::Omitted,
            CopyItem::Content {
                intent: "knowledge".into(),
                title: "Public".into(),
                display_text: "Visible".into(),
            },
        ],
        evidence: vec!["Reviewed".into()],
    })
    .unwrap();
    assert_eq!(serialized["items"][0], json!({"type": "omitted"}));
    assert!(serialized.to_string().find("revision_id").is_none());
}

#[test]
fn budget_is_package_wide() {
    let mut budget = SnapshotBudget::default();
    for _ in 0..2048 {
        budget
            .charge(SnapshotBudgetKind::ReferenceObject, 1, 1)
            .unwrap();
    }
    assert!(
        budget
            .charge(SnapshotBudgetKind::ReferenceObject, 1, 1)
            .is_err()
    );
    assert!(
        budget
            .charge(SnapshotBudgetKind::ReferenceEdge, 0, 1)
            .is_ok()
    );
    for _ in 1..4096 {
        budget
            .charge(SnapshotBudgetKind::ReferenceEdge, 0, 1)
            .unwrap();
    }
    assert!(
        budget
            .charge(SnapshotBudgetKind::ReferenceEdge, 0, 1)
            .is_err()
    );
    assert!(
        budget
            .charge(SnapshotBudgetKind::ReferenceObject, 0, 33)
            .is_err()
    );
}

#[test]
fn budget_rejects_file_and_byte_limits_without_partial_charge() {
    let mut budget = SnapshotBudget::default();
    assert!(
        budget
            .charge(SnapshotBudgetKind::JsonFile, 8 * 1024 * 1024 + 1, 0)
            .is_err()
    );
    for _ in 0..8 {
        budget
            .charge(SnapshotBudgetKind::JsonFile, 8 * 1024 * 1024, 0)
            .unwrap();
    }
    assert!(budget.charge(SnapshotBudgetKind::JsonFile, 1, 0).is_err());
    let mut assets = SnapshotBudget::default();
    assert!(
        assets
            .charge(SnapshotBudgetKind::AssetFile, 128 * 1024 * 1024 + 1, 0)
            .is_err()
    );
    for _ in 0..4 {
        assets
            .charge(SnapshotBudgetKind::AssetFile, 128 * 1024 * 1024, 0)
            .unwrap();
    }
    assert!(assets.charge(SnapshotBudgetKind::AssetFile, 1, 0).is_err());
    assert!(assets.charge(SnapshotBudgetKind::JsonFile, 1, 0).is_ok());
}

#[test]
fn unknown_table_and_nested_file_fields_rejected() {
    assert!(serde_json::from_value::<SnapshotTable>(json!("secret_table")).is_err());
    let file = json!({"path":"objects/block/u-00000000-0000-0000-0000-000000000001.json", "size":2, "sha256":"a".repeat(64)});
    assert!(serde_json::from_value::<SnapshotFile>(file.clone()).is_ok());
    let mut extra = file;
    extra["private_path"] = json!("/tmp/private");
    assert!(serde_json::from_value::<SnapshotFile>(extra).is_err());
}

#[test]
fn malformed_file_digests_and_unsafe_paths_are_rejected() {
    let valid = json!({
        "path": "validation.json", "size": 2,
        "sha256": "a".repeat(64)
    });
    assert!(serde_json::from_value::<SnapshotFile>(valid.clone()).is_ok());
    for path in [
        "../secret",
        "/tmp/secret",
        "C:/secret",
        "objects\\block\\id.json",
    ] {
        let mut bad = valid.clone();
        bad["path"] = json!(path);
        assert!(
            serde_json::from_value::<SnapshotFile>(bad).is_err(),
            "{path}"
        );
    }
    for digest in ["A".repeat(64), "g".repeat(64), "a".repeat(63)] {
        let mut bad = valid.clone();
        bad["sha256"] = json!(digest);
        assert!(serde_json::from_value::<SnapshotFile>(bad).is_err());
    }
}

#[test]
fn duplicate_or_unsorted_manifest_files_are_rejected() {
    let file = json!({"path": "validation.json", "size": 2, "sha256": "a".repeat(64)});
    let manifest = json!({
        "capability": "exact_import_v1", "format_version": 1,
        "root": {"view_id": id(1), "revision_id": id(2)},
        "files": [file.clone(), file], "requires_destination_assets": false
    });
    assert!(serde_json::from_value::<SnapshotManifest>(manifest).is_err());
    let unsorted = json!({
        "capability": "reading_copy_v1", "format_version": 1, "copy_id": id(3),
        "files": [
            {"path": "validation.json", "size": 2, "sha256": "a".repeat(64)},
            {"path": "reading.md", "size": 3, "sha256": "b".repeat(64)}
        ]
    });
    assert!(serde_json::from_value::<SnapshotManifest>(unsorted).is_err());
}

#[test]
fn typed_identity_paths_round_trip_without_primary_key_collisions() {
    let object = snapshot_object_path(
        SnapshotTable::ReferenceObject,
        &[
            SnapshotIdentityPart::Kind(ReferenceKind::Block),
            SnapshotIdentityPart::Uuid(id(1)),
            SnapshotIdentityPart::Uuid(id(2)),
        ],
    )
    .unwrap();
    let relation = snapshot_object_path(
        SnapshotTable::ReferenceObject,
        &[
            SnapshotIdentityPart::Kind(ReferenceKind::Relation),
            SnapshotIdentityPart::Uuid(id(1)),
            SnapshotIdentityPart::Uuid(id(2)),
        ],
    )
    .unwrap();
    assert_ne!(object, relation);
    assert_eq!(
        object,
        "objects/reference_object/k-block__u-00000000-0000-0000-0000-000000000001__u-00000000-0000-0000-0000-000000000002.json"
    );
    assert_eq!(
        parse_snapshot_object_path(&object).unwrap().1,
        vec![
            SnapshotIdentityPart::Kind(ReferenceKind::Block),
            SnapshotIdentityPart::Uuid(id(1)),
            SnapshotIdentityPart::Uuid(id(2)),
        ]
    );
    let dependency = snapshot_object_path(
        SnapshotTable::ReferenceDependency,
        &[
            SnapshotIdentityPart::Kind(ReferenceKind::Block),
            SnapshotIdentityPart::Uuid(id(1)),
            SnapshotIdentityPart::Uuid(id(2)),
            SnapshotIdentityPart::Position(3),
        ],
    )
    .unwrap();
    assert!(dependency.ends_with("__p-3.json"));
    assert_eq!(
        parse_snapshot_object_path(&dependency).unwrap().1.last(),
        Some(&SnapshotIdentityPart::Position(3))
    );
}

#[test]
fn identity_paths_reject_noncanonical_tokens_and_wrong_pk_shapes() {
    let base = "objects/reading_relation_selection/u-00000000-0000-0000-0000-000000000001__p-";
    for suffix in ["03.json", "-1.json", "4294967296.json", "3/evil.json"] {
        assert!(parse_snapshot_object_path(&format!("{base}{suffix}")).is_err());
    }
    assert!(
        snapshot_object_path(
            SnapshotTable::ReadingRelationSelection,
            &[
                SnapshotIdentityPart::Uuid(id(1)),
                SnapshotIdentityPart::Uuid(id(2))
            ],
        )
        .is_err()
    );
    assert!(parse_snapshot_object_path(
        "objects/reference_object/k-secret__u-00000000-0000-0000-0000-000000000001__u-00000000-0000-0000-0000-000000000002.json"
    ).is_err());
}

#[test]
fn row_decoder_rejects_wrong_full_record_hash() {
    let values = json!({"id": id(1), "author_id": id(2), "reason": "created"});
    let mut row = json!({
        "table": "block_revision",
        "identity": [{"type": "uuid", "value": id(1)}],
        "immutable_values": values,
        "sha256": canonical_record_hash(&values)
    });
    assert!(serde_json::from_value::<SnapshotRow>(row.clone()).is_ok());
    row["immutable_values"]["author_id"] = json!(id(3));
    assert!(serde_json::from_value::<SnapshotRow>(row).is_err());
}

#[test]
fn asset_use_decoder_rejects_mismatched_storage_key() {
    let use_row = json!({
        "use_ref": {"type": "block", "block_id": id(1), "revision_id": id(2)},
        "asset": {"space_id": id(3), "asset_id": id(4)},
        "sha256": "a".repeat(64), "byte_size": 10,
        "storage_key": format!("sha256/aa/{}", "a".repeat(64))
    });
    assert!(serde_json::from_value::<SnapshotAssetUse>(use_row.clone()).is_ok());
    let mut bad = use_row;
    bad["storage_key"] = json!(format!("sha256/bb/{}", "a".repeat(64)));
    assert!(serde_json::from_value::<SnapshotAssetUse>(bad).is_err());
}

#[test]
fn timestamp_wire_form_is_utc_with_six_fractional_digits() {
    let time = chrono::DateTime::parse_from_rfc3339("2026-09-24T15:08:09.123456789+08:00")
        .unwrap()
        .with_timezone(&chrono::Utc);
    assert_eq!(
        canonical_snapshot_timestamp(time),
        "2026-09-24T07:08:09.123456Z"
    );
}

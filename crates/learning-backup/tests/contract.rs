use learning_backup::{
    AssetRow, BackupError, BackupManifestV1, BackupPlan, BackupState, FileRecord, MigrationRecord,
    SourceIdentity,
};
use uuid::Uuid;

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn row(space: u128, id: u128, sha: &str, size: i64) -> AssetRow {
    AssetRow {
        space_id: Uuid::from_u128(space),
        id: Uuid::from_u128(id),
        sha256: sha.into(),
        byte_size: size,
        storage_key: format!("sha256/{}/{}", &sha[..2], sha),
    }
}

fn file(path: &str, sha: &str, size: u64) -> FileRecord {
    FileRecord {
        path: path.into(),
        size,
        sha256: sha.into(),
    }
}

fn manifest(plan: &BackupPlan) -> BackupManifestV1 {
    BackupManifestV1::from_plan(
        Uuid::from_u128(42),
        SourceIdentity::from_migrations(
            A.into(),
            "0123456789abcdef0123456789abcdef01234567".into(),
            18,
            vec![MigrationRecord {
                version: 1,
                checksum_hex: "a".repeat(96),
            }],
        )
        .unwrap(),
        plan,
        file("database.dump", A, 10),
        file("roles.json", B, 20),
    )
    .unwrap()
}

#[test]
fn source_identity_binds_ordered_complete_migration_records() {
    let migrations = vec![
        MigrationRecord {
            version: 1,
            checksum_hex: "a".repeat(96),
        },
        MigrationRecord {
            version: 2,
            checksum_hex: "b".repeat(96),
        },
    ];
    let source = SourceIdentity::from_migrations(
        A.into(),
        "0123456789abcdef0123456789abcdef01234567".into(),
        18,
        migrations.clone(),
    )
    .unwrap();
    assert_eq!(source.migration_version, 2);
    assert_eq!(source.migrations, migrations);
    assert!(
        SourceIdentity::from_migrations(
            A.into(),
            "0123456789abcdef0123456789abcdef01234567".into(),
            18,
            vec![migrations[1].clone(), migrations[0].clone()]
        )
        .is_err()
    );
    assert!(
        SourceIdentity::from_migrations(
            A.into(),
            "0123456789abcdef0123456789abcdef01234567".into(),
            18,
            vec![migrations[0].clone(), migrations[0].clone()]
        )
        .is_err()
    );
    let plan = BackupPlan::from_rows(vec![]).unwrap();
    let original = manifest(&plan);
    assert_eq!(
        original.source.migration_fingerprint,
        "a884a21d26949efeb2698121a122d0e040ef7da5c4b89252c849606ebd535235"
    );
    let mut omitted = original.clone();
    omitted.source.migrations.clear();
    assert!(
        omitted
            .validate_with_index(plan.asset_index_bytes())
            .is_err()
    );
    let shortened = SourceIdentity::from_migrations(
        A.into(),
        "0123456789abcdef0123456789abcdef01234567".into(),
        18,
        vec![migrations[0].clone()],
    )
    .unwrap();
    assert_ne!(
        source.migration_fingerprint,
        shortened.migration_fingerprint
    );
    let mut changed = original.clone();
    changed.source.migrations[0].checksum_hex = "b".repeat(96);
    assert!(
        changed
            .validate_with_index(plan.asset_index_bytes())
            .is_err()
    );
    let changed_source = SourceIdentity::from_migrations(
        A.into(),
        "0123456789abcdef0123456789abcdef01234567".into(),
        18,
        vec![MigrationRecord {
            version: 1,
            checksum_hex: "b".repeat(96),
        }],
    )
    .unwrap();
    assert_ne!(
        original.source.migration_fingerprint,
        changed_source.migration_fingerprint
    );
    assert!(
        SourceIdentity::from_migrations(
            A.into(),
            "ABCDEF0123456789abcdef0123456789abcdef01".into(),
            18,
            migrations
        )
        .is_err()
    );
}

#[test]
fn independent_c4_catalog_limits_reject_excess_rows_and_index_bytes() {
    let rows = (1..=100_001).map(|id| row(1, id, A, 1)).collect();
    assert!(matches!(
        BackupPlan::from_rows(rows),
        Err(BackupError::Capacity(_))
    ));
    let plan = BackupPlan::from_rows(vec![]).unwrap();
    let oversized_index = vec![b' '; 64 * 1024 * 1024 + 1];
    assert!(matches!(
        manifest(&plan).validate_with_index(&oversized_index),
        Err(BackupError::Capacity(_))
    ));
}

#[test]
fn every_logical_row_is_indexed_while_equal_digest_bytes_are_deduplicated() {
    let plan =
        BackupPlan::from_rows(vec![row(2, 1, A, 7), row(1, 2, A, 7), row(1, 1, B, 5)]).unwrap();
    assert_eq!(plan.state(), BackupState::Staging);
    assert_eq!(plan.logical_asset_count(), 3);
    assert_eq!(plan.unique_asset_bytes(), 12);
    assert_eq!(plan.asset_files().len(), 2);
    let index: serde_json::Value = serde_json::from_slice(plan.asset_index_bytes()).unwrap();
    assert_eq!(index["assets"].as_array().unwrap().len(), 3);
    assert_eq!(index["assets"][0]["id"], Uuid::from_u128(1).to_string());
    assert_eq!(
        index["assets"][0]["space_id"],
        Uuid::from_u128(1).to_string()
    );
    assert_eq!(index["assets"][1]["id"], Uuid::from_u128(2).to_string());
    assert_eq!(
        index["assets"][2]["space_id"],
        Uuid::from_u128(2).to_string()
    );
    assert_eq!(plan.asset_index_file().path, "asset-index.json");
    assert_eq!(
        plan.asset_index_file().size,
        plan.asset_index_bytes().len() as u64
    );
}

#[test]
fn ordering_does_not_change_canonical_index_or_manifest_bytes() {
    let first = BackupPlan::from_rows(vec![row(2, 1, A, 7), row(1, 1, B, 5)]).unwrap();
    let second = BackupPlan::from_rows(vec![row(1, 1, B, 5), row(2, 1, A, 7)]).unwrap();
    assert_eq!(first.asset_index_bytes(), second.asset_index_bytes());
    assert_eq!(
        manifest(&first).canonical_bytes().unwrap(),
        manifest(&second).canonical_bytes().unwrap()
    );
}

#[test]
fn empty_asset_set_still_has_index_dump_and_roles() {
    let plan = BackupPlan::from_rows(vec![]).unwrap();
    let m = manifest(&plan);
    assert_eq!(plan.logical_asset_count(), 0);
    assert_eq!(plan.unique_asset_bytes(), 0);
    assert_eq!(
        plan.asset_index_bytes(),
        br#"{"format_version":1,"assets":[]}"#
    );
    assert_eq!(
        plan.asset_index_file().sha256,
        "d00a967be70e2511218465a948673547be45f4979f077377e5d339e29f8fc7df"
    );
    assert_eq!(
        m.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
        vec!["asset-index.json", "database.dump", "roles.json"]
    );
    m.validate_with_index(plan.asset_index_bytes()).unwrap();
}

#[test]
fn rejects_bad_digest_key_negative_size_duplicate_identity_and_conflicting_digest() {
    let mut bad = row(1, 1, A, 1);
    bad.storage_key = "sha256/aa/../".into();
    assert!(BackupPlan::from_rows(vec![bad]).is_err());
    assert!(BackupPlan::from_rows(vec![row(1, 1, &A.to_uppercase(), 1)]).is_err());
    assert!(BackupPlan::from_rows(vec![row(1, 1, A, -1)]).is_err());
    assert!(BackupPlan::from_rows(vec![row(1, 1, A, 1), row(1, 1, A, 1)]).is_err());
    assert!(BackupPlan::from_rows(vec![row(1, 1, A, 1), row(2, 2, A, 2)]).is_err());
}

#[test]
fn rejects_unique_byte_count_overflow() {
    assert!(
        BackupPlan::from_rows(vec![
            row(1, 1, A, i64::MAX),
            row(1, 2, B, i64::MAX),
            row(
                1,
                3,
                "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                i64::MAX
            )
        ])
        .is_err()
    );
}

#[test]
fn manifest_rejects_missing_extra_noncanonical_and_mismatched_files() {
    let plan = BackupPlan::from_rows(vec![row(1, 1, A, 7)]).unwrap();
    let m = manifest(&plan);
    m.validate_with_index(plan.asset_index_bytes()).unwrap();
    let mut missing = m.clone();
    missing.files.pop();
    assert!(
        missing
            .validate_with_index(plan.asset_index_bytes())
            .is_err()
    );
    let mut extra = m.clone();
    extra.files.push(file("extra.txt", B, 1));
    assert!(extra.validate_with_index(plan.asset_index_bytes()).is_err());
    let mut bad_path = m.clone();
    bad_path.files[0].path = "../asset-index.json".into();
    assert!(
        bad_path
            .validate_with_index(plan.asset_index_bytes())
            .is_err()
    );
    let mut changed = m.clone();
    changed.files[0].sha256 = B.into();
    assert!(
        changed
            .validate_with_index(plan.asset_index_bytes())
            .is_err()
    );
}

#[test]
fn manifest_rejects_unknown_version_and_claimed_complete_state() {
    let plan = BackupPlan::from_rows(vec![]).unwrap();
    let m = manifest(&plan);
    let mut unknown: serde_json::Value =
        serde_json::from_slice(&m.canonical_bytes().unwrap()).unwrap();
    unknown["format_version"] = 2.into();
    let parsed: BackupManifestV1 = serde_json::from_value(unknown).unwrap();
    assert!(
        parsed
            .validate_with_index(plan.asset_index_bytes())
            .is_err()
    );
    let mut claimed: serde_json::Value =
        serde_json::from_slice(&m.canonical_bytes().unwrap()).unwrap();
    claimed["state"] = "complete".into();
    assert!(serde_json::from_value::<BackupManifestV1>(claimed).is_err());
}

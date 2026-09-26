#![cfg(windows)]
use learning_assets::{
    FsAssetStore, SnapshotIoError, SnapshotJobDirectory, stage_incoming, stage_reading_copy,
    stage_snapshot,
};
use learning_core::{
    ExactSnapshotManifest, ReadingCopy, ReadingCopyManifest, ReadingRef, SNAPSHOT_FORMAT_VERSION,
};
use std::io::Read;
use uuid::Uuid;

#[test]
fn windows_staging_fails_before_any_file_or_directory_side_effect() {
    let root = std::env::temp_dir().join(format!("missing-snapshot-root-{}", Uuid::new_v4()));
    let id = Uuid::from_u128(7);
    let copy_manifest = ReadingCopyManifest {
        format_version: SNAPSHOT_FORMAT_VERSION,
        copy_id: id,
        files: vec![],
    };
    let copy = ReadingCopy {
        items: vec![],
        evidence: vec![],
    };
    assert!(matches!(
        stage_reading_copy(&root, id, &copy_manifest, &copy),
        Err(SnapshotIoError::InvalidPackage)
    ));
    assert!(matches!(
        stage_incoming(&root, std::iter::empty::<(String, Box<dyn Read>)>()),
        Err(SnapshotIoError::InvalidPackage)
    ));
    let exact = ExactSnapshotManifest {
        format_version: SNAPSHOT_FORMAT_VERSION,
        root: ReadingRef {
            view_id: id,
            revision_id: id,
        },
        files: vec![],
        requires_destination_assets: true,
    };
    // The function must refuse before inspecting the store; this dummy root is never created.
    let store = FsAssetStore::new(std::env::temp_dir(), std::env::temp_dir()).unwrap();
    assert!(matches!(
        stage_snapshot(&root, id, &exact, &[], &[], &store),
        Err(SnapshotIoError::InvalidPackage)
    ));
    assert!(SnapshotJobDirectory::open(&root, id).is_err());
    assert!(!root.exists());
}

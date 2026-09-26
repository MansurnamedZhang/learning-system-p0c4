use learning_assets::{
    FsAssetStore, SnapshotIoError, UploadDeclaration, stage_incoming, stage_reading_copy,
    stage_snapshot, verify_snapshot,
};
use learning_core::{
    AssetRef, AssetUseRef, BlockRef, CopyItem, ExactSnapshotManifest, ReadingCopy,
    ReadingCopyManifest, ReadingRef, SNAPSHOT_FORMAT_VERSION, SnapshotAssetUse, SnapshotFile,
    SnapshotIdentityPart, SnapshotRow, SnapshotTable, canonical_json, canonical_record_hash,
    hex_digest, snapshot_object_path,
};
use std::{
    fs,
    io::{Cursor, Read},
    path::PathBuf,
};
use uuid::Uuid;

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("snapshot-package-{}", Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn copy_manifest() -> ReadingCopyManifest {
    ReadingCopyManifest {
        format_version: SNAPSHOT_FORMAT_VERSION,
        copy_id: Uuid::from_u128(7),
        files: vec![],
    }
}
fn copy() -> ReadingCopy {
    ReadingCopy {
        items: vec![
            CopyItem::Heading {
                title: "Chapter".into(),
            },
            CopyItem::Content {
                intent: "note".into(),
                title: "A".into(),
                display_text: "Body".into(),
            },
        ],
        evidence: vec![],
    }
}
fn stream(bytes: &[u8]) -> Box<dyn Read> {
    Box::new(Cursor::new(bytes.to_vec()))
}
fn exact_row() -> (SnapshotRow, SnapshotFile) {
    let id = Uuid::from_u128(10);
    let immutable_values =
        serde_json::json!({"id": id.to_string(), "space_id": Uuid::from_u128(1).to_string()});
    let row = SnapshotRow {
        table: SnapshotTable::Block,
        identity: vec![SnapshotIdentityPart::Uuid(id)],
        sha256: canonical_record_hash(&immutable_values),
        immutable_values,
    };
    let bytes = canonical_json(&serde_json::to_value(&row).unwrap());
    let path = snapshot_object_path(row.table, &row.identity).unwrap();
    let file = SnapshotFile {
        path,
        size: bytes.len() as u64,
        sha256: hex_digest(bytes.as_bytes()),
    };
    (row, file)
}
fn exact_manifest(files: Vec<SnapshotFile>, metadata_only: bool) -> ExactSnapshotManifest {
    ExactSnapshotManifest {
        format_version: SNAPSHOT_FORMAT_VERSION,
        root: ReadingRef {
            view_id: Uuid::from_u128(1),
            revision_id: Uuid::from_u128(2),
        },
        files,
        requires_destination_assets: metadata_only,
    }
}

#[test]
fn deterministic_copy_and_verified_stream_round_trip() {
    let a = Temp::new();
    let b = Temp::new();
    let first = stage_reading_copy(&a.0, Uuid::from_u128(7), &copy_manifest(), &copy()).unwrap();
    let second = stage_reading_copy(&b.0, Uuid::from_u128(7), &copy_manifest(), &copy()).unwrap();
    assert_eq!(first.manifest_sha256(), second.manifest_sha256());
    let files = first.open_verified_files().unwrap();
    assert!(files.iter().any(|(name, _)| name == "manifest.json"));
    let incoming = stage_incoming(
        &b.0,
        files
            .into_iter()
            .map(|(name, file)| (name, Box::new(file) as Box<dyn Read>)),
    )
    .unwrap();
    assert_eq!(incoming.manifest_sha256(), first.manifest_sha256());
    assert!(verify_snapshot(&incoming).is_ok());
}

#[test]
fn rejects_traversal_absolute_collision_and_partial_delivery() {
    let root = Temp::new();
    for name in ["../escape", "/tmp/escape", "MANIFEST.JSON"] {
        assert!(matches!(
            stage_incoming(&root.0, [(name.into(), stream(b"x"))].into_iter()),
            Err(SnapshotIoError::InvalidPackage)
        ));
    }
    assert!(
        stage_incoming(
            &root.0,
            [("manifest.json".into(), stream(b"{}"))].into_iter()
        )
        .is_err()
    );
    assert!(fs::read_dir(&root.0).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".ready")
    }));
}

#[test]
fn rejects_extra_files_corrupt_content_and_reparse_points() {
    let root = Temp::new();
    let stage = stage_reading_copy(&root.0, Uuid::from_u128(7), &copy_manifest(), &copy()).unwrap();
    let named = fs::read_dir(&root.0)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::write(named.join("unexpected"), b"secret").unwrap();
    assert!(matches!(
        verify_snapshot(&stage),
        Err(SnapshotIoError::InvalidPackage)
    ));
    fs::remove_file(named.join("unexpected")).unwrap();
    fs::write(named.join("reading.md"), b"tampered").unwrap();
    assert!(verify_snapshot(&stage).is_err());
}

#[test]
fn rejects_symlink_in_sealed_package_when_platform_allows_creation() {
    let root = Temp::new();
    let stage = stage_reading_copy(&root.0, Uuid::from_u128(7), &copy_manifest(), &copy()).unwrap();
    let named = fs::read_dir(&root.0)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let outside = root.0.join("outside");
    fs::write(&outside, b"secret").unwrap();
    fs::remove_file(named.join("reading.md")).unwrap();
    #[cfg(windows)]
    let created = std::os::windows::fs::symlink_file(&outside, named.join("reading.md"));
    #[cfg(unix)]
    let created = std::os::unix::fs::symlink(&outside, named.join("reading.md"));
    if created.is_ok() {
        assert!(matches!(
            verify_snapshot(&stage),
            Err(SnapshotIoError::InvalidPackage)
        ));
    }
}

#[cfg(unix)]
#[test]
fn incoming_reads_open_handle_when_source_name_is_replaced_with_symlink() {
    let root = Temp::new();
    let source_root = root.0.join("source-packages");
    let package_root = root.0.join("packages");
    fs::create_dir(&source_root).unwrap();
    fs::create_dir(&package_root).unwrap();
    let original =
        stage_reading_copy(&source_root, Uuid::from_u128(7), &copy_manifest(), &copy()).unwrap();
    let mut files = original.open_verified_files().unwrap();
    let mut markdown = Vec::new();
    files
        .iter_mut()
        .find(|(name, _)| name == "reading.md")
        .unwrap()
        .1
        .read_to_end(&mut markdown)
        .unwrap();
    let source = root.0.join("source");
    let outside = root.0.join("outside");
    fs::write(&source, &markdown).unwrap();
    fs::write(&outside, b"secret").unwrap();
    let opened = fs::File::open(&source).unwrap();
    fs::remove_file(&source).unwrap();
    std::os::unix::fs::symlink(&outside, &source).unwrap();
    let incoming = files.into_iter().map(|(name, file)| {
        if name == "reading.md" {
            (name, Box::new(opened.try_clone().unwrap()) as Box<dyn Read>)
        } else {
            (name, Box::new(file) as Box<dyn Read>)
        }
    });
    let staged = stage_incoming(&package_root, incoming).unwrap();
    assert_eq!(staged.manifest_sha256(), original.manifest_sha256());
    let mut copied = Vec::new();
    staged
        .open_verified_files()
        .unwrap()
        .into_iter()
        .find(|(name, _)| name == "reading.md")
        .unwrap()
        .1
        .read_to_end(&mut copied)
        .unwrap();
    assert_eq!(copied, markdown);
}

#[test]
fn rejects_large_stream_before_publication() {
    let root = Temp::new();
    let oversized = vec![0u8; learning_core::SNAPSHOT_MAX_JSON_FILE_BYTES + 1];
    assert!(matches!(
        stage_incoming(
            &root.0,
            [("validation.json".into(), stream(&oversized))].into_iter()
        ),
        Err(SnapshotIoError::LimitExceeded)
    ));
}

struct Interrupted(bool);
impl Read for Interrupted {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "interrupted",
            ));
        }
        self.0 = true;
        out[..4].copy_from_slice(b"part");
        Ok(4)
    }
}
#[test]
fn interrupted_input_never_creates_ready_package() {
    let root = Temp::new();
    let result = stage_incoming(
        &root.0,
        [(
            "reading.md".into(),
            Box::new(Interrupted(false)) as Box<dyn Read>,
        )]
        .into_iter(),
    );
    assert!(matches!(result, Err(SnapshotIoError::Io(_))));
    assert!(fs::read_dir(&root.0).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".ready")
    }));
}

#[test]
fn stages_exact_rows_using_planned_canonical_bytes() {
    let root = Temp::new();
    let store = FsAssetStore::new(root.0.join("assets"), root.0.join("asset-staging")).unwrap();
    let package_root = root.0.join("packages");
    fs::create_dir(&package_root).unwrap();
    let (row, file) = exact_row();
    let stage = stage_snapshot(
        &package_root,
        Uuid::new_v4(),
        &exact_manifest(vec![file], true),
        &[row],
        &[],
        &store,
    )
    .unwrap();
    let verified = verify_snapshot(&stage).unwrap();
    assert_eq!(verified.rows.len(), 1);
    assert!(
        stage
            .open_verified_files()
            .unwrap()
            .iter()
            .any(|(name, _)| name.starts_with("objects/block/"))
    );
}

#[test]
fn missing_and_corrupt_asset_bytes_never_publish() {
    let root = Temp::new();
    let store = FsAssetStore::new(root.0.join("assets"), root.0.join("asset-staging")).unwrap();
    let package_root = root.0.join("packages");
    fs::create_dir(&package_root).unwrap();
    let bytes = b"asset bytes";
    let sha = hex_digest(bytes);
    let source = root.0.join("source");
    fs::write(&source, bytes).unwrap();
    let blob = store
        .put_from_file(
            Uuid::new_v4(),
            &source,
            UploadDeclaration {
                expected_size_bytes: bytes.len() as u64,
                max_size_bytes: 100,
            },
        )
        .unwrap();
    let asset = SnapshotAssetUse {
        use_ref: AssetUseRef::Block(BlockRef {
            block_id: Uuid::from_u128(3),
            revision_id: Uuid::from_u128(4),
        }),
        asset: AssetRef {
            space_id: Uuid::from_u128(1),
            asset_id: Uuid::from_u128(2),
        },
        sha256: sha.clone(),
        byte_size: bytes.len() as u64,
        storage_key: blob.storage_key().into(),
    };
    let entry = SnapshotFile {
        path: format!("assets/{}", blob.storage_key()),
        size: bytes.len() as u64,
        sha256: sha.clone(),
    };
    let (row, object) = exact_row();
    let manifest = exact_manifest(vec![entry, object], false);
    let store_missing =
        FsAssetStore::new(root.0.join("missing"), root.0.join("missing-stage")).unwrap();
    assert!(matches!(
        stage_snapshot(
            &package_root,
            Uuid::new_v4(),
            &manifest,
            std::slice::from_ref(&row),
            std::slice::from_ref(&asset),
            &store_missing
        ),
        Err(SnapshotIoError::MissingAsset)
    ));
    let good = stage_snapshot(
        &package_root,
        Uuid::new_v4(),
        &manifest,
        std::slice::from_ref(&row),
        std::slice::from_ref(&asset),
        &store,
    )
    .unwrap();
    assert!(
        verify_snapshot(&good)
            .unwrap()
            .files
            .iter()
            .any(|f| f.path.starts_with("assets/"))
    );
    fs::write(root.0.join("assets").join(blob.storage_key()), b"corrupt").unwrap();
    assert!(matches!(
        stage_snapshot(
            &package_root,
            Uuid::new_v4(),
            &manifest,
            &[row],
            &[asset],
            &store
        ),
        Err(SnapshotIoError::CorruptAsset)
    ));
    assert_eq!(
        fs::read_dir(package_root)
            .unwrap()
            .filter(|entry| entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".ready"))
            .count(),
        1
    );
}

#[test]
fn reading_renderers_escape_markup_and_unsafe_links() {
    let root = Temp::new();
    let copy = ReadingCopy {
        items: vec![CopyItem::Content {
            intent: "note".into(),
            title: "<script>".into(),
            display_text: "[click](javascript:alert(1)) & <img>".into(),
        }],
        evidence: vec![],
    };
    let stage = stage_reading_copy(&root.0, Uuid::from_u128(7), &copy_manifest(), &copy).unwrap();
    let mut files = stage.open_verified_files().unwrap();
    let mut html = String::new();
    files
        .iter_mut()
        .find(|(name, _)| name == "reading.html")
        .unwrap()
        .1
        .read_to_string(&mut html)
        .unwrap();
    assert!(!html.contains("<script>"));
    assert!(!html.contains("<img>"));
    assert!(html.contains("&lt;script&gt;"));
    let mut md = String::new();
    files
        .iter_mut()
        .find(|(name, _)| name == "reading.md")
        .unwrap()
        .1
        .read_to_string(&mut md)
        .unwrap();
    assert!(!md.contains("]("));
    assert!(md.contains("\\]\\("));
}

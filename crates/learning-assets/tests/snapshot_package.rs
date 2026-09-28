#![cfg(target_os = "linux")]
use learning_assets::{
    FsAssetStore, SnapshotIoError, SnapshotJobDirectory, UploadDeclaration, stage_incoming,
    stage_reading_copy, stage_snapshot, verify_snapshot,
};
use learning_core::{
    AssetRef, AssetUseRef, BlockRef, CopyItem, ExactSnapshotManifest, ReadingCopy,
    ReadingCopyManifest, ReadingRef, SNAPSHOT_FORMAT_VERSION, SnapshotAssetUse, SnapshotFile,
    SnapshotIdentityPart, SnapshotRow, SnapshotTable, canonical_json, canonical_record_hash,
    hex_digest, snapshot_object_path,
};
use std::{
    fs,
    io::{Cursor, Read, Write},
    path::PathBuf,
};
use uuid::Uuid;

struct Temp(PathBuf);
fn private_dir(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
fn writable_tree(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_dir() {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
            for entry in fs::read_dir(path).unwrap() {
                writable_tree(&entry.unwrap().path());
            }
        } else if meta.file_type().is_file() {
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        }
    }
}
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("snapshot-package-{}", Uuid::new_v4()));
        private_dir(&path);
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        writable_tree(&self.0);
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
    exact_row_id(10)
}
fn exact_row_id(n: u128) -> (SnapshotRow, SnapshotFile) {
    let id = Uuid::from_u128(n);
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
fn declared_asset(n: u128, size: u64) -> SnapshotAssetUse {
    let sha256 = format!("{n:064x}");
    SnapshotAssetUse {
        use_ref: AssetUseRef::Block(BlockRef {
            block_id: Uuid::from_u128(n),
            revision_id: Uuid::from_u128(n),
        }),
        asset: AssetRef {
            space_id: Uuid::from_u128(1),
            asset_id: Uuid::from_u128(n),
        },
        storage_key: format!("sha256/{}/{}", &sha256[..2], sha256),
        sha256,
        byte_size: size,
    }
}
fn ready_count(root: &std::path::Path) -> usize {
    fs::read_dir(root)
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".ready")
        })
        .count()
}

#[test]
fn package_file_count_accepts_2048_and_rejects_2049_without_publication() {
    let root = Temp::new();
    let packages = root.0.join("packages");
    private_dir(&packages);
    let store = FsAssetStore::new(root.0.join("assets"), root.0.join("uploads")).unwrap();
    let build = |count: usize| {
        let (rows, mut files): (Vec<_>, Vec<_>) =
            (1..=count).map(|n| exact_row_id(n as u128)).unzip();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        (rows, files)
    };
    let (rows, files) = build(learning_core::SNAPSHOT_MAX_FILES - 2);
    let accepted = stage_snapshot(
        &packages,
        Uuid::new_v4(),
        &exact_manifest(files, true),
        &rows,
        &[],
        &store,
    )
    .unwrap();
    assert_eq!(verify_snapshot(&accepted).unwrap().files.len() + 1, 2048);
    assert_eq!(ready_count(&packages), 1);

    let (rows, files) = build(learning_core::SNAPSHOT_MAX_FILES - 1);
    assert!(matches!(
        stage_snapshot(
            &packages,
            Uuid::new_v4(),
            &exact_manifest(files, true),
            &rows,
            &[],
            &store,
        ),
        Err(SnapshotIoError::LimitExceeded)
    ));
    assert_eq!(
        ready_count(&packages),
        1,
        "over-limit package must not publish"
    );
}

#[test]
fn metadata_only_ignores_original_byte_budget_but_included_originals_do_not() {
    let root = Temp::new();
    let packages = root.0.join("packages");
    private_dir(&packages);
    let store = FsAssetStore::new(root.0.join("assets"), root.0.join("uploads")).unwrap();
    let (row, file) = exact_row();
    let one = learning_core::SNAPSHOT_MAX_ASSET_FILE_BYTES as u64;
    let at_total = (1..=4).map(|n| declared_asset(n, one)).collect::<Vec<_>>();
    let mut over_total = at_total;
    over_total.push(declared_asset(5, one));
    // None of these originals enters a metadata-only directory package.
    let accepted = stage_snapshot(
        &packages,
        Uuid::new_v4(),
        &exact_manifest(vec![file.clone()], true),
        std::slice::from_ref(&row),
        &over_total,
        &store,
    )
    .unwrap();
    assert!(verify_snapshot(&accepted).is_ok());
    assert_eq!(ready_count(&packages), 1);

    assert!(matches!(
        stage_snapshot(
            &packages,
            Uuid::new_v4(),
            &exact_manifest(vec![file.clone()], false),
            std::slice::from_ref(&row),
            &[declared_asset(1, one + 1)],
            &store,
        ),
        Err(SnapshotIoError::LimitExceeded)
    ));
    assert_eq!(ready_count(&packages), 1);

    assert!(matches!(
        stage_snapshot(
            &packages,
            Uuid::new_v4(),
            &exact_manifest(vec![file], false),
            &[row],
            &over_total,
            &store,
        ),
        Err(SnapshotIoError::LimitExceeded)
    ));
    assert_eq!(
        ready_count(&packages),
        1,
        "over-limit originals must not publish"
    );
}

#[test]
fn deterministic_copy_and_verified_stream_round_trip() {
    let a = Temp::new();
    let b = Temp::new();
    let first = stage_reading_copy(&a.0, Uuid::from_u128(7), &copy_manifest(), &copy()).unwrap();
    let second = stage_reading_copy(&b.0, Uuid::from_u128(7), &copy_manifest(), &copy()).unwrap();
    assert_eq!(first.manifest_sha256(), second.manifest_sha256());
    assert!(verify_snapshot(&first).is_ok());
    assert!(verify_snapshot(&first).is_ok());
    let files = first.open_verified_files().unwrap();
    assert_eq!(first.open_verified_files().unwrap().len(), files.len());
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
    writable_tree(&named);
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
fn rejects_symlink_in_sealed_package() {
    let root = Temp::new();
    let stage = stage_reading_copy(&root.0, Uuid::from_u128(7), &copy_manifest(), &copy()).unwrap();
    let named = fs::read_dir(&root.0)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    writable_tree(&named);
    let outside = root.0.join("outside");
    fs::write(&outside, b"secret").unwrap();
    fs::remove_file(named.join("reading.md")).unwrap();
    std::os::unix::fs::symlink(&outside, named.join("reading.md")).unwrap();
    assert!(matches!(
        verify_snapshot(&stage),
        Err(SnapshotIoError::InvalidPackage)
    ));
}

#[cfg(unix)]
#[test]
fn incoming_reads_open_handle_when_source_name_is_replaced_with_symlink() {
    let root = Temp::new();
    let source_root = root.0.join("source-packages");
    let package_root = root.0.join("packages");
    private_dir(&source_root);
    private_dir(&package_root);
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

struct CrashReader(bool);
impl Read for CrashReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.0 {
            std::process::exit(73);
        }
        self.0 = true;
        out[..4].copy_from_slice(b"part");
        Ok(4)
    }
}
#[test]
fn process_exit_before_seal_never_publishes() {
    if let Ok(root) = std::env::var("SNAPSHOT_CRASH_ROOT") {
        let _ = stage_incoming(
            std::path::Path::new(&root),
            [(
                "reading.md".into(),
                Box::new(CrashReader(false)) as Box<dyn Read>,
            )]
            .into_iter(),
        );
        panic!("child should have exited while copying");
    }
    let root = Temp::new();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("process_exit_before_seal_never_publishes")
        .env("SNAPSHOT_CRASH_ROOT", &root.0)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(73), "child output: {output:?}");
    let names = fs::read_dir(&root.0)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    assert!(names.iter().any(|name| name.contains(".partial-")));
    assert!(!names.iter().any(|name| name.ends_with(".ready")));
}

#[test]
fn rejects_aggregate_json_budget_before_publication() {
    let root = Temp::new();
    let files = (0..9).map(|n| {
        let name = format!("objects/block/u-{}.json", Uuid::from_u128(n + 1));
        (
            name,
            Box::new(std::io::repeat(b'x').take(learning_core::SNAPSHOT_MAX_JSON_FILE_BYTES as u64))
                as Box<dyn Read>,
        )
    });
    assert!(matches!(
        stage_incoming(&root.0, files),
        Err(SnapshotIoError::LimitExceeded)
    ));
    assert!(!fs::read_dir(&root.0).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".ready")
    }));
}

#[test]
fn rejects_premature_eof_against_declared_manifest_length() {
    let source_root = Temp::new();
    let target_root = Temp::new();
    let source = stage_reading_copy(
        &source_root.0,
        Uuid::from_u128(7),
        &copy_manifest(),
        &copy(),
    )
    .unwrap();
    let files = source
        .open_verified_files()
        .unwrap()
        .into_iter()
        .map(|(name, mut file)| {
            if name == "reading.md" {
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes).unwrap();
                bytes.pop();
                (name, Box::new(Cursor::new(bytes)) as Box<dyn Read>)
            } else {
                (name, Box::new(file) as Box<dyn Read>)
            }
        });
    assert!(matches!(
        stage_incoming(&target_root.0, files),
        Err(SnapshotIoError::InvalidPackage)
    ));
}

#[test]
fn private_root_and_sealed_delivery_are_enforced() {
    use std::os::unix::fs::PermissionsExt;
    let root = Temp::new();
    fs::set_permissions(&root.0, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        stage_reading_copy(&root.0, Uuid::from_u128(7), &copy_manifest(), &copy()),
        Err(SnapshotIoError::InvalidPackage)
    ));
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 0);
    fs::set_permissions(&root.0, fs::Permissions::from_mode(0o700)).unwrap();
    let stage = stage_reading_copy(&root.0, Uuid::from_u128(7), &copy_manifest(), &copy()).unwrap();
    let mut delivered = stage.open_verified_files().unwrap();
    let (_, handle) = delivered
        .iter_mut()
        .find(|(name, _)| name == "reading.md")
        .unwrap();
    let mut original = Vec::new();
    handle.read_to_end(&mut original).unwrap();
    assert!(
        handle.write_all(b"attack").is_err(),
        "delivery memfd must be sealed"
    );
    let named = fs::read_dir(&root.0)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    writable_tree(&named);
    fs::write(named.join("reading.md"), b"changed after delivery").unwrap();
    use std::io::{Seek, SeekFrom};
    handle.seek(SeekFrom::Start(0)).unwrap();
    let mut still_original = Vec::new();
    handle.read_to_end(&mut still_original).unwrap();
    assert_eq!(still_original, original);
}

#[test]
fn verified_package_keeps_its_directory_handle_after_ready_name_is_replaced() {
    let root = Temp::new();
    let stage = stage_reading_copy(&root.0, Uuid::from_u128(7), &copy_manifest(), &copy()).unwrap();
    let ready = fs::read_dir(&root.0)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let moved = root.0.join("retained-package");
    fs::rename(&ready, &moved).unwrap();
    let decoy = root.0.join("decoy");
    private_dir(&decoy);
    fs::write(decoy.join("reading.md"), b"external replacement").unwrap();
    std::os::unix::fs::symlink(&decoy, &ready).unwrap();
    assert!(verify_snapshot(&stage).is_ok());
    assert_eq!(stage.open_verified_files().unwrap().len(), 4);
}

#[test]
fn stages_exact_rows_using_planned_canonical_bytes() {
    let root = Temp::new();
    let store = FsAssetStore::new(root.0.join("assets"), root.0.join("asset-staging")).unwrap();
    let package_root = root.0.join("packages");
    private_dir(&package_root);
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
    private_dir(&package_root);
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

#[test]
fn job_attempt_reopen_rejects_wrong_hash_symlinks_and_does_not_replace_other_attempts() {
    let root = Temp::new();
    let job = Uuid::new_v4();
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();
    let directory = SnapshotJobDirectory::open(&root.0, job).unwrap();
    let stage = directory.stage_copy(first, &copy()).unwrap();
    let hash = stage.manifest_sha256().to_owned();
    drop(stage);
    drop(directory);
    let reopened = SnapshotJobDirectory::open(&root.0, job).unwrap();
    assert!(reopened.reopen(first, &"a".repeat(64)).is_err());
    assert!(
        !reopened
            .reopen(first, &hash)
            .unwrap()
            .open_verified_files()
            .unwrap()
            .is_empty()
    );
    let retry = reopened.stage_copy(second, &copy()).unwrap();
    assert_eq!(
        retry.manifest_sha256(),
        hash,
        "retry token must not enter the logical manifest"
    );
    assert!(
        root.0
            .join(job.to_string())
            .join(format!("{first}.ready"))
            .is_dir()
    );
    assert!(
        root.0
            .join(job.to_string())
            .join(format!("{second}.ready"))
            .is_dir()
    );
    assert!(reopened.stage_copy(first, &copy()).is_err());
    assert!(reopened.reopen(first, &hash).is_ok());
    let alias = Uuid::new_v4();
    std::os::unix::fs::symlink(root.0.join(job.to_string()), root.0.join(alias.to_string()))
        .unwrap();
    assert!(SnapshotJobDirectory::open(&root.0, alias).is_err());
    fs::remove_file(root.0.join(alias.to_string())).unwrap();
}

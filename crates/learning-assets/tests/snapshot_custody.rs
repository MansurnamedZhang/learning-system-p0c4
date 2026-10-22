#![cfg(target_os = "linux")]
//! Explicit Root-only custody checks. Run with --ignored; prerequisites fail
//! loudly instead of silently skipping the Linux security boundary.
use learning_assets::{
    FsAssetStore, RootSnapshotCustody, SnapshotIoError, SnapshotJobDirectory, UploadDeclaration,
    stage_incoming, stage_reading_copy, stage_snapshot, verify_snapshot,
};
use learning_core::{
    AssetRef, AssetUseRef, BlockRef, CopyItem, ExactSnapshotManifest, ReadingCopy,
    ReadingCopyManifest, ReadingRef, SNAPSHOT_FORMAT_VERSION, SnapshotAssetUse, SnapshotFile,
    SnapshotIdentityPart, SnapshotRow, SnapshotTable, canonical_json, canonical_record_hash,
    hex_digest, snapshot_object_path,
};
use std::{
    fs,
    io::{Read, Write},
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::{Path, PathBuf},
};
use uuid::Uuid;

const MARKER: &str = ".snapshot-root-custody-v2";

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        assert_eq!(unsafe { libc::geteuid() }, 0, "requires Linux UID 0");
        let parent = std::env::var_os("TASK6_CUSTODY_TEST_PARENT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/root"));
        assert!(parent.is_absolute(), "test parent must be absolute");
        let path = parent.join(format!("snapshot-custody-test-{}", Uuid::new_v4()));
        private_dir(&path);
        Self(path)
    }
    fn dir(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        private_dir(&path);
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fn writable(path: &Path) {
            let meta = fs::symlink_metadata(path).unwrap();
            if meta.is_dir() {
                fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
                for entry in fs::read_dir(path).unwrap() {
                    writable(&entry.unwrap().path());
                }
            }
        }
        writable(&self.0);
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn private_dir(path: &Path) {
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
fn copy() -> ReadingCopy {
    ReadingCopy {
        items: vec![CopyItem::Content {
            intent: "note".into(),
            title: "Custody".into(),
            display_text: "real nonempty package bytes".into(),
        }],
        evidence: vec![],
    }
}
fn copy_manifest(id: Uuid) -> ReadingCopyManifest {
    ReadingCopyManifest {
        format_version: SNAPSHOT_FORMAT_VERSION,
        copy_id: id,
        files: vec![],
    }
}

#[test]
#[ignore = "requires Linux UID 0 and a trusted root-owned parent; run explicitly"]
fn metered_copy_prepays_before_first_mkdir_and_failed_stage_never_refunds() {
    use learning_assets::root_journal::SharedJournalBudget;
    let fixture = Fixture::new();
    let root = fixture.0.join("metered");
    let custody = RootSnapshotCustody::create_fresh(&root).unwrap();
    let budget = SharedJournalBudget::default();
    budget.charge(512 * 1024 * 1024 - 1000, 0).unwrap();
    let job = Uuid::new_v4();
    assert!(
        custody
            .writer()
            .unwrap()
            .stage_copy_metered(job, Uuid::new_v4(), &copy(), budget.clone())
            .is_err()
    );
    assert!(
        !root.join(job.to_string()).exists(),
        "no job/partial/ready before paid admission"
    );
    assert!(budget.usage().unwrap().exhausted);
    assert!(
        custody
            .writer()
            .unwrap()
            .stage_copy_metered(Uuid::new_v4(), Uuid::new_v4(), &copy(), budget.clone())
            .is_err()
    );

    let budget = SharedJournalBudget::default();
    let blocked = Uuid::new_v4();
    fs::write(
        root.join(blocked.to_string()),
        b"Root fixture injects real ENOTDIR",
    )
    .unwrap();
    let before = budget.usage().unwrap().charged_bytes;
    assert!(
        custody
            .writer()
            .unwrap()
            .stage_copy_metered(blocked, Uuid::new_v4(), &copy(), budget.clone())
            .is_err()
    );
    let failed = budget.usage().unwrap();
    assert!(
        failed.charged_bytes > before,
        "all prepaid render/write/verify work survives actual error"
    );
    assert!(failed.charged_entries > 0);
    assert_eq!(
        budget.clone().usage().unwrap().charged_bytes,
        failed.charged_bytes
    );

    let job = Uuid::new_v4();
    let token = Uuid::new_v4();
    let staged = custody
        .writer()
        .unwrap()
        .stage_copy_metered(job, token, &copy(), budget.clone())
        .unwrap();
    let verified = verify_snapshot(staged.directory()).unwrap();
    assert_eq!(verified.files.len(), 3);
    let total: u64 = fs::read_dir(root.join(job.to_string()).join(format!("{token}.ready")))
        .unwrap()
        .map(|entry| entry.unwrap().metadata().unwrap().len())
        .sum();
    let used = budget.usage().unwrap();
    assert!(
        used.charged_bytes - failed.charged_bytes >= total * 4,
        "both verifications include manifest read+hash"
    );
    assert!(used.charged_entries > failed.charged_entries);
    assert!(custody.try_capture().is_err());
    drop(staged);
    assert!(custody.try_capture().is_ok());
}
fn exact_manifest() -> ExactSnapshotManifest {
    ExactSnapshotManifest {
        format_version: SNAPSHOT_FORMAT_VERSION,
        root: ReadingRef {
            view_id: Uuid::from_u128(1),
            revision_id: Uuid::from_u128(2),
        },
        files: vec![],
        requires_destination_assets: true,
    }
}

#[test]
#[ignore = "requires Linux UID 0 and a trusted root-owned parent; run explicitly"]
fn legacy_entries_reject_registered_root_and_descendant_before_mutation() {
    let fixture = Fixture::new();
    let ordinary = fixture.dir("ordinary");
    let id = Uuid::new_v4();
    let source = stage_reading_copy(&ordinary, id, &copy_manifest(id), &copy()).unwrap();
    let store = FsAssetStore::new(fixture.0.join("assets"), fixture.0.join("uploads")).unwrap();
    let registered = fixture.dir("registered");
    let mut marker = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(registered.join(MARKER))
        .unwrap();
    marker.write_all(b"os-custody-fence-v2\n").unwrap();
    marker.sync_all().unwrap();
    let child = registered.join("nested");
    private_dir(&child);
    for root in [&registered, &child] {
        let before = fs::read_dir(root).unwrap().count();
        assert!(
            stage_reading_copy(root, id, &copy_manifest(id), &copy()).is_err(),
            "legacy reading writer crossed registered ancestor: {}",
            root.display()
        );
        assert!(
            stage_snapshot(root, Uuid::new_v4(), &exact_manifest(), &[], &[], &store).is_err(),
            "legacy exact writer crossed registered ancestor: {}",
            root.display()
        );
        let streams = source
            .open_verified_files()
            .unwrap()
            .into_iter()
            .map(|(name, file)| (name, Box::new(file) as Box<dyn Read>));
        assert!(
            stage_incoming(root, streams).is_err(),
            "legacy incoming writer crossed registered ancestor: {}",
            root.display()
        );
        assert!(
            SnapshotJobDirectory::open(root, Uuid::new_v4()).is_err(),
            "legacy job writer crossed registered ancestor: {}",
            root.display()
        );
        assert_eq!(
            fs::read_dir(root).unwrap().count(),
            before,
            "refusal must precede file or directory mutation"
        );
    }
}

fn busy<T>(result: Result<T, SnapshotIoError>) {
    assert!(
        matches!(result, Err(SnapshotIoError::Io(ref e))
        if e.kind() == std::io::ErrorKind::WouldBlock),
        "expected real flock contention"
    );
}

#[test]
#[ignore = "requires Linux UID 0 and a trusted root-owned parent; run explicitly"]
fn custody_nonempty_copy_holds_shared_lock_through_staged_lifetime() {
    let fixture = Fixture::new();
    let root = fixture.0.join("custody");
    let custody = RootSnapshotCustody::create_fresh(&root).unwrap();
    let capture = custody.try_capture().unwrap();
    capture.revalidate().unwrap();
    busy(custody.writer());
    drop(capture);
    let writer = custody.writer().unwrap();
    let second = custody.writer().unwrap();
    busy(custody.try_capture());
    drop(second);
    let job = Uuid::new_v4();
    let token = Uuid::new_v4();
    let staged = writer.stage_copy(job, token, &copy()).unwrap();
    staged.revalidate().unwrap();
    let verified = verify_snapshot(staged.directory()).unwrap();
    assert_eq!(verified.files.len(), 3);
    let mut reading = String::new();
    staged
        .directory()
        .open_verified_files()
        .unwrap()
        .into_iter()
        .find(|(name, _)| name == "reading.md")
        .unwrap()
        .1
        .read_to_string(&mut reading)
        .unwrap();
    assert!(reading.contains("real nonempty package bytes"));
    busy(custody.try_capture());
    assert!(SnapshotJobDirectory::open(&root.join(job.to_string()), job).is_err());
    drop(staged);
    let capture = custody.try_capture().unwrap();
    busy(custody.writer());
    capture.revalidate().unwrap();
}

#[test]
#[ignore = "requires Linux UID 0 and a trusted root-owned parent; run explicitly"]
fn custody_existing_symlink_untrusted_parent_and_reopen_are_refused() {
    let fixture = Fixture::new();
    let existing = fixture.dir("existing");
    fs::write(existing.join("preserve"), b"keep").unwrap();
    assert!(RootSnapshotCustody::create_fresh(&existing).is_err());
    assert_eq!(fs::read(existing.join("preserve")).unwrap(), b"keep");
    let link = fixture.0.join("link");
    std::os::unix::fs::symlink(&existing, &link).unwrap();
    assert!(RootSnapshotCustody::create_fresh(&link).is_err());
    assert!(RootSnapshotCustody::create_fresh(&link.join("nested")).is_err());
    let untrusted = fixture.dir("untrusted");
    fs::set_permissions(&untrusted, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(RootSnapshotCustody::create_fresh(&untrusted.join("fresh")).is_err());
    assert!(!untrusted.join("fresh").exists());
    let root = fixture.0.join("custody");
    let custody = RootSnapshotCustody::create_fresh(&root).unwrap();
    assert!(root.join(MARKER).is_file());
    assert_eq!(fs::metadata(root.join("effect.lock")).unwrap().len(), 0);
    drop(custody);
    assert!(
        RootSnapshotCustody::create_fresh(&root).is_err(),
        "no disk adoption"
    );
    assert!(
        SnapshotJobDirectory::open(&root, Uuid::new_v4()).is_err(),
        "the fence persists after the original object is gone"
    );
}

#[test]
#[ignore = "requires Linux UID 0 and a trusted root-owned parent; run explicitly"]
fn custody_lock_marker_and_root_replacement_refuse_existing_writer_and_capture() {
    for leaf in ["effect.lock", MARKER, "root"] {
        let fixture = Fixture::new();
        let root = fixture.0.join("custody");
        let custody = RootSnapshotCustody::create_fresh(&root).unwrap();
        let writer = custody.writer().unwrap();
        if leaf == "root" {
            fs::rename(&root, fixture.0.join("moved")).unwrap();
            private_dir(&root);
        } else {
            let bytes = fs::read(root.join(leaf)).unwrap();
            fs::rename(root.join(leaf), root.join("original-control")).unwrap();
            fs::write(root.join(leaf), bytes).unwrap();
            fs::set_permissions(root.join(leaf), fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert!(custody.writer().is_err(), "must reject {leaf} replacement");
        assert!(
            custody.try_capture().is_err(),
            "must reject {leaf} replacement"
        );
        assert!(
            writer
                .stage_copy(Uuid::new_v4(), Uuid::new_v4(), &copy())
                .is_err(),
            "already-issued OS writer must recheck {leaf}"
        );
    }
}

#[test]
#[ignore = "requires Linux UID 0 and a trusted root-owned parent; run explicitly"]
fn custody_failed_exact_and_ready_collision_do_not_overwrite_or_retain_lock() {
    let fixture = Fixture::new();
    let root = fixture.0.join("custody");
    let custody = RootSnapshotCustody::create_fresh(&root).unwrap();
    let store = FsAssetStore::new(fixture.0.join("assets"), fixture.0.join("uploads")).unwrap();
    let job = Uuid::new_v4();
    let token = Uuid::new_v4();
    let mut invalid = exact_manifest();
    invalid.format_version = 999;
    assert!(
        custody
            .writer()
            .unwrap()
            .stage_exact(job, token, &invalid, &[], &[], &store)
            .is_err()
    );
    assert_eq!(
        fs::read_dir(root.join(job.to_string())).unwrap().count(),
        0,
        "failed stage must discard its partial before releasing guard"
    );
    drop(custody.try_capture().unwrap());
    let first = custody
        .writer()
        .unwrap()
        .stage_copy(job, token, &copy())
        .unwrap();
    let original_hash = first.directory().manifest_sha256().to_string();
    let bytes = fs::read(
        root.join(job.to_string())
            .join(format!("{token}.ready/manifest.json")),
    )
    .unwrap();
    assert!(
        custody
            .writer()
            .unwrap()
            .stage_copy(job, token, &copy())
            .is_err()
    );
    assert_eq!(
        fs::read(
            root.join(job.to_string())
                .join(format!("{token}.ready/manifest.json"))
        )
        .unwrap(),
        bytes
    );
    assert_eq!(first.directory().manifest_sha256(), original_hash);
    assert_eq!(fs::read_dir(root.join(job.to_string())).unwrap().count(), 1);
    busy(custody.try_capture());
    drop(first);
    custody.try_capture().unwrap().revalidate().unwrap();
}

#[test]
#[ignore = "requires Linux UID 0 and a trusted root-owned parent; run explicitly"]
fn custody_uid65532_cannot_touch_controls_or_any_parent() {
    let fixture = Fixture::new();
    let root = fixture.0.join("custody");
    let custody = RootSnapshotCustody::create_fresh(&root).unwrap();
    let marker = fs::read(root.join(MARKER)).unwrap();
    let status = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(r#"test "$(id -u)" = 65532 || exit 10
for leaf in effect.lock .snapshot-root-custody-v2 forged; do
  if ( : >> "$CUSTODY_TEST_ROOT/$leaf" ) 2>/dev/null; then exit 11; fi
done
if mkdir "$CUSTODY_TEST_ROOT/job" 2>/dev/null; then exit 12; fi
if mv "$CUSTODY_TEST_ROOT/.snapshot-root-custody-v2" "$CUSTODY_TEST_ROOT/replaced" 2>/dev/null; then exit 13; fi
if rm "$CUSTODY_TEST_ROOT/effect.lock" 2>/dev/null; then exit 14; fi
if chmod 777 "$CUSTODY_TEST_ROOT" 2>/dev/null; then exit 15; fi
if mv "$CUSTODY_TEST_ROOT" "$CUSTODY_TEST_ROOT-moved" 2>/dev/null; then exit 16; fi
exit 0"#)
        .env("CUSTODY_TEST_ROOT", &root)
        .current_dir("/")
        .uid(65532).gid(65532).status().unwrap();
    assert!(status.success(), "real UID65532 denial probe: {status}");
    assert_eq!(fs::read(root.join(MARKER)).unwrap(), marker);
    assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
    custody.try_capture().unwrap().revalidate().unwrap();
}

#[test]
#[ignore = "requires Linux UID 0 and a trusted root-owned parent; run explicitly"]
fn custody_exact_copies_real_original_and_row_through_existing_verifier() {
    let fixture = Fixture::new();
    let root = fixture.0.join("custody");
    let custody = RootSnapshotCustody::create_fresh(&root).unwrap();
    let assets_root = fixture.0.join("assets");
    let store = FsAssetStore::new(assets_root.clone(), fixture.0.join("uploads")).unwrap();
    let source = fixture.0.join("original");
    fs::write(&source, b"abc").unwrap();
    let blob = store
        .put_from_file(
            Uuid::new_v4(),
            &source,
            UploadDeclaration {
                expected_size_bytes: 3,
                max_size_bytes: 3,
            },
        )
        .unwrap();
    assert_eq!(
        blob.sha256(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let id = Uuid::from_u128(10);
    let values = serde_json::json!({"id": id.to_string()});
    let row = SnapshotRow {
        table: SnapshotTable::Block,
        identity: vec![SnapshotIdentityPart::Uuid(id)],
        sha256: canonical_record_hash(&values),
        immutable_values: values,
    };
    let row_bytes = canonical_json(&serde_json::to_value(&row).unwrap());
    let asset_path = format!("assets/{}", blob.storage_key());
    let mut manifest = exact_manifest();
    manifest.requires_destination_assets = false;
    manifest.files = vec![
        SnapshotFile {
            path: asset_path.clone(),
            size: 3,
            sha256: blob.sha256().into(),
        },
        SnapshotFile {
            path: snapshot_object_path(row.table, &row.identity).unwrap(),
            size: row_bytes.len() as u64,
            sha256: hex_digest(row_bytes.as_bytes()),
        },
    ];
    let asset = SnapshotAssetUse {
        use_ref: AssetUseRef::Block(BlockRef {
            block_id: id,
            revision_id: id,
        }),
        asset: AssetRef {
            space_id: Uuid::from_u128(1),
            asset_id: id,
        },
        storage_key: blob.storage_key().into(),
        sha256: blob.sha256().into(),
        byte_size: 3,
    };
    let staged = custody
        .writer()
        .unwrap()
        .stage_exact(
            Uuid::new_v4(),
            Uuid::new_v4(),
            &manifest,
            &[row],
            &[asset],
            &store,
        )
        .unwrap();
    let verified = verify_snapshot(staged.directory()).unwrap();
    assert_eq!(verified.rows.len(), 1);
    assert_eq!(verified.files.len(), 3);
    drop(store);
    fs::remove_file(source).unwrap();
    fs::remove_dir_all(assets_root).unwrap();
    let mut bytes = vec![];
    staged
        .directory()
        .open_verified_files()
        .unwrap()
        .into_iter()
        .find(|(name, _)| name == &asset_path)
        .unwrap()
        .1
        .read_to_end(&mut bytes)
        .unwrap();
    assert_eq!(bytes, b"abc");
    busy(custody.try_capture());
    drop(staged);
    custody.try_capture().unwrap().revalidate().unwrap();
}

#[test]
#[ignore = "requires Linux UID 0 and a trusted root-owned parent; run explicitly"]
fn custody_control_symlinks_and_marker_tamper_fail_closed() {
    for leaf in ["effect.lock", MARKER] {
        let fixture = Fixture::new();
        let root = fixture.0.join("custody");
        let custody = RootSnapshotCustody::create_fresh(&root).unwrap();
        fs::rename(root.join(leaf), root.join("original-control")).unwrap();
        std::os::unix::fs::symlink("original-control", root.join(leaf)).unwrap();
        assert!(custody.writer().is_err());
        assert!(custody.try_capture().is_err());
        assert!(SnapshotJobDirectory::open(&root, Uuid::new_v4()).is_err());
    }
    let fixture = Fixture::new();
    let root = fixture.0.join("custody");
    let custody = RootSnapshotCustody::create_fresh(&root).unwrap();
    let marker = fs::read(root.join(MARKER)).unwrap();
    fs::write(root.join(MARKER), vec![b'x'; marker.len()]).unwrap();
    assert!(custody.writer().is_err());
    assert!(custody.try_capture().is_err());
    assert!(SnapshotJobDirectory::open(&root, Uuid::new_v4()).is_err());
}

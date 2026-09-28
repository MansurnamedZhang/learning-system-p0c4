use learning_assets::{AssetIoError, FsAssetStore, UploadDeclaration, VerifiedBlob};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
use uuid::Uuid;

const PDF: &[u8] = b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\n%%EOF\n";
const PNG: &[u8] = &[137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 0, 73, 69, 78, 68];
const NOTEBOOK: &[u8] = br#"{"cells":[{"cell_type":"code","source":["print(42)"]}],"nbformat":4}"#;

fn put(store: &FsAssetStore, upload_id: Uuid, source: &Path) -> Result<VerifiedBlob, AssetIoError> {
    let expected_size_bytes = fs::metadata(source)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    store.put_from_file(
        upload_id,
        source,
        UploadDeclaration {
            expected_size_bytes,
            max_size_bytes: 1_000_000,
        },
    )
}

struct TestDirs {
    root: PathBuf,
}

impl TestDirs {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("learning-assets-test-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        Self { root }
    }

    fn assets(&self) -> PathBuf {
        self.root.join("assets")
    }

    fn staging(&self) -> PathBuf {
        self.root.join("staging")
    }

    fn source(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let source = self.root.join(name);
        fs::write(&source, bytes).unwrap();
        source
    }

    fn store(&self) -> FsAssetStore {
        FsAssetStore::new(self.assets(), self.staging()).unwrap()
    }
}

impl Drop for TestDirs {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn original_pdf_png_and_notebook_bytes_round_trip_with_literal_digests() {
    let dirs = TestDirs::new();
    let store = dirs.store();
    for (name, bytes, digest) in [
        (
            "handout.pdf",
            PDF,
            "904636248025ad20fb9c6bd8b700179a2a42edb5df3636e926c7e09055ee3f75",
        ),
        (
            "figure.png",
            PNG,
            "df79318095b5d776a44103b1a7b1c419c09e981bc75bd6ff8d2d39d411482bd1",
        ),
        (
            "cells.ipynb",
            NOTEBOOK,
            "e60b1dd5444bb8b1e8adb9f808058b2b953b57c3a359fae605bae25721687e44",
        ),
    ] {
        let source = dirs.source(name, bytes);
        let blob = put(&store, Uuid::new_v4(), &source).unwrap();
        assert_eq!(blob.sha256(), digest);
        assert_eq!(blob.size_bytes(), bytes.len() as u64);
        assert_eq!(
            blob.storage_key(),
            format!("sha256/{}/{digest}", &digest[..2])
        );
        assert!(!blob.storage_key().contains(name));
        store.verify(&blob).unwrap();
        let mut actual = Vec::new();
        store.open(&blob).unwrap().read_to_end(&mut actual).unwrap();
        assert_eq!(actual, bytes);
    }
}

#[test]
fn repeated_content_keeps_one_immutable_final_blob() {
    let dirs = TestDirs::new();
    let store = dirs.store();
    let first = dirs.source("first.pdf", PDF);
    let second = dirs.source("renamed.pdf", PDF);
    let a = put(&store, Uuid::new_v4(), &first).unwrap();
    let b = put(&store, Uuid::new_v4(), &second).unwrap();
    assert_eq!(a.storage_key(), b.storage_key());
    assert_eq!(a.sha256(), b.sha256());
    assert_eq!(fs::read(dirs.assets().join(a.storage_key())).unwrap(), PDF);
}

#[test]
fn interrupted_staging_never_produces_a_verified_blob_and_retry_restarts_cleanly() {
    let dirs = TestDirs::new();
    let store = dirs.store();
    let upload_id = Uuid::new_v4();
    let partial = dirs.staging().join(upload_id.to_string());
    fs::write(&partial, b"partial untrusted bytes").unwrap();
    assert!(put(&store, upload_id, &dirs.root.join("missing.pdf")).is_err());
    assert!(!dirs.assets().join("sha256").exists());

    let source = dirs.source("complete.pdf", PDF);
    let blob = put(&store, upload_id, &source).unwrap();
    store.verify(&blob).unwrap();
    assert!(!partial.exists());
    assert_eq!(
        fs::read(dirs.assets().join(blob.storage_key())).unwrap(),
        PDF
    );
}

#[test]
fn source_aliasing_its_staging_path_cannot_be_truncated_or_published() {
    let dirs = TestDirs::new();
    let store = dirs.store();
    let upload_id = Uuid::new_v4();
    let stage_path = dirs.staging().join(upload_id.to_string());
    fs::write(&stage_path, PDF).unwrap();

    assert!(put(&store, upload_id, &stage_path).is_err());
    assert_eq!(fs::read(&stage_path).unwrap(), PDF);
    assert!(!dirs.assets().join("sha256").exists());
}

#[test]
fn poisoned_existing_digest_target_is_rejected_without_overwrite() {
    let dirs = TestDirs::new();
    let store = dirs.store();
    let source = dirs.source("handout.pdf", PDF);
    let target = dirs
        .assets()
        .join("sha256/90/904636248025ad20fb9c6bd8b700179a2a42edb5df3636e926c7e09055ee3f75");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, b"poisoned bytes").unwrap();
    assert!(put(&store, Uuid::new_v4(), &source).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"poisoned bytes");
}

#[test]
fn missing_or_mutated_final_blob_fails_open_and_verify() {
    let dirs = TestDirs::new();
    let store = dirs.store();
    let source = dirs.source("figure.png", PNG);
    let blob = put(&store, Uuid::new_v4(), &source).unwrap();
    let target = dirs.assets().join(blob.storage_key());
    fs::write(&target, b"tampered").unwrap();
    assert!(store.verify(&blob).is_err());
    assert!(store.open(&blob).is_err());
    fs::remove_file(&target).unwrap();
    assert!(store.verify(&blob).is_err());
    assert!(store.open(&blob).is_err());
}

#[test]
fn failure_to_copy_into_assets_never_returns_a_blob() {
    let dirs = TestDirs::new();
    let store = dirs.store();
    let source = dirs.source("cells.ipynb", NOTEBOOK);
    fs::write(dirs.assets().join("sha256"), b"blocks target directory").unwrap();
    assert!(put(&store, Uuid::new_v4(), &source).is_err());
    assert_eq!(
        fs::read(dirs.assets().join("sha256")).unwrap(),
        b"blocks target directory"
    );
}

fn make_old(path: &std::path::Path) {
    let old = SystemTime::now() - Duration::from_secs(7200);
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(old))
        .unwrap();
}

#[test]
fn dry_run_reconcile_only_reports_old_unprotected_digest_and_interrupted_files() {
    let dirs = TestDirs::new();
    let store = dirs.store();
    let referenced = put(&store, Uuid::new_v4(), &dirs.source("referenced.pdf", PDF)).unwrap();
    let backup = put(&store, Uuid::new_v4(), &dirs.source("backup.png", PNG)).unwrap();
    let orphan = put(
        &store,
        Uuid::new_v4(),
        &dirs.source("orphan.ipynb", NOTEBOOK),
    )
    .unwrap();
    let young = put(
        &store,
        Uuid::new_v4(),
        &dirs.source("young.bin", b"young bytes"),
    )
    .unwrap();
    for blob in [&referenced, &backup, &orphan] {
        make_old(&dirs.assets().join(blob.storage_key()));
    }
    fs::create_dir_all(dirs.assets().join(".finalizing")).unwrap();
    let upload = Uuid::new_v4();
    let active_stage = format!("staging/{upload}.incoming-{}", Uuid::new_v4());
    let abandoned_stage = format!("staging/{}.incoming-{}", Uuid::new_v4(), Uuid::new_v4());
    let abandoned_named_stage = format!("staging/{}", Uuid::new_v4());
    let active_pending = format!(".finalizing/{upload}.pending-{}", Uuid::new_v4());
    let abandoned_pending = format!(".finalizing/{}.pending-{}", Uuid::new_v4(), Uuid::new_v4());
    for name in [&active_stage, &abandoned_stage, &abandoned_named_stage] {
        let path = dirs.root.join(name);
        fs::write(&path, b"unfinished stage").unwrap();
        make_old(&path);
    }
    for name in [&active_pending, &abandoned_pending] {
        let path = dirs.assets().join(name);
        fs::write(&path, b"unfinished copy").unwrap();
        make_old(&path);
    }
    let protected: HashSet<String> = [
        referenced.storage_key().to_owned(),
        backup.storage_key().to_owned(),
        upload.to_string(),
    ]
    .into();
    let cutoff = SystemTime::now() - Duration::from_secs(3600);
    let while_upload_active = store.reconcile(&protected, cutoff).unwrap();
    let active_candidates: HashSet<String> = while_upload_active
        .candidates
        .into_iter()
        .map(|candidate| candidate.path)
        .collect();
    assert_eq!(
        active_candidates,
        [
            abandoned_stage.clone(),
            abandoned_named_stage.clone(),
            abandoned_pending.clone(),
        ]
        .into(),
        "an old finalized digest may still belong to an active upload"
    );
    // Exact staging/pending keys also signal an active upload. They must not
    // allow a finalized digest of unknown upload origin to be proposed.
    for active_key in [&active_stage, &active_pending] {
        let exact_protected: HashSet<String> = [
            referenced.storage_key().to_owned(),
            backup.storage_key().to_owned(),
            (*active_key).clone(),
        ]
        .into();
        let exact_report = store.reconcile(&exact_protected, cutoff).unwrap();
        let exact_candidates: HashSet<String> = exact_report
            .candidates
            .into_iter()
            .map(|candidate| candidate.path)
            .collect();
        assert_eq!(exact_candidates, active_candidates);
    }
    // The upload has finished: its incomplete files are gone. An idle run can
    // now report the old orphan digest while referenced/backup keys stay safe.
    fs::remove_file(dirs.root.join(&active_stage)).unwrap();
    fs::remove_file(dirs.assets().join(&active_pending)).unwrap();
    let idle_protected: HashSet<String> = [
        referenced.storage_key().to_owned(),
        backup.storage_key().to_owned(),
    ]
    .into();
    let report = store.reconcile(&idle_protected, cutoff).unwrap();
    let candidates: HashSet<String> = report
        .candidates
        .into_iter()
        .map(|candidate| candidate.path)
        .collect();
    assert_eq!(
        candidates,
        [
            orphan.storage_key().to_owned(),
            abandoned_stage.clone(),
            abandoned_named_stage.clone(),
            abandoned_pending.clone(),
        ]
        .into()
    );
    assert!(!candidates.contains(young.storage_key()));
    // A dry run has no deletion side effect, even for reported candidates.
    assert!(dirs.assets().join(orphan.storage_key()).exists());
    assert!(dirs.root.join(abandoned_stage).exists());
    assert!(dirs.root.join(abandoned_named_stage).exists());
    assert!(dirs.assets().join(abandoned_pending).exists());
    assert!(dirs.assets().join(referenced.storage_key()).exists());
    assert!(dirs.assets().join(backup.storage_key()).exists());
    assert!(!dirs.root.join(active_stage).exists());
    assert!(!dirs.assets().join(active_pending).exists());
}

#[test]
fn reconcile_rejects_an_untrusted_protection_key_instead_of_traversing_outside_roots() {
    let dirs = TestDirs::new();
    let store = dirs.store();
    let protected: HashSet<String> = ["../outside".to_owned()].into();
    assert!(store.reconcile(&protected, SystemTime::now()).is_err());
}

#[test]
fn reconcile_fails_closed_if_a_required_volume_root_disappears() {
    let dirs = TestDirs::new();
    let store = dirs.store();
    let protected = HashSet::new();
    fs::remove_dir(dirs.staging()).unwrap();
    assert!(store.reconcile(&protected, SystemTime::now()).is_err());
    fs::create_dir(dirs.staging()).unwrap();
    fs::remove_dir(dirs.assets()).unwrap();
    assert!(store.reconcile(&protected, SystemTime::now()).is_err());
}

#[test]
fn registered_byte_open_rejects_forged_keys_and_rehashes_the_original() {
    let dirs = TestDirs::new();
    let store = dirs.store();
    let blob = put(&store, Uuid::new_v4(), &dirs.source("handout.pdf", PDF)).unwrap();
    assert!(
        store
            .open_record("../outside", blob.sha256(), PDF.len() as i64)
            .is_err()
    );
    assert!(
        store
            .open_record(blob.storage_key(), "0", PDF.len() as i64)
            .is_err()
    );
    let mut opened = Vec::new();
    store
        .open_record(blob.storage_key(), blob.sha256(), PDF.len() as i64)
        .unwrap()
        .read_to_end(&mut opened)
        .unwrap();
    assert_eq!(opened, PDF);
}

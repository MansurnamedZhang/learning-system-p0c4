use learning_assets::FsAssetStore;
use std::{fs, io::Read, path::PathBuf};
use uuid::Uuid;

const PDF: &[u8] = b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\n%%EOF\n";
const PNG: &[u8] = &[137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 0, 73, 69, 78, 68];
const NOTEBOOK: &[u8] = br#"{"cells":[{"cell_type":"code","source":["print(42)"]}],"nbformat":4}"#;

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
        let blob = store.put_from_file(Uuid::new_v4(), &source).unwrap();
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
    let a = store.put_from_file(Uuid::new_v4(), &first).unwrap();
    let b = store.put_from_file(Uuid::new_v4(), &second).unwrap();
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
    assert!(
        store
            .put_from_file(upload_id, &dirs.root.join("missing.pdf"))
            .is_err()
    );
    assert!(!dirs.assets().join("sha256").exists());

    let source = dirs.source("complete.pdf", PDF);
    let blob = store.put_from_file(upload_id, &source).unwrap();
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

    assert!(store.put_from_file(upload_id, &stage_path).is_err());
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
    assert!(store.put_from_file(Uuid::new_v4(), &source).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"poisoned bytes");
}

#[test]
fn missing_or_mutated_final_blob_fails_open_and_verify() {
    let dirs = TestDirs::new();
    let store = dirs.store();
    let source = dirs.source("figure.png", PNG);
    let blob = store.put_from_file(Uuid::new_v4(), &source).unwrap();
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
    assert!(store.put_from_file(Uuid::new_v4(), &source).is_err());
    assert_eq!(
        fs::read(dirs.assets().join("sha256")).unwrap(),
        b"blocks target directory"
    );
}

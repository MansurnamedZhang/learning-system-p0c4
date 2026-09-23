use learning_assets::{FsAssetStore, UploadDeclaration};
use std::{collections::HashSet, fs, io::Read, path::PathBuf, time::SystemTime};
use uuid::Uuid;

struct TestDirs(PathBuf);

impl TestDirs {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("learning-upload-declaration-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn store(&self) -> FsAssetStore {
        FsAssetStore::new(self.0.join("assets"), self.0.join("staging")).unwrap()
    }

    fn source(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    fn final_objects(&self) -> Vec<PathBuf> {
        let root = self.0.join("assets/sha256");
        if !root.exists() {
            return vec![];
        }
        fs::read_dir(root)
            .unwrap()
            .map(|prefix| prefix.unwrap().path())
            .flat_map(|prefix| {
                fs::read_dir(prefix)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}

impl Drop for TestDirs {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn upload_requires_trusted_declared_size_and_limit_before_publishing() {
    let dirs = TestDirs::new();
    let store = dirs.store();
    let upload = Uuid::new_v4();
    let full = vec![b'x'; 100_001];
    let source = dirs.source("too-long.bin", &full);

    // An impossible declaration is rejected before it creates an upload file.
    assert!(
        store
            .put_from_file(
                upload,
                &source,
                UploadDeclaration {
                    expected_size_bytes: 100_001,
                    max_size_bytes: 100_000,
                },
            )
            .is_err()
    );
    assert_eq!(fs::read_dir(dirs.0.join("staging")).unwrap().count(), 0);
    assert!(dirs.final_objects().is_empty());

    // The declared maximum is enforced while streaming, before writing the
    // chunk that exceeds it. No verified digest object becomes visible.
    assert!(
        store
            .put_from_file(
                upload,
                &source,
                UploadDeclaration {
                    expected_size_bytes: 100_000,
                    max_size_bytes: 100_000,
                },
            )
            .is_err()
    );
    assert!(dirs.final_objects().is_empty());
    let interrupted: Vec<_> = fs::read_dir(dirs.0.join("staging"))
        .unwrap()
        .map(|entry| entry.unwrap())
        .collect();
    assert_eq!(interrupted.len(), 1);
    assert!(
        interrupted[0]
            .file_name()
            .to_string_lossy()
            .starts_with(&format!("{upload}.incoming-"))
    );
    assert!(interrupted[0].metadata().unwrap().len() <= 100_000);
    let report = store
        .reconcile(
            &HashSet::new(),
            SystemTime::now() + std::time::Duration::from_secs(1),
        )
        .unwrap();
    assert!(report.candidates.iter().any(|candidate| {
        candidate
            .path
            .starts_with(&format!("staging/{upload}.incoming-"))
    }));

    // Fewer bytes than declared also never publish. Retrying the same upload
    // ID with exactly the declared limit succeeds and returns original bytes.
    let short = dirs.source("too-short.bin", &full[..99_999]);
    assert!(
        store
            .put_from_file(
                upload,
                &short,
                UploadDeclaration {
                    expected_size_bytes: 100_000,
                    max_size_bytes: 100_000,
                },
            )
            .is_err()
    );
    assert!(dirs.final_objects().is_empty());
    let exact = dirs.source("exact.bin", &full[..100_000]);
    let blob = store
        .put_from_file(
            upload,
            &exact,
            UploadDeclaration {
                expected_size_bytes: 100_000,
                max_size_bytes: 100_000,
            },
        )
        .unwrap();
    assert_eq!(blob.size_bytes(), 100_000);
    assert_eq!(dirs.final_objects().len(), 1);
    let mut read_back = Vec::new();
    store
        .open(&blob)
        .unwrap()
        .read_to_end(&mut read_back)
        .unwrap();
    assert_eq!(read_back, &full[..100_000]);
}

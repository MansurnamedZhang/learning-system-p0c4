use learning_assets::FsAssetStore;
#[cfg(target_os = "linux")]
use learning_assets::UploadDeclaration;
use learning_backup::{
    BackupManifestV1, BackupPlan, FileRecord, MigrationRecord, SourceIdentity, seal_backup,
    verify_sealed,
};
use sha2::{Digest, Sha256};
use std::{fs, fs::File, path::PathBuf};

#[cfg(target_os = "linux")]
fn cleanup(root: PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    fn make_dirs_writable(path: &std::path::Path) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                fs::set_permissions(entry.path(), fs::Permissions::from_mode(0o700)).unwrap();
                make_dirs_writable(&entry.path());
            }
        }
    }
    make_dirs_writable(&root);
    fs::remove_dir_all(root).unwrap();
}
use uuid::Uuid;

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn fixture() -> (
    PathBuf,
    FsAssetStore,
    BackupPlan,
    BackupManifestV1,
    File,
    File,
) {
    let root = std::env::temp_dir().join(format!("c4-sealed-{}", Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let assets = FsAssetStore::new(root.join("asset-root"), root.join("asset-stage")).unwrap();
    let plan = BackupPlan::from_rows(vec![]).unwrap();
    let source = SourceIdentity::from_migrations(
        "a".repeat(64),
        "b".repeat(40),
        18,
        vec![MigrationRecord {
            version: 1,
            checksum_hex: "c".repeat(96),
        }],
    )
    .unwrap();
    let dump = b"database dump";
    let roles = b"{}";
    let manifest = BackupManifestV1::from_plan(
        Uuid::new_v4(),
        source,
        &plan,
        FileRecord {
            path: "database.dump".into(),
            size: dump.len() as u64,
            sha256: digest(dump),
        },
        FileRecord {
            path: "roles.json".into(),
            size: roles.len() as u64,
            sha256: digest(roles),
        },
    )
    .unwrap();
    fs::write(root.join("dump-source"), dump).unwrap();
    fs::write(root.join("roles-source"), roles).unwrap();
    let dump = File::open(root.join("dump-source")).unwrap();
    let roles = File::open(root.join("roles-source")).unwrap();
    (root, assets, plan, manifest, dump, roles)
}

#[cfg(target_os = "linux")]
#[test]
fn sealed_copy_is_read_back_and_never_publishes_complete() {
    let (root, store, plan, manifest, mut dump, mut roles) = fixture();
    let sealed = seal_backup(&root, &manifest, &plan, &store, &mut dump, &mut roles).unwrap();
    assert_eq!(sealed.backup_id(), manifest.backup_id);
    assert_eq!(
        sealed.manifest_sha256(),
        manifest.canonical_sha256().unwrap()
    );
    verify_sealed(&root, manifest.backup_id).unwrap();
    let names: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(
        names
            .iter()
            .any(|n| n.to_string_lossy().ends_with(".sealed"))
    );
    assert!(
        !names
            .iter()
            .any(|n| n.to_string_lossy().ends_with(".complete"))
    );
    cleanup(root);
}

#[cfg(target_os = "linux")]
#[test]
fn corrupt_missing_extra_and_linked_target_files_are_rejected() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    for damage in [
        "corrupt",
        "missing",
        "extra",
        "extra_dir",
        "link",
        "dir_link",
        "hard_link",
    ] {
        let (root, store, plan, manifest, mut dump, mut roles) = fixture();
        seal_backup(&root, &manifest, &plan, &store, &mut dump, &mut roles).unwrap();
        let sealed = root.join(format!("{}.sealed", manifest.backup_id));
        fs::set_permissions(&sealed, fs::Permissions::from_mode(0o700)).unwrap();
        let dump_path = sealed.join("database.dump");
        match damage {
            "corrupt" => {
                fs::set_permissions(&dump_path, fs::Permissions::from_mode(0o600)).unwrap();
                fs::write(&dump_path, b"corrupt").unwrap();
            }
            "missing" => fs::remove_file(&dump_path).unwrap(),
            "extra" => fs::write(sealed.join("extra"), b"extra").unwrap(),
            "extra_dir" => fs::create_dir(sealed.join("extra_dir")).unwrap(),
            "link" => {
                fs::remove_file(&dump_path).unwrap();
                symlink(root.join("dump-source"), &dump_path).unwrap();
            }
            "dir_link" => {
                symlink(&root, sealed.join("linked_dir")).unwrap();
            }
            "hard_link" => {
                fs::remove_file(&dump_path).unwrap();
                fs::hard_link(root.join("dump-source"), &dump_path).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            verify_sealed(&root, manifest.backup_id).is_err(),
            "{damage}"
        );
        cleanup(root);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn failed_second_copy_cannot_replace_an_existing_sealed_directory() {
    let (root, store, plan, manifest, mut dump, mut roles) = fixture();
    seal_backup(&root, &manifest, &plan, &store, &mut dump, &mut roles).unwrap();
    let mut bad_dump = File::open(root.join("roles-source")).unwrap();
    let mut roles_again = File::open(root.join("roles-source")).unwrap();
    assert!(
        seal_backup(
            &root,
            &manifest,
            &plan,
            &store,
            &mut bad_dump,
            &mut roles_again
        )
        .is_err()
    );
    verify_sealed(&root, manifest.backup_id).unwrap();
    cleanup(root);
}

#[cfg(target_os = "linux")]
#[test]
fn duplicate_logical_assets_copy_one_verified_digest_object() {
    use learning_backup::AssetRow;
    let (root, store, _, original, mut dump, mut roles) = fixture();
    fs::write(root.join("original"), b"same original").unwrap();
    let blob = store
        .put_from_file(
            Uuid::new_v4(),
            &root.join("original"),
            UploadDeclaration {
                expected_size_bytes: 13,
                max_size_bytes: 13,
            },
        )
        .unwrap();
    let plan = BackupPlan::from_rows(
        [1, 2]
            .map(|id| AssetRow {
                space_id: Uuid::from_u128(1),
                id: Uuid::from_u128(id),
                sha256: blob.sha256().into(),
                byte_size: 13,
                storage_key: blob.storage_key().into(),
            })
            .to_vec(),
    )
    .unwrap();
    let dump_record = original
        .files
        .iter()
        .find(|f| f.path == "database.dump")
        .unwrap()
        .clone();
    let role_record = original
        .files
        .iter()
        .find(|f| f.path == "roles.json")
        .unwrap()
        .clone();
    let manifest = BackupManifestV1::from_plan(
        original.backup_id,
        original.source.clone(),
        &plan,
        dump_record,
        role_record,
    )
    .unwrap();
    assert_eq!(plan.asset_files().len(), 1);
    seal_backup(&root, &manifest, &plan, &store, &mut dump, &mut roles).unwrap();
    verify_sealed(&root, manifest.backup_id).unwrap();
    let path = root.join(format!(
        "{}.sealed/assets/{}",
        manifest.backup_id,
        blob.storage_key()
    ));
    assert_eq!(fs::read(path).unwrap(), b"same original");
    cleanup(root);
}

#[cfg(target_os = "linux")]
#[test]
fn short_source_leaves_no_sealed_entry() {
    let (root, store, plan, mut manifest, mut dump, mut roles) = fixture();
    manifest
        .files
        .iter_mut()
        .find(|f| f.path == "database.dump")
        .unwrap()
        .size += 1;
    assert!(seal_backup(&root, &manifest, &plan, &store, &mut dump, &mut roles).is_err());
    assert!(verify_sealed(&root, manifest.backup_id).is_err());
    cleanup(root);
}

#[cfg(not(target_os = "linux"))]
#[test]
fn filesystem_sealing_refuses_platforms_without_no_follow_handles() {
    let (root, store, plan, manifest, mut dump, mut roles) = fixture();
    let mut before = fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert!(seal_backup(&root, &manifest, &plan, &store, &mut dump, &mut roles).is_err());
    assert!(verify_sealed(&root, manifest.backup_id).is_err());
    let mut after = fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    before.sort();
    after.sort();
    assert_eq!(before, after);
    fs::remove_dir_all(root).unwrap();
}

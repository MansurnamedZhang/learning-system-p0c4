//! A local, checked copy is only `sealed`. Source consistency and independent
//! fault-domain evidence must be supplied by the management process later.
use crate::{BackupError, BackupManifestV1, BackupPlan};
#[cfg(target_os = "linux")]
use crate::{FileRecord, MAX_ASSET_INDEX_BYTES};
use learning_assets::FsAssetStore;
#[cfg(target_os = "linux")]
use learning_assets::backup_fs::{BackupDir, BackupEntryKind};
#[cfg(target_os = "linux")]
use sha2::{Digest, Sha256};
#[cfg(target_os = "linux")]
use std::{
    collections::BTreeSet,
    io::{Read, Seek, SeekFrom, Write},
};
use std::{fs::File, path::Path};
use uuid::Uuid;

#[cfg(target_os = "linux")]
const MAX_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug)]
pub struct SealedBackup {
    backup_id: Uuid,
    manifest_sha256: String,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, PartialEq, Eq)]
enum FaultPoint {
    AfterTargetChunk,
    BeforeRename,
    BeforeParentSync,
}
impl SealedBackup {
    pub fn backup_id(&self) -> Uuid {
        self.backup_id
    }
    pub fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }
}

/// Copy all declared files into a fresh private directory, read the target
/// back completely, then publish `<id>.sealed` without replacing any entry.
/// A failure before rename cannot publish this attempt; any partial staging
/// directory stays unusable, and an existing sealed directory is unchanged.
/// If parent sync fails after rename, a visible `.sealed` entry is indeterminate:
/// this function returns an error, and a later verifier must sync and recheck it.
pub fn seal_backup(
    destination_root: &Path,
    manifest: &BackupManifestV1,
    plan: &BackupPlan,
    assets: &FsAssetStore,
    dump: &mut File,
    roles: &mut File,
) -> Result<SealedBackup, BackupError> {
    manifest.validate_with_index(plan.asset_index_bytes())?;
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (destination_root, assets, dump, roles);
        Err(BackupError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "secure backup staging requires Linux",
        )))
    }
    #[cfg(target_os = "linux")]
    {
        seal_backup_with_hook(
            destination_root,
            manifest,
            plan,
            assets,
            dump,
            roles,
            |_| Ok(()),
        )
    }
}

#[cfg(target_os = "linux")]
fn seal_backup_with_hook(
    destination_root: &Path,
    manifest: &BackupManifestV1,
    plan: &BackupPlan,
    assets: &FsAssetStore,
    dump: &mut File,
    roles: &mut File,
    hook: impl FnMut(FaultPoint) -> Result<(), BackupError>,
) -> Result<SealedBackup, BackupError> {
    let root = BackupDir::open_private_root(destination_root)?;
    seal_backup_in_with_hook(&root, manifest, plan, assets, dump, roles, hook)
}

#[cfg(target_os = "linux")]
pub(crate) fn seal_backup_in(
    root: &BackupDir,
    manifest: &BackupManifestV1,
    plan: &BackupPlan,
    assets: &FsAssetStore,
    dump: &mut File,
    roles: &mut File,
) -> Result<SealedBackup, BackupError> {
    seal_backup_in_with_hook(root, manifest, plan, assets, dump, roles, |_| Ok(()))
}
#[cfg(target_os = "linux")]
#[allow(clippy::too_many_arguments)]
fn seal_backup_in_with_hook(
    root: &BackupDir,
    manifest: &BackupManifestV1,
    plan: &BackupPlan,
    assets: &FsAssetStore,
    dump: &mut File,
    roles: &mut File,
    mut hook: impl FnMut(FaultPoint) -> Result<(), BackupError>,
) -> Result<SealedBackup, BackupError> {
    manifest.validate_with_index(plan.asset_index_bytes())?;
    let stage_name = format!("{}.staging-{}", manifest.backup_id, Uuid::new_v4());
    let stage = root.create_dir(&stage_name)?;
    let dump_record = required_record(manifest, "database.dump")?;
    let role_record = required_record(manifest, "roles.json")?;
    dump.seek(SeekFrom::Start(0))?;
    roles.seek(SeekFrom::Start(0))?;
    write_source(&stage, dump_record, dump, &mut hook)?;
    write_source(&stage, role_record, roles, &mut hook)?;
    write_source(
        &stage,
        plan.asset_index_file(),
        &mut plan.asset_index_bytes(),
        &mut hook,
    )?;
    for asset in plan.asset_files() {
        let (parent, leaf) = ensure_parent(&stage, &asset.path)?;
        let mut target = parent.create_file(&leaf)?;
        let key = asset
            .path
            .strip_prefix("assets/")
            .ok_or(BackupError::Invalid("asset package path"))?;
        assets.copy_verified(key, &asset.sha256, asset.size, &mut target)?;
        target.sync_all()?;
        check_file(&stage, asset)?;
    }
    let manifest_bytes = manifest.canonical_bytes()?;
    let manifest_record = FileRecord {
        path: "manifest.json".into(),
        size: u64::try_from(manifest_bytes.len()).map_err(|_| BackupError::Overflow)?,
        sha256: manifest.canonical_sha256()?,
    };
    write_source(
        &stage,
        &manifest_record,
        &mut manifest_bytes.as_slice(),
        &mut hook,
    )?;
    verify_directory(&stage, manifest.backup_id)?;
    sync_and_seal_tree(&stage)?;
    hook(FaultPoint::BeforeRename)?;
    root.rename_noreplace_without_sync(&stage_name, &format!("{}.sealed", manifest.backup_id))?;
    hook(FaultPoint::BeforeParentSync)?;
    #[cfg(test)]
    crate::source::lifecycle_tests::hook("sealed_after_rename")?;
    root.sync()?;
    // Reopen the published name after the parent sync, then recheck the bytes.
    verify_sealed_in(root, manifest.backup_id)
}

/// Re-read the manifest, index, every byte, and every directory entry using
/// no-follow handles. This does not make the package restorable.
pub fn verify_sealed(root: &Path, backup_id: Uuid) -> Result<SealedBackup, BackupError> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (root, backup_id);
        Err(BackupError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "secure backup verification requires Linux",
        )))
    }
    #[cfg(target_os = "linux")]
    {
        verify_sealed_with_hook(root, backup_id, |_| Ok(()))
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn verify_sealed_in(root: &BackupDir, id: Uuid) -> Result<SealedBackup, BackupError> {
    root.sync()?;
    let directory = root.open_dir(&format!("{id}.sealed"))?;
    verify_directory(&directory, id)
}

/// Stream a fully verified source pin into a fresh destination staging tree.
/// The destination is independently reopened and hashed before its own
/// `.sealed` name is made durable. This never publishes `.complete` and does
/// not assert that two paths occupy different physical failure domains.
pub fn transfer_sealed_backup(
    source_root: &Path,
    destination_root: &Path,
    backup_id: Uuid,
) -> Result<SealedBackup, BackupError> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (source_root, destination_root, backup_id);
        Err(BackupError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "secure destination transfer requires Linux",
        )))
    }
    #[cfg(target_os = "linux")]
    {
        transfer_sealed_with_hook(source_root, destination_root, backup_id, |_| Ok(()))
    }
}

#[cfg(target_os = "linux")]
fn transfer_sealed_with_hook(
    source_root: &Path,
    destination_root: &Path,
    backup_id: Uuid,
    mut hook: impl FnMut(FaultPoint) -> Result<(), BackupError>,
) -> Result<SealedBackup, BackupError> {
    let source = BackupDir::open_private_root(source_root)?;
    source.sync()?;
    let source_dir = source.open_dir(&format!("{backup_id}.sealed"))?;
    let source_checked = verify_directory(&source_dir, backup_id)?;
    let manifest_bytes = read_small(&source_dir, "manifest.json", MAX_MANIFEST_BYTES)?;
    let manifest: BackupManifestV1 = serde_json::from_slice(&manifest_bytes)?;
    let destination = BackupDir::open_private_root(destination_root)?;
    let stage_name = format!("{backup_id}.transfer-staging-{}", Uuid::new_v4());
    let stage = destination.create_dir(&stage_name)?;
    for record in &manifest.files {
        let mut input = open_file(&source_dir, &record.path)?;
        write_source(&stage, record, &mut input, &mut hook)?;
    }
    let manifest_record = FileRecord {
        path: "manifest.json".into(),
        size: manifest_bytes.len() as u64,
        sha256: source_checked.manifest_sha256.clone(),
    };
    write_source(
        &stage,
        &manifest_record,
        &mut manifest_bytes.as_slice(),
        &mut hook,
    )?;
    let destination_checked = verify_directory(&stage, backup_id)?;
    if destination_checked.manifest_sha256 != source_checked.manifest_sha256 {
        return Err(BackupError::Invalid("destination manifest changed"));
    }
    sync_and_seal_tree(&stage)?;
    hook(FaultPoint::BeforeRename)?;
    destination.rename_noreplace_without_sync(&stage_name, &format!("{backup_id}.sealed"))?;
    hook(FaultPoint::BeforeParentSync)?;
    destination.sync()?;
    verify_sealed(destination_root, backup_id)
}

#[cfg(all(test, target_os = "linux"))]
pub(crate) fn interrupt_test_transfer(
    source: &Path,
    destination: &Path,
    id: Uuid,
) -> Result<SealedBackup, BackupError> {
    transfer_sealed_with_hook(source, destination, id, |point| {
        if point == FaultPoint::AfterTargetChunk {
            Err(BackupError::Invalid(
                "controlled destination transfer interruption",
            ))
        } else {
            Ok(())
        }
    })
}

#[cfg(target_os = "linux")]
fn verify_sealed_with_hook(
    root: &Path,
    backup_id: Uuid,
    mut hook: impl FnMut(FaultPoint) -> Result<(), BackupError>,
) -> Result<SealedBackup, BackupError> {
    let root = BackupDir::open_private_root(root)?;
    hook(FaultPoint::BeforeParentSync)?;
    root.sync()?;
    // Opening by the published name only after the successful sync ensures
    // validation covers the entry that the parent has made durable.
    let stage = root.open_dir(&format!("{backup_id}.sealed"))?;
    verify_directory(&stage, backup_id)
}

#[cfg(target_os = "linux")]
fn required_record<'a>(
    manifest: &'a BackupManifestV1,
    path: &str,
) -> Result<&'a FileRecord, BackupError> {
    manifest
        .files
        .iter()
        .find(|f| f.path == path)
        .ok_or(BackupError::Invalid("missing required file"))
}

#[cfg(target_os = "linux")]
fn write_source(
    stage: &BackupDir,
    record: &FileRecord,
    source: &mut dyn Read,
    hook: &mut impl FnMut(FaultPoint) -> Result<(), BackupError>,
) -> Result<(), BackupError> {
    let (parent, leaf) = ensure_parent(stage, &record.path)?;
    let mut target = parent.create_file(&leaf)?;
    let mut hash = Sha256::new();
    let mut size = 0_u64;
    let mut buf = [0_u8; 64 * 1024];
    loop {
        let n = source.read(&mut buf)?;
        if n == 0 {
            break;
        }
        size = size.checked_add(n as u64).ok_or(BackupError::Overflow)?;
        if size > record.size {
            return Err(BackupError::Invalid("source file size"));
        }
        target.write_all(&buf[..n])?;
        hook(FaultPoint::AfterTargetChunk)?;
        hash.update(&buf[..n]);
    }
    if size != record.size || format!("{:x}", hash.finalize()) != record.sha256 {
        return Err(BackupError::Invalid("source file digest or size"));
    }
    target.sync_all()?;
    check_file(stage, record)
}

#[cfg(target_os = "linux")]
fn ensure_parent(stage: &BackupDir, path: &str) -> Result<(BackupDir, String), BackupError> {
    let parts = path.split('/').collect::<Vec<_>>();
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == "." || *p == ".." || p.contains('\\'))
    {
        return Err(BackupError::Invalid("noncanonical package path"));
    }
    let mut owned = None;
    for part in &parts[..parts.len() - 1] {
        let parent = owned.as_ref().unwrap_or(stage);
        owned = Some(match parent.open_dir(part) {
            Ok(child) => child,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => parent.create_dir(part)?,
            Err(e) => return Err(e.into()),
        });
    }
    let parent = match owned {
        Some(dir) => dir,
        None => stage.try_clone()?,
    };
    Ok((parent, parts[parts.len() - 1].into()))
}

#[cfg(target_os = "linux")]
fn open_file(stage: &BackupDir, path: &str) -> Result<File, BackupError> {
    let parts = path.split('/').collect::<Vec<_>>();
    let mut owned = None;
    for part in &parts[..parts.len() - 1] {
        let parent = owned.as_ref().unwrap_or(stage);
        owned = Some(parent.open_dir(part)?);
    }
    Ok(owned
        .as_ref()
        .unwrap_or(stage)
        .open_file(parts[parts.len() - 1])?)
}

#[cfg(target_os = "linux")]
pub(crate) fn check_file(stage: &BackupDir, record: &FileRecord) -> Result<(), BackupError> {
    let mut file = open_file(stage, &record.path)?;
    if file.metadata()?.len() != record.size {
        return Err(BackupError::Invalid("target file size"));
    }
    let mut size = 0_u64;
    let mut hash = Sha256::new();
    let mut buf = [0_u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        size = size.checked_add(n as u64).ok_or(BackupError::Overflow)?;
        if size > record.size {
            return Err(BackupError::Invalid("target file size"));
        }
        hash.update(&buf[..n]);
    }
    if size != record.size || format!("{:x}", hash.finalize()) != record.sha256 {
        return Err(BackupError::Invalid("target file digest or size"));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub(crate) fn read_small(stage: &BackupDir, path: &str, max: u64) -> Result<Vec<u8>, BackupError> {
    let file = open_file(stage, path)?;
    if file.metadata()?.len() > max {
        return Err(BackupError::Capacity("package metadata bytes"));
    }
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(BackupError::Capacity("package metadata bytes"));
    }
    Ok(bytes)
}

#[cfg(target_os = "linux")]
pub(crate) fn verify_directory(stage: &BackupDir, id: Uuid) -> Result<SealedBackup, BackupError> {
    let manifest_bytes = read_small(stage, "manifest.json", MAX_MANIFEST_BYTES)?;
    let manifest: BackupManifestV1 = serde_json::from_slice(&manifest_bytes)?;
    if manifest.backup_id != id || manifest.canonical_bytes()? != manifest_bytes {
        return Err(BackupError::Invalid("manifest identity or canonical bytes"));
    }
    let index = read_small(stage, "asset-index.json", MAX_ASSET_INDEX_BYTES as u64)?;
    manifest.validate_with_index(&index)?;
    let mut expected_files = BTreeSet::from(["manifest.json".to_string()]);
    let mut expected_dirs = BTreeSet::new();
    for record in &manifest.files {
        expected_files.insert(record.path.clone());
        let mut prefix = String::new();
        let mut parts = record.path.split('/').peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                break;
            }
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            expected_dirs.insert(prefix.clone());
        }
        check_file(stage, record)?;
    }
    let manifest_record = FileRecord {
        path: "manifest.json".into(),
        size: manifest_bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(&manifest_bytes)),
    };
    check_file(stage, &manifest_record)?;
    let mut found_files = BTreeSet::new();
    let mut found_dirs = BTreeSet::new();
    collect_tree(
        stage,
        "",
        &expected_files,
        &expected_dirs,
        &mut found_files,
        &mut found_dirs,
    )?;
    if found_files != expected_files || found_dirs != expected_dirs {
        return Err(BackupError::Invalid("extra or missing package entries"));
    }
    Ok(SealedBackup {
        backup_id: id,
        manifest_sha256: manifest_record.sha256,
    })
}

#[cfg(target_os = "linux")]
fn collect_tree(
    dir: &BackupDir,
    prefix: &str,
    expected_files: &BTreeSet<String>,
    expected_dirs: &BTreeSet<String>,
    files: &mut BTreeSet<String>,
    dirs: &mut BTreeSet<String>,
) -> Result<(), BackupError> {
    for name in dir.list()? {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        match dir.kind(&name)? {
            BackupEntryKind::Directory => {
                if !expected_dirs.contains(&path) {
                    return Err(BackupError::Invalid("extra package directory"));
                }
                dirs.insert(path.clone());
                collect_tree(
                    &dir.open_dir(&name)?,
                    &path,
                    expected_files,
                    expected_dirs,
                    files,
                    dirs,
                )?;
            }
            BackupEntryKind::File => {
                if !expected_files.contains(&path) {
                    return Err(BackupError::Invalid("extra package file"));
                }
                files.insert(path);
            }
            BackupEntryKind::Other => {
                return Err(BackupError::Invalid("linked or special package entry"));
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn sync_and_seal_tree(dir: &BackupDir) -> Result<(), BackupError> {
    for name in dir.list()? {
        match dir.kind(&name)? {
            BackupEntryKind::Directory => sync_and_seal_tree(&dir.open_dir(&name)?)?,
            BackupEntryKind::File => dir.seal_file(&name)?,
            BackupEntryKind::Other => {
                return Err(BackupError::Invalid("linked or special package entry"));
            }
        }
    }
    dir.sync()?;
    dir.seal_dir()?;
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod fault_tests {
    use super::*;
    use crate::{MigrationRecord, SourceIdentity};
    use std::os::unix::fs::PermissionsExt;
    use std::{fs, io, process::Command, thread, time::Duration};

    const ID: Uuid = Uuid::from_u128(42);

    fn dump_bytes() -> Vec<u8> {
        vec![b'x'; 128 * 1024 + 13]
    }

    fn root() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("c4-seal-fault-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(root.join("dump-source"), dump_bytes()).unwrap();
        fs::write(root.join("roles-source"), b"{}").unwrap();
        root
    }

    fn clean(root: &Path) {
        fn writable_dirs(path: &Path) {
            for entry in fs::read_dir(path).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    fs::set_permissions(entry.path(), fs::Permissions::from_mode(0o700)).unwrap();
                    writable_dirs(&entry.path());
                }
            }
        }
        writable_dirs(root);
        fs::remove_dir_all(root).unwrap();
    }

    fn invoke(
        root: &Path,
        hook: impl FnMut(FaultPoint) -> Result<(), BackupError>,
    ) -> Result<SealedBackup, BackupError> {
        let dump_bytes = dump_bytes();
        let plan = BackupPlan::from_rows(vec![])?;
        let source = SourceIdentity::from_migrations(
            "a".repeat(64),
            "b".repeat(40),
            18,
            vec![MigrationRecord {
                version: 1,
                checksum_hex: "c".repeat(96),
            }],
        )?;
        let manifest = BackupManifestV1::from_plan(
            ID,
            source,
            &plan,
            FileRecord {
                path: "database.dump".into(),
                size: dump_bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(&dump_bytes)),
            },
            FileRecord {
                path: "roles.json".into(),
                size: 2,
                sha256: format!("{:x}", Sha256::digest(b"{}")),
            },
        )?;
        let assets = FsAssetStore::new(root.join("asset-root"), root.join("asset-stage"))?;
        let mut dump = File::open(root.join("dump-source"))?;
        let mut roles = File::open(root.join("roles-source"))?;
        seal_backup_with_hook(root, &manifest, &plan, &assets, &mut dump, &mut roles, hook)
    }

    #[test]
    fn injected_partial_write_leaves_only_unusable_staging() {
        let root = root();
        let result = invoke(&root, |point| {
            if point == FaultPoint::AfterTargetChunk {
                Err(BackupError::Io(io::Error::other("injected short write")))
            } else {
                Ok(())
            }
        });
        assert!(result.is_err());
        let names = fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        let stage = names
            .iter()
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .contains(".staging-")
            })
            .unwrap();
        let staged_len = fs::metadata(stage.join("database.dump")).unwrap().len();
        assert!(staged_len > 0 && staged_len < dump_bytes().len() as u64);
        assert!(!root.join(format!("{ID}.sealed")).exists());
        assert!(!root.join(format!("{ID}.complete")).exists());
        clean(&root);
    }

    #[test]
    fn interrupted_destination_stream_cannot_publish_sealed_or_complete() {
        let root = root();
        invoke(&root, |_| Ok(())).unwrap();
        let destination = root.join("destination");
        fs::create_dir(&destination).unwrap();
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o700)).unwrap();
        let failed = transfer_sealed_with_hook(&root, &destination, ID, |point| {
            if point == FaultPoint::AfterTargetChunk {
                Err(BackupError::Io(io::Error::other(
                    "injected transfer interruption",
                )))
            } else {
                Ok(())
            }
        });
        assert!(failed.is_err());
        assert!(!destination.join(format!("{ID}.sealed")).exists());
        assert!(!destination.join(format!("{ID}.complete")).exists());
        let staged = fs::read_dir(&destination).unwrap().count();
        assert_eq!(staged, 1);
        clean(&root);
    }

    #[test]
    fn post_rename_parent_sync_failure_returns_error_and_reverify_must_sync_again() {
        let root = root();
        let result = invoke(&root, |point| {
            if point == FaultPoint::BeforeParentSync {
                Err(BackupError::Io(io::Error::other(
                    "injected parent fsync failure",
                )))
            } else {
                Ok(())
            }
        });
        assert!(result.is_err());
        assert!(root.join(format!("{ID}.sealed")).is_dir());
        assert!(!root.join(format!("{ID}.complete")).exists());
        let refused = verify_sealed_with_hook(&root, ID, |point| {
            if point == FaultPoint::BeforeParentSync {
                Err(BackupError::Io(io::Error::other(
                    "retry parent fsync failure",
                )))
            } else {
                Ok(())
            }
        });
        assert!(refused.is_err());
        assert_eq!(verify_sealed(&root, ID).unwrap().backup_id(), ID);
        clean(&root);
    }

    #[test]
    fn sigkill_child() {
        let Ok(root) = std::env::var("C4_SIGKILL_STAGE_ROOT") else {
            return;
        };
        let root = std::path::PathBuf::from(root);
        let _ = invoke(&root, |point| {
            if point == FaultPoint::BeforeRename {
                fs::write(root.join("copy-finished-marker"), b"ready").unwrap();
                loop {
                    thread::sleep(Duration::from_secs(1));
                }
            }
            Ok(())
        });
    }

    #[test]
    fn sigkill_before_rename_cannot_publish_sealed_or_complete() {
        let root = root();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "sealed::fault_tests::sigkill_child"])
            .env("C4_SIGKILL_STAGE_ROOT", &root)
            .spawn()
            .unwrap();
        let mut marker_seen = false;
        for _ in 0..200 {
            if root.join("copy-finished-marker").exists() {
                marker_seen = true;
                break;
            }
            assert!(
                child.try_wait().unwrap().is_none(),
                "child exited before stage barrier"
            );
            thread::sleep(Duration::from_millis(50));
        }
        child.kill().unwrap();
        assert!(!child.wait().unwrap().success());
        assert!(marker_seen, "child did not reach the pre-rename barrier");
        assert!(!root.join(format!("{ID}.sealed")).exists());
        assert!(!root.join(format!("{ID}.complete")).exists());
        clean(&root);
    }
}

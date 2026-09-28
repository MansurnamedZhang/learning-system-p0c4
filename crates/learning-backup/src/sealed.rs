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
/// A failed attempt leaves only an unusable random staging directory.
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
        let root = BackupDir::open_private_root(destination_root)?;
        let stage_name = format!("{}.staging-{}", manifest.backup_id, Uuid::new_v4());
        let stage = root.create_dir(&stage_name)?;
        let dump_record = required_record(manifest, "database.dump")?;
        let role_record = required_record(manifest, "roles.json")?;
        dump.seek(SeekFrom::Start(0))?;
        roles.seek(SeekFrom::Start(0))?;
        write_source(&stage, dump_record, dump)?;
        write_source(&stage, role_record, roles)?;
        write_source(
            &stage,
            plan.asset_index_file(),
            &mut plan.asset_index_bytes(),
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
        write_source(&stage, &manifest_record, &mut manifest_bytes.as_slice())?;
        let checked = verify_directory(&stage, manifest.backup_id)?;
        sync_and_seal_tree(&stage)?;
        root.rename_noreplace(&stage_name, &format!("{}.sealed", manifest.backup_id))?;
        Ok(checked)
    }
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
        let root = BackupDir::open_private_root(root)?;
        let stage = root.open_dir(&format!("{backup_id}.sealed"))?;
        verify_directory(&stage, backup_id)
    }
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
fn check_file(stage: &BackupDir, record: &FileRecord) -> Result<(), BackupError> {
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
fn read_small(stage: &BackupDir, path: &str, max: u64) -> Result<Vec<u8>, BackupError> {
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
fn verify_directory(stage: &BackupDir, id: Uuid) -> Result<SealedBackup, BackupError> {
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

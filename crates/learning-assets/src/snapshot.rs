//! Private, ordinary-directory packages. Incoming bytes are accepted only as
//! already-open streams; a caller-controlled directory path is never traversed.
use crate::{
    AssetIoError, FsAssetStore,
    secure_dir::{Dir, EntryKind, sealed_delivery},
};
use learning_core::{
    BodyV2, BodyV3, ContentDraft, ContentError, CopyItem, ExactSnapshotManifest, ReadingCopy,
    ReadingCopyManifest, ReadingItemData, ReadingMode, ReviewProjection, SNAPSHOT_FORMAT_VERSION,
    SNAPSHOT_MAX_ASSET_BYTES, SNAPSHOT_MAX_ASSET_FILE_BYTES, SNAPSHOT_MAX_FILES,
    SNAPSHOT_MAX_JSON_BYTES, SNAPSHOT_MAX_JSON_FILE_BYTES, SnapshotAssetUse, SnapshotFile,
    SnapshotManifest, SnapshotRow, VersionedReadingProjection, canonical_json, hex_digest,
    parse_snapshot_object_path, snapshot_object_path,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{self, Read, Write},
    path::Path,
};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum SnapshotIoError {
    #[error("invalid snapshot package")]
    InvalidPackage,
    #[error("snapshot asset missing")]
    MissingAsset,
    #[error("snapshot asset corrupt")]
    CorruptAsset,
    #[error("snapshot package limit exceeded")]
    LimitExceeded,
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[derive(Debug)]
pub struct SnapshotDirectory {
    dir: Dir,
    manifest_sha256: String,
}

#[derive(Debug)]
pub struct VerifiedSnapshot {
    pub manifest: SnapshotManifest,
    pub rows: Vec<SnapshotRow>,
    pub files: Vec<SnapshotFile>,
}

impl SnapshotDirectory {
    pub fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }

    /// Open only names recorded and freshly verified against the manifest.
    pub fn open_verified_files(&self) -> Result<Vec<(String, File)>, SnapshotIoError> {
        let verified = verify_snapshot(self)?;
        let mut names = verified
            .files
            .into_iter()
            .map(|f| f.path)
            .collect::<Vec<_>>();
        names.push("manifest.json".into());
        names.sort();
        names
            .into_iter()
            .map(|name| {
                let file = open_staged_file(&self.dir, &name)?;
                // Recheck the opened handle after verification; no named-path reopen
                // can silently swap bytes between validation and delivery.
                let expected = if name == "manifest.json" {
                    (self.manifest_sha256.as_str(), None)
                } else {
                    let entry = manifest_files(&verified.manifest)
                        .iter()
                        .find(|f| f.path == name)
                        .ok_or(SnapshotIoError::InvalidPackage)?;
                    (entry.sha256.as_str(), Some(entry.size))
                };
                let (file, hash, size) = sealed_delivery(file, size_limit(&name)?)?;
                if hash != expected.0 || expected.1.is_some_and(|n| n != size) {
                    return Err(SnapshotIoError::InvalidPackage);
                }
                Ok((name, file))
            })
            .collect()
    }
}

fn manifest_files(manifest: &SnapshotManifest) -> &[SnapshotFile] {
    match manifest {
        SnapshotManifest::ExactImportV1(value) => &value.files,
        SnapshotManifest::ReadingCopyV1(value) => &value.files,
    }
}

fn map_asset(error: AssetIoError) -> SnapshotIoError {
    match error {
        AssetIoError::Missing(_) => SnapshotIoError::MissingAsset,
        AssetIoError::Corrupt(_)
        | AssetIoError::DeclaredSizeMismatch
        | AssetIoError::InvalidMetadata => SnapshotIoError::CorruptAsset,
        AssetIoError::UploadLimitExceeded | AssetIoError::SizeOverflow => {
            SnapshotIoError::LimitExceeded
        }
        AssetIoError::Io(error) => SnapshotIoError::Io(error),
        _ => SnapshotIoError::InvalidPackage,
    }
}

pub fn stage_snapshot(
    root: &Path,
    job_id: Uuid,
    manifest: &ExactSnapshotManifest,
    rows: &[SnapshotRow],
    assets: &[SnapshotAssetUse],
    files: &FsAssetStore,
) -> Result<SnapshotDirectory, SnapshotIoError> {
    if manifest.format_version != SNAPSHOT_FORMAT_VERSION {
        return Err(SnapshotIoError::InvalidPackage);
    }
    let mut writer = StageWriter::new(root, job_id)?;
    let result = (|| {
        let mut declared_assets = BTreeMap::new();
        for asset in assets {
            if asset.sha256.len() != 64
                || !asset
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                || asset.storage_key != format!("sha256/{}/{}", &asset.sha256[..2], asset.sha256)
                || asset.byte_size > SNAPSHOT_MAX_ASSET_FILE_BYTES as u64
            {
                return Err(SnapshotIoError::InvalidPackage);
            }
            if let Some(previous) =
                declared_assets.insert(&asset.storage_key, (&asset.sha256, asset.byte_size))
                && previous != (&asset.sha256, asset.byte_size)
            {
                return Err(SnapshotIoError::InvalidPackage);
            }
        }
        let mut seen = BTreeSet::new();
        for row in rows {
            let name = snapshot_object_path(row.table, &row.identity)
                .map_err(|_| SnapshotIoError::InvalidPackage)?;
            if !seen.insert(name.clone())
                || row.sha256 != learning_core::canonical_record_hash(&row.immutable_values)
            {
                return Err(SnapshotIoError::InvalidPackage);
            }
            let bytes = canonical_json(
                &serde_json::to_value(row).map_err(|_| SnapshotIoError::InvalidPackage)?,
            );
            writer.write_stream(&name, &mut bytes.as_bytes())?;
        }
        if !manifest.requires_destination_assets {
            let mut copied = BTreeSet::new();
            for asset in assets {
                let name = format!("assets/{}", asset.storage_key);
                if !copied.insert(name.clone()) {
                    continue;
                }
                writer.write_asset(&name, asset, files)?;
            }
        }
        writer.write_stream(
            "validation.json",
            &mut br#"{"status":"validated"}"#.as_slice(),
        )?;
        let mut final_manifest = manifest.clone();
        let planned = final_manifest
            .files
            .iter()
            .filter(|f| f.path != "validation.json")
            .cloned()
            .collect::<Vec<_>>();
        let mut actual = writer
            .files
            .iter()
            .filter(|f| f.path != "validation.json")
            .cloned()
            .collect::<Vec<_>>();
        actual.sort_by(|a, b| a.path.cmp(&b.path));
        if planned != actual {
            return Err(SnapshotIoError::InvalidPackage);
        }
        writer.files.sort_by(|a, b| a.path.cmp(&b.path));
        final_manifest.files = writer.files.clone();
        writer.finish(&SnapshotManifest::ExactImportV1(final_manifest))
    })();
    if result.is_err() {
        writer.discard();
    }
    result
}

pub fn stage_reading_copy(
    root: &Path,
    copy_id: Uuid,
    manifest: &ReadingCopyManifest,
    copy: &ReadingCopy,
) -> Result<SnapshotDirectory, SnapshotIoError> {
    if manifest.format_version != SNAPSHOT_FORMAT_VERSION
        || manifest.copy_id != copy_id
        || !manifest.files.is_empty()
    {
        return Err(SnapshotIoError::InvalidPackage);
    }
    let mut writer = StageWriter::new(root, copy_id)?;
    let result = (|| {
        let markdown = render_markdown(copy);
        let html = render_html(copy);
        writer.write_stream("reading.md", &mut markdown.as_bytes())?;
        writer.write_stream("reading.html", &mut html.as_bytes())?;
        writer.write_stream(
            "validation.json",
            &mut br#"{"status":"validated"}"#.as_slice(),
        )?;
        writer.files.sort_by(|a, b| a.path.cmp(&b.path));
        let final_manifest = ReadingCopyManifest {
            format_version: SNAPSHOT_FORMAT_VERSION,
            copy_id: manifest.copy_id,
            files: writer.files.clone(),
        };
        writer.finish(&SnapshotManifest::ReadingCopyV1(final_manifest))
    })();
    if result.is_err() {
        writer.discard();
    }
    result
}

/// The caller supplies opened streams, never names to open on the source disk.
pub fn stage_incoming(
    root: &Path,
    files: impl Iterator<Item = (String, Box<dyn Read>)>,
) -> Result<SnapshotDirectory, SnapshotIoError> {
    let mut writer = StageWriter::new(root, Uuid::new_v4())?;
    let result = (|| {
        let mut manifest_bytes = None;
        for (name, mut source) in files {
            if name == "manifest.json" {
                if manifest_bytes.is_some() {
                    return Err(SnapshotIoError::InvalidPackage);
                }
                let bytes = bounded_read(&mut source, SNAPSHOT_MAX_JSON_FILE_BYTES)?;
                manifest_bytes = Some(bytes.clone());
                writer.write_stream(&name, &mut bytes.as_slice())?;
            } else {
                writer.write_stream(&name, &mut source)?;
            }
        }
        let bytes = manifest_bytes.ok_or(SnapshotIoError::InvalidPackage)?;
        let manifest: SnapshotManifest =
            serde_json::from_slice(&bytes).map_err(|_| SnapshotIoError::InvalidPackage)?;
        if canonical_json(
            &serde_json::to_value(&manifest).map_err(|_| SnapshotIoError::InvalidPackage)?,
        )
        .as_bytes()
            != bytes
        {
            return Err(SnapshotIoError::InvalidPackage);
        }
        writer.finish_existing(&manifest, hex_digest(&bytes))
    })();
    if result.is_err() {
        writer.discard();
    }
    result
}

pub fn verify_snapshot(stage: &SnapshotDirectory) -> Result<VerifiedSnapshot, SnapshotIoError> {
    let manifest_bytes = read_staged(&stage.dir, "manifest.json", SNAPSHOT_MAX_JSON_FILE_BYTES)?;
    if hex_digest(&manifest_bytes) != stage.manifest_sha256 {
        return Err(SnapshotIoError::InvalidPackage);
    }
    let manifest: SnapshotManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|_| SnapshotIoError::InvalidPackage)?;
    if canonical_json(
        &serde_json::to_value(&manifest).map_err(|_| SnapshotIoError::InvalidPackage)?,
    )
    .as_bytes()
        != manifest_bytes
    {
        return Err(SnapshotIoError::InvalidPackage);
    }
    let entries = manifest_files(&manifest);
    if !entries.iter().any(|f| f.path == "validation.json") {
        return Err(SnapshotIoError::InvalidPackage);
    }
    let expected = entries
        .iter()
        .map(|f| f.path.clone())
        .chain(std::iter::once("manifest.json".into()))
        .collect::<BTreeSet<_>>();
    let expected_dirs = expected
        .iter()
        .flat_map(|name| {
            let parts = name.split('/').collect::<Vec<_>>();
            (1..parts.len()).map(move |n| parts[..n].join("/"))
        })
        .collect::<BTreeSet<_>>();
    let mut found = BTreeSet::new();
    let mut found_dirs = BTreeSet::new();
    collect_names(&stage.dir, "", &expected_dirs, &mut found_dirs, &mut found)?;
    if found_dirs != expected_dirs {
        return Err(SnapshotIoError::InvalidPackage);
    }
    if expected
        .iter()
        .any(|name| name.starts_with("assets/") && !found.contains(name))
    {
        return Err(SnapshotIoError::MissingAsset);
    }
    if found != expected {
        return Err(SnapshotIoError::InvalidPackage);
    }
    let mut rows = Vec::new();
    let mut json_total = manifest_bytes.len();
    let mut asset_total = 0usize;
    for entry in entries {
        let limit = size_limit(&entry.path)?;
        if entry.path.starts_with("assets/") {
            if entry.path != format!("assets/sha256/{}/{}", &entry.sha256[..2], entry.sha256) {
                return Err(SnapshotIoError::InvalidPackage);
            }
            let mut file = open_staged_file(&stage.dir, &entry.path)?;
            let (hash, size) = hash_reader(&mut file, limit)?;
            if entry.size != size || entry.sha256 != hash {
                return Err(SnapshotIoError::CorruptAsset);
            }
            asset_total = asset_total
                .checked_add(size as usize)
                .ok_or(SnapshotIoError::LimitExceeded)?;
        } else {
            let bytes = read_staged(&stage.dir, &entry.path, limit)?;
            if entry.size != bytes.len() as u64 || entry.sha256 != hex_digest(&bytes) {
                return Err(SnapshotIoError::InvalidPackage);
            }
            json_total = json_total
                .checked_add(bytes.len())
                .ok_or(SnapshotIoError::LimitExceeded)?;
            if entry.path.starts_with("objects/") {
                let row: SnapshotRow =
                    serde_json::from_slice(&bytes).map_err(|_| SnapshotIoError::InvalidPackage)?;
                if snapshot_object_path(row.table, &row.identity)
                    .map_err(|_| SnapshotIoError::InvalidPackage)?
                    != entry.path
                {
                    return Err(SnapshotIoError::InvalidPackage);
                }
                if canonical_json(
                    &serde_json::to_value(&row).map_err(|_| SnapshotIoError::InvalidPackage)?,
                )
                .as_bytes()
                    != bytes
                {
                    return Err(SnapshotIoError::InvalidPackage);
                }
                rows.push(row);
            } else if entry.path == "validation.json" {
                if bytes != br#"{"status":"validated"}"# {
                    return Err(SnapshotIoError::InvalidPackage);
                }
            } else if std::str::from_utf8(&bytes).is_err() {
                return Err(SnapshotIoError::InvalidPackage);
            }
        }
    }
    if entries.len() + 1 > SNAPSHOT_MAX_FILES
        || json_total > SNAPSHOT_MAX_JSON_BYTES
        || asset_total > SNAPSHOT_MAX_ASSET_BYTES
    {
        return Err(SnapshotIoError::LimitExceeded);
    }
    if matches!(manifest, SnapshotManifest::ReadingCopyV1(_)) && !rows.is_empty() {
        return Err(SnapshotIoError::InvalidPackage);
    }
    let files = entries.to_vec();
    Ok(VerifiedSnapshot {
        manifest,
        rows,
        files,
    })
}

struct StageWriter {
    root: Dir,
    partial: Dir,
    partial_name: String,
    ready_name: String,
    files: Vec<SnapshotFile>,
    json_bytes: usize,
    asset_bytes: usize,
    names: BTreeSet<String>,
}
impl StageWriter {
    fn new(root: &Path, id: Uuid) -> Result<Self, SnapshotIoError> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (root, id);
            Err(SnapshotIoError::InvalidPackage)
        }
        #[cfg(target_os = "linux")]
        {
            let root = Dir::open_private_root(root).map_err(|_| SnapshotIoError::InvalidPackage)?;
            let partial_name = format!("{id}.partial-{}", Uuid::new_v4());
            let partial = root.create_dir(&partial_name)?;
            Ok(Self {
                root,
                partial,
                partial_name,
                ready_name: format!("{id}.ready"),
                files: vec![],
                json_bytes: 0,
                asset_bytes: 0,
                names: BTreeSet::new(),
            })
        }
    }
    fn write_stream(&mut self, name: &str, source: &mut dyn Read) -> Result<(), SnapshotIoError> {
        self.begin(name)?;
        let limit = size_limit(name)?;
        let (parent, leaf) = self.file_parent(name)?;
        let mut target = parent.create_file(&leaf)?;
        let (sha256, size) = copy_hash_bounded(source, &mut target, limit)?;
        target.sync_all()?;
        self.record(name, size, sha256)?;
        Ok(())
    }
    fn write_asset(
        &mut self,
        name: &str,
        asset: &SnapshotAssetUse,
        store: &FsAssetStore,
    ) -> Result<(), SnapshotIoError> {
        self.begin(name)?;
        if asset.byte_size > SNAPSHOT_MAX_ASSET_FILE_BYTES as u64
            || name != format!("assets/sha256/{}/{}", &asset.sha256[..2], asset.sha256)
        {
            return Err(SnapshotIoError::InvalidPackage);
        }
        let (parent, leaf) = self.file_parent(name)?;
        let mut target = parent.create_file(&leaf)?;
        store
            .copy_verified(
                &asset.storage_key,
                &asset.sha256,
                asset.byte_size,
                &mut target,
            )
            .map_err(map_asset)?;
        target.sync_all()?;
        self.record(name, asset.byte_size, asset.sha256.clone())
    }
    fn begin(&mut self, name: &str) -> Result<(), SnapshotIoError> {
        valid_name(name)?;
        if !self.names.insert(name.to_ascii_lowercase()) {
            return Err(SnapshotIoError::InvalidPackage);
        }
        if self.names.len() > SNAPSHOT_MAX_FILES {
            return Err(SnapshotIoError::LimitExceeded);
        }
        Ok(())
    }
    fn file_parent(&self, name: &str) -> Result<(Dir, String), SnapshotIoError> {
        let parts = name.split('/').collect::<Vec<_>>();
        let mut dir = self.partial.try_clone()?;
        for part in &parts[..parts.len() - 1] {
            dir = match dir.open_dir(part) {
                Ok(child) => child,
                Err(error) if error.kind() == io::ErrorKind::NotFound => dir.create_dir(part)?,
                Err(error) => return Err(error.into()),
            };
        }
        Ok((dir, parts[parts.len() - 1].into()))
    }
    fn record(&mut self, name: &str, size: u64, sha256: String) -> Result<(), SnapshotIoError> {
        let length = usize::try_from(size).map_err(|_| SnapshotIoError::LimitExceeded)?;
        if name.starts_with("assets/") {
            self.asset_bytes = self
                .asset_bytes
                .checked_add(length)
                .ok_or(SnapshotIoError::LimitExceeded)?;
            if self.asset_bytes > SNAPSHOT_MAX_ASSET_BYTES {
                return Err(SnapshotIoError::LimitExceeded);
            }
        } else {
            self.json_bytes = self
                .json_bytes
                .checked_add(length)
                .ok_or(SnapshotIoError::LimitExceeded)?;
            if self.json_bytes > SNAPSHOT_MAX_JSON_BYTES {
                return Err(SnapshotIoError::LimitExceeded);
            }
        }
        if name != "manifest.json" {
            self.files.push(SnapshotFile {
                path: name.into(),
                size,
                sha256,
            });
        }
        Ok(())
    }
    fn finish(
        &mut self,
        manifest: &SnapshotManifest,
    ) -> Result<SnapshotDirectory, SnapshotIoError> {
        self.files.sort_by(|a, b| a.path.cmp(&b.path));
        if manifest_files(manifest) != self.files {
            return Err(SnapshotIoError::InvalidPackage);
        }
        let bytes = canonical_json(
            &serde_json::to_value(manifest).map_err(|_| SnapshotIoError::InvalidPackage)?,
        );
        let _: SnapshotManifest =
            serde_json::from_str(&bytes).map_err(|_| SnapshotIoError::InvalidPackage)?;
        self.write_stream("manifest.json", &mut bytes.as_bytes())?;
        self.finish_existing(manifest, hex_digest(bytes.as_bytes()))
    }
    fn finish_existing(
        &mut self,
        manifest: &SnapshotManifest,
        hash: String,
    ) -> Result<SnapshotDirectory, SnapshotIoError> {
        self.files.sort_by(|a, b| a.path.cmp(&b.path));
        if manifest_files(manifest) != self.files || !self.names.contains("manifest.json") {
            return Err(SnapshotIoError::InvalidPackage);
        }
        sync_tree(&self.partial)?;
        seal_tree(&self.partial)?;
        let stage = SnapshotDirectory {
            dir: self.partial.try_clone()?,
            manifest_sha256: hash.clone(),
        };
        verify_snapshot(&stage)?;
        self.root.rename(&self.partial_name, &self.ready_name)?;
        Ok(SnapshotDirectory {
            dir: self.partial.try_clone()?,
            manifest_sha256: hash,
        })
    }
    fn discard(&self) {
        let _ = self.root.remove_tree(&self.partial_name);
    }
}

fn valid_name(name: &str) -> Result<(), SnapshotIoError> {
    if matches!(
        name,
        "manifest.json" | "validation.json" | "reading.html" | "reading.md"
    ) {
        return Ok(());
    }
    if name.starts_with("objects/") && parse_snapshot_object_path(name).is_ok() {
        return Ok(());
    }
    let parts = name.split('/').collect::<Vec<_>>();
    if parts.len() == 4
        && parts[0] == "assets"
        && parts[1] == "sha256"
        && parts[3].len() == 64
        && parts[3]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && parts[2] == &parts[3][..2]
    {
        return Ok(());
    }
    Err(SnapshotIoError::InvalidPackage)
}
fn size_limit(name: &str) -> Result<usize, SnapshotIoError> {
    valid_name(name)?;
    Ok(if name.starts_with("assets/") {
        SNAPSHOT_MAX_ASSET_FILE_BYTES
    } else {
        SNAPSHOT_MAX_JSON_FILE_BYTES
    })
}
fn copy_hash_bounded(
    source: &mut dyn Read,
    target: &mut File,
    limit: usize,
) -> Result<(String, u64), SnapshotIoError> {
    let mut hash = Sha256::new();
    let mut size = 0usize;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = source.read(&mut buf)?;
        if n == 0 {
            break;
        }
        size = size.checked_add(n).ok_or(SnapshotIoError::LimitExceeded)?;
        if size > limit {
            return Err(SnapshotIoError::LimitExceeded);
        }
        target.write_all(&buf[..n])?;
        hash.update(&buf[..n]);
    }
    Ok((format!("{:x}", hash.finalize()), size as u64))
}
fn bounded_read(source: &mut dyn Read, limit: usize) -> Result<Vec<u8>, SnapshotIoError> {
    let mut out = Vec::new();
    source.take((limit as u64) + 1).read_to_end(&mut out)?;
    if out.len() > limit {
        return Err(SnapshotIoError::LimitExceeded);
    }
    Ok(out)
}
fn hash_reader(source: &mut File, limit: usize) -> Result<(String, u64), SnapshotIoError> {
    let mut hash = Sha256::new();
    let mut size = 0usize;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = source.read(&mut buf)?;
        if n == 0 {
            break;
        }
        size = size.checked_add(n).ok_or(SnapshotIoError::LimitExceeded)?;
        if size > limit {
            return Err(SnapshotIoError::LimitExceeded);
        }
        hash.update(&buf[..n]);
    }
    Ok((format!("{:x}", hash.finalize()), size as u64))
}
fn read_staged(root: &Dir, name: &str, limit: usize) -> Result<Vec<u8>, SnapshotIoError> {
    let mut file = open_staged_file(root, name)?;
    bounded_read(&mut file, limit)
}
fn open_staged_file(root: &Dir, name: &str) -> Result<File, SnapshotIoError> {
    valid_name(name)?;
    let parts = name.split('/').collect::<Vec<_>>();
    let mut dir = root.try_clone()?;
    for part in &parts[..parts.len() - 1] {
        dir = dir
            .open_dir(part)
            .map_err(|_| SnapshotIoError::InvalidPackage)?;
    }
    dir.open_file(parts[parts.len() - 1])
        .map_err(|_| SnapshotIoError::InvalidPackage)
}
fn sync_tree(dir: &Dir) -> Result<(), SnapshotIoError> {
    for name in dir.list()? {
        match dir.kind(&name)? {
            EntryKind::Directory => sync_tree(&dir.open_dir(&name)?)?,
            EntryKind::File => dir.open_file(&name)?.sync_all()?,
            EntryKind::Other => return Err(SnapshotIoError::InvalidPackage),
        }
    }
    dir.sync()?;
    Ok(())
}
fn seal_tree(dir: &Dir) -> Result<(), SnapshotIoError> {
    for name in dir.list()? {
        match dir.kind(&name)? {
            EntryKind::Directory => seal_tree(&dir.open_dir(&name)?)?,
            EntryKind::File => dir.seal_file(&name)?,
            EntryKind::Other => return Err(SnapshotIoError::InvalidPackage),
        }
    }
    dir.chmod(0o500)?;
    dir.sync()?;
    Ok(())
}
fn collect_names(
    dir: &Dir,
    prefix: &str,
    expected_dirs: &BTreeSet<String>,
    found_dirs: &mut BTreeSet<String>,
    names: &mut BTreeSet<String>,
) -> Result<(), SnapshotIoError> {
    for leaf in dir.list()? {
        let name = if prefix.is_empty() {
            leaf.clone()
        } else {
            format!("{prefix}/{leaf}")
        };
        match dir.kind(&leaf)? {
            EntryKind::Directory => {
                if !expected_dirs.contains(&name) || !found_dirs.insert(name) {
                    return Err(SnapshotIoError::InvalidPackage);
                }
                collect_names(
                    &dir.open_dir(&leaf)?,
                    &if prefix.is_empty() {
                        leaf.clone()
                    } else {
                        format!("{prefix}/{leaf}")
                    },
                    expected_dirs,
                    found_dirs,
                    names,
                )?;
            }
            EntryKind::File => {
                valid_name(&name)?;
                if !names.insert(name) {
                    return Err(SnapshotIoError::InvalidPackage);
                }
            }
            EntryKind::Other => return Err(SnapshotIoError::InvalidPackage),
        }
    }
    Ok(())
}

pub fn sanitize_reading_copy(
    projection: &VersionedReadingProjection,
    mode: ReadingMode,
    include_personal: bool,
) -> Result<ReadingCopy, ContentError> {
    if projection.contract_version != 1 {
        return Err(ContentError::Invalid("unsupported_content_version".into()));
    }
    let mut items = Vec::new();
    let mut omitted = false;
    for item in &projection.items {
        match item {
            ReadingItemData::SectionStart { title, .. } if mode != ReadingMode::Personal => items
                .push(CopyItem::Heading {
                    title: title.clone(),
                }),
            ReadingItemData::Original { revision, .. } if mode != ReadingMode::Personal => {
                push_revision(revision, &mut items, &mut omitted)
            }
            ReadingItemData::Personal { item }
                if mode != ReadingMode::Original && include_personal =>
            {
                push_revision(&item.revision, &mut items, &mut omitted)
            }
            ReadingItemData::Personal { .. } if mode != ReadingMode::Original => omitted = true,
            _ => {}
        }
    }
    if mode != ReadingMode::Original {
        if include_personal {
            for item in &projection.unplaced {
                push_revision(&item.revision, &mut items, &mut omitted);
            }
        } else if !projection.unplaced.is_empty() {
            omitted = true;
        }
    }
    let mut evidence = Vec::new();
    if include_personal && mode != ReadingMode::Original {
        for selection in &projection.evidence.selections {
            if !selection.relation.rationale.is_empty() {
                evidence.push(selection.relation.rationale.clone());
            }
            if let Some(ReviewProjection::Available(review)) = &selection.review {
                evidence.push(review.explanation.clone());
            }
            if matches!(
                selection.review,
                Some(ReviewProjection::Incomplete | ReviewProjection::Unavailable)
            ) {
                omitted = true;
            }
        }
        for review in &projection.evidence.epistemic_reviews {
            if let ReviewProjection::Available(value) = review {
                evidence.push(value.explanation.clone());
            } else {
                omitted = true;
            }
        }
    } else if mode != ReadingMode::Original
        && (!projection.evidence.selections.is_empty()
            || !projection.evidence.epistemic_reviews.is_empty())
    {
        omitted = true;
    }
    if mode != ReadingMode::Personal
        && matches!(
            projection.source,
            learning_core::SourceProjection::Unavailable
        )
    {
        omitted = true;
    }
    if omitted {
        items.push(CopyItem::Omitted);
    }
    Ok(ReadingCopy { items, evidence })
}
fn push_revision(
    revision: &learning_core::ContentRevision,
    items: &mut Vec<CopyItem>,
    omitted: &mut bool,
) {
    let visible = match &revision.draft {
        ContentDraft::V1(v) => Some((
            format!("{:?}", v.intent).to_ascii_lowercase(),
            v.title.clone(),
            v.payload.text.clone(),
        )),
        ContentDraft::V2(v) => match &v.body {
            BodyV2::Text(p) => Some((
                format!("{:?}", v.intent).to_ascii_lowercase(),
                v.title.clone(),
                p.text.clone(),
            )),
            _ => None,
        },
        ContentDraft::V3(v) => match &v.body {
            BodyV3::Text(p) => Some((
                format!("{:?}", v.intent).to_ascii_lowercase(),
                v.title.clone(),
                p.text.clone(),
            )),
            BodyV3::Figure { caption, alt, .. } => Some((
                format!("{:?}", v.intent).to_ascii_lowercase(),
                v.title.clone(),
                format!("{caption} {alt}"),
            )),
            BodyV3::Attachment { display_name, .. } => Some((
                format!("{:?}", v.intent).to_ascii_lowercase(),
                v.title.clone(),
                display_name.clone(),
            )),
            _ => None,
        },
    };
    if let Some((intent, title, display_text)) = visible {
        items.push(CopyItem::Content {
            intent,
            title,
            display_text,
        });
    } else {
        *omitted = true;
    }
}
fn render_markdown(copy: &ReadingCopy) -> String {
    let mut out = String::new();
    for item in &copy.items {
        match item {
            CopyItem::Heading { title } => {
                out.push_str("## ");
                out.push_str(&escape_markdown(title));
                out.push_str("\n\n");
            }
            CopyItem::Content {
                title,
                display_text,
                ..
            } => {
                if !title.is_empty() {
                    out.push_str("### ");
                    out.push_str(&escape_markdown(title));
                    out.push_str("\n\n");
                }
                out.push_str(&escape_markdown(display_text));
                out.push_str("\n\n");
            }
            CopyItem::Omitted => out.push_str("[Content omitted]\n\n"),
        }
    }
    for value in &copy.evidence {
        out.push_str("- ");
        out.push_str(&escape_markdown(value));
        out.push('\n');
    }
    out
}
fn escape_markdown(value: &str) -> String {
    // Escape brackets/links and markup so user text cannot create an external URL.
    value
        .chars()
        .flat_map(|ch| {
            if "\\`*_{}[]()#+-.!<>".contains(ch) {
                vec!['\\', ch]
            } else {
                vec![ch]
            }
        })
        .collect()
}
fn render_html(copy: &ReadingCopy) -> String {
    let mut out = String::from("<!doctype html><meta charset=\"utf-8\"><main>");
    for item in &copy.items {
        match item {
            CopyItem::Heading { title } => {
                out.push_str("<h2>");
                out.push_str(&escape_html(title));
                out.push_str("</h2>");
            }
            CopyItem::Content {
                title,
                display_text,
                ..
            } => {
                out.push_str("<article><h3>");
                out.push_str(&escape_html(title));
                out.push_str("</h3><p>");
                out.push_str(&escape_html(display_text));
                out.push_str("</p></article>");
            }
            CopyItem::Omitted => out.push_str("<p>Content omitted</p>"),
        }
    }
    for value in &copy.evidence {
        out.push_str("<p>");
        out.push_str(&escape_html(value));
        out.push_str("</p>");
    }
    out.push_str("</main>");
    out
}
fn escape_html(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

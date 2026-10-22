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

mod copy_meter;
mod custody;
#[cfg(target_os = "linux")]
pub use custody::InstalledSnapshotBinding;
pub use custody::{CustodyCaptureGuard, CustodyStaged, CustodyWriter, RootSnapshotCustody};

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
    #[cfg(target_os = "linux")]
    reading_bounds: Option<std::sync::Arc<reading_budget::Bounds>>,
}

#[derive(Debug)]
pub struct VerifiedSnapshot {
    pub manifest: SnapshotManifest,
    pub rows: Vec<SnapshotRow>,
    pub files: Vec<SnapshotFile>,
}

/// A verified package's sealed files, opened one at a time. Keeping the iterator
/// does not keep a descriptor for every package member. Each `next` verifies the
/// newly opened handle again before returning it.
pub struct SnapshotFileIter {
    directory: Dir,
    files: std::vec::IntoIter<(String, String, Option<u64>)>,
    failed: bool,
    #[cfg(target_os = "linux")]
    reading_bounds: Option<std::sync::Arc<reading_budget::Bounds>>,
}

impl Iterator for SnapshotFileIter {
    type Item = Result<(String, File), SnapshotIoError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        let (name, digest, size) = self.files.next()?;
        let result = (|| {
            #[cfg(target_os = "linux")]
            let (file, limit) = if let Some(bounds) = &self.reading_bounds {
                (bounds.open_stream(&name)?, bounds.size(&name)?)
            } else {
                (
                    open_staged_file(&self.directory, &name)?,
                    size_limit(&name)?,
                )
            };
            #[cfg(not(target_os = "linux"))]
            let (file, limit) = (
                open_staged_file(&self.directory, &name)?,
                size_limit(&name)?,
            );
            let (file, actual_digest, actual_size) = sealed_delivery(file, limit)?;
            #[cfg(target_os = "linux")]
            if let Some(bounds) = &self.reading_bounds {
                bounds.recheck()?;
            }
            if actual_digest != digest || size.is_some_and(|expected| expected != actual_size) {
                return Err(SnapshotIoError::InvalidPackage);
            }
            Ok((name, file))
        })();
        self.failed = result.is_err();
        Some(result)
    }
}

impl SnapshotDirectory {
    pub fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }

    /// One fresh, bounded four-file ReadingCopy verification. The original
    /// handle is retained; the complete actual-size work is paid before reads.
    #[cfg(target_os = "linux")]
    pub fn verify_reading_copy_with_budget(
        &self,
        budget: &crate::root_journal::SharedJournalBudget,
    ) -> Result<VerifiedSnapshot, SnapshotIoError> {
        let bounded = self.budgeted_reading_copy(budget, reading_budget::Mode::Verifier)?;
        let result = verify_snapshot(&bounded)?;
        bounded
            .reading_bounds
            .as_ref()
            .expect("bounded constructor")
            .finish()?;
        Ok(result)
    }
    #[cfg(target_os = "linux")]
    fn budgeted_reading_copy(
        &self,
        budget: &crate::root_journal::SharedJournalBudget,
        mode: reading_budget::Mode,
    ) -> Result<Self, SnapshotIoError> {
        if self.reading_bounds.is_some() {
            return Err(SnapshotIoError::InvalidPackage);
        }
        let bounds = reading_budget::Bounds::observe(&self.dir, budget, mode)?;
        Ok(Self {
            dir: self.dir.try_clone()?,
            manifest_sha256: self.manifest_sha256.clone(),
            reading_bounds: Some(std::sync::Arc::new(bounds)),
        })
    }
    #[cfg(target_os = "linux")]
    fn with_reading_copy_capture_budget<T>(
        &self,
        budget: &crate::root_journal::SharedJournalBudget,
        visitor: impl FnOnce(&SnapshotDirectory) -> Result<T, SnapshotIoError>,
    ) -> Result<T, SnapshotIoError> {
        let bounded = self.budgeted_reading_copy(budget, reading_budget::Mode::Capture)?;
        verify_snapshot(&bounded)?;
        let result = visitor(&bounded)?;
        bounded
            .reading_bounds
            .as_ref()
            .expect("bounded constructor")
            .finish()?;
        Ok(result)
    }

    /// Open only names recorded and freshly verified against the manifest.
    pub fn open_verified_files(&self) -> Result<Vec<(String, File)>, SnapshotIoError> {
        self.open_verified_file_iter()?.collect()
    }

    /// Recovery uses the same verification as delivery without opening up to
    /// SNAPSHOT_MAX_FILES descriptors at once. This performs no writes.
    pub fn open_verified_file_iter(&self) -> Result<SnapshotFileIter, SnapshotIoError> {
        let verified = verify_snapshot(self)?;
        let mut files = verified
            .files
            .into_iter()
            .map(|f| (f.path, f.sha256, Some(f.size)))
            .collect::<Vec<_>>();
        files.push(("manifest.json".into(), self.manifest_sha256.clone(), None));
        files.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(SnapshotFileIter {
            directory: self.dir.try_clone()?,
            files: files.into_iter(),
            failed: false,
            #[cfg(target_os = "linux")]
            reading_bounds: self.reading_bounds.clone(),
        })
    }
}

/// Trusted service-root-relative job storage. Job and attempt identities are
/// typed UUIDs, never caller supplied paths. Each lease gets an isolated package.
pub struct SnapshotJobDirectory {
    dir: Dir,
    copy_id: Uuid,
}
impl SnapshotJobDirectory {
    pub fn open(root: &Path, job: Uuid) -> Result<Self, SnapshotIoError> {
        let root = Dir::open_private_root(root)?;
        root.reject_custody_ancestry()?;
        let dir = open_job_directory(&root, job)?;
        Ok(Self { dir, copy_id: job })
    }
    /// Read-only opener for an independently copied recovery package. A missing
    /// job directory fails; it never invokes the ordinary staging mkdir path.
    pub fn open_existing(root: &Path, job: Uuid) -> Result<Self, SnapshotIoError> {
        let root = Dir::open_private_root(root)?;
        root.reject_custody_ancestry()?;
        let dir = root.open_dir(&job.to_string())?;
        Ok(Self { dir, copy_id: job })
    }
    /// Hold-relative recovery reader. It never traverses an external pathname
    /// or creates a missing job directory.
    #[cfg(target_os = "linux")]
    pub fn open_existing_in(
        root: &crate::backup_fs::BackupDir,
        job: Uuid,
    ) -> Result<Self, SnapshotIoError> {
        let root = root.clone_inner()?;
        root.reject_custody_ancestry()?;
        let dir = root.open_dir(&job.to_string())?;
        Ok(Self { dir, copy_id: job })
    }
    pub fn stage_exact(
        &self,
        token: Uuid,
        manifest: &ExactSnapshotManifest,
        rows: &[SnapshotRow],
        assets: &[SnapshotAssetUse],
        files: &FsAssetStore,
    ) -> Result<SnapshotDirectory, SnapshotIoError> {
        stage_exact_writer(
            StageWriter::in_directory(self.dir.try_clone()?, token)?,
            manifest,
            rows,
            assets,
            files,
        )
        .map(|staged| staged.directory)
    }
    pub fn stage_copy(
        &self,
        token: Uuid,
        copy: &ReadingCopy,
    ) -> Result<SnapshotDirectory, SnapshotIoError> {
        let manifest = ReadingCopyManifest {
            format_version: SNAPSHOT_FORMAT_VERSION,
            copy_id: self.copy_id,
            files: vec![],
        };
        stage_copy_writer(
            StageWriter::in_directory(self.dir.try_clone()?, token)?,
            self.copy_id,
            &manifest,
            copy,
        )
        .map(|staged| staged.directory)
    }
    /// Reopen after restart, requiring the winning database manifest digest.
    pub fn reopen(
        &self,
        token: Uuid,
        expected_sha256: &str,
    ) -> Result<SnapshotDirectory, SnapshotIoError> {
        self.dir.reject_custody_ancestry()?;
        let stage = SnapshotDirectory {
            dir: self.dir.open_dir(&format!("{token}.ready"))?,
            manifest_sha256: expected_sha256.into(),
            #[cfg(target_os = "linux")]
            reading_bounds: None,
        };
        verify_snapshot(&stage)?;
        Ok(stage)
    }
}

fn open_job_directory(root: &Dir, job: Uuid) -> Result<Dir, SnapshotIoError> {
    let name = job.to_string();
    match root.open_dir(&name) {
        Ok(dir) => Ok(dir),
        Err(error) if error.kind() == io::ErrorKind::NotFound => match root.create_dir(&name) {
            Ok(dir) => Ok(dir),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(root.open_dir(&name)?),
            Err(error) => Err(error.into()),
        },
        Err(error) => Err(error.into()),
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

fn validate_asset_declarations(
    assets: &[SnapshotAssetUse],
    include_originals: bool,
) -> Result<(), SnapshotIoError> {
    let mut seen = BTreeMap::new();
    let mut total = 0u64;
    for asset in assets {
        if asset.sha256.len() != 64
            || !asset
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || asset.storage_key != format!("sha256/{}/{}", &asset.sha256[..2], asset.sha256)
        {
            return Err(SnapshotIoError::InvalidPackage);
        }
        if include_originals && asset.byte_size > SNAPSHOT_MAX_ASSET_FILE_BYTES as u64 {
            return Err(SnapshotIoError::LimitExceeded);
        }
        if let Some(previous) = seen.insert(asset.storage_key.as_str(), asset.byte_size) {
            if previous != asset.byte_size {
                return Err(SnapshotIoError::InvalidPackage);
            }
        } else if include_originals {
            total = total
                .checked_add(asset.byte_size)
                .ok_or(SnapshotIoError::LimitExceeded)?;
            if total > SNAPSHOT_MAX_ASSET_BYTES as u64 {
                return Err(SnapshotIoError::LimitExceeded);
            }
        }
    }
    Ok(())
}

pub fn stage_snapshot(
    root: &Path,
    job_id: Uuid,
    manifest: &ExactSnapshotManifest,
    rows: &[SnapshotRow],
    assets: &[SnapshotAssetUse],
    files: &FsAssetStore,
) -> Result<SnapshotDirectory, SnapshotIoError> {
    stage_exact_writer(
        StageWriter::new(root, job_id)?,
        manifest,
        rows,
        assets,
        files,
    )
    .map(|staged| staged.directory)
}

fn stage_exact_writer(
    mut writer: StageWriter,
    manifest: &ExactSnapshotManifest,
    rows: &[SnapshotRow],
    assets: &[SnapshotAssetUse],
    files: &FsAssetStore,
) -> Result<WrittenStage, SnapshotIoError> {
    let result = (|| {
        if manifest.format_version != SNAPSHOT_FORMAT_VERSION {
            return Err(SnapshotIoError::InvalidPackage);
        }
        validate_asset_declarations(assets, !manifest.requires_destination_assets)?;
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
    stage_copy_writer(StageWriter::new(root, copy_id)?, copy_id, manifest, copy)
        .map(|staged| staged.directory)
}
fn stage_copy_writer(
    mut writer: StageWriter,
    copy_id: Uuid,
    manifest: &ReadingCopyManifest,
    copy: &ReadingCopy,
) -> Result<WrittenStage, SnapshotIoError> {
    let result = (|| {
        if manifest.format_version != SNAPSHOT_FORMAT_VERSION
            || manifest.copy_id != copy_id
            || !manifest.files.is_empty()
        {
            return Err(SnapshotIoError::InvalidPackage);
        }
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
    result.map(|staged| staged.directory)
}

#[cfg(any(test, target_os = "linux"))]
const READING_COPY_MEMBER_NAMES: [&str; 4] = [
    "manifest.json",
    "reading.html",
    "reading.md",
    "validation.json",
];

#[cfg(any(test, target_os = "linux"))]
fn reading_member_names_match(names: &mut [String]) -> bool {
    if names.len() != READING_COPY_MEMBER_NAMES.len() {
        return false;
    }
    names.sort_unstable();
    names
        .iter()
        .map(String::as_str)
        .eq(READING_COPY_MEMBER_NAMES)
}

// The metered path parses the closed shallow wire directly. An internally
// tagged enum would first materialize arbitrary Content before rejecting it.
#[cfg(any(test, target_os = "linux"))]
fn parse_reading_manifest(
    bytes: &[u8],
    sizes: [u64; 3],
) -> Result<SnapshotManifest, serde_json::Error> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct FixedFile<'a> {
        path: &'a str,
        size: u64,
        sha256: &'a str,
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct FixedManifest<'a> {
        capability: &'a str,
        format_version: u32,
        copy_id: &'a str,
        #[serde(borrow)]
        files: [FixedFile<'a>; 3],
    }
    let raw: FixedManifest<'_> = serde_json::from_slice(bytes)?;
    let invalid =
        || <serde_json::Error as serde::de::Error>::custom("invalid fixed reading manifest");
    let copy_id = Uuid::parse_str(raw.copy_id).map_err(|_| invalid())?;
    if raw.capability != "reading_copy_v1"
        || raw.format_version != SNAPSHOT_FORMAT_VERSION
        || raw.copy_id.len() != 36
        || copy_id.hyphenated().to_string() != raw.copy_id
        || raw.files.iter().enumerate().any(|(index, file)| {
            file.path != ["reading.html", "reading.md", "validation.json"][index]
                || file.size != sizes[index]
                || file.size > SNAPSHOT_MAX_JSON_FILE_BYTES as u64
                || file.sha256.len() != 64
                || !file
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
    {
        return Err(invalid());
    }
    Ok(SnapshotManifest::ReadingCopyV1(ReadingCopyManifest {
        format_version: raw.format_version,
        copy_id,
        files: raw
            .files
            .map(|file| SnapshotFile {
                path: file.path.into(),
                size: file.size,
                sha256: file.sha256.into(),
            })
            .into(),
    }))
}

#[cfg(test)]
mod fixed_reading_manifest_tests {
    use super::*;

    #[test]
    fn reading_members_accept_every_readdir_order_and_refuse_different_rosters() {
        for a in 0..4 {
            for b in 0..4 {
                for c in 0..4 {
                    for d in 0..4 {
                        let indices = [a, b, c, d];
                        if indices.iter().copied().collect::<BTreeSet<_>>().len() == 4 {
                            let mut names =
                                indices.map(|i| READING_COPY_MEMBER_NAMES[i].to_owned());
                            assert!(
                                reading_member_names_match(&mut names),
                                "permutation {indices:?}"
                            );
                        }
                    }
                }
            }
        }
        let mut missing = READING_COPY_MEMBER_NAMES[..3]
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>();
        assert!(!reading_member_names_match(&mut missing));
        let mut extra = READING_COPY_MEMBER_NAMES.map(str::to_owned).to_vec();
        extra.push("extra".into());
        assert!(!reading_member_names_match(&mut extra));
        let mut duplicate = READING_COPY_MEMBER_NAMES.map(str::to_owned);
        duplicate[0] = duplicate[1].clone();
        assert!(!reading_member_names_match(&mut duplicate));
    }

    #[test]
    fn fixed_reading_parser_keeps_exact_wire_sizes_and_three_file_shape() {
        let manifest = SnapshotManifest::ReadingCopyV1(ReadingCopyManifest {
            format_version: SNAPSHOT_FORMAT_VERSION,
            copy_id: Uuid::nil(),
            files: ["reading.html", "reading.md", "validation.json"]
                .map(|path| SnapshotFile {
                    path: path.into(),
                    size: 0,
                    sha256: "a".repeat(64),
                })
                .into(),
        });
        let value = serde_json::to_value(&manifest).unwrap();
        let bytes = canonical_json(&value).into_bytes();
        assert_eq!(parse_reading_manifest(&bytes, [0; 3]).unwrap(), manifest);
        assert!(parse_reading_manifest(&bytes, [0, 1, 0]).is_err());
        let mut four = value.clone();
        let first = four["files"][0].clone();
        four["files"].as_array_mut().unwrap().push(first);
        assert!(parse_reading_manifest(canonical_json(&four).as_bytes(), [0; 3]).is_err());
        let mut unknown = value.clone();
        unknown["unknown"] = serde_json::json!([]);
        assert!(parse_reading_manifest(canonical_json(&unknown).as_bytes(), [0; 3]).is_err());
        let mut bad = value;
        bad["files"][0]["sha256"] = serde_json::json!("A".repeat(64));
        assert!(parse_reading_manifest(canonical_json(&bad).as_bytes(), [0; 3]).is_err());
    }

    #[test]
    fn fixed_reading_parser_rejects_large_invalid_tree_before_materialization() {
        let mut bytes = br#"{"capability":"reading_copy_v1","copy_id":"00000000-0000-0000-0000-000000000000","files":[["#.to_vec();
        for _ in 0..262_145 {
            bytes.extend_from_slice(b"[],");
        }
        bytes.extend_from_slice(b"[]]],\"format_version\":1}");
        let error = parse_reading_manifest(&bytes, [0; 3]).unwrap_err();
        assert!(
            error.line() == 1 && error.column() < 256,
            "invalid first file must refuse directly on the original input before scanning its nested tree: {error}"
        );
    }
}

pub fn verify_snapshot(stage: &SnapshotDirectory) -> Result<VerifiedSnapshot, SnapshotIoError> {
    #[cfg(target_os = "linux")]
    if let Some(bounds) = &stage.reading_bounds {
        bounds.start_verifier()?;
    }
    let manifest_bytes = read_for_verifier(stage, "manifest.json", SNAPSHOT_MAX_JSON_FILE_BYTES)?;
    if hex_digest(&manifest_bytes) != stage.manifest_sha256 {
        return Err(SnapshotIoError::InvalidPackage);
    }
    #[cfg(target_os = "linux")]
    let manifest: SnapshotManifest = if let Some(bounds) = &stage.reading_bounds {
        parse_reading_manifest(
            &manifest_bytes,
            [
                bounds.size("reading.html")? as u64,
                bounds.size("reading.md")? as u64,
                bounds.size("validation.json")? as u64,
            ],
        )
        .map_err(|_| SnapshotIoError::InvalidPackage)?
    } else {
        serde_json::from_slice(&manifest_bytes).map_err(|_| SnapshotIoError::InvalidPackage)?
    };
    #[cfg(not(target_os = "linux"))]
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
            let bytes = read_for_verifier(stage, &entry.path, limit)?;
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
    #[cfg(target_os = "linux")]
    if let Some(bounds) = &stage.reading_bounds {
        bounds.recheck()?;
    }
    let files = entries.to_vec();
    Ok(VerifiedSnapshot {
        manifest,
        rows,
        files,
    })
}

#[cfg(target_os = "linux")]
mod reading_budget {
    use super::*;
    use std::{
        os::unix::fs::{FileExt, MetadataExt},
        sync::atomic::{AtomicU8, Ordering},
    };
    const NAMES: [&str; 4] = READING_COPY_MEMBER_NAMES;
    // Includes fixed table/list/fingerprint work, the small metadata trees,
    // and <=4*64KiB reads past a concurrently truncated/grown stream's bound.
    const METADATA_WORK: u64 = 512 * 1024;
    #[derive(Clone, Copy)]
    pub(super) enum Mode {
        Verifier,
        Capture,
    }
    #[derive(Debug, PartialEq, Eq)]
    struct Fingerprint(u64, u64, u64, u64, u32, u32, u32, i64, i64, i64, i64);
    impl Fingerprint {
        fn of(m: &std::fs::Metadata) -> Self {
            Self(
                m.dev(),
                m.ino(),
                m.nlink(),
                m.len(),
                m.uid(),
                m.gid(),
                m.mode(),
                m.mtime(),
                m.mtime_nsec(),
                m.ctime(),
                m.ctime_nsec(),
            )
        }
    }
    #[derive(Debug)]
    pub(super) struct Bounds {
        dir: Dir,
        directory: Fingerprint,
        files: [(Fingerprint, File); 4],
        verifiers: AtomicU8,
        streams: AtomicU8,
    }
    fn work(sizes: [u64; 4], mode: Mode) -> Result<u64, SnapshotIoError> {
        // A successful metered manifest is exactly ReadingCopy, three fixed
        // file declarations, UUID/scalar leaves, and no extra nesting. The
        // longest canonical leaf has nine character-emission passes (leaf;
        // file pair/join/object; array join/brackets; root pair/join/object).
        // 20*n also pays exact buffer zero/read, hash, fixed borrowed parsing,
        // Value conversion, equality, declarations/verified-file clones.
        // Payloads pay exact buffer zero/read, hash and UTF-8/equality:4*n.
        let payload = sizes[1..].iter().try_fold(0u64, |n, s| {
            n.checked_add(*s).ok_or(SnapshotIoError::LimitExceeded)
        })?;
        let one = sizes[0]
            .checked_mul(20)
            .and_then(|n| payload.checked_mul(4).and_then(|p| n.checked_add(p)))
            .ok_or(SnapshotIoError::LimitExceeded)?;
        match mode {
            Mode::Verifier => one.checked_add(METADATA_WORK),
            Mode::Capture => one
                .checked_mul(2)
                .and_then(|n| {
                    sizes
                        .iter()
                        .try_fold(0u64, |n, s| n.checked_add(*s))
                        .and_then(|total| total.checked_mul(3))
                        .and_then(|stream| n.checked_add(stream))
                })
                .and_then(|n| n.checked_add(METADATA_WORK)),
        }
        .ok_or(SnapshotIoError::LimitExceeded)
    }
    impl Bounds {
        pub(super) fn observe(
            dir: &Dir,
            budget: &crate::root_journal::SharedJournalBudget,
            mode: Mode,
        ) -> Result<Self, SnapshotIoError> {
            // Fixed quotas bound every repeated four-member list/open/stat
            // before/after each permitted verifier and sealed stream.
            budget.charge(
                METADATA_WORK,
                match mode {
                    Mode::Verifier => 512,
                    Mode::Capture => 1024,
                },
            )?;
            let metadata = dir.root_metadata()?;
            if metadata.uid() != 0
                || metadata.gid() != 0
                || !metadata.is_dir()
                || !matches!(metadata.mode() & 0o7777, 0o500 | 0o700)
                || !reading_member_names_match(&mut dir.list_bounded(4)?)
            {
                return Err(SnapshotIoError::InvalidPackage);
            }
            let mut files = Vec::with_capacity(4);
            let mut sizes = [0u64; 4];
            for (index, name) in NAMES.iter().enumerate() {
                let file = dir.open_file(name)?;
                let m = file.metadata()?;
                if !m.is_file()
                    || m.uid() != 0
                    || m.gid() != 0
                    || m.nlink() != 1
                    || m.mode() & 0o7777 != 0o400
                    || m.len() > SNAPSHOT_MAX_JSON_FILE_BYTES as u64
                {
                    return Err(SnapshotIoError::InvalidPackage);
                }
                sizes[index] = m.len();
                files.push((Fingerprint::of(&m), file));
            }
            budget.charge(
                work(sizes, mode)?
                    .checked_sub(METADATA_WORK)
                    .ok_or(SnapshotIoError::LimitExceeded)?,
                0,
            )?;
            let bound = Self {
                dir: dir.try_clone()?,
                directory: Fingerprint::of(&metadata),
                files: files
                    .try_into()
                    .map_err(|_| SnapshotIoError::InvalidPackage)?,
                verifiers: AtomicU8::new(match mode {
                    Mode::Verifier => 1,
                    Mode::Capture => 2,
                }),
                streams: AtomicU8::new(match mode {
                    Mode::Verifier => 0,
                    Mode::Capture => 15,
                }),
            };
            bound.recheck()?;
            Ok(bound)
        }
        fn index(name: &str) -> Result<usize, SnapshotIoError> {
            NAMES
                .iter()
                .position(|n| *n == name)
                .ok_or(SnapshotIoError::InvalidPackage)
        }
        pub(super) fn size(&self, name: &str) -> Result<usize, SnapshotIoError> {
            usize::try_from(self.files[Self::index(name)?].0.3)
                .map_err(|_| SnapshotIoError::LimitExceeded)
        }
        pub(super) fn recheck(&self) -> Result<(), SnapshotIoError> {
            if Fingerprint::of(&self.dir.root_metadata()?) != self.directory
                || !reading_member_names_match(&mut self.dir.list_bounded(4)?)
            {
                return Err(SnapshotIoError::InvalidPackage);
            }
            for (index, name) in NAMES.iter().enumerate() {
                if Fingerprint::of(&self.files[index].1.metadata()?) != self.files[index].0
                    || Fingerprint::of(&self.dir.open_file(name)?.metadata()?)
                        != self.files[index].0
                {
                    return Err(SnapshotIoError::InvalidPackage);
                }
            }
            Ok(())
        }
        pub(super) fn start_verifier(&self) -> Result<(), SnapshotIoError> {
            self.verifiers
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .map_err(|_| SnapshotIoError::InvalidPackage)?;
            self.recheck()
        }
        pub(super) fn read(&self, name: &str) -> Result<Vec<u8>, SnapshotIoError> {
            self.recheck()?;
            let index = Self::index(name)?;
            let n = self.size(name)?;
            let mut raw = Vec::new();
            raw.try_reserve_exact(n)
                .map_err(|_| SnapshotIoError::LimitExceeded)?;
            raw.resize(n, 0);
            self.files[index].1.read_exact_at(&mut raw, 0)?;
            let mut extra = [0u8];
            if self.files[index].1.read_at(&mut extra, n as u64)? != 0 {
                return Err(SnapshotIoError::InvalidPackage);
            }
            self.recheck()?;
            Ok(raw)
        }
        pub(super) fn open_stream(&self, name: &str) -> Result<File, SnapshotIoError> {
            self.recheck()?;
            let index = Self::index(name)?;
            let flag = 1u8 << index;
            self.streams
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |mask| {
                    if mask & flag == 0 {
                        None
                    } else {
                        Some(mask & !flag)
                    }
                })
                .map_err(|_| SnapshotIoError::InvalidPackage)?;
            Ok(self.dir.open_file(name)?)
        }
        pub(super) fn finish(&self) -> Result<(), SnapshotIoError> {
            self.recheck()?;
            if self.verifiers.load(Ordering::SeqCst) != 0
                || self.streams.load(Ordering::SeqCst) != 0
            {
                return Err(SnapshotIoError::InvalidPackage);
            }
            Ok(())
        }
    }
    #[cfg(test)]
    pub(super) fn real_metadata_rejection_cases(parent: &Path) {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(unsafe { libc::geteuid() }, 0, "explicit Root OS gate");
        let root = parent.join("bounded-reading-copy");
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut declarations = Vec::new();
        for (name, raw) in [
            ("reading.html", b"<p>small</p>".to_vec()),
            ("reading.md", vec![b'x'; 1024 * 1024]),
            ("validation.json", br#"{"status":"validated"}"#.to_vec()),
        ] {
            std::fs::write(root.join(name), &raw).unwrap();
            std::fs::set_permissions(root.join(name), std::fs::Permissions::from_mode(0o400))
                .unwrap();
            declarations.push(SnapshotFile {
                path: name.into(),
                size: raw.len() as u64,
                sha256: hex_digest(&raw),
            });
        }
        let manifest = SnapshotManifest::ReadingCopyV1(ReadingCopyManifest {
            format_version: 1,
            copy_id: Uuid::new_v4(),
            files: declarations,
        });
        let raw = canonical_json(&serde_json::to_value(&manifest).unwrap()).into_bytes();
        std::fs::write(root.join("manifest.json"), &raw).unwrap();
        std::fs::set_permissions(
            root.join("manifest.json"),
            std::fs::Permissions::from_mode(0o400),
        )
        .unwrap();
        let stage = SnapshotDirectory {
            dir: Dir::open_private_root(&root).unwrap(),
            manifest_sha256: hex_digest(&raw),
            reading_bounds: None,
        };
        let sizes = NAMES.map(|name| std::fs::metadata(root.join(name)).unwrap().len());
        let meter = crate::root_journal::SharedJournalBudget::default();
        assert_eq!(
            stage
                .verify_reading_copy_with_budget(&meter)
                .unwrap()
                .files
                .len(),
            3
        );
        assert_eq!(
            meter.usage().unwrap().charged_bytes,
            work(sizes, Mode::Verifier).unwrap()
        );
        let meter = crate::root_journal::SharedJournalBudget::default();
        stage
            .with_reading_copy_capture_budget(&meter, |package| {
                let mut count = 0;
                for file in package.open_verified_file_iter()? {
                    let (_, file) = file?;
                    let _ = file.metadata()?;
                    count += 1;
                }
                assert_eq!(count, 4);
                Ok(())
            })
            .unwrap();
        assert_eq!(
            meter.usage().unwrap().charged_bytes,
            work(sizes, Mode::Capture).unwrap()
        );
        let meter = crate::root_journal::SharedJournalBudget::default();
        meter
            .charge(
                512 * 1024 * 1024 - work(sizes, Mode::Verifier).unwrap() + 1,
                0,
            )
            .unwrap();
        assert!(stage.verify_reading_copy_with_budget(&meter).is_err());
        assert!(meter.charge(0, 0).is_err());
        let meter = crate::root_journal::SharedJournalBudget::default();
        let bounded = stage.budgeted_reading_copy(&meter, Mode::Verifier).unwrap();
        std::fs::set_permissions(
            root.join("reading.md"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        std::fs::write(root.join("reading.md"), b"changed size after actual stat").unwrap();
        std::fs::set_permissions(
            root.join("reading.md"),
            std::fs::Permissions::from_mode(0o400),
        )
        .unwrap();
        assert!(
            verify_snapshot(&bounded).is_err(),
            "fresh size/inode/time changed before verifier must refuse"
        );
        std::fs::write(root.join("extra"), b"not referenced").unwrap();
        let meter = crate::root_journal::SharedJournalBudget::default();
        assert!(
            stage.verify_reading_copy_with_budget(&meter).is_err(),
            "extra fixed-tree member must refuse before bytes"
        );
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn verifier_work_is_linear_for_one_large_and_two_small_payloads() {
            let small = work([512, 32, 64, 22], Mode::Verifier).unwrap();
            let large = work([512, 8 * 1024 * 1024, 64, 22], Mode::Verifier).unwrap();
            assert_eq!(large - small, 4 * (8 * 1024 * 1024 - 32));
            assert_eq!(
                work([512, 32, 64, 22], Mode::Capture).unwrap(),
                2 * (small - METADATA_WORK) + 3 * (512 + 32 + 64 + 22) + METADATA_WORK
            );
            assert_eq!(
                work([512, 0, 0, 22], Mode::Verifier).unwrap(),
                20 * 512 + 4 * 22 + METADATA_WORK
            );
            assert_eq!(
                work([512, 0, 0, 22], Mode::Capture).unwrap(),
                2 * (20 * 512 + 4 * 22) + 3 * (512 + 22) + METADATA_WORK
            );
            assert!(large < 40 * 1024 * 1024);
            assert!(work([u64::MAX, 1, 1, 1], Mode::Capture).is_err());
        }
    }
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
    custody: Option<custody::CustodyGuard>,
}
struct WrittenStage {
    directory: SnapshotDirectory,
    custody: Option<custody::CustodyGuard>,
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
            Self::in_directory(root, id)
        }
    }
    fn in_directory(root: Dir, id: Uuid) -> Result<Self, SnapshotIoError> {
        root.reject_custody_ancestry()?;
        Self::create(root, id, None)
    }
    /// Only the custody module can reach this private bypass with its live
    /// move-only OS guard. Legacy path/Dir admission never calls it.
    fn in_custody_directory(
        root: Dir,
        id: Uuid,
        guard: custody::CustodyGuard,
    ) -> Result<Self, SnapshotIoError> {
        guard.revalidate()?;
        Self::create(root, id, Some(guard))
    }
    fn create(
        root: Dir,
        id: Uuid,
        custody: Option<custody::CustodyGuard>,
    ) -> Result<Self, SnapshotIoError> {
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
            custody,
        })
    }
    fn revalidate_custody(&self) -> Result<(), SnapshotIoError> {
        if let Some(guard) = &self.custody {
            guard.revalidate()?;
        }
        Ok(())
    }
    fn write_stream(&mut self, name: &str, source: &mut dyn Read) -> Result<(), SnapshotIoError> {
        self.revalidate_custody()?;
        self.begin(name)?;
        let limit = size_limit(name)?;
        let (parent, leaf) = self.file_parent(name)?;
        let mut target = parent.create_file(&leaf)?;
        let (sha256, size) = copy_hash_bounded(source, &mut target, limit)?;
        target.sync_all()?;
        self.revalidate_custody()?;
        self.record(name, size, sha256)?;
        Ok(())
    }
    fn write_asset(
        &mut self,
        name: &str,
        asset: &SnapshotAssetUse,
        store: &FsAssetStore,
    ) -> Result<(), SnapshotIoError> {
        self.revalidate_custody()?;
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
        self.revalidate_custody()?;
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
    fn finish(&mut self, manifest: &SnapshotManifest) -> Result<WrittenStage, SnapshotIoError> {
        self.files.sort_by(|a, b| a.path.cmp(&b.path));
        if self.files.len() + 1 > SNAPSHOT_MAX_FILES {
            return Err(SnapshotIoError::LimitExceeded);
        }
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
    ) -> Result<WrittenStage, SnapshotIoError> {
        self.revalidate_custody()?;
        self.files.sort_by(|a, b| a.path.cmp(&b.path));
        if manifest_files(manifest) != self.files || !self.names.contains("manifest.json") {
            return Err(SnapshotIoError::InvalidPackage);
        }
        sync_tree(&self.partial)?;
        seal_tree(&self.partial)?;
        let stage = SnapshotDirectory {
            dir: self.partial.try_clone()?,
            manifest_sha256: hash.clone(),
            #[cfg(target_os = "linux")]
            reading_bounds: None,
        };
        verify_snapshot(&stage)?;
        self.revalidate_custody()?;
        self.root.rename(&self.partial_name, &self.ready_name)?;
        self.revalidate_custody()?;
        Ok(WrittenStage {
            directory: SnapshotDirectory {
                dir: self.partial.try_clone()?,
                manifest_sha256: hash,
                #[cfg(target_os = "linux")]
                reading_bounds: None,
            },
            custody: self.custody.take(),
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
fn read_for_verifier(
    stage: &SnapshotDirectory,
    name: &str,
    limit: usize,
) -> Result<Vec<u8>, SnapshotIoError> {
    #[cfg(target_os = "linux")]
    if let Some(bounds) = &stage.reading_bounds {
        return bounds.read(name);
    }
    read_staged(&stage.dir, name, limit)
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
    if !matches!(projection.contract_version, 1 | 2) {
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
    render_markdown_to(copy, &mut out).expect("String rendering cannot fail");
    out
}
fn render_markdown_to(copy: &ReadingCopy, out: &mut impl std::fmt::Write) -> std::fmt::Result {
    for item in &copy.items {
        match item {
            CopyItem::Heading { title } => {
                out.write_str("## ")?;
                escape_markdown_to(title, out)?;
                out.write_str("\n\n")?;
            }
            CopyItem::Content {
                title,
                display_text,
                ..
            } => {
                if !title.is_empty() {
                    out.write_str("### ")?;
                    escape_markdown_to(title, out)?;
                    out.write_str("\n\n")?;
                }
                escape_markdown_to(display_text, out)?;
                out.write_str("\n\n")?;
            }
            CopyItem::Omitted => out.write_str("[Content omitted]\n\n")?,
        }
    }
    for value in &copy.evidence {
        out.write_str("- ")?;
        escape_markdown_to(value, out)?;
        out.write_char('\n')?;
    }
    Ok(())
}
fn escape_markdown_to(value: &str, out: &mut impl std::fmt::Write) -> std::fmt::Result {
    // Identical escaping for legacy Strings, paid counters and bounded buffers.
    for ch in value.chars() {
        if "\\`*_{}[]()#+-.!<>".contains(ch) {
            out.write_char('\\')?;
        }
        out.write_char(ch)?;
    }
    Ok(())
}
fn render_html(copy: &ReadingCopy) -> String {
    let mut out = String::new();
    render_html_to(copy, &mut out).expect("String rendering cannot fail");
    out
}
fn render_html_to(copy: &ReadingCopy, out: &mut impl std::fmt::Write) -> std::fmt::Result {
    out.write_str("<!doctype html><meta charset=\"utf-8\"><main>")?;
    for item in &copy.items {
        match item {
            CopyItem::Heading { title } => {
                out.write_str("<h2>")?;
                escape_html_to(title, out)?;
                out.write_str("</h2>")?;
            }
            CopyItem::Content {
                title,
                display_text,
                ..
            } => {
                out.write_str("<article><h3>")?;
                escape_html_to(title, out)?;
                out.write_str("</h3><p>")?;
                escape_html_to(display_text, out)?;
                out.write_str("</p></article>")?;
            }
            CopyItem::Omitted => out.write_str("<p>Content omitted</p>")?,
        }
    }
    for value in &copy.evidence {
        out.write_str("<p>")?;
        escape_html_to(value, out)?;
        out.write_str("</p>")?;
    }
    out.write_str("</main>")
}
fn escape_html_to(value: &str, out: &mut impl std::fmt::Write) -> std::fmt::Result {
    for ch in value.chars() {
        match ch {
            '&' => out.write_str("&amp;")?,
            '<' => out.write_str("&lt;")?,
            '>' => out.write_str("&gt;")?,
            '"' => out.write_str("&quot;")?,
            '\'' => out.write_str("&#39;")?,
            _ => out.write_char(ch)?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    use learning_core::{AssetRef, AssetUseRef, BlockRef};

    fn asset(n: u128, bytes: u64) -> SnapshotAssetUse {
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
            byte_size: bytes,
        }
    }

    #[test]
    fn included_original_budget_applies_only_to_package_bytes() {
        let one = SNAPSHOT_MAX_ASSET_FILE_BYTES as u64;
        let at_total = (1..=4).map(|n| asset(n, one)).collect::<Vec<_>>();
        assert!(validate_asset_declarations(&at_total, true).is_ok());
        assert!(matches!(
            validate_asset_declarations(&[asset(1, one + 1)], true),
            Err(SnapshotIoError::LimitExceeded)
        ));
        let mut over_total = at_total;
        over_total.push(asset(5, 1));
        assert!(matches!(
            validate_asset_declarations(&over_total, true),
            Err(SnapshotIoError::LimitExceeded)
        ));
        assert!(validate_asset_declarations(&over_total, false).is_ok());
        assert!(validate_asset_declarations(&[asset(1, one + 1)], false).is_ok());
        let five_full = (1..=5).map(|n| asset(n, one)).collect::<Vec<_>>();
        assert!(validate_asset_declarations(&five_full, false).is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn real_original_streams_enforce_single_and_aggregate_limits_without_publication() {
        use std::os::unix::fs::PermissionsExt;

        struct Root(std::path::PathBuf);
        impl Drop for Root {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let root = Root(std::env::temp_dir().join(format!("snapshot-stream-{}", Uuid::new_v4())));
        std::fs::create_dir(&root.0).unwrap();
        std::fs::set_permissions(&root.0, std::fs::Permissions::from_mode(0o700)).unwrap();
        let name = |n: u128| {
            let sha256 = format!("{n:064x}");
            format!("assets/sha256/{}/{}", &sha256[..2], sha256)
        };

        let mut writer = StageWriter::new(&root.0, Uuid::new_v4()).unwrap();
        let one = SNAPSHOT_MAX_ASSET_FILE_BYTES as u64;
        for n in 1..=4 {
            writer
                .write_stream(&name(n), &mut std::io::repeat(n as u8).take(one))
                .unwrap();
            assert_eq!(writer.files.last().unwrap().size, one);
            assert_eq!(
                open_staged_file(&writer.partial, &name(n))
                    .unwrap()
                    .metadata()
                    .unwrap()
                    .len(),
                one
            );
        }
        assert_eq!(writer.asset_bytes, SNAPSHOT_MAX_ASSET_BYTES);
        assert!(matches!(
            writer.write_stream(&name(5), &mut std::io::repeat(5).take(1)),
            Err(SnapshotIoError::LimitExceeded)
        ));
        assert!(!std::fs::read_dir(&root.0).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".ready")
        }));
        writer.discard();

        let mut writer = StageWriter::new(&root.0, Uuid::new_v4()).unwrap();
        assert!(matches!(
            writer.write_stream(&name(6), &mut std::io::repeat(6).take(one + 1)),
            Err(SnapshotIoError::LimitExceeded)
        ));
        assert!(!std::fs::read_dir(&root.0).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".ready")
        }));
        writer.discard();
    }
}

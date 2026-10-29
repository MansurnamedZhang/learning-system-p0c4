//! M5 retains genuine original objects by value. The data bundle never owns
//! SQL, a caller path, an authority constructor or a replacement for the cut.
use super::profile;
use crate::source::snapshot_effects::{CapturedSnapshotEffectsV2, files};
use crate::{BackupError, FileRecord, SourceLocalPin, registry::canonical};
use learning_assets::backup_fs::{BackupDir, BackupEntryKind};
use learning_db::snapshot_broker::B1InstalledSourceProcess;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs::File, io::Read, os::unix::fs::MetadataExt, sync::Arc};

const SOURCE_FILES: [&str; 8] = [
    "protection-catalog.json",
    "protection-pending.json",
    "protection-retained.json",
    "protection-snapshot.json",
    "source-authority.json",
    "source-generation.json",
    "source-group.json",
    "source-released.json",
];

#[derive(Serialize)]
struct CapturedData {
    schema_version: u64,
    capability: &'static str,
    job_id: uuid::Uuid,
    backup_id: uuid::Uuid,
    capture_start_sha256: String,
    capture_generation_sha256: String,
    source_group_id: uuid::Uuid,
    source_registry_generation_sha256: String,
    manifest_sha256: String,
    captured_roster_sha256: String,
    snapshot_effects_roster_sha256: String,
}

pub(super) struct OriginalB1CaptureExport {
    // Do not split these into summaries. In particular effects keeps its private
    // _cut and _enrollment, preserving the old consuming LocalDcopy contract.
    pin: SourceLocalPin,
    effects: CapturedSnapshotEffectsV2,
    evidence: Option<BackupDir>,
    pin_directory: Option<BackupDir>,
    manifest_file: Option<File>,
    captured_data_sha256: Option<[u8; 32]>,
    attempted: bool,
}

impl OriginalB1CaptureExport {
    pub(super) fn retain(pin: SourceLocalPin, effects: CapturedSnapshotEffectsV2) -> Self {
        Self {
            pin,
            effects,
            evidence: None,
            pin_directory: None,
            manifest_file: None,
            captured_data_sha256: None,
            attempted: false,
        }
    }

    fn charge(&self, bytes: u64, entries: usize) -> Result<(), BackupError> {
        files::charge(
            &self.effects.budget,
            &self.effects.journal_budget,
            bytes,
            entries,
        )
    }

    /// Closed shallow DATA conversion, reserved before either Value or output
    /// allocation. No arbitrary JSON, input path or control body is accepted.
    fn encode(&self, value: &impl Serialize, cap: usize) -> Result<Vec<u8>, BackupError> {
        let n = self.prepay_encoding(value, cap)?;
        let raw = canonical(value)?;
        if raw.len() != n {
            return Err(invalid());
        }
        Ok(raw)
    }

    fn encode_typed(&self, value: &impl Serialize, cap: usize) -> Result<Vec<u8>, BackupError> {
        let n = self.prepay_encoding(value, cap)?;
        let raw = serde_json::to_vec(value)?;
        if raw.len() != n {
            return Err(invalid());
        }
        Ok(raw)
    }

    fn prepay_encoding(&self, value: &impl Serialize, cap: usize) -> Result<usize, BackupError> {
        struct Count<'a> {
            owner: &'a OriginalB1CaptureExport,
            n: usize,
            cap: usize,
        }
        impl std::io::Write for Count<'_> {
            fn write(&mut self, raw: &[u8]) -> std::io::Result<usize> {
                let n = self
                    .n
                    .checked_add(raw.len())
                    .filter(|n| *n <= self.cap)
                    .ok_or_else(|| std::io::Error::other("B1 capture DATA size"))?;
                self.owner
                    .charge(raw.len() as u64, 0)
                    .map_err(std::io::Error::other)?;
                self.n = n;
                Ok(raw.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut counter = Count {
            owner: self,
            n: 0,
            cap,
        };
        serde_json::to_writer(&mut counter, value)?;
        self.charge(
            (counter.n as u64)
                .checked_mul(32)
                .ok_or(BackupError::Overflow)?,
            1,
        )?;
        Ok(counter.n)
    }

    pub(super) fn publish_from_original(
        &mut self,
        peer: &Arc<B1InstalledSourceProcess>,
    ) -> Result<[u8; 32], BackupError> {
        if std::mem::replace(&mut self.attempted, true) {
            return Err(invalid());
        }
        let material = self.effects.b1_export_material()?;
        material.require_original_peer(peer)?;
        let (job, backup, capture_start, capture_generation) = material.identity_data()?;
        if peer.capture_backup_id_data().map_err(|_| invalid())? != backup
            || backup != self.pin.manifest().backup_id
            || backup != self.pin.sealed().backup_id()
            || self.pin.sealed().manifest_sha256() != self.effects.record.manifest_sha256
        {
            return Err(invalid());
        }
        self.charge(4096, 32)?;
        validate_source_names(material.evidence().keys().map(String::as_str))?;
        let evidence_bytes = material.evidence().values().try_fold(0u64, |n, raw| {
            n.checked_add(raw.len() as u64).ok_or(BackupError::Overflow)
        })?;
        let source_copy = material.evidence().keys().try_fold(0usize, |n, name| {
            n.checked_add(name.len() + 64 + std::mem::size_of::<FileRecord>())
                .ok_or(BackupError::Overflow)
        })?;
        self.charge(
            evidence_bytes
                .checked_add(source_copy as u64)
                .ok_or(BackupError::Overflow)?,
            24,
        )?;
        let mut source_records = Vec::new();
        source_records
            .try_reserve_exact(12)
            .map_err(|_| invalid())?;
        for (name, raw) in material.evidence() {
            source_records.push(FileRecord {
                path: name.clone(),
                size: raw.len() as u64,
                sha256: hash(raw),
            });
        }
        let source_generation = source_records
            .iter()
            .find(|r| r.path == "source-generation.json")
            .ok_or_else(invalid)?
            .sha256
            .clone();
        if source_generation != self.effects.record.source_registry_generation_sha256 {
            return Err(invalid());
        }
        let group = material.group_id_data()?;
        if self.effects.record.files.len() >= profile::FILES as usize {
            return Err(invalid());
        }
        let effect_copy = self
            .effects
            .record
            .files
            .iter()
            .try_fold(256usize, |n, r| {
                n.checked_add(r.path.len() + r.sha256.len() + 2 * std::mem::size_of::<FileRecord>())
                    .ok_or(BackupError::Overflow)
            })?;
        self.charge(effect_copy as u64, self.effects.record.files.len() + 1)?;
        let mut effect_records = self.effects.record.files.clone();
        let capture_bytes = files::encode(
            &self.effects.record,
            &self.effects.budget,
            &self.effects.journal_budget,
        )?;
        effect_records.push(FileRecord {
            path: "capture.json".into(),
            size: capture_bytes.len() as u64,
            sha256: hash(&capture_bytes),
        });
        effect_records.sort_by(|a, b| a.path.cmp(&b.path));
        validate_roster(&source_records)?;
        validate_roster(&effect_records)?;
        let effect_bytes = effect_records.iter().try_fold(0u64, |n, r| {
            n.checked_add(r.size).ok_or(BackupError::Overflow)
        })?;
        if evidence_bytes
            .checked_add(effect_bytes)
            .is_none_or(|n| n > profile::EVIDENCE_BYTES)
            || source_records.len() + effect_records.len() + self.pin.manifest().files.len() + 1
                > profile::FILES as usize
        {
            return Err(BackupError::Capacity("offhost B1 original captured files"));
        }
        // Existing adjunct verifier reads every original byte under the same
        // actual cut; no transported copy is reconstructed into an authority.
        files::exact_tree(
            &self.effects.directory,
            &effect_records,
            &self.effects.budget,
            &self.effects.journal_budget,
        )?;

        // The existing manifest uses its original typed field order, unlike
        // the lexical JSON records. Pay before serialization/Vec allocation.
        let manifest_raw = self.encode_typed(self.pin.manifest(), 64 * 1024 * 1024)?;
        self.charge(
            (manifest_raw.len() as u64)
                .checked_mul(4)
                .ok_or(BackupError::Overflow)?,
            4,
        )?;
        if hash(&manifest_raw) != self.pin.sealed().manifest_sha256() {
            return Err(invalid());
        }
        self.pin_directory = Some(material.pin_root().open_dir(&format!("{backup}.sealed"))?);
        self.manifest_file = Some(
            self.pin_directory
                .as_ref()
                .expect("retained original pin directory")
                .open_file("manifest.json")?,
        );
        read_exact_original(
            self.manifest_file
                .as_mut()
                .expect("retained original manifest FD"),
            &manifest_raw,
        )?;
        self.verify_pin(
            self.pin_directory
                .as_ref()
                .expect("retained original pin directory"),
            &manifest_raw,
        )?;
        let roster_raw = self.encode(&source_records, 1024 * 1024)?;
        let effects_roster_raw = self.encode(&effect_records, 1024 * 1024)?;
        let data = CapturedData {
            schema_version: 1,
            capability: "b1_captured_input_data_only_v1",
            job_id: job,
            backup_id: backup,
            capture_start_sha256: hex::encode(capture_start),
            capture_generation_sha256: hex::encode(capture_generation),
            source_group_id: group,
            source_registry_generation_sha256: source_generation,
            manifest_sha256: hash(&manifest_raw),
            captured_roster_sha256: hash(&roster_raw),
            snapshot_effects_roster_sha256: hash(&effects_roster_raw),
        };
        let data_raw = self.encode(&data, 4096)?;

        // Data-only mirror lives under the original control root. Writing in
        // snapshot-export-v2 would alter the cut's protected namespace.
        let staging_name = format!("{backup}.b1-captured.staging");
        let final_name = format!("{backup}.b1-captured");
        self.charge(0, 8)?;
        self.evidence = Some(material.control().create_dir(&staging_name)?);
        let staging = self
            .evidence
            .as_ref()
            .expect("retained original evidence directory");
        for (name, raw) in material.evidence() {
            files::put(
                staging,
                name,
                raw,
                &self.effects.budget,
                &self.effects.journal_budget,
            )?;
        }
        files::exact_tree(
            staging,
            &source_records,
            &self.effects.budget,
            &self.effects.journal_budget,
        )?;
        files::seal_tree(staging)?;
        material
            .control()
            .rename_noreplace_without_sync(&staging_name, &final_name)?;
        material.control().sync()?;
        self.charge(0, 4)?;
        let named = material.control().open_dir(&final_name)?;
        if named.identity()? != staging.identity()? {
            return Err(invalid());
        }
        files::exact_tree(
            &named,
            &source_records,
            &self.effects.budget,
            &self.effects.journal_budget,
        )?;
        self.effects.b1_export_material()?;
        self.charge(data_raw.len() as u64, 1)?;
        let digest: [u8; 32] = Sha256::digest(&data_raw).into();
        self.captured_data_sha256 = Some(digest);
        Ok(digest)
    }

    fn verify_pin(&self, directory: &BackupDir, manifest_raw: &[u8]) -> Result<(), BackupError> {
        let files = &self.pin.manifest().files;
        let package_bytes = files.iter().try_fold(manifest_raw.len() as u64, |n, r| {
            n.checked_add(r.size).ok_or(BackupError::Overflow)
        })?;
        if package_bytes > profile::PACKAGE_BYTES || files.len() + 1 > profile::FILES as usize {
            return Err(BackupError::Capacity("offhost B1 original package"));
        }
        let copy_bytes = files.iter().try_fold(256usize, |n, r| {
            n.checked_add(r.path.len() + r.sha256.len() + 2 * std::mem::size_of::<FileRecord>())
                .ok_or(BackupError::Overflow)
        })?;
        self.charge(copy_bytes as u64, files.len() + 1)?;
        let mut records = files.clone();
        records.push(FileRecord {
            path: "manifest.json".into(),
            size: manifest_raw.len() as u64,
            sha256: hash(manifest_raw),
        });
        records.sort_by(|a, b| a.path.cmp(&b.path));
        validate_roster(&records)?;
        self.verify_pin_tree(directory, "", &records)
    }

    fn verify_pin_tree(
        &self,
        directory: &BackupDir,
        prefix: &str,
        records: &[FileRecord],
    ) -> Result<(), BackupError> {
        self.charge(0, 2)?;
        let identity = directory.identity()?;
        directory.require_sealed_directory()?;
        let relevant = records.iter().filter_map(|r| r.path.strip_prefix(prefix));
        let names_bytes = relevant.clone().try_fold(0usize, |n, path| {
            n.checked_add(path.len() + 128).ok_or(BackupError::Overflow)
        })?;
        self.charge(names_bytes as u64, records.len())?;
        let expected: BTreeSet<String> = relevant
            .map(|path| path.split('/').next().unwrap_or("").to_owned())
            .collect();
        if expected.contains("") {
            return Err(invalid());
        }
        self.charge(names_bytes as u64, expected.len())?;
        let before = directory.list_bounded(expected.len())?;
        if before
            .iter()
            .map(String::as_str)
            .ne(expected.iter().map(String::as_str))
        {
            return Err(invalid());
        }
        for name in &before {
            self.charge((prefix.len() + name.len() + 1) as u64, 4)?;
            let path = format!("{prefix}{name}");
            match directory.kind(name)? {
                BackupEntryKind::Directory => {
                    let child = directory.open_dir(name)?;
                    let child_identity = child.identity()?;
                    self.verify_pin_tree(&child, &format!("{path}/"), records)?;
                    if directory.open_dir(name)?.identity()? != child_identity {
                        return Err(invalid());
                    }
                }
                BackupEntryKind::File => {
                    let record = records
                        .iter()
                        .find(|r| r.path == path)
                        .ok_or_else(invalid)?;
                    self.charge(
                        record
                            .size
                            .checked_mul(2)
                            .and_then(|n| n.checked_add(1))
                            .ok_or(BackupError::Overflow)?,
                        4,
                    )?;
                    let mut file = directory.open_file(name)?;
                    let before = file.metadata()?;
                    if !before.is_file()
                        || before.uid() != 0
                        || before.gid() != 0
                        || before.mode() & 0o7777 != 0o400
                        || before.nlink() != 1
                        || before.len() != record.size
                    {
                        return Err(invalid());
                    }
                    let mut hash = Sha256::new();
                    let mut buffer = [0u8; 64 * 1024];
                    let mut size = 0u64;
                    loop {
                        let remaining = record.size.checked_sub(size).ok_or_else(invalid)?;
                        let allowance = buffer
                            .len()
                            .min(usize::try_from(remaining + 1).map_err(|_| invalid())?);
                        let count = file.read(&mut buffer[..allowance])?;
                        if count == 0 {
                            break;
                        }
                        size = size
                            .checked_add(count as u64)
                            .ok_or(BackupError::Overflow)?;
                        if size > record.size {
                            return Err(invalid());
                        }
                        hash.update(&buffer[..count]);
                    }
                    if size != record.size
                        || hex::encode(hash.finalize()) != record.sha256
                        || !same_stamp(&before, &file.metadata()?)
                        || !same_stamp(&before, &directory.open_file(name)?.metadata()?)
                    {
                        return Err(invalid());
                    }
                }
                BackupEntryKind::Other => return Err(invalid()),
            }
        }
        self.charge(names_bytes as u64, expected.len() + 2)?;
        if directory.identity()? != identity || directory.list_bounded(expected.len())? != before {
            return Err(invalid());
        }
        Ok(())
    }

    pub(super) fn require_stop_data(
        &self,
        stop: &learning_db::snapshot_broker::B1CaptureStopRequestedData,
    ) -> Result<(), BackupError> {
        if stop.backup_id() != self.pin.sealed().backup_id()
            || Some(stop.captured_data_sha256()) != self.captured_data_sha256
        {
            return Err(invalid());
        }
        Ok(())
    }
}

fn invalid() -> BackupError {
    BackupError::Invalid("B1 original Capture export rejected")
}
fn hash(raw: &[u8]) -> String {
    hex::encode(Sha256::digest(raw))
}

fn read_exact_original(file: &mut File, expected: &[u8]) -> Result<(), BackupError> {
    let before = file.metadata()?;
    if !before.is_file()
        || before.uid() != 0
        || before.gid() != 0
        || before.mode() & 0o7777 != 0o400
        || before.nlink() != 1
        || before.len() != expected.len() as u64
    {
        return Err(invalid());
    }
    let mut chunk = [0u8; 8192];
    let mut offset = 0;
    while offset < expected.len() {
        let count = chunk.len().min(expected.len() - offset);
        file.read_exact(&mut chunk[..count])?;
        if chunk[..count] != expected[offset..offset + count] {
            return Err(invalid());
        }
        offset += count;
    }
    if file.read(&mut chunk[..1])? != 0 {
        return Err(invalid());
    }
    let after = file.metadata()?;
    if !same_stamp(&before, &after) {
        return Err(invalid());
    }
    Ok(())
}

fn same_stamp(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    (
        a.dev(),
        a.ino(),
        a.uid(),
        a.gid(),
        a.mode(),
        a.nlink(),
        a.len(),
        a.mtime(),
        a.mtime_nsec(),
        a.ctime(),
        a.ctime_nsec(),
    ) == (
        b.dev(),
        b.ino(),
        b.uid(),
        b.gid(),
        b.mode(),
        b.nlink(),
        b.len(),
        b.mtime(),
        b.mtime_nsec(),
        b.ctime(),
        b.ctime_nsec(),
    )
}

fn validate_roster(records: &[FileRecord]) -> Result<(), BackupError> {
    if records.is_empty()
        || records.len() > profile::FILES as usize
        || records.iter().any(|r| {
            r.size == 0
                || !crate::valid_digest(&r.sha256)
                || !crate::source::snapshot_effects::wire::path(&r.path)
        })
        || records.windows(2).any(|r| r[0].path >= r[1].path)
    {
        return Err(invalid());
    }
    Ok(())
}

fn validate_source_names<'a>(names: impl Iterator<Item = &'a str>) -> Result<(), BackupError> {
    let names: BTreeSet<_> = names.collect();
    let roots: Vec<_> = names
        .iter()
        .filter_map(|name| name.strip_prefix("roots/"))
        .collect();
    if names.len() != 12
        || !SOURCE_FILES.iter().all(|name| names.contains(name))
        || roots.len() != 4
        || roots
            .iter()
            .any(|name| !name.strip_suffix(".json").is_some_and(crate::registry::v4))
    {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_twelve_source_names_reject_extra_or_non_v4_root() {
        let mut names: Vec<_> = SOURCE_FILES.iter().map(|name| (*name).to_owned()).collect();
        for n in 1..=4 {
            names.push(format!("roots/{n:08x}-1111-4111-8111-111111111111.json"));
        }
        assert!(validate_source_names(names.iter().map(String::as_str)).is_ok());
        names.push("captured-data.json".into());
        assert!(validate_source_names(names.iter().map(String::as_str)).is_err());
        names.pop();
        names[8] = "roots/00000000-0000-0000-0000-000000000000.json".into();
        assert!(validate_source_names(names.iter().map(String::as_str)).is_err());
    }
    #[test]
    fn original_rosters_require_complete_sorted_unique_file_refs() {
        let a = FileRecord {
            path: "a.json".into(),
            size: 1,
            sha256: "1".repeat(64),
        };
        let b = FileRecord {
            path: "b.json".into(),
            size: 2,
            sha256: "2".repeat(64),
        };
        assert!(validate_roster(&[a.clone(), b.clone()]).is_ok());
        assert!(validate_roster(&[b, a.clone()]).is_err());
        assert!(validate_roster(&[a.clone(), a]).is_err());
        assert!(validate_roster(&[]).is_err());
    }
}

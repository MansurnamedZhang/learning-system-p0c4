//! Destination-local captured evidence. No source filesystem access on recovery.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod attempt;
#[cfg(target_os = "linux")]
pub(crate) mod snapshot_effects;
#[cfg(any(test, target_os = "linux"))]
use crate::registry::canonical;
use crate::{
    BackupError, BackupManifestV1, CompleteBackup, EnrolledDestination, FileRecord, GatePhase,
    ProtectionSnapshot, RegistryLease, SourceGateRecord, VerifierTrustConfig, digest,
    registry::{self, Authority, Generation, RootRecord, SourceGroup, canonical_record, v4},
    valid_digest,
};
#[cfg(target_os = "linux")]
use crate::{DestinationStatement, DestinationWitness};
#[cfg(target_os = "linux")]
pub(crate) use attempt::ErrorFence as PublicationErrorFence;
pub(crate) use attempt::OperationAuthority;
#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
pub(crate) use attempt::remote::Consumption as RemoteConsumption;
#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
pub(crate) use b1_signer::B1DestinationSignerOriginal;
#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
pub(crate) use remote_finalization::{
    CommittedRemoteOriginal, OriginalRemoteFinalization, PreparedRemoteDestinationOriginal,
    RemoteFinalBindingData, prepare_original_remote_destination,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProofRecord {
    format_version: u32,
    capability: String,
    backup_id: Uuid,
    manifest_sha256: String,
    source_control_sha256: String,
    source_group_id: String,
    source_registry_generation_sha256: String,
    protection_snapshot_sha256: String,
    files: Vec<FileRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SnapshotRecord {
    pub(crate) format_version: u32,
    pub(crate) capability: String,
    pub(crate) source_group_id: String,
    pub(crate) registry_generation_sha256: String,
    pub(crate) records: Vec<FileRecord>,
    pub(crate) protected_digest_count: u64,
    pub(crate) scan_bytes: u64,
}

/// Validated destination-owned historical evidence. Not restore authority alone.
#[derive(Debug)]
pub struct CapturedSourceProof {
    record: ProofRecord,
}
impl CapturedSourceProof {
    pub fn backup_id(&self) -> Uuid {
        self.record.backup_id
    }
    pub fn manifest_sha256(&self) -> &str {
        &self.record.manifest_sha256
    }
}

const FIXED_FILES: [&str; 8] = [
    "protection-catalog.json",
    "protection-pending.json",
    "protection-retained.json",
    "protection-snapshot.json",
    "source-authority.json",
    "source-generation.json",
    "source-group.json",
    "source-released.json",
];

fn file_cap(path: &str) -> u64 {
    match path {
        "source-released.json" => 4096,
        "protection-snapshot.json" => crate::MAX_ASSET_INDEX_BYTES as u64,
        "source-generation.json" => 524288,
        _ => 16384,
    }
}

/// Fixed legacy single-file caps only. The receiver supplies an already
/// derived exact12 logical name; this DATA lookup grants no path or authority.
#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
pub(crate) fn source_evidence_file_cap_data(path: &str) -> u64 {
    file_cap(path)
}

fn validate_envelope(record: &ProofRecord) -> Result<(), BackupError> {
    if record.format_version != 1
        || record.capability != "captured_source_proof_v1"
        || !v4(&record.backup_id.to_string())
        || !v4(&record.source_group_id)
        || [
            &record.manifest_sha256,
            &record.source_control_sha256,
            &record.source_registry_generation_sha256,
            &record.protection_snapshot_sha256,
        ]
        .iter()
        .any(|v| !valid_digest(v))
        || record.files.len() != 12
    {
        return Err(BackupError::Invalid(
            "captured source proof identity or inventory",
        ));
    }
    let mut previous = "";
    let mut fixed = BTreeSet::new();
    let mut roots = 0;
    for file in &record.files {
        if file.path.as_str() <= previous
            || file.size == 0
            || file.size > file_cap(&file.path)
            || !valid_digest(&file.sha256)
        {
            return Err(BackupError::Invalid("captured source proof file reference"));
        }
        previous = &file.path;
        if FIXED_FILES.contains(&file.path.as_str()) {
            fixed.insert(file.path.as_str());
        } else if file
            .path
            .strip_prefix("roots/")
            .and_then(|v| v.strip_suffix(".json"))
            .is_some_and(v4)
        {
            roots += 1;
        } else {
            return Err(BackupError::Invalid("captured source proof path"));
        }
    }
    if fixed.len() != 8 || roots != 4 {
        return Err(BackupError::Invalid(
            "captured source proof exact inventory",
        ));
    }
    Ok(())
}

fn checked_files(
    record: &ProofRecord,
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<(), BackupError> {
    validate_envelope(record)?;
    if files.len() != record.files.len() {
        return Err(BackupError::Invalid(
            "captured source proof extra or missing file",
        ));
    }
    for entry in &record.files {
        let raw = files
            .get(&entry.path)
            .ok_or(BackupError::Invalid("captured source proof missing file"))?;
        if raw.len() as u64 != entry.size || digest(raw) != entry.sha256 {
            return Err(BackupError::Invalid("captured source proof changed file"));
        }
    }
    Ok(())
}

// Fixed canonical DATA encoder shared by the real Source pin view and the
// existing D preparation. It constructs no proof capability or operation.
#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
pub(crate) fn captured_proof_record_bytes_from_files(
    backup_id: Uuid,
    manifest: &BackupManifestV1,
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<u8>, BackupError> {
    let record = proof_record_from_files(backup_id, manifest, files)?;
    let raw = canonical(&record)?;
    if raw.len() > 16_384 {
        return Err(BackupError::Capacity("captured source proof record"));
    }
    Ok(raw)
}

#[cfg(target_os = "linux")]
fn proof_record_from_files(
    backup_id: Uuid,
    manifest: &BackupManifestV1,
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<ProofRecord, BackupError> {
    if !v4(&backup_id.to_string()) || manifest.backup_id != backup_id || files.len() != 12 {
        return Err(BackupError::Invalid("captured proof DATA input identity"));
    }
    for (name, raw) in files {
        if raw.is_empty() || raw.len() as u64 > file_cap(name) {
            return Err(BackupError::Capacity("captured proof DATA input bytes"));
        }
    }
    let required = |name: &str| {
        files.get(name).ok_or(BackupError::Invalid(
            "captured proof DATA missing fixed file",
        ))
    };
    let group: SourceGroup = canonical_record(required("source-group.json")?, 16_384)?;
    let record = ProofRecord {
        format_version: 1,
        capability: "captured_source_proof_v1".into(),
        backup_id,
        manifest_sha256: manifest.canonical_sha256()?,
        source_control_sha256: digest(required("source-released.json")?),
        source_group_id: group.group_id,
        source_registry_generation_sha256: digest(required("source-generation.json")?),
        protection_snapshot_sha256: digest(required("protection-snapshot.json")?),
        files: files
            .iter()
            .map(|(path, raw)| FileRecord {
                path: path.clone(),
                size: raw.len() as u64,
                sha256: digest(raw),
            })
            .collect(),
    };
    checked_files(&record, files)?;
    Ok(record)
}

enum EvidenceLocation<'a> {
    Destination(&'a str),
    #[cfg(test)]
    CapturedSource(&'a str),
}
trait EvidenceAccess {
    fn read(&mut self, location: EvidenceLocation<'_>) -> Result<Vec<u8>, BackupError>;
}
fn reopen_evidence(
    access: &mut impl EvidenceAccess,
    id: Uuid,
    manifest: &BackupManifestV1,
    index: &[u8],
    trust: &VerifierTrustConfig,
) -> Result<CapturedSourceProof, BackupError> {
    let raw = access.read(EvidenceLocation::Destination("source-proof.json"))?;
    let record: ProofRecord = canonical_record(&raw, 16384)?;
    validate_envelope(&record)?;
    if record.backup_id != id {
        return Err(BackupError::Invalid("captured backup directory identity"));
    }
    let mut files = BTreeMap::new();
    for reference in &record.files {
        files.insert(
            reference.path.clone(),
            access.read(EvidenceLocation::Destination(&reference.path))?,
        );
    }
    validate_copies(&record, &files, manifest, index, trust)?;
    Ok(CapturedSourceProof { record })
}

fn validate_copies(
    record: &ProofRecord,
    files: &BTreeMap<String, Vec<u8>>,
    manifest: &BackupManifestV1,
    index: &[u8],
    trust: &VerifierTrustConfig,
) -> Result<(), BackupError> {
    checked_files(record, files)?;
    manifest.validate_with_index(index)?;
    if manifest.backup_id != record.backup_id
        || manifest.canonical_sha256()? != record.manifest_sha256
    {
        return Err(BackupError::Invalid("captured manifest binding"));
    }
    let authority: Authority = canonical_record(&files["source-authority.json"], 4096)?;
    let generation: Generation = canonical_record(&files["source-generation.json"], 524288)?;
    let group: SourceGroup = canonical_record(&files["source-group.json"], 16384)?;
    if authority.format_version != 1
        || authority.capability != "backup_registry_v1"
        || !v4(&authority.deployment_id)
        || authority.registry_path != registry::REGISTRY_PATH
        || authority.initial_generation != 1
        || !valid_digest(&authority.initial_generation_sha256)
        || authority.registry_dev == 0
        || authority.registry_ino == 0
        || authority.lock_dev == 0
        || authority.lock_ino == 0
        || authority
            .directories
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>()
            != registry::DIRECTORY_NAMES
        || authority
            .directories
            .values()
            .any(|v| v.dev == 0 || v.ino == 0)
        || generation.format_version != 1
        || generation.capability != "backup_registry_generation_v1"
        || generation.deployment_id != authority.deployment_id
        || generation.generation == 0
        || generation.generation > 128
        || (generation.generation == 1
            && (generation.previous_generation_sha256.is_some()
                || digest(&files["source-generation.json"]) != authority.initial_generation_sha256))
        || (generation.generation > 1
            && !generation
                .previous_generation_sha256
                .as_deref()
                .is_some_and(valid_digest))
        || digest(&files["source-generation.json"]) != record.source_registry_generation_sha256
        || group.format_version != 1
        || group.capability != "backup_source_group_v1"
        || group.deployment_id != authority.deployment_id
        || group.group_id != record.source_group_id
        || group.database_oid == 0
        || !crate::maintenance::valid_c4_database(&group.database)
        || !group
            .system_identifier
            .parse::<u64>()
            .is_ok_and(|v| v > 0 && v.to_string() == group.system_identifier)
        || !valid_digest(&group.source_binding_sha256)
        || group.application_commit != manifest.source.application_commit
        || group.application_build_sha256 != manifest.source.application_build_sha256
        || group.roots.len() != 4
    {
        return Err(BackupError::Invalid("captured registry historical linkage"));
    }
    for (roster, cap) in [(&generation.roots, 1024), (&generation.groups, 256)] {
        if roster.len() > cap
            || roster.windows(2).any(|w| w[0].id >= w[1].id)
            || roster
                .iter()
                .any(|r| !v4(&r.id) || !valid_digest(&r.sha256))
        {
            return Err(BackupError::Invalid("captured registry roster"));
        }
    }
    if !generation
        .groups
        .iter()
        .any(|r| r.id == group.group_id && r.sha256 == digest(&files["source-group.json"]))
    {
        return Err(BackupError::Invalid("captured source group roster"));
    }
    let mut paths = BTreeSet::new();
    let mut identities = BTreeSet::new();
    for (kind, expected) in [
        ("assets", "source_assets"),
        ("control", "source_control"),
        ("local_pins", "local_pins"),
        ("staging", "source_staging"),
    ] {
        let id = group
            .roots
            .get(kind)
            .filter(|id| v4(id))
            .ok_or(BackupError::Invalid("captured source roots"))?;
        let bytes = files
            .get(&format!("roots/{id}.json"))
            .ok_or(BackupError::Invalid("captured root absent"))?;
        let root: RootRecord = canonical_record(bytes, 16384)?;
        if root.format_version != 1
            || root.capability != "backup_root_v1"
            || root.enrollment_id != *id
            || root.group_id != group.group_id
            || root.deployment_id != authority.deployment_id
            || root.kind != expected
            || !registry::canonical_path(&root.path)
            || root.dev == 0
            || root.ino == 0
            || root.uid != 0
            || root.mode != 448
            || root.enrolled_generation == 0
            || root.enrolled_generation > generation.generation
            || !paths.insert(root.path.clone())
            || !identities.insert((root.dev, root.ino))
            || !generation
                .roots
                .iter()
                .any(|r| r.id == *id && r.sha256 == digest(bytes))
            || (kind == "control" && root.path != trust.source_control_root)
            || (kind == "local_pins" && root.path != trust.source_root)
        {
            return Err(BackupError::Invalid("captured source root linkage"));
        }
    }
    let released_bytes = &files["source-released.json"];
    if released_bytes.len() > 4096 {
        return Err(BackupError::Capacity("captured journal v1 record"));
    }
    let released =
        decode_source_released_record(released_bytes, record.backup_id, &record.manifest_sha256)?;
    released.validate_for(record.backup_id)?;
    if released.phase() != GatePhase::Released
        || released.pinned_manifest_sha256() != Some(&record.manifest_sha256)
        || serde_json::to_value(&released)?["dump_and_index_sha256"] != record.manifest_sha256
        || digest(&files["source-released.json"]) != record.source_control_sha256
    {
        return Err(BackupError::Invalid("captured source release binding"));
    }
    let snapshot: SnapshotRecord = canonical_record(
        &files["protection-snapshot.json"],
        crate::MAX_ASSET_INDEX_BYTES as u64,
    )?;
    if snapshot.format_version != 1
        || snapshot.capability != "protection_snapshot_v1"
        || snapshot.source_group_id != group.group_id
        || snapshot.registry_generation_sha256 != record.source_registry_generation_sha256
        || snapshot.protected_digest_count > 100000
        || snapshot.scan_bytes > registry::MAX_SCAN_BYTES
        || snapshot.records.len() > registry::MAX_ENTRIES
        || digest(&files["protection-snapshot.json"]) != record.protection_snapshot_sha256
        || snapshot.records.windows(2).any(|w| w[0].path >= w[1].path)
    {
        return Err(BackupError::Invalid(
            "captured protection snapshot limits or linkage",
        ));
    }
    let mut total = 0u64;
    for item in &snapshot.records {
        if !relative_metadata_path(&item.path)
            || !valid_digest(&item.sha256)
            || item.size > crate::MAX_ASSET_INDEX_BYTES as u64
        {
            return Err(BackupError::Invalid(
                "captured protection snapshot reference",
            ));
        }
        total = total.checked_add(item.size).ok_or(BackupError::Overflow)?;
    }
    if total > snapshot.scan_bytes {
        return Err(BackupError::Invalid("captured protection scan accounting"));
    }
    validate_protection_chain(
        record,
        files,
        &group,
        &generation,
        manifest,
        index,
        &snapshot,
    )?;
    Ok(())
}

// Version only this inner record. The outer CapturedSourceProof remains V1 with
// exactly twelve files and its original hash/protection checks. No failed V2
// parse is retried as V1, and no caller selects a schema/profile fallback.
fn decode_source_released_record(
    bytes: &[u8],
    backup_id: Uuid,
    manifest_sha256: &str,
) -> Result<SourceGateRecord, BackupError> {
    if bytes.is_empty() || bytes.len() > 4096 {
        return Err(BackupError::Capacity("captured source release record"));
    }
    let shape: serde_json::Value = serde_json::from_slice(bytes)?;
    match shape.get("format_version") {
        None => {
            let record: SourceGateRecord = serde_json::from_slice(bytes)?;
            if serde_json::to_vec(&record)? != bytes {
                return Err(BackupError::Invalid("captured journal v1 record bytes"));
            }
            Ok(record)
        }
        Some(version) if version.as_u64() == Some(2) => {
            crate::source::c0_d1::validate_full_c_released_record(bytes, backup_id, manifest_sha256)
        }
        Some(_) => Err(BackupError::Invalid(
            "captured source release record version",
        )),
    }
}

fn relative_metadata_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\\', '\0', ':'])
        && path
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != "..")
}

fn snapshot_ref(snapshot: &SnapshotRecord, path: &str, bytes: &[u8]) -> Result<(), BackupError> {
    if !snapshot
        .records
        .iter()
        .any(|r| r.path == path && r.size == bytes.len() as u64 && r.sha256 == digest(bytes))
    {
        return Err(BackupError::Invalid(
            "captured snapshot reference missing or changed",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoricalProtection {
    format_version: u32,
    capability: String,
    deployment_id: String,
    group_id: String,
    backup_id: String,
    registry_generation: u64,
    registry_generation_sha256: String,
    source_binding_sha256: String,
    application_commit: String,
    application_build_sha256: String,
    state: String,
    previous_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    roots: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    asset_index: Option<crate::FileRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    logical_asset_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    asset_index_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    manifest_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sealed_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sealed_dev: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sealed_ino: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    abandonment_ready_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_journal_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    abandonment_terminal_sha256: Option<String>,
}
fn validate_protection_chain(
    record: &ProofRecord,
    files: &BTreeMap<String, Vec<u8>>,
    group: &SourceGroup,
    generation: &Generation,
    manifest: &BackupManifestV1,
    index: &[u8],
    snapshot: &SnapshotRecord,
) -> Result<(), BackupError> {
    let mut predecessor = None;
    let mut capture_generation = None;
    for (n, (name, original, state, extras)) in [
        (
            "protection-pending.json",
            "00-pending.json",
            "capture_pending",
            vec!["roots"],
        ),
        (
            "protection-catalog.json",
            "10-catalog.json",
            "catalog_durable",
            vec!["asset_index", "logical_asset_count"],
        ),
        (
            "protection-retained.json",
            "20-retained.json",
            "retained",
            vec![
                "asset_index_sha256",
                "manifest_sha256",
                "sealed_name",
                "sealed_dev",
                "sealed_ino",
            ],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let raw = &files[name];
        // Value parsing is for historical fields only; require exact keys and original serialization order is preserved in hashes.
        let parsed: HistoricalProtection = canonical_record(raw, 16384)?;
        let value = serde_json::to_value(parsed)?;
        let object = value
            .as_object()
            .ok_or(BackupError::Invalid("captured protection record"))?;
        let mut keys = vec![
            "format_version",
            "capability",
            "deployment_id",
            "group_id",
            "backup_id",
            "registry_generation",
            "registry_generation_sha256",
            "source_binding_sha256",
            "application_commit",
            "application_build_sha256",
            "state",
            "previous_sha256",
        ];
        keys.extend(extras);
        if object.len() != keys.len()
            || keys.iter().any(|k| !object.contains_key(*k))
            || value["format_version"] != 1
            || value["capability"] != "source_protection_v1"
            || value["deployment_id"] != group.deployment_id
            || value["group_id"] != group.group_id
            || value["backup_id"] != record.backup_id.to_string()
            || value["state"] != state
            || value["source_binding_sha256"] != group.source_binding_sha256
            || value["application_commit"] != group.application_commit
            || value["application_build_sha256"] != group.application_build_sha256
            || value["previous_sha256"]
                != predecessor
                    .clone()
                    .map(serde_json::Value::String)
                    .unwrap_or(serde_json::Value::Null)
        {
            return Err(BackupError::Invalid("captured protection chain"));
        }
        let gen_number = value["registry_generation"]
            .as_u64()
            .filter(|v| *v > 0 && *v <= generation.generation)
            .ok_or(BackupError::Invalid("captured protection generation"))?;
        let gen_hash = value["registry_generation_sha256"]
            .as_str()
            .filter(|v| valid_digest(v))
            .ok_or(BackupError::Invalid("captured protection generation hash"))?;
        if capture_generation
            .as_ref()
            .is_some_and(|v| v != &(gen_number, gen_hash.to_owned()))
            || !snapshot.records.iter().any(|r| {
                r.path == format!("registry/generations/{gen_number:020}.json")
                    && r.sha256 == gen_hash
            })
        {
            return Err(BackupError::Invalid(
                "captured protection generation linkage",
            ));
        }
        capture_generation = Some((gen_number, gen_hash.to_owned()));
        match n {
            0 if value["roots"] == serde_json::to_value(&group.roots)? => (),
            1 if value["asset_index"]
                == serde_json::to_value(
                    manifest
                        .files
                        .iter()
                        .find(|r| r.path == "asset-index.json")
                        .ok_or(BackupError::Invalid("captured index absent"))?,
                )?
                && value["logical_asset_count"] == manifest.logical_asset_count => {}
            2 if value["asset_index_sha256"] == digest(index)
                && value["manifest_sha256"] == record.manifest_sha256
                && value["sealed_name"] == format!("{}.sealed", record.backup_id)
                && value["sealed_dev"].as_u64().is_some_and(|v| v > 0)
                && value["sealed_ino"].as_u64().is_some_and(|v| v > 0) => {}
            _ => return Err(BackupError::Invalid("captured protection stage fields")),
        }
        snapshot_ref(
            snapshot,
            &format!("protection/{}/{original}", record.backup_id),
            raw,
        )?;
        predecessor = Some(digest(raw));
    }
    snapshot_ref(
        snapshot,
        &format!("protection/{}/asset-index.json", record.backup_id),
        index,
    )?;
    snapshot_ref(
        snapshot,
        &format!("control/{}.control/released.json", record.backup_id),
        &files["source-released.json"],
    )?;
    snapshot_ref(
        snapshot,
        "registry/authority.json",
        &files["source-authority.json"],
    )?;
    snapshot_ref(
        snapshot,
        &format!("registry/groups/{}.json", group.group_id),
        &files["source-group.json"],
    )?;
    for id in group.roots.values() {
        snapshot_ref(
            snapshot,
            &format!("registry/roots/{id}.json"),
            &files[&format!("roots/{id}.json")],
        )?;
    }
    let unique = manifest
        .files
        .iter()
        .filter(|r| r.path.starts_with("assets/"))
        .count() as u64;
    if snapshot.protected_digest_count < unique {
        return Err(BackupError::Invalid("captured protected digest count"));
    }
    Ok(())
}

/// Fixed installed management namespace; there are no caller path/key overrides.
pub const DESTINATION_TRUST_PATH: &str = "/var/lib/knowweave-c4/verifier/trust.json";
const KEY_ROOT: &str = "/var/lib/knowweave-c4/verifier";

pub struct DestinationSigner {
    #[cfg(target_os = "linux")]
    key: ring::signature::EcdsaKeyPair,
    #[cfg(target_os = "linux")]
    trust: VerifierTrustConfig,
}
impl DestinationSigner {
    pub fn open_installed() -> Result<Self, BackupError> {
        #[cfg(target_os = "linux")]
        {
            use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
            use std::io::Read;
            use std::os::unix::fs::MetadataExt;
            let trust = installed_trust()?;
            crate::complete::require_deployment_verifier_key(&trust)?;
            let root = learning_assets::backup_fs::BackupDir::open_trusted_private_root(
                std::path::Path::new(KEY_ROOT),
            )?;
            let file = root.open_file("destination.pk8")?;
            let metadata = file.metadata()?;
            if metadata.uid() != 0
                || metadata.mode() & 0o7777 != 0o600
                || metadata.nlink() != 1
                || metadata.len() > 4096
            {
                return Err(BackupError::Invalid(
                    "installed destination private key custody",
                ));
            }
            let mut raw = Vec::new();
            file.take(4097).read_to_end(&mut raw)?;
            let parsed = EcdsaKeyPair::from_pkcs8(
                &ECDSA_P256_SHA256_FIXED_SIGNING,
                &raw,
                &ring::rand::SystemRandom::new(),
            );
            raw.fill(0);
            let key =
                parsed.map_err(|_| BackupError::Invalid("installed destination private key"))?;
            if hex::encode(key.public_key().as_ref()) != trust.verifier_public_key_hex {
                return Err(BackupError::Invalid(
                    "destination key differs from installed trust",
                ));
            }
            Ok(Self { key, trust })
        }
        #[cfg(not(target_os = "linux"))]
        Err(BackupError::Invalid(
            "destination signer requires Linux and independent deployment",
        ))
    }
}

#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
mod b1_signer {
    use super::*;
    use learning_assets::backup_fs::BackupDir;
    use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
    use std::{
        fs::File,
        io::{Read, Seek, SeekFrom},
        os::unix::fs::MetadataExt,
        sync::{Arc, Mutex},
    };

    #[derive(Clone, Copy, PartialEq, Eq)]
    struct Stamp {
        dev: u64,
        ino: u64,
        uid: u32,
        gid: u32,
        mode: u32,
        links: u64,
        len: u64,
        mtime: i64,
        mtime_ns: i64,
        ctime: i64,
        ctime_ns: i64,
    }
    impl Stamp {
        fn from(value: &std::fs::Metadata) -> Self {
            Self {
                dev: value.dev(),
                ino: value.ino(),
                uid: value.uid(),
                gid: value.gid(),
                mode: value.mode(),
                links: value.nlink(),
                len: value.len(),
                mtime: value.mtime(),
                mtime_ns: value.mtime_nsec(),
                ctime: value.ctime(),
                ctime_ns: value.ctime_nsec(),
            }
        }
        fn require_record(self) -> Result<(), BackupError> {
            if self.mode & libc::S_IFMT != libc::S_IFREG
                || self.mode & 0o7777 != 0o600
                || self.uid != 0
                || self.gid != 0
                || self.links != 1
                || self.len == 0
                || self.len > 4096
            {
                return Err(BackupError::Invalid(
                    "B1 first installed signer file custody",
                ));
            }
            Ok(())
        }
    }

    struct PrivateBytes(Vec<u8>);
    impl Drop for PrivateBytes {
        fn drop(&mut self) {
            self.0.fill(0);
        }
    }

    pub(crate) struct B1DestinationSignerOriginal {
        entry: &'static crate::offhost_b1::destination_actor::OriginalDEntryScope,
        budget: Arc<Mutex<registry::ScanBudget>>,
        root: BackupDir,
        root_identity: (u64, u64),
        key_file: File,
        key_stamp: Stamp,
        key_bytes_sha256: String,
        trust_file: File,
        trust_stamp: Stamp,
        trust_raw: Vec<u8>,
        trust: VerifierTrustConfig,
        key: EcdsaKeyPair,
    }

    impl B1DestinationSignerOriginal {
        pub(crate) fn open_fixed_for_original(
            entry: &'static crate::offhost_b1::destination_actor::OriginalDEntryScope,
        ) -> Result<Arc<Self>, BackupError> {
            let budget = entry.take_fixed_signer_scope()?;
            entry.prepay_wire_work(4096 * 32, 32)?;
            let root = BackupDir::open_trusted_private_root(std::path::Path::new(KEY_ROOT))?;
            root.require_private_directory()?;
            let root_identity = root.identity()?;
            let trust_file = root.open_file("trust.json")?;
            let (trust_stamp, mut trust_bytes) = read_first(&trust_file, &budget)?;
            let trust_raw = std::mem::take(&mut trust_bytes.0);
            registry::prepay_current_b1_json(&budget, &trust_raw)?;
            let trust: VerifierTrustConfig = serde_json::from_slice(&trust_raw)?;
            if trust.canonical_bytes()? != trust_raw {
                return Err(BackupError::Invalid("B1 original verifier trust bytes"));
            }
            crate::complete::require_deployment_verifier_key(&trust)?;
            let key_file = root.open_file("destination.pk8")?;
            let (key_stamp, raw) = read_first(&key_file, &budget)?;
            let key_bytes_sha256 = digest(&raw.0);
            let key = EcdsaKeyPair::from_pkcs8(
                &ECDSA_P256_SHA256_FIXED_SIGNING,
                &raw.0,
                &ring::rand::SystemRandom::new(),
            )
            .map_err(|_| BackupError::Invalid("B1 original verifier PKCS8"))?;
            if hex::encode(key.public_key().as_ref()) != trust.verifier_public_key_hex
                || digest(key.public_key().as_ref()) != entry.verifier_public_key_pin_data()
            {
                return Err(BackupError::Invalid("B1 original verifier public key pins"));
            }
            drop(raw);
            let original = Arc::new(Self {
                entry,
                budget,
                root,
                root_identity,
                key_file,
                key_stamp,
                key_bytes_sha256,
                trust_file,
                trust_stamp,
                trust_raw,
                trust,
                key,
            });
            original.recheck_held()?;
            Ok(original)
        }

        pub(crate) fn recheck_pending_original(
            &self,
            original: &crate::offhost_b1::destination_actor::OriginalAuthenticatedSourceInD,
        ) -> Result<(), BackupError> {
            original.require_destination(original.destination_original())?;
            if !std::ptr::eq(self.entry, original.entry_original())
                || !std::ptr::eq(self, original.signer_original()?.as_ref())
            {
                return Err(BackupError::Invalid(
                    "first B1 signer belongs to another original",
                ));
            }
            self.require_original_budget(&original.destination_original().lease.budget)?;
            self.recheck_held()
        }

        pub(crate) fn trust_data(&self) -> &VerifierTrustConfig {
            &self.trust
        }

        pub(super) fn require_original_budget(
            &self,
            budget: &Arc<Mutex<registry::ScanBudget>>,
        ) -> Result<(), BackupError> {
            if !Arc::ptr_eq(&self.budget, budget) {
                return Err(BackupError::Invalid("B1 signer original cumulative budget"));
            }
            Ok(())
        }

        fn recheck_held(&self) -> Result<(), BackupError> {
            self.entry.prepay_wire_work(4096 * 32, 32)?;
            self.root.require_private_directory()?;
            if self.root.identity()? != self.root_identity
                || BackupDir::open_trusted_private_root(std::path::Path::new(KEY_ROOT))?
                    .identity()?
                    != self.root_identity
            {
                return Err(BackupError::Invalid("B1 signer original root replaced"));
            }
            let trust_raw = reread_original(&self.trust_file, self.trust_stamp, &self.budget)?;
            let key_raw = reread_original(&self.key_file, self.key_stamp, &self.budget)?;
            if trust_raw.0 != self.trust_raw
                || digest(&key_raw.0) != self.key_bytes_sha256
                || Stamp::from(&self.root.open_file("trust.json")?.metadata()?) != self.trust_stamp
                || Stamp::from(&self.root.open_file("destination.pk8")?.metadata()?)
                    != self.key_stamp
            {
                return Err(BackupError::Invalid("B1 original signer/trust changed"));
            }
            Ok(())
        }

        pub(super) fn recheck_consuming(
            &self,
            original: &OriginalRemoteFinalization,
            owner: &RemoteConsumption<'_>,
        ) -> Result<(), BackupError> {
            original.require_own_original_signer(owner, self)?;
            self.recheck_held()
        }

        pub(super) fn recheck_committed_original(
            &self,
            committed: &CommittedRemoteOriginal,
        ) -> Result<(), BackupError> {
            committed.require_own_committed_signer(self)?;
            self.recheck_held()
        }
        pub(super) fn sign_original_consuming(
            &self,
            original: &OriginalRemoteFinalization,
            owner: &RemoteConsumption<'_>,
        ) -> Result<DestinationWitness, BackupError> {
            self.recheck_consuming(original, owner)?;
            let statement = original.statement_for_receipt(owner)?;
            let bytes = statement.canonical_bytes()?;
            if bytes.len() > 8192 {
                return Err(BackupError::Capacity("B1 original signer statement"));
            }
            original.check_deadline()?;
            let signature = self
                .key
                .sign(&ring::rand::SystemRandom::new(), &bytes)
                .map_err(|_| BackupError::Invalid("B1 original destination signature"))?;
            self.recheck_consuming(original, owner)?;
            Ok(DestinationWitness {
                statement,
                signature_hex: hex::encode(signature.as_ref()),
            })
        }
    }

    fn pay(
        budget: &Arc<Mutex<registry::ScanBudget>>,
        bytes: u64,
        entries: usize,
    ) -> Result<(), BackupError> {
        let mut meter = budget
            .lock()
            .map_err(|_| BackupError::Invalid("B1 signer budget poison"))?;
        meter.charge(bytes)?;
        meter.entries(entries)
    }
    fn read_first(
        file: &File,
        budget: &Arc<Mutex<registry::ScanBudget>>,
    ) -> Result<(Stamp, PrivateBytes), BackupError> {
        pay(budget, 0, 2)?;
        let stamp = Stamp::from(&file.metadata()?);
        stamp.require_record()?;
        let raw = reread_original(file, stamp, budget)?;
        Ok((stamp, raw))
    }
    fn reread_original(
        mut file: &File,
        stamp: Stamp,
        budget: &Arc<Mutex<registry::ScanBudget>>,
    ) -> Result<PrivateBytes, BackupError> {
        stamp.require_record()?;
        pay(
            budget,
            stamp
                .len
                .checked_mul(4)
                .and_then(|n| n.checked_add(1))
                .ok_or(BackupError::Overflow)?,
            4,
        )?;
        if Stamp::from(&file.metadata()?) != stamp {
            return Err(BackupError::Invalid("B1 first signer descriptor changed"));
        }
        file.seek(SeekFrom::Start(0))?;
        let mut raw = PrivateBytes(vec![
            0;
            usize::try_from(stamp.len)
                .map_err(|_| BackupError::Overflow)?
        ]);
        file.read_exact(&mut raw.0)?;
        let mut extra = [0];
        if file.read(&mut extra)? != 0 || Stamp::from(&file.metadata()?) != stamp {
            return Err(BackupError::Invalid("B1 first signer bytes changed"));
        }
        Ok(raw)
    }
}

#[derive(Debug)]
pub struct VerifiedDestination {
    #[cfg(target_os = "linux")]
    destination: EnrolledDestination,
    #[cfg(target_os = "linux")]
    statement: DestinationStatement,
    #[cfg(target_os = "linux")]
    proof_sha256: String,
    #[cfg(target_os = "linux")]
    identity: attempt::CommitIdentity,
}
impl VerifiedDestination {
    pub fn sign_statement(&self, signer: &DestinationSigner) -> Result<Vec<u8>, BackupError> {
        #[cfg(target_os = "linux")]
        {
            let fence = attempt::ErrorFence::new(&self.destination.lease.destination_operation);
            self.destination
                .lease
                .destination_operation
                .lock()
                .expect("destination operation")
                .require_id(self.statement.backup_id)?;
            let trust = installed_trust()?;
            crate::complete::require_deployment_verifier_key(&trust)?;
            if trust != signer.trust {
                return Err(BackupError::Invalid("installed destination trust changed"));
            }
            let proof = linux::read_proof(&self.destination, self.statement.backup_id, &trust)?;
            if digest(&canonical(&proof.record)?) != self.proof_sha256
                || statement(&proof, &trust) != self.statement
            {
                return Err(BackupError::Invalid(
                    "verified destination changed before signature",
                ));
            }
            self.destination.recheck()?;
            let statement_bytes = self.statement.canonical_bytes()?;
            let bytes = attempt::finalize_use(
                &self.destination.lease.destination_operation,
                &self.identity,
                || {
                    let signature = signer
                        .key
                        .sign(&ring::rand::SystemRandom::new(), &statement_bytes)
                        .map_err(|_| BackupError::Invalid("destination signature failed"))?;
                    DestinationWitness {
                        statement: self.statement.clone(),
                        signature_hex: hex::encode(signature.as_ref()),
                    }
                    .canonical_bytes()
                },
            )?;
            fence.complete();
            Ok(bytes)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = signer;
            Err(BackupError::Invalid("destination verifier requires Linux"))
        }
    }
}

/// The remote final consumer is deliberately separate from VerifiedDestination.
/// There is no public or test-success constructor and no conversion from a
/// Local proof, signature DATA, or a recovered record. The fixed authenticated
/// stream/eleven-step/adjunct producer below supplies its actual retained owner.
#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
#[allow(
    dead_code,
    reason = "default-off original D; installation, CODE and runtime acceptance remain separate"
)]
mod remote_finalization {
    use super::*;
    use learning_assets::backup_fs::{BackupDir, BackupEntryKind};
    use sha2::{Digest, Sha256};
    use std::{
        io::Read,
        os::unix::fs::MetadataExt,
        sync::{Arc, Mutex},
        time::Instant,
    };

    // DATA projection of the fixed original session/Ready, never a capability.
    // No serde/from-path/boolean selector or signing entry is attached to it.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) struct RemoteFinalBindingData {
        pub(crate) backup_id: Uuid,
        pub(crate) attempt_id: Uuid,
        pub(crate) source_operation_id: Uuid,
        pub(crate) session_binding_sha256: String,
        pub(crate) source_auth_frame_sha256: String,
        pub(crate) source_auth_key_sha256: String,
        pub(crate) s_nonce: String,
        pub(crate) d_nonce: String,
        pub(crate) manifest_sha256: String,
        pub(crate) proof_sha256: String,
        pub(crate) snapshot_effects_proof_sha256: String,
        pub(crate) snapshot_effects_input_sha256: String,
        pub(crate) snapshot_effects_intent_sha256: String,
        pub(crate) fence_anchor_sha256: String,
        pub(crate) protection_snapshot_sha256: String,
    }
    impl RemoteFinalBindingData {
        pub(crate) fn validate_data(&self) -> Result<(), BackupError> {
            if ![self.backup_id, self.attempt_id, self.source_operation_id]
                .iter()
                .all(|id| v4(&id.to_string()))
                || ![
                    &self.session_binding_sha256,
                    &self.source_auth_frame_sha256,
                    &self.source_auth_key_sha256,
                    &self.s_nonce,
                    &self.d_nonce,
                    &self.manifest_sha256,
                    &self.proof_sha256,
                    &self.snapshot_effects_proof_sha256,
                    &self.snapshot_effects_input_sha256,
                    &self.snapshot_effects_intent_sha256,
                    &self.fence_anchor_sha256,
                    &self.protection_snapshot_sha256,
                ]
                .iter()
                .all(|value| valid_digest(value))
            {
                return Err(BackupError::Invalid("remote original Final binding DATA"));
            }
            Ok(())
        }
    }

    /// Retained actual Ready owner. Only the completed authenticated stream,
    /// the original eleven natural steps and the real remote adjunct writer
    /// construct it. It remains in Book even during Final/sign/receipt unwind.
    pub(crate) struct PreparedRemoteDestinationOriginal {
        original: Arc<crate::offhost_b1::destination_actor::OriginalAuthenticatedSourceInD>,
        proof: CapturedSourceProof,
        identity: attempt::CommitIdentity,
        adjunct: Arc<snapshot_effects::RemoteCopiedSnapshotEffectsOriginal>,
        manifest_raw: Vec<u8>,
        destination_roster: Vec<String>,
        evidence_roster: Vec<String>,
    }
    pub(crate) fn prepare_original_remote_destination(
        received: &crate::offhost_b1::destination_actor::OriginalReceivedFiles<'_>,
    ) -> Result<Arc<PreparedRemoteDestinationOriginal>, BackupError> {
        let original = received.original();
        original.require_destination(original.destination_original())?;
        original.prepay_original_work(65_536, 32)?;
        let signer = original.signer_original()?;
        signer.recheck_pending_original(original)?;
        linux::trust_matches(original.destination_original(), signer.trust_data())?;
        let (proof, identity) = linux::persist_original_remote(received)?;
        attempt::remote::require_original_pending(original, &identity)?;
        let adjunct = snapshot_effects::copy_original_remote_effects(
            received,
            &proof,
            &identity,
            signer.trust_data(),
        )?;
        original.prepay_original_work(
            (received.manifest_raw().len() as u64)
                .checked_mul(2)
                .and_then(|n| n.checked_add(65_536))
                .ok_or(BackupError::Overflow)?,
            32,
        )?;
        let destination_roster = registry::prepaid_b1_roster(
            original.destination_original().destination(),
            &original.destination_original().lease.budget,
            registry::MAX_ENTRIES,
        )?;
        let evidence_roster = registry::prepaid_b1_roster(
            original.destination_original().evidence(),
            &original.destination_original().lease.budget,
            registry::MAX_ENTRIES,
        )?;
        attempt::require_committed_names(&evidence_roster, proof.backup_id())?;
        let owner = Arc::new(PreparedRemoteDestinationOriginal {
            original: Arc::clone(original),
            proof,
            identity,
            adjunct: Arc::new(adjunct),
            manifest_raw: received.manifest_raw().to_vec(),
            destination_roster,
            evidence_roster,
        });
        owner.recheck_original_ready()?;
        Ok(owner)
    }
    impl PreparedRemoteDestinationOriginal {
        pub(crate) fn recheck_original_ready(&self) -> Result<(), BackupError> {
            self.original
                .require_destination(self.original.destination_original())?;
            self.original
                .signer_original()?
                .recheck_pending_original(&self.original)?;
            attempt::remote::require_original_pending(&self.original, &self.identity)?;
            self.adjunct
                .recheck_pending_original(&self.original, &self.proof, &self.identity)
        }
        pub(crate) fn ready_witness_bytes(&self) -> Result<&[u8], BackupError> {
            Ok(self.adjunct.witness_bytes())
        }
        pub(crate) fn ready_intent_bytes(&self) -> Result<&[u8], BackupError> {
            Ok(self.adjunct.intent_bytes())
        }
        pub(crate) fn proof_sha256_data(&self) -> &str {
            &self.identity.intent.proof_sha256
        }
        pub(crate) fn manifest_sha256_data(&self) -> &str {
            &self.identity.intent.manifest_sha256
        }
        pub(crate) fn ready_proof_sha256_data(&self) -> String {
            digest(self.adjunct.witness_bytes())
        }
        pub(crate) fn ready_intent_sha256_data(&self) -> String {
            digest(self.adjunct.intent_bytes())
        }
        pub(crate) fn input_sha256_data(&self) -> &str {
            self.adjunct.input_sha256_data()
        }
        pub(crate) fn original_actor(
            &self,
        ) -> &Arc<crate::offhost_b1::destination_actor::OriginalAuthenticatedSourceInD> {
            &self.original
        }
        pub(crate) fn finalize_fixed(
            &self,
            decision: crate::offhost_b1::finalize::AuthenticatedFinalInOriginalD,
        ) -> Result<CommittedRemoteOriginal, BackupError> {
            self.recheck_original_ready()?;
            self.original.prepay_original_work(
                (self.manifest_raw.len() as u64)
                    .checked_mul(2)
                    .and_then(|n| n.checked_add(2 * 16_384 * 32))
                    .ok_or(BackupError::Overflow)?,
                32,
            )?;
            let binding = decision.binding_data().clone();
            decision.require_original(
                &self
                    .original
                    .destination_original()
                    .lease
                    .destination_operation,
                &binding,
            )?;
            if binding.backup_id != self.identity.intent.backup_id
                || binding.attempt_id != self.identity.intent.attempt_id
                || binding.proof_sha256 != self.identity.intent.proof_sha256
                || binding.manifest_sha256 != self.identity.intent.manifest_sha256
                || binding.snapshot_effects_proof_sha256 != self.ready_proof_sha256_data()
                || binding.snapshot_effects_intent_sha256 != self.ready_intent_sha256_data()
                || binding.snapshot_effects_input_sha256 != self.input_sha256_data()
            {
                return Err(BackupError::Invalid("Final differs from original Ready"));
            }
            let context = OriginalRemoteFinalization {
                destination: Arc::clone(self.original.destination_arc_original()),
                identity: self.identity.clone(),
                proof: CapturedSourceProof {
                    record: self.proof.record.clone(),
                },
                manifest_raw: self.manifest_raw.clone(),
                destination_roster: self.destination_roster.clone(),
                evidence_roster: self.evidence_roster.clone(),
                signer: Arc::clone(self.original.signer_original()?),
                native: Arc::clone(self.original.native_original()?),
                adjunct: Arc::clone(&self.adjunct),
                original: Arc::clone(&self.original),
                binding,
                deadline: self.original.entry_original().operation_deadline_data(),
            };
            context.finish(decision)
        }
    }

    pub(crate) struct OriginalRemoteFinalization {
        destination: Arc<EnrolledDestination>,
        identity: attempt::CommitIdentity,
        proof: CapturedSourceProof,
        manifest_raw: Vec<u8>,
        destination_roster: Vec<String>,
        evidence_roster: Vec<String>,
        signer: Arc<B1DestinationSignerOriginal>,
        native: Arc<crate::offhost_b1::destination_actor::NativeDestinationLocalCustody>,
        adjunct: Arc<snapshot_effects::RemoteCopiedSnapshotEffectsOriginal>,
        original: Arc<crate::offhost_b1::destination_actor::OriginalAuthenticatedSourceInD>,
        binding: RemoteFinalBindingData,
        // This is the original D deadline. There is no clock/deadline argument
        // to the consumer, no fresh clock and no producer in this component.
        deadline: Instant,
    }

    /// Keeps the original D roots/lease/budget alive after disk Committed,
    /// while the future actor checks result ACK/End/actual EOF/natural exit.
    /// This is not that transport completion and has no normal-success method.
    pub(crate) struct CommittedRemoteOriginal {
        original: OriginalRemoteFinalization,
        receipt: crate::complete::OriginalRemoteReceipt,
    }
    impl CommittedRemoteOriginal {
        pub(crate) fn receipt_data(&self) -> &[u8] {
            self.receipt.bytes()
        }
        pub(crate) fn receipt_sha256_data(&self) -> &str {
            self.receipt.receipt_sha256()
        }
        pub(crate) fn original_backup_id_data(&self) -> Uuid {
            self.original.identity.intent.backup_id
        }
        pub(crate) fn held_destination_for_committed(&self) -> &EnrolledDestination {
            &self.original.destination
        }
        pub(crate) fn statement_for_committed(&self) -> DestinationStatement {
            statement(&self.original.proof, self.original.signer.trust_data())
        }
        pub(crate) fn trust_for_committed(&self) -> &VerifierTrustConfig {
            self.original.signer.trust_data()
        }
        pub(crate) fn require_own_committed_native(
            &self,
            native: &crate::offhost_b1::destination_actor::NativeDestinationLocalCustody,
        ) -> Result<(), BackupError> {
            attempt::remote::require_committed_original(
                self.original.operation(),
                &self.original.identity,
            )?;
            if !std::ptr::eq(self.original.native.as_ref(), native) {
                return Err(BackupError::Invalid("different committed native owner"));
            }
            native.require_operation_original(self.original.operation())
        }
        pub(crate) fn require_own_committed_signer(
            &self,
            signer: &B1DestinationSignerOriginal,
        ) -> Result<(), BackupError> {
            attempt::remote::require_committed_original(
                self.original.operation(),
                &self.original.identity,
            )?;
            if !std::ptr::eq(self.original.signer.as_ref(), signer) {
                return Err(BackupError::Invalid("different committed signer owner"));
            }
            signer.require_original_budget(&self.original.destination.lease.budget)
        }
        pub(crate) fn recheck_local_committed(&self) -> Result<(), BackupError> {
            attempt::remote::require_committed_original(
                self.original.operation(),
                &self.original.identity,
            )?;
            self.original.check_deadline()?;
            self.original.native.recheck_committed_original(self)?;
            self.original.signer.recheck_committed_original(self)?;
            self.original.destination.recheck()?;
            self.original.original.recheck_committed_namespace(self)?;
            self.receipt.recheck_committed_original(self)?;
            self.original.check_deadline()?;
            attempt::remote::require_committed_original(
                self.original.operation(),
                &self.original.identity,
            )
        }
    }

    impl OriginalRemoteFinalization {
        pub(crate) fn finish(
            self,
            decision: crate::offhost_b1::finalize::AuthenticatedFinalInOriginalD,
        ) -> Result<CommittedRemoteOriginal, BackupError> {
            let receipt = attempt::remote::consume_original(&self, &decision)?;
            Ok(CommittedRemoteOriginal {
                original: self,
                receipt,
            })
        }
        pub(super) fn operation(&self) -> &Arc<Mutex<OperationAuthority>> {
            &self.destination.lease.destination_operation
        }
        pub(super) fn identity(&self) -> &attempt::CommitIdentity {
            &self.identity
        }
        pub(super) fn final_binding_data(&self) -> &RemoteFinalBindingData {
            &self.binding
        }
        fn charge(&self, bytes: u64, entries: usize) -> Result<(), BackupError> {
            let mut budget = self.destination.lease.budget.lock().map_err(|_| {
                BackupError::Invalid("remote original cumulative scan budget poisoned")
            })?;
            budget.charge(bytes)?;
            budget.entries(entries)
        }
        pub(super) fn prepay_consumer_entry(&self) -> Result<(), BackupError> {
            self.check_deadline()?;
            self.charge(16_384, 16)
        }
        pub(crate) fn check_deadline(&self) -> Result<(), BackupError> {
            if Instant::now() >= self.deadline {
                return Err(BackupError::Invalid("remote original local deadline"));
            }
            Ok(())
        }
        pub(crate) fn observe_remote_rejection(&self) -> Result<(), BackupError> {
            // A negative observation is ordered by the exact same mutex.
            // Committed remains a disk fact if its later transport fails.
            attempt::remote::observe_rejection(self.operation(), self.identity.intent.backup_id)
                .map(|_| ())
        }
        pub(super) fn require_pending_inputs(&self) -> Result<(), BackupError> {
            self.check_deadline()?;
            // Called while still Pending under the original authority mutex.
            // This observes a pre-consumption cancellation/rejection before the
            // transition. It never calls require_pending (which would relock).
            self.original.require_destination(&self.destination)?;
            self.signer.recheck_pending_original(&self.original)?;
            self.binding.validate_data()?;
            if self.binding.backup_id != self.identity.intent.backup_id
                || self.binding.attempt_id != self.identity.intent.attempt_id
                || self.binding.proof_sha256 != self.identity.intent.proof_sha256
                || self.binding.manifest_sha256 != self.identity.intent.manifest_sha256
                || self.proof.record.backup_id != self.binding.backup_id
                || self.proof.record.manifest_sha256 != self.binding.manifest_sha256
            {
                return Err(BackupError::Invalid("remote original prepared identity"));
            }
            Ok(())
        }
        fn require_guard(&self, owner: &RemoteConsumption<'_>) -> Result<(), BackupError> {
            owner.require_original(self.operation(), &self.identity)?;
            self.check_deadline()
        }
        pub(crate) fn require_own_original_signer(
            &self,
            owner: &RemoteConsumption<'_>,
            signer: &B1DestinationSignerOriginal,
        ) -> Result<(), BackupError> {
            self.require_guard(owner)?;
            if !std::ptr::eq(self.signer.as_ref(), signer) {
                return Err(BackupError::Invalid("different original D signing owner"));
            }
            signer.require_original_budget(&self.destination.lease.budget)
        }
        pub(crate) fn require_own_local_native(
            &self,
            owner: &RemoteConsumption<'_>,
            native: &crate::offhost_b1::destination_actor::NativeDestinationLocalCustody,
        ) -> Result<(), BackupError> {
            self.require_guard(owner)?;
            if !std::ptr::eq(self.native.as_ref(), native) {
                return Err(BackupError::Invalid(
                    "different original D local native owner",
                ));
            }
            native.require_operation_original(self.operation())
        }

        // Only a genuine consuming original can lend these handles to the
        // concrete complete.rs writer. No caller-selected root, trust or lease.
        pub(crate) fn held_destination_for_receipt(
            &self,
            owner: &RemoteConsumption<'_>,
        ) -> Result<&EnrolledDestination, BackupError> {
            self.require_guard(owner)?;
            Ok(&self.destination)
        }
        pub(crate) fn trust_for_receipt(
            &self,
            owner: &RemoteConsumption<'_>,
        ) -> Result<&VerifierTrustConfig, BackupError> {
            self.require_guard(owner)?;
            Ok(self.signer.trust_data())
        }
        pub(crate) fn statement_for_receipt(
            &self,
            owner: &RemoteConsumption<'_>,
        ) -> Result<DestinationStatement, BackupError> {
            self.require_guard(owner)?;
            Ok(statement(&self.proof, self.signer.trust_data()))
        }
        pub(crate) fn recheck_for_receipt(
            &self,
            owner: &RemoteConsumption<'_>,
        ) -> Result<(), BackupError> {
            self.require_guard(owner)?;
            self.charge(16_384 * 8, 32)?;
            self.native.recheck_consuming_original(self, owner)?;
            self.signer.recheck_consuming(self, owner)?;
            self.destination.recheck()?;
            let (manifest, index) = self.read_package()?;
            let (proof, identity) = self.read_evidence(&manifest, &index)?;
            if identity != self.identity
                || canonical(&proof.record)? != canonical(&self.proof.record)?
            {
                return Err(BackupError::Invalid("remote original evidence changed"));
            }
            self.adjunct.recheck_consuming_original(
                &self.destination,
                &self.proof,
                &self.identity,
                &self.binding,
                self.signer.trust_data(),
                owner,
            )?;
            self.destination.recheck()?;
            self.require_guard(owner)
        }
        pub(crate) fn recheck_published_receipt_roster(
            &self,
            owner: &RemoteConsumption<'_>,
        ) -> Result<(), BackupError> {
            self.require_guard(owner)?;
            self.charge(
                (self.destination_roster.len() as u64 + 1)
                    .checked_mul(512)
                    .ok_or(BackupError::Overflow)?,
                1,
            )?;
            let mut expected = self.destination_roster.clone();
            expected.push(format!("{}.complete", self.identity.intent.backup_id));
            expected.sort();
            self.exact_names(self.destination.destination(), &expected)
        }
        pub(super) fn sign_and_publish_fixed(
            &self,
            owner: &RemoteConsumption<'_>,
        ) -> Result<crate::complete::OriginalRemoteReceipt, BackupError> {
            self.recheck_for_receipt(owner)?;
            let id = self.identity.intent.backup_id;
            let prefix = id.to_string();
            let sealed_name = format!("{id}.sealed");
            self.exact_names(self.destination.destination(), &self.destination_roster)?;
            let names = &self.destination_roster;
            if names
                .iter()
                .any(|name| name.starts_with(&prefix) && name != &sealed_name)
            {
                return Err(BackupError::Invalid("remote original complete residue"));
            }
            self.require_guard(owner)?;
            self.charge(8192 * 8, 4)?;
            let witness = self.signer.sign_original_consuming(self, owner)?;
            self.require_guard(owner)?;
            crate::complete::publish_original_remote_receipt(self, owner, witness)
        }

        fn read_manifest_file(&self, root: &BackupDir) -> Result<Vec<u8>, BackupError> {
            let expected = self.manifest_raw.len();
            if expected == 0 || expected > crate::MAX_ASSET_INDEX_BYTES {
                return Err(BackupError::Capacity("remote original manifest bytes"));
            }
            self.charge(
                (expected as u64)
                    .checked_mul(4)
                    .ok_or(BackupError::Overflow)?,
                4,
            )?;
            let mut file = root.open_file("manifest.json")?;
            let before = file.metadata()?;
            if before.uid() != 0
                || before.gid() != 0
                || before.nlink() != 1
                || before.mode() & 0o7777 != 0o400
                || before.len() != expected as u64
            {
                return Err(BackupError::Invalid("remote original manifest custody"));
            }
            let mut raw = vec![0; expected];
            file.read_exact(&mut raw)?;
            let mut extra = [0];
            if file.read(&mut extra)? != 0
                || !same_file(&before, &file.metadata()?)
                || raw != self.manifest_raw
            {
                return Err(BackupError::Invalid("remote original manifest changed"));
            }
            let named = root.open_file("manifest.json")?.metadata()?;
            if !same_file(&before, &named) {
                return Err(BackupError::Invalid("remote original manifest replaced"));
            }
            Ok(raw)
        }
        fn read_package(&self) -> Result<(BackupManifestV1, Vec<u8>), BackupError> {
            self.check_deadline()?;
            let id = self.identity.intent.backup_id;
            let root = self
                .destination
                .destination()
                .open_dir(&format!("{id}.sealed"))?;
            root.require_sealed_directory()?;
            let root_identity = root.identity()?;
            let raw = self.read_manifest_file(&root)?;
            registry::prepay_current_b1_json(&self.destination.lease.budget, &raw)?;
            let manifest: BackupManifestV1 = serde_json::from_slice(&raw)?;
            if manifest.backup_id != id
                || manifest.canonical_bytes()? != raw
                || digest(&raw) != self.identity.intent.manifest_sha256
                || manifest.files.len() as u64 + 1 > crate::offhost_b1::finalize::FILES
            {
                return Err(BackupError::Invalid("remote original package manifest"));
            }
            let index_record = manifest
                .files
                .iter()
                .find(|r| r.path == "asset-index.json")
                .ok_or(BackupError::Invalid("remote original asset index"))?;
            let total = manifest
                .files
                .iter()
                .try_fold(raw.len() as u64, |n, record| {
                    n.checked_add(record.size).ok_or(BackupError::Overflow)
                })?;
            if total > crate::offhost_b1::finalize::PACKAGE_BYTES
                || index_record.size == 0
                || index_record.size > crate::MAX_ASSET_INDEX_BYTES as u64
            {
                return Err(BackupError::Capacity("remote original package profile"));
            }
            self.charge(
                index_record
                    .size
                    .checked_mul(4)
                    .ok_or(BackupError::Overflow)?,
                4,
            )?;
            let mut index_file = root.open_file("asset-index.json")?;
            let before = index_file.metadata()?;
            if before.uid() != 0
                || before.gid() != 0
                || before.nlink() != 1
                || before.mode() & 0o7777 != 0o400
                || before.len() != index_record.size
            {
                return Err(BackupError::Invalid("remote original index custody"));
            }
            let mut index =
                vec![0; usize::try_from(index_record.size).map_err(|_| BackupError::Overflow)?];
            index_file.read_exact(&mut index)?;
            let mut extra = [0];
            if index_file.read(&mut extra)? != 0
                || !same_file(&before, &index_file.metadata()?)
                || !same_file(&before, &root.open_file("asset-index.json")?.metadata()?)
                || digest(&index) != index_record.sha256
            {
                return Err(BackupError::Invalid("remote original index changed"));
            }
            registry::prepay_current_b1_json(&self.destination.lease.budget, &index)?;
            manifest.validate_with_index(&index)?;
            self.package_inventory(&root, &manifest)?;
            for record in &manifest.files {
                self.read_package_file(&root, record)?;
            }
            self.package_inventory(&root, &manifest)?;
            root.require_sealed_directory()?;
            if self.read_manifest_file(&root)? != raw
                || self
                    .destination
                    .destination()
                    .open_dir(&format!("{id}.sealed"))?
                    .identity()?
                    != root_identity
            {
                return Err(BackupError::Invalid("remote original package replaced"));
            }
            self.check_deadline()?;
            Ok((manifest, index))
        }
        fn read_package_file(
            &self,
            root: &BackupDir,
            record: &FileRecord,
        ) -> Result<(), BackupError> {
            self.check_deadline()?;
            let (relative, name) = record
                .path
                .rsplit_once('/')
                .unwrap_or(("", record.path.as_str()));
            self.charge(
                record
                    .size
                    .checked_mul(2)
                    .and_then(|n| n.checked_add(65_537))
                    .ok_or(BackupError::Overflow)?,
                6,
            )?;
            let parent = self.open_relative_directory(root, relative)?;
            parent.require_sealed_directory()?;
            let parent_identity = parent.identity()?;
            let mut file = parent.open_file(name)?;
            let before = file.metadata()?;
            if before.uid() != 0
                || before.gid() != 0
                || before.nlink() != 1
                || before.mode() & 0o7777 != 0o400
                || before.len() != record.size
            {
                return Err(BackupError::Invalid("remote original package file custody"));
            }
            let mut hash = Sha256::new();
            let mut remaining = record.size;
            let mut buffer = [0_u8; 65_536];
            while remaining > 0 {
                self.check_deadline()?;
                let limit = usize::try_from(remaining.min(buffer.len() as u64))
                    .map_err(|_| BackupError::Overflow)?;
                let count = match file.read(&mut buffer[..limit]) {
                    Ok(count) => count,
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error.into()),
                };
                if count == 0 {
                    return Err(BackupError::Invalid(
                        "remote original premature package EOF",
                    ));
                }
                remaining -= count as u64;
                hash.update(&buffer[..count]);
            }
            let mut extra = [0];
            if file.read(&mut extra)? != 0
                || !same_file(&before, &file.metadata()?)
                || !same_file(&before, &parent.open_file(name)?.metadata()?)
                || format!("{:x}", hash.finalize()) != record.sha256
                || self.open_relative_directory(root, relative)?.identity()? != parent_identity
            {
                return Err(BackupError::Invalid("remote original package file changed"));
            }
            self.check_deadline()
        }
        fn package_inventory(
            &self,
            root: &BackupDir,
            manifest: &BackupManifestV1,
        ) -> Result<(), BackupError> {
            let mut files = BTreeSet::from(["manifest.json".to_owned()]);
            let mut dirs = BTreeSet::from([String::new()]);
            for record in &manifest.files {
                self.charge(
                    (record.path.len() as u64)
                        .checked_mul(4)
                        .ok_or(BackupError::Overflow)?,
                    2,
                )?;
                if !files.insert(record.path.clone()) {
                    return Err(BackupError::Invalid(
                        "remote original duplicate package file",
                    ));
                }
                let mut path = record.path.as_str();
                while let Some((parent, _)) = path.rsplit_once('/') {
                    dirs.insert(parent.to_owned());
                    path = parent;
                }
            }
            // Reopen each exact relative directory iteratively. At most two
            // temporary directory FDs are live, independent of path depth.
            for relative in &dirs {
                self.check_deadline()?;
                let dir = self.open_relative_directory(root, relative)?;
                dir.require_sealed_directory()?;
                let before = dir.identity()?;
                let expected = files
                    .iter()
                    .chain(dirs.iter())
                    .filter(|path| {
                        if relative.is_empty() {
                            !path.is_empty() && !path.contains('/')
                        } else {
                            path.rsplit_once('/')
                                .is_some_and(|(p, _)| p == relative.as_str())
                        }
                    })
                    .count();
                let names = self.bounded_names(&dir, expected)?;
                for leaf in &names {
                    let name = if relative.is_empty() {
                        leaf.clone()
                    } else {
                        format!("{relative}/{leaf}")
                    };
                    match dir.kind(leaf)? {
                        BackupEntryKind::File if files.contains(&name) => (),
                        BackupEntryKind::Directory if dirs.contains(&name) => (),
                        _ => {
                            return Err(BackupError::Invalid("remote original exact package tree"));
                        }
                    }
                }
                if names.len() != expected
                    || self.open_relative_directory(root, relative)?.identity()? != before
                    || self.bounded_names(&dir, expected)? != names
                {
                    return Err(BackupError::Invalid(
                        "remote original package inventory changed",
                    ));
                }
            }
            Ok(())
        }
        fn bounded_names(
            &self,
            root: &BackupDir,
            expected: usize,
        ) -> Result<Vec<String>, BackupError> {
            let cap = expected.checked_add(1).ok_or(BackupError::Overflow)?;
            if cap > registry::MAX_ENTRIES {
                return Err(BackupError::Capacity("remote original directory entries"));
            }
            // Linux leaf names are <=255 bytes. Include String/Vec capacity,
            // sorting and the one unexpected-entry probe before allocation.
            self.charge(
                (cap as u64).checked_mul(512).ok_or(BackupError::Overflow)?,
                cap,
            )?;
            let mut names = root.list_bounded(cap)?;
            names.sort();
            Ok(names)
        }
        fn exact_names(&self, root: &BackupDir, expected: &[String]) -> Result<(), BackupError> {
            if expected.windows(2).any(|pair| pair[0] >= pair[1])
                || self.bounded_names(root, expected.len())?.as_slice() != expected
            {
                return Err(BackupError::Invalid(
                    "remote original held directory roster changed",
                ));
            }
            Ok(())
        }
        fn open_relative_directory(
            &self,
            root: &BackupDir,
            path: &str,
        ) -> Result<BackupDir, BackupError> {
            self.charge(path.len() as u64, path.split('/').count() + 1)?;
            let mut current = root.try_clone()?;
            if !path.is_empty() {
                for component in path.split('/') {
                    current = current.open_dir(component)?;
                }
            }
            Ok(current)
        }
        fn read_evidence(
            &self,
            manifest: &BackupManifestV1,
            index: &[u8],
        ) -> Result<(CapturedSourceProof, attempt::CommitIdentity), BackupError> {
            let id = self.identity.intent.backup_id;
            self.exact_names(self.destination.evidence(), &self.evidence_roster)?;
            attempt::require_committed_names(&self.evidence_roster, id)?;
            let attempt = self
                .destination
                .evidence()
                .open_dir(&format!("{id}.attempt"))?;
            attempt.require_private_directory()?;
            if self.bounded_names(&attempt, 2)? != ["intent.json", "terminal.json"] {
                return Err(BackupError::Invalid(
                    "remote original exact evidence attempt",
                ));
            }
            let (intent, _) = registry::read(
                &attempt,
                "intent.json",
                4096,
                &self.destination.lease.budget,
            )?;
            let (terminal, _) = registry::read(
                &attempt,
                "terminal.json",
                4096,
                &self.destination.lease.budget,
            )?;
            let identity = attempt::validate_committed(&intent, &terminal)?;
            let expected_bytes = self.proof.record.files.iter().try_fold(0u64, |n, record| {
                n.checked_add(record.size).ok_or(BackupError::Overflow)
            })?;
            if expected_bytes > crate::offhost_b1::finalize::EVIDENCE_BYTES {
                return Err(BackupError::Capacity("remote original evidence bytes"));
            }
            self.charge(
                expected_bytes.checked_mul(4).ok_or(BackupError::Overflow)?,
                self.proof.record.files.len() * 2,
            )?;
            let proof = linux::read_payload_from_verified_package(
                &self.destination,
                id,
                self.signer.trust_data(),
                manifest,
                index,
            )?;
            if identity != self.identity
                || (
                    identity.intent.destination_dev,
                    identity.intent.destination_ino,
                ) != self.destination.destination().identity()?
                || (identity.intent.evidence_dev, identity.intent.evidence_ino)
                    != self.destination.evidence().identity()?
                || identity.intent.manifest_sha256 != proof.record.manifest_sha256
                || identity.intent.proof_sha256 != digest(&canonical(&proof.record)?)
            {
                return Err(BackupError::Invalid("remote original readback identity"));
            }
            self.check_deadline()?;
            Ok((proof, identity))
        }
    }

    fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
        (
            a.dev(),
            a.ino(),
            a.uid(),
            a.gid(),
            a.len(),
            a.mode(),
            a.nlink(),
            a.mtime(),
            a.mtime_nsec(),
            a.ctime(),
            a.ctime_nsec(),
        ) == (
            b.dev(),
            b.ino(),
            b.uid(),
            b.gid(),
            b.len(),
            b.mode(),
            b.nlink(),
            b.mtime(),
            b.mtime_nsec(),
            b.ctime(),
            b.ctime_nsec(),
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn binding_data() -> RemoteFinalBindingData {
            RemoteFinalBindingData {
                backup_id: Uuid::new_v4(),
                attempt_id: Uuid::new_v4(),
                source_operation_id: Uuid::new_v4(),
                session_binding_sha256: "a".repeat(64),
                source_auth_frame_sha256: "b".repeat(64),
                source_auth_key_sha256: "6".repeat(64),
                s_nonce: "c".repeat(64),
                d_nonce: "d".repeat(64),
                manifest_sha256: "e".repeat(64),
                proof_sha256: "f".repeat(64),
                snapshot_effects_proof_sha256: "1".repeat(64),
                snapshot_effects_input_sha256: "2".repeat(64),
                snapshot_effects_intent_sha256: "3".repeat(64),
                fence_anchor_sha256: "4".repeat(64),
                protection_snapshot_sha256: "5".repeat(64),
            }
        }

        #[test]
        fn remote_final_binding_projection_is_data_and_rejects_missing_nonce_or_identity() {
            let mut data = binding_data();
            data.validate_data().unwrap();
            data.d_nonce.clear();
            assert!(data.validate_data().is_err());
            data.d_nonce = "d".repeat(64);
            data.source_operation_id = Uuid::nil();
            assert!(data.validate_data().is_err());
            // No original D, authenticated Final or receipt can be built here.
        }
    }
}

pub fn verify_enrolled_destination(
    registry: &RegistryLease,
    backup_id: Uuid,
) -> Result<VerifiedDestination, BackupError> {
    #[cfg(target_os = "linux")]
    {
        let fence = attempt::ErrorFence::new(&registry.destination_operation);
        registry
            .destination_operation
            .lock()
            .expect("destination operation")
            .require_id(backup_id)?;
        let trust = installed_trust()?;
        let destination =
            registry.admit_destination(std::path::Path::new(&trust.destination_root))?;
        let (proof, identity) = linux::read_authorized(&destination, backup_id, &trust)?;
        let verified = VerifiedDestination {
            statement: statement(&proof, &trust),
            proof_sha256: digest(&canonical(&proof.record)?),
            destination,
            identity: identity.clone(),
        };
        let verified =
            attempt::finalize_use(&registry.destination_operation, &identity, || Ok(verified))?;
        fence.complete();
        Ok(verified)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (registry, backup_id);
        Err(BackupError::Invalid("destination verifier requires Linux"))
    }
}

pub fn persist_captured_source_proof(
    destination: &EnrolledDestination,
    snapshot: &ProtectionSnapshot,
    backup_id: Uuid,
) -> Result<CapturedSourceProof, BackupError> {
    #[cfg(target_os = "linux")]
    {
        linux::persist(destination, snapshot, backup_id)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (destination, snapshot, backup_id);
        Err(BackupError::Invalid("captured source proof requires Linux"))
    }
}

/// Full recovery pre-write gate, composed with an already verified CompleteBackup.
/// The legacy low-level v2 reader remains available for older bare receipts.
pub fn open_captured_source_proof(
    destination: &EnrolledDestination,
    complete: &CompleteBackup,
) -> Result<CapturedSourceProof, BackupError> {
    #[cfg(target_os = "linux")]
    {
        let fence = attempt::ErrorFence::new(&destination.lease.destination_operation);
        let trust = installed_trust()?;
        // Reopen the actual receipt too: the passed handle is not a stale pin.
        let current = crate::open_complete_backup(
            destination.destination_path(),
            std::path::Path::new(DESTINATION_TRUST_PATH),
            complete.backup_id(),
        )?;
        if current.receipt_sha256() != complete.receipt_sha256() {
            return Err(BackupError::Invalid("complete receipt replaced"));
        }
        // Prior signed receipt authority permits destination-only recovery;
        // a disk terminal alone never enters the new-signing path.
        let (proof, _) = linux::inspect_committed(destination, complete.backup_id(), &trust)?;
        if proof.record.manifest_sha256 != current.manifest_sha256()
            || proof.record.source_control_sha256 != current.source_control_sha256
        {
            return Err(BackupError::Invalid(
                "captured proof differs from complete witness",
            ));
        }
        fence.complete();
        Ok(proof)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (destination, complete);
        Err(BackupError::Invalid("captured source proof requires Linux"))
    }
}

#[cfg(target_os = "linux")]
fn installed_trust() -> Result<VerifierTrustConfig, BackupError> {
    // Check trusted ancestors in addition to the unchanged low-level trust format.
    learning_assets::backup_fs::BackupDir::open_trusted_private_root(std::path::Path::new(
        KEY_ROOT,
    ))?;
    crate::complete::load_trust(std::path::Path::new(DESTINATION_TRUST_PATH))
}
#[cfg(target_os = "linux")]
fn statement(proof: &CapturedSourceProof, trust: &VerifierTrustConfig) -> DestinationStatement {
    DestinationStatement {
        format_version: 2,
        backup_id: proof.backup_id(),
        manifest_sha256: proof.record.manifest_sha256.clone(),
        source_control_sha256: proof.record.source_control_sha256.clone(),
        source_root: trust.source_root.clone(),
        source_control_root: trust.source_control_root.clone(),
        source_host_id: trust.source_host_id,
        source_storage_id: trust.source_storage_id,
        destination_host_id: trust.destination_host_id,
        destination_storage_id: trust.destination_storage_id,
        destination_root: trust.destination_root.clone(),
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn check_publication_proof(
    destination: &EnrolledDestination,
    snapshot: &ProtectionSnapshot,
    expected: &DestinationStatement,
    trust: &VerifierTrustConfig,
) -> Result<(), BackupError> {
    let fence = attempt::ErrorFence::new(&destination.lease.destination_operation);
    let (proof, identity) = linux::read_authorized(destination, expected.backup_id, trust)?;
    if statement(&proof, trust) != *expected {
        return Err(BackupError::Invalid("publication captured proof witness"));
    }
    let current = snapshot.captured_files(expected.backup_id)?;
    // scan_bytes is the monotonically charged operation observation, not a
    // stable source identity. All actual metadata references/counts must match.
    let old = proof
        .record
        .files
        .iter()
        .filter(|r| r.path != "protection-snapshot.json");
    for reference in old {
        let raw = current
            .get(&reference.path)
            .ok_or(BackupError::Invalid("publication captured source missing"))?;
        if reference.size != raw.len() as u64 || reference.sha256 != digest(raw) {
            return Err(BackupError::Invalid("publication captured source changed"));
        }
    }
    let dir = destination
        .evidence()
        .open_dir(&expected.backup_id.to_string())?;
    let (raw, _) = registry::read(
        &dir,
        "protection-snapshot.json",
        crate::MAX_ASSET_INDEX_BYTES as u64,
        &destination.lease.budget,
    )?;
    let before: SnapshotRecord = canonical_record(&raw, crate::MAX_ASSET_INDEX_BYTES as u64)?;
    let now: SnapshotRecord = canonical_record(
        &current["protection-snapshot.json"],
        crate::MAX_ASSET_INDEX_BYTES as u64,
    )?;
    if before.records != now.records
        || before.protected_digest_count != now.protected_digest_count
        || before.source_group_id != now.source_group_id
        || before.registry_generation_sha256 != now.registry_generation_sha256
    {
        return Err(BackupError::Invalid(
            "publication protection observation changed",
        ));
    }
    snapshot.recheck_held()?;
    destination.recheck()?;
    attempt::finalize_use(&destination.lease.destination_operation, &identity, || {
        Ok(())
    })?;
    fence.complete();
    Ok(())
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use learning_assets::backup_fs::BackupDir;
    use std::io::Write;

    fn package(
        destination: &EnrolledDestination,
        id: Uuid,
    ) -> Result<(BackupManifestV1, Vec<u8>), BackupError> {
        destination.recheck()?;
        destination.destination().sync()?;
        let dir = destination
            .destination()
            .open_dir(&format!("{id}.sealed"))?;
        crate::protection::verify_registered_package(&destination.lease, &dir, id)
    }
    pub(super) fn trust_matches(
        destination: &EnrolledDestination,
        trust: &VerifierTrustConfig,
    ) -> Result<(), BackupError> {
        if destination.destination_path().to_str() != Some(trust.destination_root.as_str())
            || destination.group.destination_host_id != trust.destination_host_id.to_string()
            || destination.group.destination_storage_id != trust.destination_storage_id.to_string()
        {
            return Err(BackupError::Invalid(
                "enrolled destination differs from trust",
            ));
        }
        Ok(())
    }
    fn read_files(
        dir: &BackupDir,
        record: &ProofRecord,
        destination: &EnrolledDestination,
    ) -> Result<BTreeMap<String, Vec<u8>>, BackupError> {
        validate_envelope(record)?;
        dir.require_private_directory()?;
        let mut names = dir.list_bounded(10)?;
        destination
            .lease
            .budget
            .lock()
            .expect("scan budget")
            .entries(names.len())?;
        names.sort();
        let mut expected = FIXED_FILES
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        expected.extend(["roots".into(), "source-proof.json".into()]);
        expected.sort();
        if names != expected {
            return Err(BackupError::Invalid("captured evidence exact directory"));
        }
        let roots = dir.open_dir("roots")?;
        roots.require_private_directory()?;
        let mut names = roots.list_bounded(4)?;
        destination
            .lease
            .budget
            .lock()
            .expect("scan budget")
            .entries(names.len())?;
        names.sort();
        let expected = record
            .files
            .iter()
            .filter_map(|r| r.path.strip_prefix("roots/").map(str::to_owned))
            .collect::<Vec<_>>();
        if names != expected {
            return Err(BackupError::Invalid("captured evidence exact roots"));
        }
        let mut files = BTreeMap::new();
        for entry in &record.files {
            let (parent, name) = if let Some(name) = entry.path.strip_prefix("roots/") {
                (&roots, name)
            } else {
                (dir, entry.path.as_str())
            };
            let (raw, _) = registry::read(
                parent,
                name,
                file_cap(&entry.path),
                &destination.lease.budget,
            )?;
            files.insert(entry.path.clone(), raw);
        }
        checked_files(record, &files)?;
        Ok(files)
    }
    fn read_payload(
        destination: &EnrolledDestination,
        id: Uuid,
        trust: &VerifierTrustConfig,
    ) -> Result<CapturedSourceProof, BackupError> {
        trust_matches(destination, trust)?;
        let (manifest, index) = package(destination, id)?;
        read_payload_from_verified_package(destination, id, trust, &manifest, &index)
    }
    pub(super) fn read_payload_from_verified_package(
        destination: &EnrolledDestination,
        id: Uuid,
        trust: &VerifierTrustConfig,
        manifest: &BackupManifestV1,
        index: &[u8],
    ) -> Result<CapturedSourceProof, BackupError> {
        trust_matches(destination, trust)?;
        let dir = destination.evidence().open_dir(&id.to_string())?;
        let (raw, _) = registry::read(&dir, "source-proof.json", 16384, &destination.lease.budget)?;
        let record: ProofRecord = canonical_record(&raw, 16384)?;
        if record.backup_id != id {
            return Err(BackupError::Invalid("captured backup directory identity"));
        }
        let files = read_files(&dir, &record, destination)?;
        struct Reader {
            files: BTreeMap<String, Vec<u8>>,
        }
        impl EvidenceAccess for Reader {
            fn read(&mut self, location: EvidenceLocation<'_>) -> Result<Vec<u8>, BackupError> {
                match location {
                    EvidenceLocation::Destination(path) => self
                        .files
                        .remove(path)
                        .ok_or(BackupError::Invalid("captured evidence missing")),
                    #[cfg(test)]
                    EvidenceLocation::CapturedSource(_) => {
                        Err(BackupError::Invalid("recovery source access forbidden"))
                    }
                }
            }
        }
        let mut reader = Reader { files };
        reader.files.insert("source-proof.json".into(), raw);
        #[cfg(feature = "offhost-b1")]
        {
            registry::prepay_current_b1_json(&destination.lease.budget, index)?;
            for bytes in reader.files.values() {
                registry::prepay_current_b1_json(&destination.lease.budget, bytes)?;
            }
        }
        let proof = reopen_evidence(&mut reader, id, manifest, index, trust)?;
        dir.sync()?;
        destination.evidence().sync()?;
        destination.recheck()?;
        Ok(proof)
    }
    pub(super) fn inspect_committed(
        destination: &EnrolledDestination,
        id: Uuid,
        trust: &VerifierTrustConfig,
    ) -> Result<(CapturedSourceProof, attempt::CommitIdentity), BackupError> {
        let fence = attempt::ErrorFence::new(&destination.lease.destination_operation);
        destination.recheck()?;
        let names = registry::list(destination.evidence(), &destination.lease)?;
        attempt::require_committed_names(&names, id)?;
        let attempt_dir = destination.evidence().open_dir(&format!("{id}.attempt"))?;
        attempt_dir.require_private_directory()?;
        if registry::list(&attempt_dir, &destination.lease)? != ["intent.json", "terminal.json"] {
            return Err(BackupError::Invalid(
                "destination evidence attempt exact records",
            ));
        }
        let (intent, _) =
            registry::read(&attempt_dir, "intent.json", 4096, &destination.lease.budget)?;
        let (terminal, _) = registry::read(
            &attempt_dir,
            "terminal.json",
            4096,
            &destination.lease.budget,
        )?;
        let identity = attempt::validate_committed(&intent, &terminal)?;
        let proof = read_payload(destination, id, trust)?;
        let current_destination = destination.destination().identity()?;
        let current_evidence = destination.evidence().identity()?;
        if identity.intent.backup_id != id
            || (
                identity.intent.destination_dev,
                identity.intent.destination_ino,
            ) != current_destination
            || (identity.intent.evidence_dev, identity.intent.evidence_ino) != current_evidence
            || identity.intent.manifest_sha256 != proof.record.manifest_sha256
            || identity.intent.proof_sha256 != digest(&canonical(&proof.record)?)
        {
            return Err(BackupError::Invalid("destination committed proof identity"));
        }
        destination.recheck()?;
        fence.complete();
        Ok((proof, identity))
    }
    pub(super) fn read_authorized(
        destination: &EnrolledDestination,
        id: Uuid,
        trust: &VerifierTrustConfig,
    ) -> Result<(CapturedSourceProof, attempt::CommitIdentity), BackupError> {
        let fence = attempt::ErrorFence::new(&destination.lease.destination_operation);
        destination
            .lease
            .destination_operation
            .lock()
            .expect("destination operation")
            .require_id(id)?;
        let (proof, identity) = inspect_committed(destination, id, trust)?;
        let checked =
            attempt::finalize_use(&destination.lease.destination_operation, &identity, || {
                Ok((proof, identity.clone()))
            })?;
        fence.complete();
        Ok(checked)
    }
    pub(super) fn read_proof(
        destination: &EnrolledDestination,
        id: Uuid,
        trust: &VerifierTrustConfig,
    ) -> Result<CapturedSourceProof, BackupError> {
        read_authorized(destination, id, trust).map(|(proof, _)| proof)
    }
    struct Prepared {
        trust: VerifierTrustConfig,
        files: BTreeMap<String, Vec<u8>>,
        record: ProofRecord,
        raw: Vec<u8>,
        intent: attempt::Intent,
    }
    fn prepared(
        destination: &EnrolledDestination,
        id: Uuid,
        attempt_id: Uuid,
        trust: VerifierTrustConfig,
        files: BTreeMap<String, Vec<u8>>,
    ) -> Result<Prepared, BackupError> {
        trust_matches(destination, &trust)?;
        let (manifest, index) = package(destination, id)?;
        let record = proof_record_from_files(id, &manifest, &files)?;
        #[cfg(feature = "offhost-b1")]
        {
            registry::prepay_current_b1_json(&destination.lease.budget, &index)?;
            for bytes in files.values() {
                registry::prepay_current_b1_json(&destination.lease.budget, bytes)?;
            }
        }
        validate_copies(&record, &files, &manifest, &index, &trust)?;
        let raw = canonical(&record)?;
        if raw.len() > 16384 {
            return Err(BackupError::Capacity("captured source proof record"));
        }
        let (destination_dev, destination_ino) = destination.destination().identity()?;
        let (evidence_dev, evidence_ino) = destination.evidence().identity()?;
        let intent = attempt::Intent {
            format_version: 1,
            capability: "destination_evidence_attempt_v1".into(),
            backup_id: id,
            attempt_id,
            destination_dev,
            destination_ino,
            evidence_dev,
            evidence_ino,
            manifest_sha256: record.manifest_sha256.clone(),
            proof_sha256: digest(&raw),
        };
        Ok(Prepared {
            trust,
            files,
            record,
            raw,
            intent,
        })
    }
    enum SourceInput<'a> {
        Captured(&'a ProtectionSnapshot),
        #[cfg(feature = "offhost-b1")]
        OriginalRemote(&'a crate::offhost_b1::destination_actor::OriginalReceivedFiles<'a>),
        #[cfg(test)]
        Synthetic {
            trust: VerifierTrustConfig,
            files: BTreeMap<String, Vec<u8>>,
        },
    }
    struct Writer<'a> {
        destination: &'a EnrolledDestination,
        input: SourceInput<'a>,
        id: Uuid,
        attempt_id: Uuid,
        attempt_dir: Option<BackupDir>,
        prepared: Option<Prepared>,
        proof: Option<CapturedSourceProof>,
        identity: Option<attempt::CommitIdentity>,
        terminal: Option<std::fs::File>,
        #[cfg(test)]
        fault: Option<attempt::Step>,
    }
    impl Writer<'_> {
        fn stage_name(&self) -> String {
            format!("{}.staging-{}", self.id, self.attempt_id)
        }
        fn source_recheck(&self) -> Result<(), BackupError> {
            match &self.input {
                SourceInput::Captured(snapshot) => snapshot.recheck_held(),
                #[cfg(feature = "offhost-b1")]
                SourceInput::OriginalRemote(received) => received.recheck(),
                #[cfg(test)]
                SourceInput::Synthetic { .. } => Ok(()),
            }
        }
    }
    impl attempt::PublicationIo for Writer<'_> {
        type Output = CapturedSourceProof;
        fn step(&mut self, step: attempt::Step) -> Result<(), BackupError> {
            #[cfg(test)]
            if self.fault == Some(step) {
                return Err(BackupError::Invalid(
                    "controlled evidence publication fault",
                ));
            }
            use attempt::Step;
            let destination = self.destination;
            match step {
                Step::Admit => {
                    if !v4(&self.id.to_string()) {
                        return Err(BackupError::Invalid("destination evidence backup UUID"));
                    }
                    match &self.input {
                        SourceInput::Captured(snapshot) => {
                            snapshot.require_same_operation(&destination.lease)?
                        }
                        #[cfg(feature = "offhost-b1")]
                        SourceInput::OriginalRemote(received) => {
                            received.original().require_destination(destination)?
                        }
                        #[cfg(test)]
                        SourceInput::Synthetic { .. } => (),
                    }
                    destination.recheck()?;
                    let names = registry::list(destination.evidence(), &destination.lease)?;
                    attempt::require_fresh(&names, self.id)?;
                    self.attempt_dir = Some(
                        destination
                            .evidence()
                            .create_dir(&format!("{}.attempt", self.id))?,
                    );
                    destination.evidence().sync()?;
                }
                Step::Prepare => {
                    let (trust, files) = match &self.input {
                        SourceInput::Captured(snapshot) => {
                            (installed_trust()?, snapshot.captured_files(self.id)?)
                        }
                        #[cfg(feature = "offhost-b1")]
                        SourceInput::OriginalRemote(received) => {
                            let original = received.original();
                            original.require_destination(destination)?;
                            let files = received.read_source_files_data()?;
                            original.require_source_binding_files(&files)?;
                            let signer = original.signer_original()?;
                            signer.recheck_pending_original(original)?;
                            original.prepay_original_work(16_384, 4)?;
                            (signer.trust_data().clone(), files)
                        }
                        #[cfg(test)]
                        SourceInput::Synthetic { trust, files } => (trust.clone(), files.clone()),
                    };
                    let value = prepared(destination, self.id, self.attempt_id, trust, files)?;
                    let dir = self.attempt_dir.as_ref().expect("admitted attempt");
                    let mut file = dir.create_file("intent.json")?;
                    file.write_all(&canonical(&value.intent)?)?;
                    file.sync_all()?;
                    dir.sync()?;
                    self.prepared = Some(value);
                }
                Step::Stage => {
                    let value = self.prepared.as_ref().expect("prepared evidence");
                    let stage = destination.evidence().create_dir(&self.stage_name())?;
                    let roots = stage.create_dir("roots")?;
                    for (path, bytes) in &value.files {
                        let (parent, name) = if let Some(name) = path.strip_prefix("roots/") {
                            (&roots, name)
                        } else {
                            (&stage, path.as_str())
                        };
                        let mut file = parent.create_file(name)?;
                        file.write_all(bytes)?;
                        file.sync_all()?;
                    }
                    let mut file = stage.create_file("source-proof.json")?;
                    file.write_all(&value.raw)?;
                    file.sync_all()?;
                    drop(file);
                    roots.sync()?;
                    stage.sync()?;
                    let readback = read_files(&stage, &value.record, destination)?;
                    checked_files(&value.record, &readback)?;
                }
                Step::BeforeRename => {
                    self.source_recheck()?;
                    destination.recheck()?;
                }
                Step::Rename => destination
                    .evidence()
                    .rename_noreplace_without_sync(&self.stage_name(), &self.id.to_string())?,
                Step::ParentSync => destination.evidence().sync()?,
                Step::FinalReadback => {
                    let value = self.prepared.as_ref().expect("prepared evidence");
                    let proof = read_payload(destination, self.id, &value.trust)?;
                    if digest(&canonical(&proof.record)?) != value.intent.proof_sha256 {
                        return Err(BackupError::Invalid(
                            "destination evidence final readback changed",
                        ));
                    }
                    self.proof = Some(proof);
                }
                Step::TerminalWrite => {
                    let value = self.prepared.as_ref().expect("prepared evidence");
                    let mut file = self
                        .attempt_dir
                        .as_ref()
                        .expect("admitted attempt")
                        .create_file("terminal.json")?;
                    file.write_all(&canonical(&value.intent.terminal()?)?)?;
                    self.terminal = Some(file);
                }
                Step::TerminalSync => {
                    self.terminal.as_ref().expect("terminal write").sync_all()?;
                    self.attempt_dir
                        .as_ref()
                        .expect("admitted attempt")
                        .sync()?;
                    destination.evidence().sync()?;
                }
                Step::TerminalReadback => {
                    let value = self.prepared.as_ref().expect("prepared evidence");
                    let (proof, identity) = inspect_committed(destination, self.id, &value.trust)?;
                    if identity.intent != value.intent {
                        return Err(BackupError::Invalid(
                            "destination terminal readback changed",
                        ));
                    }
                    self.proof = Some(proof);
                    self.identity = Some(identity);
                }
                Step::FinalRecheck => {
                    self.source_recheck()?;
                    destination.recheck()?;
                }
            }
            Ok(())
        }
        fn outcome(
            &mut self,
        ) -> Result<(CapturedSourceProof, attempt::CommitIdentity), BackupError> {
            Ok((
                self.proof
                    .take()
                    .ok_or(BackupError::Invalid("destination final proof missing"))?,
                self.identity
                    .take()
                    .ok_or(BackupError::Invalid("destination terminal missing"))?,
            ))
        }
    }
    pub(super) fn persist(
        destination: &EnrolledDestination,
        snapshot: &ProtectionSnapshot,
        id: Uuid,
    ) -> Result<CapturedSourceProof, BackupError> {
        let mut writer = Writer {
            destination,
            input: SourceInput::Captured(snapshot),
            id,
            attempt_id: Uuid::new_v4(),
            attempt_dir: None,
            prepared: None,
            proof: None,
            identity: None,
            terminal: None,
            #[cfg(test)]
            fault: None,
        };
        attempt::publish(&destination.lease.destination_operation, id, &mut writer)
    }
    #[cfg(feature = "offhost-b1")]
    pub(super) struct OriginalRemoteWriter<'a>(Writer<'a>);
    #[cfg(feature = "offhost-b1")]
    impl attempt::PublicationIo for OriginalRemoteWriter<'_> {
        type Output = (CapturedSourceProof, attempt::CommitIdentity);
        fn step(&mut self, step: attempt::Step) -> Result<(), BackupError> {
            let received = match &self.0.input {
                SourceInput::OriginalRemote(received) => *received,
                _ => return Err(BackupError::Invalid("fixed original remote writer input")),
            };
            let bytes = received.source_records().iter().try_fold(0u64, |n, r| {
                n.checked_add(r.size).ok_or(BackupError::Overflow)
            })?;
            // The old eleven concrete filesystem steps remain intact.
            // Their remote route additionally prepays fixed list/stat/open
            // work and all source-derived parse/hash/copy/write work before
            // entering the old step. No later consumption repays this IO.
            let dynamic = match step {
                attempt::Step::Prepare
                | attempt::Step::FinalReadback
                | attempt::Step::TerminalReadback => bytes.checked_mul(32),
                attempt::Step::Stage => bytes.checked_mul(4),
                _ => Some(0),
            }
            .and_then(|n| n.checked_add(1_048_576))
            .ok_or(BackupError::Overflow)?;
            received.original().prepay_original_work(dynamic, 256)?;
            attempt::PublicationIo::step(&mut self.0, step)
        }
        fn outcome(&mut self) -> Result<(Self::Output, attempt::CommitIdentity), BackupError> {
            let (proof, identity) = attempt::PublicationIo::outcome(&mut self.0)?;
            Ok(((proof, identity.clone()), identity))
        }
    }

    #[cfg(feature = "offhost-b1")]
    pub(super) fn persist_original_remote(
        received: &crate::offhost_b1::destination_actor::OriginalReceivedFiles<'_>,
    ) -> Result<(CapturedSourceProof, attempt::CommitIdentity), BackupError> {
        let original = received.original();
        original.require_destination(original.destination_original())?;
        let mut writer = OriginalRemoteWriter(Writer {
            destination: original.destination_original(),
            input: SourceInput::OriginalRemote(received),
            id: original.backup_id_data()?,
            attempt_id: original.destination_attempt_data()?,
            attempt_dir: None,
            prepared: None,
            proof: None,
            identity: None,
            terminal: None,
            #[cfg(test)]
            fault: None,
        });
        attempt::remote::publish_original(original, &mut writer)
    }

    #[cfg(test)]
    pub(super) fn persist_synthetic(
        destination: &EnrolledDestination,
        id: Uuid,
        trust: VerifierTrustConfig,
        files: BTreeMap<String, Vec<u8>>,
        fault: Option<attempt::Step>,
    ) -> Result<CapturedSourceProof, BackupError> {
        let mut writer = Writer {
            destination,
            input: SourceInput::Synthetic { trust, files },
            id,
            attempt_id: Uuid::new_v4(),
            attempt_dir: None,
            prepared: None,
            proof: None,
            identity: None,
            terminal: None,
            fault,
        };
        attempt::publish(&destination.lease.destination_operation, id, &mut writer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        record: ProofRecord,
        files: BTreeMap<String, Vec<u8>>,
        manifest: BackupManifestV1,
        index: Vec<u8>,
        trust: VerifierTrustConfig,
        source_calls: usize,
        destination_calls: usize,
    }
    impl EvidenceAccess for Fixture {
        fn read(&mut self, location: EvidenceLocation<'_>) -> Result<Vec<u8>, BackupError> {
            match location {
                EvidenceLocation::Destination(path) => {
                    self.destination_calls += 1;
                    if path == "source-proof.json" {
                        return canonical(&self.record);
                    }
                    self.files
                        .get(path)
                        .cloned()
                        .ok_or(BackupError::Invalid("fixture missing destination byte"))
                }
                EvidenceLocation::CapturedSource(path) => {
                    let _ = path;
                    self.source_calls += 1;
                    Err(BackupError::Invalid("source unavailable"))
                }
            }
        }
    }
    impl Fixture {
        fn refresh(&mut self) {
            self.record.files = self
                .files
                .iter()
                .map(|(path, raw)| FileRecord {
                    path: path.clone(),
                    size: raw.len() as u64,
                    sha256: digest(raw),
                })
                .collect();
            self.record.protection_snapshot_sha256 =
                digest(&self.files["protection-snapshot.json"]);
        }
        fn reopen(&mut self) -> Result<CapturedSourceProof, BackupError> {
            let (id, manifest, index, trust) = (
                self.record.backup_id,
                self.manifest.clone(),
                self.index.clone(),
                self.trust.clone(),
            );
            reopen_evidence(self, id, &manifest, &index, &trust)
        }
    }
    fn fixture() -> Fixture {
        let id = Uuid::new_v4();
        let deployment = Uuid::new_v4().to_string();
        let group_id = Uuid::new_v4().to_string();
        let plan = crate::BackupPlan::from_rows(vec![]).unwrap();
        let source = crate::SourceIdentity::from_migrations(
            "a".repeat(64),
            "b".repeat(40),
            18,
            vec![crate::MigrationRecord {
                version: 1,
                checksum_hex: "c".repeat(96),
            }],
        )
        .unwrap();
        let manifest = BackupManifestV1::from_plan(
            id,
            source,
            &plan,
            FileRecord {
                path: "database.dump".into(),
                size: 5,
                sha256: digest(b"PGDMP"),
            },
            FileRecord {
                path: "roles.json".into(),
                size: 2,
                sha256: digest(b"{}"),
            },
        )
        .unwrap();
        let mut roots = BTreeMap::new();
        let mut files = BTreeMap::new();
        let mut roster = vec![];
        for (n, (key, kind)) in [
            ("assets", "source_assets"),
            ("control", "source_control"),
            ("local_pins", "local_pins"),
            ("staging", "source_staging"),
        ]
        .into_iter()
        .enumerate()
        {
            let enrollment = Uuid::new_v4().to_string();
            roots.insert(key.into(), enrollment.clone());
            let root = RootRecord {
                format_version: 1,
                capability: "backup_root_v1".into(),
                deployment_id: deployment.clone(),
                enrollment_id: enrollment.clone(),
                group_id: group_id.clone(),
                kind: kind.into(),
                path: format!("/unavailable/{key}"),
                dev: 1,
                ino: n as u64 + 10,
                uid: 0,
                mode: 448,
                enrolled_generation: 1,
            };
            let raw = canonical(&root).unwrap();
            roster.push(registry::Roster {
                id: enrollment.clone(),
                sha256: digest(&raw),
            });
            files.insert(format!("roots/{enrollment}.json"), raw);
        }
        roster.sort_by(|a, b| a.id.cmp(&b.id));
        let group = SourceGroup {
            format_version: 1,
            capability: "backup_source_group_v1".into(),
            deployment_id: deployment.clone(),
            group_id: group_id.clone(),
            database: format!("learning_backup_c4_task3_{}", Uuid::new_v4()),
            database_oid: 123,
            system_identifier: "123456".into(),
            source_binding_sha256: "f".repeat(64),
            application_commit: manifest.source.application_commit.clone(),
            application_build_sha256: manifest.source.application_build_sha256.clone(),
            roots,
        };
        files.insert("source-group.json".into(), canonical(&group).unwrap());
        let generation = Generation {
            format_version: 1,
            capability: "backup_registry_generation_v1".into(),
            deployment_id: deployment.clone(),
            generation: 1,
            previous_generation_sha256: None,
            roots: roster,
            groups: vec![registry::Roster {
                id: group_id.clone(),
                sha256: digest(&files["source-group.json"]),
            }],
        };
        files.insert(
            "source-generation.json".into(),
            canonical(&generation).unwrap(),
        );
        let generation_hash = digest(&files["source-generation.json"]);
        let authority = Authority {
            format_version: 1,
            capability: "backup_registry_v1".into(),
            deployment_id: deployment.clone(),
            registry_path: registry::REGISTRY_PATH.into(),
            registry_dev: 1,
            registry_ino: 2,
            lock_dev: 1,
            lock_ino: 3,
            directories: registry::DIRECTORY_NAMES
                .into_iter()
                .enumerate()
                .map(|(n, k)| {
                    (
                        k.into(),
                        registry::Identity {
                            dev: 1,
                            ino: n as u64 + 100,
                        },
                    )
                })
                .collect(),
            initial_generation: 1,
            initial_generation_sha256: generation_hash.clone(),
        };
        files.insert(
            "source-authority.json".into(),
            canonical(&authority).unwrap(),
        );
        let manifest_hash = manifest.canonical_sha256().unwrap();
        let mut released = SourceGateRecord::new(id);
        for phase in [
            GatePhase::Closed,
            GatePhase::Drained,
            GatePhase::DumpAndIndexDurable,
            GatePhase::PinsDurable,
            GatePhase::ReleaseReady,
            GatePhase::Released,
        ] {
            released
                .advance(
                    phase,
                    if matches!(
                        phase,
                        GatePhase::DumpAndIndexDurable | GatePhase::PinsDurable
                    ) {
                        Some(&manifest_hash)
                    } else {
                        None
                    },
                )
                .unwrap();
        }
        // This is the existing journal-v1 producer, not registry JSON ordering.
        files.insert(
            "source-released.json".into(),
            serde_json::to_vec(&released).unwrap(),
        );
        let mut previous = None;
        for (n, name, state) in [
            (0, "protection-pending.json", "capture_pending"),
            (1, "protection-catalog.json", "catalog_durable"),
            (2, "protection-retained.json", "retained"),
        ] {
            let mut record = serde_json::json!({"format_version":1,"capability":"source_protection_v1","deployment_id":deployment,"group_id":group_id,"backup_id":id,"registry_generation":1,"registry_generation_sha256":generation_hash,"source_binding_sha256":group.source_binding_sha256,"application_commit":group.application_commit,"application_build_sha256":group.application_build_sha256,"state":state,"previous_sha256":previous});
            if n == 0 {
                record["roots"] = serde_json::to_value(&group.roots).unwrap();
            }
            if n == 1 {
                record["asset_index"] = serde_json::to_value(plan.asset_index_file()).unwrap();
                record["logical_asset_count"] = 0.into();
            }
            if n == 2 {
                record["asset_index_sha256"] = digest(plan.asset_index_bytes()).into();
                record["manifest_sha256"] = manifest_hash.clone().into();
                record["sealed_name"] = format!("{id}.sealed").into();
                record["sealed_dev"] = 1.into();
                record["sealed_ino"] = 2.into();
            }
            let typed: HistoricalProtection = serde_json::from_value(record).unwrap();
            let raw = canonical(&typed).unwrap();
            previous = Some(digest(&raw));
            files.insert(name.into(), raw);
        }
        let mut refs = BTreeMap::new();
        for (path, raw) in &files {
            let logical = match path.as_str() {
                "source-authority.json" => "registry/authority.json".into(),
                "source-generation.json" => "registry/generations/00000000000000000001.json".into(),
                "source-group.json" => format!("registry/groups/{group_id}.json"),
                "source-released.json" => format!("control/{id}.control/released.json"),
                "protection-pending.json" => format!("protection/{id}/00-pending.json"),
                "protection-catalog.json" => format!("protection/{id}/10-catalog.json"),
                "protection-retained.json" => format!("protection/{id}/20-retained.json"),
                path => format!("registry/{path}"),
            };
            refs.insert(logical, raw.clone());
        }
        refs.insert(
            format!("protection/{id}/asset-index.json"),
            plan.asset_index_bytes().to_vec(),
        );
        let snapshot = SnapshotRecord {
            format_version: 1,
            capability: "protection_snapshot_v1".into(),
            source_group_id: group_id.clone(),
            registry_generation_sha256: generation_hash.clone(),
            protected_digest_count: 0,
            scan_bytes: refs.values().map(|b| b.len() as u64).sum(),
            records: refs
                .into_iter()
                .map(|(path, raw)| FileRecord {
                    path,
                    size: raw.len() as u64,
                    sha256: digest(&raw),
                })
                .collect(),
        };
        files.insert(
            "protection-snapshot.json".into(),
            canonical(&snapshot).unwrap(),
        );
        let trust = VerifierTrustConfig {
            format_version: 2,
            source_root: "/unavailable/local_pins".into(),
            source_control_root: "/unavailable/control".into(),
            destination_root: "/destination".into(),
            source_host_id: Uuid::new_v4(),
            source_storage_id: Uuid::new_v4(),
            destination_host_id: Uuid::new_v4(),
            destination_storage_id: Uuid::new_v4(),
            verifier_public_key_hex: format!("04{}", "a".repeat(128)),
        };
        let record = ProofRecord {
            format_version: 1,
            capability: "captured_source_proof_v1".into(),
            backup_id: id,
            manifest_sha256: manifest_hash,
            source_control_sha256: digest(&files["source-released.json"]),
            source_group_id: group_id,
            source_registry_generation_sha256: generation_hash,
            protection_snapshot_sha256: String::new(),
            files: vec![],
        };
        let mut f = Fixture {
            record,
            files,
            manifest,
            index: plan.asset_index_bytes().to_vec(),
            trust,
            source_calls: 0,
            destination_calls: 0,
        };
        f.refresh();
        f
    }
    #[test]
    fn destination_reopen_and_recovery_work_with_source_unavailable() {
        let mut f = fixture();
        assert!(
            f.read(EvidenceLocation::CapturedSource("/unavailable/control"))
                .is_err()
        );
        f.source_calls = 0;
        f.reopen().unwrap();
        assert_eq!(f.source_calls, 0);
        assert_eq!(f.destination_calls, 13);
    }
    #[test]
    fn destination_recovery_does_not_open_unrelated_historical_roots() {
        let mut f = fixture();
        let mut snapshot: SnapshotRecord =
            canonical_record(&f.files["protection-snapshot.json"], 67108864).unwrap();
        snapshot.records.push(FileRecord {
            path: "control/unrelated-historical.control/released.json".into(),
            size: 1,
            sha256: "a".repeat(64),
        });
        snapshot.records.sort_by(|a, b| a.path.cmp(&b.path));
        snapshot.scan_bytes += 1;
        f.files.insert(
            "protection-snapshot.json".into(),
            canonical(&snapshot).unwrap(),
        );
        f.refresh();
        f.reopen().unwrap();
        assert_eq!(f.source_calls, 0);
    }
    #[test]
    fn destination_full_reread_detects_missing_corrupt_or_extra_file() {
        for mode in ["missing", "corrupt", "extra"] {
            let mut f = fixture();
            match mode {
                "missing" => {
                    f.files.remove("protection-retained.json");
                }
                "corrupt" => f.files.get_mut("source-released.json").unwrap()[0] ^= 1,
                _ => {
                    f.files.insert("extra.json".into(), vec![1]);
                    f.refresh();
                }
            }
            assert!(f.reopen().is_err());
            assert_eq!(f.source_calls, 0);
        }
    }
    #[test]
    fn witness_binds_exact_source_manifest_and_control_record() {
        let mut f = fixture();
        f.record.source_control_sha256 = "a".repeat(64);
        assert!(f.reopen().is_err());
        let mut f = fixture();
        f.record.manifest_sha256 = "b".repeat(64);
        assert!(f.reopen().is_err());
        let mut f = fixture();
        f.files
            .get_mut("protection-retained.json")
            .unwrap()
            .push(b' ');
        f.refresh();
        assert!(f.reopen().is_err());
    }
    #[test]
    fn completion_rejects_stale_protection_or_replaced_enrolled_root() {
        let mut f = fixture();
        let mut snapshot: SnapshotRecord =
            canonical_record(&f.files["protection-snapshot.json"], 67108864).unwrap();
        snapshot.registry_generation_sha256 = "1".repeat(64);
        f.files.insert(
            "protection-snapshot.json".into(),
            canonical(&snapshot).unwrap(),
        );
        f.refresh();
        assert!(f.reopen().is_err());
        let mut f = fixture();
        let name = f
            .files
            .keys()
            .find(|v| v.starts_with("roots/"))
            .unwrap()
            .clone();
        let mut root: RootRecord = canonical_record(&f.files[&name], 16384).unwrap();
        root.ino += 1;
        f.files.insert(name, canonical(&root).unwrap());
        f.refresh();
        assert!(f.reopen().is_err());
    }
    #[test]
    fn captured_release_must_bind_both_dump_and_pin_to_manifest() {
        let mut f = fixture();
        let mut released: serde_json::Value =
            serde_json::from_slice(&f.files["source-released.json"]).unwrap();
        released["dump_and_index_sha256"] = serde_json::json!("9".repeat(64));
        let record: SourceGateRecord = serde_json::from_value(released).unwrap();
        let raw = serde_json::to_vec(&record).unwrap();
        f.record.source_control_sha256 = digest(&raw);
        let mut snapshot: SnapshotRecord =
            canonical_record(&f.files["protection-snapshot.json"], 67108864).unwrap();
        let reference = snapshot
            .records
            .iter_mut()
            .find(|r| r.path == format!("control/{}.control/released.json", f.record.backup_id))
            .unwrap();
        reference.sha256 = digest(&raw);
        reference.size = raw.len() as u64;
        f.files.insert("source-released.json".into(), raw);
        f.files.insert(
            "protection-snapshot.json".into(),
            canonical(&snapshot).unwrap(),
        );
        f.refresh();
        assert!(f.reopen().is_err());
    }

    #[cfg(target_os = "linux")]
    mod linux_cases {
        use super::*;
        use learning_assets::backup_fs::BackupDir;
        use std::{
            io::Write,
            os::unix::fs::{MetadataExt, PermissionsExt},
            path::PathBuf,
        };
        fn write(root: &BackupDir, name: &str, bytes: &[u8]) {
            let mut file = root.create_file(name).unwrap();
            file.write_all(bytes).unwrap();
            file.sync_all().unwrap();
            root.sync().unwrap();
        }
        fn package(root: &BackupDir, f: &Fixture) {
            let dir = root
                .create_dir(&format!("{}.sealed", f.record.backup_id))
                .unwrap();
            for (name, bytes) in [
                ("manifest.json", f.manifest.canonical_bytes().unwrap()),
                ("asset-index.json", f.index.clone()),
                ("database.dump", b"PGDMP".to_vec()),
                ("roles.json", b"{}".to_vec()),
            ] {
                write(&dir, name, &bytes);
                dir.seal_file(name).unwrap();
            }
            dir.seal_dir().unwrap();
            root.sync().unwrap();
        }
        fn destination_fixture_unpublished(f: &mut Fixture) -> (EnrolledDestination, PathBuf) {
            assert_eq!(unsafe { libc::geteuid() }, 0);
            // The consumer's build mount is deliberately read-only. Reuse the
            // driver's existing retained synthetic fs-* namespace on its fresh
            // source volume; no mount or production root authority is widened.
            let proof_root = PathBuf::from(
                std::env::var_os("TEST_C4_LIFECYCLE_PROOF_ROOT")
                    .expect("fixed fresh driver proof root"),
            );
            let base = proof_root
                .parent()
                .expect("driver source parent")
                .join(format!("fs-{}", Uuid::new_v4()));
            std::fs::create_dir(&base).unwrap();
            std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700)).unwrap();
            let parent = BackupDir::open_trusted_private_root(&base).unwrap();
            // Exercise the unchanged real journal writer before copying the
            // Released bytes into synthetic-history destination evidence.
            let journal_root = parent.create_dir("journal-source").unwrap();
            let mut journal =
                crate::SourceGateJournal::start_in(&journal_root, f.record.backup_id).unwrap();
            for phase in [
                GatePhase::Closed,
                GatePhase::Drained,
                GatePhase::DumpAndIndexDurable,
                GatePhase::PinsDurable,
                GatePhase::ReleaseReady,
                GatePhase::Released,
            ] {
                journal
                    .advance(
                        phase,
                        if matches!(
                            phase,
                            GatePhase::DumpAndIndexDurable | GatePhase::PinsDurable
                        ) {
                            Some(&f.record.manifest_sha256)
                        } else {
                            None
                        },
                    )
                    .unwrap();
            }
            let original = std::fs::read(
                base.join("journal-source")
                    .join(format!("{}.control", f.record.backup_id))
                    .join("released.json"),
            )
            .unwrap();
            assert_eq!(original, f.files["source-released.json"]);
            assert_eq!(digest(&original), f.record.source_control_sha256);
            let root = parent.create_dir("registry").unwrap();
            let deployment = Uuid::new_v4().to_string();
            let group_id = Uuid::new_v4().to_string();
            let mut directories = BTreeMap::new();
            for name in registry::DIRECTORY_NAMES {
                let d = root.create_dir(name).unwrap();
                let (dev, ino) = d.identity().unwrap();
                directories.insert(name.into(), registry::Identity { dev, ino });
            }
            let lock = root.create_file("registry.lock").unwrap();
            lock.sync_all().unwrap();
            let lm = lock.metadata().unwrap();
            let mut roots = BTreeMap::new();
            let mut roster = vec![];
            for (name, kind) in [
                ("destination", "backup_destination"),
                ("evidence", "destination_evidence"),
            ] {
                let dir = parent.create_dir(name).unwrap();
                let (dev, ino) = dir.identity().unwrap();
                let id = Uuid::new_v4().to_string();
                roots.insert(name.into(), id.clone());
                let r = RootRecord {
                    format_version: 1,
                    capability: "backup_root_v1".into(),
                    deployment_id: deployment.clone(),
                    enrollment_id: id.clone(),
                    group_id: group_id.clone(),
                    kind: kind.into(),
                    path: base.join(name).to_str().unwrap().into(),
                    dev,
                    ino,
                    uid: 0,
                    mode: 448,
                    enrolled_generation: 1,
                };
                let raw = canonical(&r).unwrap();
                write(
                    &root.open_dir("roots").unwrap(),
                    &format!("{id}.json"),
                    &raw,
                );
                roster.push(registry::Roster {
                    id,
                    sha256: digest(&raw),
                });
            }
            roster.sort_by(|a, b| a.id.cmp(&b.id));
            f.trust.destination_root = base.join("destination").to_str().unwrap().into();
            let g = registry::DestinationGroup {
                format_version: 1,
                capability: "backup_destination_group_v1".into(),
                deployment_id: deployment.clone(),
                group_id: group_id.clone(),
                destination_host_id: f.trust.destination_host_id.to_string(),
                destination_storage_id: f.trust.destination_storage_id.to_string(),
                roots,
            };
            let group_raw = canonical(&g).unwrap();
            write(
                &root.open_dir("groups").unwrap(),
                &format!("{group_id}.json"),
                &group_raw,
            );
            let generation = Generation {
                format_version: 1,
                capability: "backup_registry_generation_v1".into(),
                deployment_id: deployment.clone(),
                generation: 1,
                previous_generation_sha256: None,
                roots: roster,
                groups: vec![registry::Roster {
                    id: group_id,
                    sha256: digest(&group_raw),
                }],
            };
            let generation_raw = canonical(&generation).unwrap();
            write(
                &root.open_dir("generations").unwrap(),
                "00000000000000000001.json",
                &generation_raw,
            );
            let (dev, ino) = root.identity().unwrap();
            let authority = Authority {
                format_version: 1,
                capability: "backup_registry_v1".into(),
                deployment_id: deployment,
                registry_path: registry::REGISTRY_PATH.into(),
                registry_dev: dev,
                registry_ino: ino,
                lock_dev: lm.dev(),
                lock_ino: lm.ino(),
                directories,
                initial_generation: 1,
                initial_generation_sha256: digest(&generation_raw),
            };
            write(&root, "authority.json", &canonical(&authority).unwrap());
            root.sync().unwrap();
            parent.sync().unwrap();
            let registry = crate::ManagementRegistry::open_test(&base.join("registry")).unwrap();
            let lease = registry.try_lock().unwrap();
            let destination = lease.admit_destination(&base.join("destination")).unwrap();
            package(destination.destination(), f);
            (destination, base)
        }
        fn destination_fixture(f: &mut Fixture) -> (EnrolledDestination, PathBuf) {
            let (destination, base) = destination_fixture_unpublished(f);
            super::super::linux::persist_synthetic(
                &destination,
                f.record.backup_id,
                f.trust.clone(),
                f.files.clone(),
                None,
            )
            .unwrap();
            (destination, base)
        }
        fn evidence_publication_failure_seams() {
            use super::super::attempt::Step;
            for fault in [
                Step::BeforeRename,
                Step::ParentSync,
                Step::FinalReadback,
                Step::TerminalSync,
                Step::TerminalReadback,
                Step::FinalRecheck,
            ] {
                let mut prior = fixture();
                let (destination, base) = destination_fixture(&mut prior);
                let prior_raw = std::fs::read(
                    base.join("evidence")
                        .join(prior.record.backup_id.to_string())
                        .join("source-proof.json"),
                )
                .unwrap();
                let prior_attempt = base
                    .join("evidence")
                    .join(format!("{}.attempt", prior.record.backup_id));
                let prior_intent = std::fs::read(prior_attempt.join("intent.json")).unwrap();
                let prior_terminal = std::fs::read(prior_attempt.join("terminal.json")).unwrap();
                let prior_manifest =
                    crate::verify_sealed(&base.join("destination"), prior.record.backup_id)
                        .unwrap()
                        .manifest_sha256()
                        .to_owned();
                let mut next = fixture();
                next.trust = prior.trust.clone();
                package(destination.destination(), &next);
                let id = next.record.backup_id;
                assert!(
                    super::super::linux::persist_synthetic(
                        &destination,
                        id,
                        next.trust.clone(),
                        next.files.clone(),
                        Some(fault)
                    )
                    .is_err()
                );
                assert!(
                    destination
                        .evidence()
                        .open_dir(&format!("{id}.attempt"))
                        .is_ok()
                );
                let retained = registry::list(destination.evidence(), &destination.lease).unwrap();
                assert!(super::super::linux::read_proof(&destination, id, &next.trust).is_err());
                assert!(
                    super::super::linux::read_proof(
                        &destination,
                        prior.record.backup_id,
                        &prior.trust
                    )
                    .is_err(),
                    "error revokes all new-signing capability in this operation"
                );
                assert!(
                    super::super::linux::persist_synthetic(
                        &destination,
                        id,
                        next.trust.clone(),
                        next.files.clone(),
                        None
                    )
                    .is_err()
                );
                assert_eq!(
                    retained,
                    registry::list(destination.evidence(), &destination.lease).unwrap()
                );
                assert_eq!(
                    std::fs::read(
                        base.join("evidence")
                            .join(prior.record.backup_id.to_string())
                            .join("source-proof.json")
                    )
                    .unwrap(),
                    prior_raw
                );
                assert_eq!(
                    crate::verify_sealed(&base.join("destination"), prior.record.backup_id)
                        .unwrap()
                        .manifest_sha256(),
                    prior_manifest
                );
                assert_eq!(
                    std::fs::read(prior_attempt.join("intent.json")).unwrap(),
                    prior_intent
                );
                assert_eq!(
                    std::fs::read(prior_attempt.join("terminal.json")).unwrap(),
                    prior_terminal
                );
                assert!(
                    !base
                        .join("destination")
                        .join(format!("{id}.complete"))
                        .exists()
                );
                drop(destination);
                let registry =
                    crate::ManagementRegistry::open_test(&base.join("registry")).unwrap();
                let lease = registry.try_lock().unwrap();
                let reopened = lease.admit_destination(&base.join("destination")).unwrap();
                assert!(super::super::linux::read_proof(&reopened, id, &next.trust).is_err());
                assert!(
                    super::super::linux::persist_synthetic(
                        &reopened,
                        id,
                        next.trust.clone(),
                        next.files.clone(),
                        None
                    )
                    .is_err()
                );
                assert_eq!(
                    retained,
                    crate::registry::list(reopened.evidence(), &lease).unwrap()
                );
                // Storage inspection cannot reconstruct signing authority, even
                // when a failed terminal-sync left canonical terminal bytes.
                if matches!(
                    fault,
                    Step::TerminalSync | Step::TerminalReadback | Step::FinalRecheck
                ) {
                    let (_, identity) =
                        super::super::linux::inspect_committed(&reopened, id, &next.trust).unwrap();
                    assert!(
                        lease
                            .destination_operation
                            .lock()
                            .unwrap()
                            .require(&identity)
                            .is_err()
                    );
                }
                // This is only the committed-proof leg of archived recovery:
                // no local CompleteBackup or signer is manufactured here.
                super::super::linux::inspect_committed(
                    &reopened,
                    prior.record.backup_id,
                    &prior.trust,
                )
                .unwrap();
            }
        }
        #[test]
        #[ignore = "fixed fresh Linux root destination-negative case; no independent signing or CompleteBackup"]
        fn real_destination_negative() {
            for mutation in [
                "missing",
                "corrupt",
                "extra",
                "root-replaced",
                "evidence-replaced",
                "evidence-residue",
            ] {
                let mut f = fixture();
                let (destination, base) = destination_fixture(&mut f);
                super::super::linux::read_proof(&destination, f.record.backup_id, &f.trust)
                    .unwrap();
                let sealed = base
                    .join("destination")
                    .join(format!("{}.sealed", f.record.backup_id));
                match mutation {
                    "evidence-residue" => {
                        destination
                            .evidence()
                            .create_dir(&format!(
                                "{}.staging-{}",
                                f.record.backup_id,
                                Uuid::new_v4()
                            ))
                            .unwrap();
                    }
                    "root-replaced" | "evidence-replaced" => {
                        let name = if mutation == "root-replaced" {
                            "destination"
                        } else {
                            "evidence"
                        };
                        std::fs::rename(base.join(name), base.join(format!("old-{name}"))).unwrap();
                        std::fs::create_dir(base.join(name)).unwrap();
                        std::fs::set_permissions(
                            base.join(name),
                            std::fs::Permissions::from_mode(0o700),
                        )
                        .unwrap();
                    }
                    "missing" => {
                        std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o700))
                            .unwrap();
                        std::fs::remove_file(sealed.join("database.dump")).unwrap();
                        std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o500))
                            .unwrap();
                    }
                    "corrupt" => {
                        std::fs::set_permissions(
                            sealed.join("database.dump"),
                            std::fs::Permissions::from_mode(0o600),
                        )
                        .unwrap();
                        std::fs::write(sealed.join("database.dump"), b"bad").unwrap();
                        std::fs::set_permissions(
                            sealed.join("database.dump"),
                            std::fs::Permissions::from_mode(0o400),
                        )
                        .unwrap();
                    }
                    _ => {
                        std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o700))
                            .unwrap();
                        std::fs::write(sealed.join("extra"), b"extra").unwrap();
                        std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o500))
                            .unwrap();
                    }
                }
                assert!(
                    super::super::linux::read_proof(&destination, f.record.backup_id, &f.trust)
                        .is_err()
                );
                assert!(
                    !base
                        .join("destination")
                        .join(format!("{}.complete", f.record.backup_id))
                        .exists()
                );
            }
            evidence_publication_failure_seams();
            assert!(DestinationSigner::open_installed().is_err());
        }
        #[test]
        #[ignore = "fixed fresh Linux root transfer-interrupted case; sealed package only"]
        fn real_transfer_interrupted() {
            let mut first = fixture();
            let (destination, base) = destination_fixture(&mut first);
            let parent = BackupDir::open_trusted_private_root(&base).unwrap();
            let source = parent.create_dir("source-transfer").unwrap();
            let second = fixture();
            package(&source, &second);
            let before =
                crate::verify_sealed(&base.join("destination"), first.record.backup_id).unwrap();
            assert!(
                crate::sealed::interrupt_test_transfer(
                    &base.join("source-transfer"),
                    &base.join("destination"),
                    second.record.backup_id
                )
                .is_err()
            );
            let after =
                crate::verify_sealed(&base.join("destination"), first.record.backup_id).unwrap();
            assert_eq!(before.manifest_sha256(), after.manifest_sha256());
            super::super::linux::read_proof(&destination, first.record.backup_id, &first.trust)
                .unwrap();
            assert!(
                !base
                    .join("destination")
                    .join(format!("{}.sealed", second.record.backup_id))
                    .exists()
            );
            assert!(
                !base
                    .join("destination")
                    .join(format!("{}.complete", second.record.backup_id))
                    .exists()
            );
        }
    }
    fn envelope() -> ProofRecord {
        let record = fixture().record;
        validate_envelope(&record).expect("valid 12-file envelope baseline");
        record
    }
    #[test]
    fn captured_proof_rejects_unknown_version_and_invalid_identity() {
        let mut value = envelope();
        value.format_version = 2;
        assert!(validate_envelope(&value).is_err());
        value = envelope();
        value.backup_id = Uuid::nil();
        assert!(validate_envelope(&value).is_err());
    }
    #[test]
    fn captured_proof_rejects_path_escape_duplicate_and_unsorted_refs() {
        for path in [
            "../source",
            "/source",
            "roots/../../source",
            "roots/not-a-uuid.json",
        ] {
            let mut value = envelope();
            value.files[0].path = path.into();
            assert!(validate_envelope(&value).is_err());
        }
        let mut value = envelope();
        value.files[1] = value.files[0].clone();
        assert_eq!(value.files.len(), 12);
        assert!(validate_envelope(&value).is_err());
        let mut value = envelope();
        value.files.swap(0, 1);
        assert_eq!(value.files.len(), 12);
        assert!(validate_envelope(&value).is_err());
    }
    #[test]
    fn captured_proof_rejects_incomplete_inventory_and_oversized_metadata() {
        let mut value = envelope();
        value.files.pop();
        assert!(validate_envelope(&value).is_err());
        for index in 0..12 {
            let mut value = envelope();
            let cap = file_cap(&value.files[index].path);
            value.files[index].size = cap;
            validate_envelope(&value).unwrap();
            value.files[index].size = cap + 1;
            assert!(validate_envelope(&value).is_err());
        }
    }

    #[test]
    fn original_journal_v1_producer_bytes_reopen_without_reformatting() {
        let mut f = fixture();
        let original = f.files["source-released.json"].clone();
        let record: SourceGateRecord = serde_json::from_slice(&original).unwrap();
        assert_eq!(original, serde_json::to_vec(&record).unwrap());
        assert_ne!(original, canonical(&record).unwrap());
        let expected = digest(&original);
        let proof = f.reopen().unwrap();
        assert_eq!(proof.record.source_control_sha256, expected);
        assert_eq!(f.files["source-released.json"], original);
    }
    #[test]
    fn journal_v1_consumer_rejects_alternate_order_duplicate_extra_malformed_and_hash_substitution()
    {
        for mode in ["order", "duplicate", "extra", "malformed", "hash"] {
            let mut f = fixture();
            f.reopen().expect("valid original producer baseline");
            let raw = &f.files["source-released.json"];
            let released: SourceGateRecord = serde_json::from_slice(raw).unwrap();
            let mut changed = match mode {
                "order" => canonical(&released).unwrap(),
                "duplicate" => format!(
                    "{{\"phase\":\"released\",{}",
                    &std::str::from_utf8(raw).unwrap()[1..]
                )
                .into_bytes(),
                "extra" => format!("{{\"extra\":1,{}", &std::str::from_utf8(raw).unwrap()[1..])
                    .into_bytes(),
                "malformed" => b"{".to_vec(),
                _ => raw.clone(),
            };
            let mut snapshot: SnapshotRecord =
                canonical_record(&f.files["protection-snapshot.json"], 67108864).unwrap();
            let reference = snapshot
                .records
                .iter_mut()
                .find(|r| r.path == format!("control/{}.control/released.json", f.record.backup_id))
                .unwrap();
            snapshot.scan_bytes = snapshot.scan_bytes - reference.size + changed.len() as u64;
            reference.size = changed.len() as u64;
            reference.sha256 = digest(&changed);
            f.record.source_control_sha256 = if mode == "hash" {
                "0".repeat(64)
            } else {
                digest(&changed)
            };
            f.files
                .insert("source-released.json".into(), std::mem::take(&mut changed));
            f.files.insert(
                "protection-snapshot.json".into(),
                canonical(&snapshot).unwrap(),
            );
            f.refresh();
            assert!(f.reopen().is_err(), "{mode}");
        }
    }
}

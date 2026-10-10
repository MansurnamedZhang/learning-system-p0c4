//! Independent-target verification protocol. A source `.sealed` pin cannot
//! become a restorable backup without a separately pinned remote verifier.

use crate::{BackupError, valid_digest};
#[cfg(target_os = "linux")]
use crate::{GatePhase, SourceGateJournal, verify_sealed};
#[cfg(target_os = "linux")]
use learning_assets::backup_fs::BackupDir;
use ring::signature::{ECDSA_P256_SHA256_FIXED, UnparsedPublicKey};
use serde::{Deserialize, Serialize};
#[cfg(target_os = "linux")]
use sha2::{Digest, Sha256};
use std::path::Path;
#[cfg(target_os = "linux")]
use std::{
    io::{Read, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
};
use uuid::Uuid;

// Version 2 fixes the verifier algorithm to ECDSA P-256/SHA-256 with SEC1
// uncompressed keys and 64-byte fixed-width signatures. Version 1's Ed25519
// verifier is never accepted for a CompleteBackup.
const WITNESS_FORMAT_VERSION: u32 = 2;

fn canonical_posix_path(value: &str) -> bool {
    value.starts_with('/')
        && !value.contains('\\')
        && !value.contains('\0')
        && value
            .split('/')
            .skip(1)
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// Exactly the bytes an independently operated destination verifier signs
/// after it has re-opened and hashed every file in the destination package.
/// IDs are deployment identities, not evidence that two paths on one disk are
/// separate; the public key must be pinned to a genuinely separate verifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DestinationStatement {
    pub format_version: u32,
    pub backup_id: Uuid,
    pub manifest_sha256: String,
    pub source_control_sha256: String,
    pub source_root: String,
    pub source_control_root: String,
    pub source_host_id: Uuid,
    pub source_storage_id: Uuid,
    pub destination_host_id: Uuid,
    pub destination_storage_id: Uuid,
    pub destination_root: String,
}

impl DestinationStatement {
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, BackupError> {
        if self.format_version != WITNESS_FORMAT_VERSION
            || self.backup_id.is_nil()
            || !valid_digest(&self.manifest_sha256)
            || !valid_digest(&self.source_control_sha256)
            || !canonical_posix_path(&self.source_root)
            || !canonical_posix_path(&self.source_control_root)
            || self.source_host_id.is_nil()
            || self.destination_host_id.is_nil()
            || self.source_storage_id.is_nil()
            || self.destination_storage_id.is_nil()
            || self.source_host_id == self.destination_host_id
            || self.source_storage_id == self.destination_storage_id
            || !canonical_posix_path(&self.destination_root)
        {
            return Err(BackupError::Invalid("destination witness identity"));
        }
        Ok(serde_json::to_vec(self)?)
    }
}

/// Installed outside the package in a management-only root-owned directory.
/// Its P-256 public key must be provisioned from a verifier whose private
/// key is held in a genuinely separate host/storage failure domain. The
/// numeric identity and device checks below cannot establish that alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifierTrustConfig {
    pub format_version: u32,
    pub source_root: String,
    pub source_control_root: String,
    pub destination_root: String,
    pub source_host_id: Uuid,
    pub source_storage_id: Uuid,
    pub destination_host_id: Uuid,
    pub destination_storage_id: Uuid,
    pub verifier_public_key_hex: String,
}

impl VerifierTrustConfig {
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, BackupError> {
        if self.format_version != WITNESS_FORMAT_VERSION
            || !canonical_posix_path(&self.source_root)
            || !canonical_posix_path(&self.source_control_root)
            || !canonical_posix_path(&self.destination_root)
            || self.source_root == self.source_control_root
            || self.source_root == self.destination_root
            || self.source_control_root == self.destination_root
            || self.source_host_id.is_nil()
            || self.destination_host_id.is_nil()
            || self.source_storage_id.is_nil()
            || self.destination_storage_id.is_nil()
            || self.source_host_id == self.destination_host_id
            || self.source_storage_id == self.destination_storage_id
            || self.verifier_public_key_hex.len() != 130
            || !self.verifier_public_key_hex.starts_with("04")
            || !self
                .verifier_public_key_hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(BackupError::Invalid(
                "pinned destination verifier configuration",
            ));
        }
        Ok(serde_json::to_vec(self)?)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DestinationWitness {
    pub statement: DestinationStatement,
    pub signature_hex: String,
}

impl DestinationWitness {
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, BackupError> {
        self.statement.canonical_bytes()?;
        if self.signature_hex.len() != 128
            || !self
                .signature_hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(BackupError::Invalid(
                "destination witness signature encoding",
            ));
        }
        Ok(serde_json::to_vec(self)?)
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompleteReceiptBody {
    format_version: u32,
    backup_id: Uuid,
    witness: DestinationWitness,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompleteReceipt {
    body: CompleteReceiptBody,
    receipt_sha256: String,
}

/// An opaque handle to an independently witnessed, reverified package.
/// A `SealedBackup` has no conversion into this type.
#[derive(Debug)]
pub struct CompleteBackup {
    backup_id: Uuid,
    manifest_sha256: String,
    receipt_sha256: String,
    #[cfg(target_os = "linux")]
    pub(crate) source_control_sha256: String,
}

impl CompleteBackup {
    pub fn backup_id(&self) -> Uuid {
        self.backup_id
    }
    pub fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }
    pub fn receipt_sha256(&self) -> &str {
        &self.receipt_sha256
    }
}

#[cfg(target_os = "linux")]
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(target_os = "linux")]
pub(crate) fn require_deployment_verifier_key(
    trust: &VerifierTrustConfig,
) -> Result<(), BackupError> {
    // This build switch is intentionally insufficient by itself. A reviewed
    // deployment must compile in the fingerprint of an off-host verifier key.
    if !cfg!(feature = "independent-verifier") {
        return Err(BackupError::Invalid("independent verifier is not enabled"));
    }
    let baked = option_env!("KNOWWEAVE_C4_VERIFIER_KEY_SHA256").ok_or(BackupError::Invalid(
        "independent verifier key is not pinned in build",
    ))?;
    if !valid_digest(baked) {
        return Err(BackupError::Invalid("invalid build-pinned verifier key"));
    }
    let key = hex::decode(&trust.verifier_public_key_hex)
        .map_err(|_| BackupError::Invalid("pinned verifier public key"))?;
    if digest(&key) != baked {
        return Err(BackupError::Invalid(
            "verifier key differs from reviewed build",
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub(crate) fn load_trust(path: &Path) -> Result<VerifierTrustConfig, BackupError> {
    let parent = path
        .parent()
        .ok_or(BackupError::Invalid("verifier trust path"))?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(BackupError::Invalid("verifier trust file name"))?;
    let root = BackupDir::open_private_root(parent)?;
    let file = root.open_file(name)?;
    let meta = file.metadata()?;
    if meta.uid() != 0 || meta.permissions().mode() & 0o777 != 0o600 || meta.len() > 4096 {
        return Err(BackupError::Invalid(
            "root-owned private verifier trust file required",
        ));
    }
    let mut bytes = Vec::new();
    file.take(4097).read_to_end(&mut bytes)?;
    let config: VerifierTrustConfig = serde_json::from_slice(&bytes)?;
    if config.canonical_bytes()? != bytes {
        return Err(BackupError::Invalid("noncanonical verifier trust file"));
    }
    Ok(config)
}

#[cfg(target_os = "linux")]
fn path_matches(path: &Path, expected: &str) -> bool {
    path.to_str().is_some_and(|value| value == expected)
}

#[cfg(target_os = "linux")]
fn require_distinct_filesystems(source: &Path, destination: &Path) -> Result<(), BackupError> {
    let source = BackupDir::open_private_root(source)?;
    let destination = BackupDir::open_private_root(destination)?;
    if source.device_id()? == destination.device_id()? {
        return Err(BackupError::Invalid("destination shares source filesystem"));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn read_complete_receipt(root: &BackupDir, id: Uuid) -> Result<CompleteReceipt, BackupError> {
    let file = root.open_file(&format!("{id}.complete"))?;
    if file.metadata()?.len() > 8192 {
        return Err(BackupError::Capacity("complete receipt bytes"));
    }
    let mut bytes = Vec::new();
    file.take(8193).read_to_end(&mut bytes)?;
    let receipt: CompleteReceipt = serde_json::from_slice(&bytes)?;
    if serde_json::to_vec(&receipt)? != bytes
        || receipt.body.format_version != WITNESS_FORMAT_VERSION
        || receipt.body.backup_id != id
        || receipt.body.witness.canonical_bytes().is_err()
        || receipt.receipt_sha256 != digest(&serde_json::to_vec(&receipt.body)?)
    {
        return Err(BackupError::Invalid("complete receipt bytes or digest"));
    }
    Ok(receipt)
}

#[cfg(target_os = "linux")]
fn verify_receipt_against_target(
    destination_root: &Path,
    id: Uuid,
    trust: &VerifierTrustConfig,
) -> Result<CompleteBackup, BackupError> {
    if !path_matches(destination_root, &trust.destination_root) {
        return Err(BackupError::Invalid(
            "destination root differs from pinned verifier",
        ));
    }
    let sealed = verify_sealed(destination_root, id)?;
    let root = BackupDir::open_private_root(destination_root)?;
    root.sync()?;
    let receipt = read_complete_receipt(&root, id)?;
    let statement = &receipt.body.witness.statement;
    if statement.backup_id != id
        || statement.manifest_sha256 != sealed.manifest_sha256()
        || statement.source_root != trust.source_root
        || statement.source_control_root != trust.source_control_root
        || statement.source_host_id != trust.source_host_id
        || statement.source_storage_id != trust.source_storage_id
        || statement.destination_host_id != trust.destination_host_id
        || statement.destination_storage_id != trust.destination_storage_id
        || statement.destination_root != trust.destination_root
    {
        return Err(BackupError::Invalid("complete receipt target identity"));
    }
    let key = hex::decode(&trust.verifier_public_key_hex)
        .map_err(|_| BackupError::Invalid("pinned verifier public key"))?;
    let signature = hex::decode(&receipt.body.witness.signature_hex)
        .map_err(|_| BackupError::Invalid("destination witness signature"))?;
    verify_destination_witness(statement, &key, &signature)?;
    Ok(CompleteBackup {
        backup_id: id,
        manifest_sha256: sealed.manifest_sha256().into(),
        receipt_sha256: receipt.receipt_sha256,
        source_control_sha256: statement.source_control_sha256.clone(),
    })
}

/// Atomically publish a receipt only after a released source journal, two
/// independent full byte reads, and a pinned remote verifier signature.
/// Equal filesystem devices are rejected. Different device numbers alone do
/// not prove physical separation; the off-host key and failure-domain review
/// remain mandatory deployment gates.
pub fn publish_complete_backup(
    source_root: &Path,
    source_control_root: &Path,
    destination_root: &Path,
    trust_path: &Path,
    backup_id: Uuid,
    witness_bytes: &[u8],
) -> Result<CompleteBackup, BackupError> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (
            source_root,
            source_control_root,
            destination_root,
            trust_path,
            backup_id,
            witness_bytes,
        );
        Err(BackupError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "complete publication requires Linux and remote witness",
        )))
    }
    #[cfg(target_os = "linux")]
    {
        let trust = load_trust(trust_path)?;
        require_deployment_verifier_key(&trust)?;
        if !path_matches(source_root, &trust.source_root)
            || !path_matches(source_control_root, &trust.source_control_root)
            || !path_matches(destination_root, &trust.destination_root)
        {
            return Err(BackupError::Invalid(
                "backup roots differ from pinned verifier",
            ));
        }
        require_distinct_filesystems(source_root, destination_root)?;
        // Source authority is held only for publication. Recovery remains
        // destination/trust-only and does not reopen these captured paths.
        let registry = crate::ManagementRegistry::open_installed()?;
        let lease = registry.try_lock()?;
        // A fresh operation cannot reconstruct unsigned publication authority
        // from terminal files. Genuine cross-operation admission is Task9.
        let publication_fence =
            crate::destination::PublicationErrorFence::new(&lease.destination_operation);
        let control = BackupDir::open_trusted_private_root(source_control_root)?;
        let groups = lease
            .groups
            .values()
            .filter_map(|group| match group {
                crate::registry::Group::Source(group)
                    if lease.roots[&group.roots["control"]].path
                        == source_control_root.to_str().unwrap_or("") =>
                {
                    Some(group)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        if groups.len() != 1 {
            return Err(BackupError::Invalid("publication source enrollment"));
        }
        let enrolled = lease.admit_control_source(&control, &groups[0].database)?;
        if enrolled.roots["local_pins"].path != source_root.to_str().unwrap_or("") {
            return Err(BackupError::Invalid("publication source pin enrollment"));
        }
        let protection = crate::discover_backup_protection(&lease, &enrolled)?;
        let enrolled_destination = lease.admit_destination(destination_root)?;
        let source_dir = enrolled
            .handle("local_pins")
            .open_dir(&format!("{backup_id}.sealed"))?;
        let destination_dir = enrolled_destination
            .destination()
            .open_dir(&format!("{backup_id}.sealed"))?;
        let source_sha =
            crate::protection::verify_registered_package(&lease, &source_dir, backup_id)?
                .0
                .canonical_sha256()?;
        let destination_sha =
            crate::protection::verify_registered_package(&lease, &destination_dir, backup_id)?
                .0
                .canonical_sha256()?;
        if source_sha != destination_sha {
            return Err(BackupError::Invalid(
                "source and destination manifests differ",
            ));
        }
        let journal = SourceGateJournal::recover_in_metered(&control, backup_id, &lease.budget)?;
        if journal.record().phase() != GatePhase::Released
            || journal.record().pinned_manifest_sha256() != Some(source_sha.as_str())
        {
            return Err(BackupError::Invalid(
                "released source gate and durable pins required",
            ));
        }
        let control_sha = digest(&serde_json::to_vec(journal.record())?);
        if witness_bytes.len() > 8192 {
            return Err(BackupError::Capacity("destination witness bytes"));
        }
        let witness: DestinationWitness = serde_json::from_slice(witness_bytes)?;
        if witness.canonical_bytes()? != witness_bytes {
            return Err(BackupError::Invalid("noncanonical destination witness"));
        }
        let expected = DestinationStatement {
            format_version: WITNESS_FORMAT_VERSION,
            backup_id,
            manifest_sha256: source_sha,
            source_control_sha256: control_sha,
            source_root: trust.source_root.clone(),
            source_control_root: trust.source_control_root.clone(),
            source_host_id: trust.source_host_id,
            source_storage_id: trust.source_storage_id,
            destination_host_id: trust.destination_host_id,
            destination_storage_id: trust.destination_storage_id,
            destination_root: trust.destination_root.clone(),
        };
        if witness.statement != expected {
            return Err(BackupError::Invalid(
                "destination witness differs from source gate",
            ));
        }
        let key = hex::decode(&trust.verifier_public_key_hex)
            .map_err(|_| BackupError::Invalid("pinned verifier public key"))?;
        let signature = hex::decode(&witness.signature_hex)
            .map_err(|_| BackupError::Invalid("destination witness signature"))?;
        verify_destination_witness(&expected, &key, &signature)?;
        crate::destination::check_publication_proof(
            &enrolled_destination,
            &protection,
            &expected,
            &trust,
        )?;
        let body = CompleteReceiptBody {
            format_version: WITNESS_FORMAT_VERSION,
            backup_id,
            witness,
        };
        let receipt = CompleteReceipt {
            receipt_sha256: digest(&serde_json::to_vec(&body)?),
            body,
        };
        let bytes = serde_json::to_vec(&receipt)?;
        let root = enrolled_destination.destination().try_clone()?;
        let temporary = format!("{backup_id}.complete-staging-{}", Uuid::new_v4());
        let mut file = root.create_file(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        root.seal_file(&temporary)?;
        protection.recheck(&lease)?;
        enrolled_destination.recheck()?;
        root.rename_noreplace_without_sync(&temporary, &format!("{backup_id}.complete"))?;
        root.sync()?;
        let complete = verify_receipt_against_target(destination_root, backup_id, &trust)?;
        publication_fence.complete();
        Ok(complete)
    }
}

/// Recovery entrypoint: re-open the complete receipt and rehash every package
/// byte before any target database or asset write. Task 4 uses this handle,
/// never a `SealedBackup`, as its pre-write gate.
pub fn open_complete_backup(
    destination_root: &Path,
    trust_path: &Path,
    backup_id: Uuid,
) -> Result<CompleteBackup, BackupError> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (destination_root, trust_path, backup_id);
        Err(BackupError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "complete verification requires Linux",
        )))
    }
    #[cfg(target_os = "linux")]
    {
        let trust = load_trust(trust_path)?;
        require_deployment_verifier_key(&trust)?;
        verify_receipt_against_target(destination_root, backup_id, &trust)
    }
}

pub fn verify_destination_witness(
    statement: &DestinationStatement,
    pinned_public_key: &[u8],
    signature: &[u8],
) -> Result<(), BackupError> {
    let bytes = statement.canonical_bytes()?;
    // SEC1 uncompressed P-256 key and fixed-width (r || s) signature.
    // ring validates the curve point and nonzero, in-range r/s values. P-256
    // has cofactor one, avoiding Ed25519's small-order-key acceptance class.
    if pinned_public_key.len() != 65 || pinned_public_key[0] != 0x04 || signature.len() != 64 {
        return Err(BackupError::Invalid("destination witness key or signature"));
    }
    UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, pinned_public_key)
        .verify(&bytes, signature)
        .map_err(|_| BackupError::Invalid("destination witness signature"))
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};

    #[test]
    fn same_device_can_never_publish_complete() {
        let root = std::env::temp_dir().join(format!("c4-complete-device-{}", Uuid::new_v4()));
        let source = root.join("source");
        let destination = root.join("destination");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir(&destination).unwrap();
        for path in [&source, &destination] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        assert!(require_distinct_filesystems(&source, &destination).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(not(feature = "independent-verifier"))]
    #[test]
    fn default_build_cannot_open_or_publish_complete() {
        let trust = VerifierTrustConfig {
            format_version: WITNESS_FORMAT_VERSION,
            source_root: "/private/source".into(),
            source_control_root: "/private/control".into(),
            destination_root: "/private/destination".into(),
            source_host_id: Uuid::new_v4(),
            source_storage_id: Uuid::new_v4(),
            destination_host_id: Uuid::new_v4(),
            destination_storage_id: Uuid::new_v4(),
            verifier_public_key_hex: format!("04{}", "a".repeat(128)),
        };
        assert!(require_deployment_verifier_key(&trust).is_err());
    }
}

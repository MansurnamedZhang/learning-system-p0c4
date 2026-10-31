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

#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
pub(crate) use original_remote::{OriginalRemoteReceipt, publish_original_remote_receipt};

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

/// Concrete original-D writer. Never reopens source paths or acquires another
/// registry lease; the existing public Local publisher and reader stay intact.
#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
#[allow(
    dead_code,
    reason = "default-off original D receipt core awaits genuine M6 producers"
)]
mod original_remote {
    use super::*;
    use crate::{
        destination::{OriginalRemoteFinalization, RemoteConsumption},
        registry::ScanBudget,
    };
    use std::sync::{Arc, Mutex};

    const RECEIPT_BYTES: u64 = 8192;

    // Constructed only after concrete full readback; it is still kept inside
    // the original consume call until the same authority becomes Committed.
    // The caller never receives the pre-commit witness or CompleteBackup.
    pub(crate) struct OriginalRemoteReceipt {
        complete: CompleteBackup,
        raw: Vec<u8>,
        // Wire/Ack digest is over the entire canonical receipt. CompleteBackup
        // deliberately retains its historical v2 *body* digest contract.
        wire_sha256: String,
        written: std::fs::File,
        written_stamp: std::fs::Metadata,
    }
    impl OriginalRemoteReceipt {
        pub(crate) fn bytes(&self) -> &[u8] {
            &self.raw
        }
        pub(crate) fn receipt_sha256(&self) -> &str {
            &self.wire_sha256
        }
        pub(crate) fn recheck_committed_original(
            &self,
            committed: &crate::destination::CommittedRemoteOriginal,
        ) -> Result<(), BackupError> {
            let destination = committed.held_destination_for_committed();
            let expected = committed.statement_for_committed();
            let trust = committed.trust_for_committed();
            charge(&destination.lease.budget, RECEIPT_BYTES * 32, 16)?;
            if !same(&self.written_stamp, &self.written.metadata()?) {
                return Err(BackupError::Invalid(
                    "committed original receipt FD changed",
                ));
            }
            let raw = read_receipt_held(
                destination.destination(),
                &format!("{}.complete", expected.backup_id),
                &self.written,
                &destination.lease.budget,
            )?;
            if raw != self.raw || digest(&raw) != self.wire_sha256 {
                return Err(BackupError::Invalid(
                    "committed original receipt bytes changed",
                ));
            }
            checked_receipt(&raw, &expected, trust)?;
            Ok(())
        }
    }

    fn charge(
        budget: &Arc<Mutex<ScanBudget>>,
        bytes: u64,
        entries: usize,
    ) -> Result<(), BackupError> {
        let mut meter = budget
            .lock()
            .map_err(|_| BackupError::Invalid("remote original receipt budget poisoned"))?;
        meter.charge(bytes)?;
        meter.entries(entries)
    }

    fn checked_receipt(
        raw: &[u8],
        expected: &DestinationStatement,
        trust: &VerifierTrustConfig,
    ) -> Result<CompleteBackup, BackupError> {
        if raw.is_empty() || raw.len() as u64 > RECEIPT_BYTES {
            return Err(BackupError::Capacity("remote original receipt bytes"));
        }
        let receipt: CompleteReceipt = serde_json::from_slice(raw)?;
        if serde_json::to_vec(&receipt)? != raw
            || receipt.body.format_version != WITNESS_FORMAT_VERSION
            || receipt.body.backup_id != expected.backup_id
            || &receipt.body.witness.statement != expected
            || receipt.receipt_sha256 != digest(&serde_json::to_vec(&receipt.body)?)
            || expected.source_root != trust.source_root
            || expected.source_control_root != trust.source_control_root
            || expected.source_host_id != trust.source_host_id
            || expected.source_storage_id != trust.source_storage_id
            || expected.destination_host_id != trust.destination_host_id
            || expected.destination_storage_id != trust.destination_storage_id
            || expected.destination_root != trust.destination_root
        {
            return Err(BackupError::Invalid(
                "remote original receipt target linkage",
            ));
        }
        let public = hex::decode(&trust.verifier_public_key_hex)
            .map_err(|_| BackupError::Invalid("remote original witness public key"))?;
        let signature = hex::decode(&receipt.body.witness.signature_hex)
            .map_err(|_| BackupError::Invalid("remote original witness signature"))?;
        verify_destination_witness(expected, &public, &signature)?;
        Ok(CompleteBackup {
            backup_id: expected.backup_id,
            manifest_sha256: expected.manifest_sha256.clone(),
            receipt_sha256: receipt.receipt_sha256,
            source_control_sha256: expected.source_control_sha256.clone(),
        })
    }

    fn same(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
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

    fn read_receipt_held(
        root: &BackupDir,
        name: &str,
        written: &std::fs::File,
        budget: &Arc<Mutex<ScanBudget>>,
    ) -> Result<Vec<u8>, BackupError> {
        charge(budget, RECEIPT_BYTES * 8, 6)?;
        let mut file = root.open_file(name)?;
        let before = file.metadata()?;
        let original = written.metadata()?;
        if before.uid() != 0
            || before.gid() != 0
            || before.nlink() != 1
            || before.mode() & 0o7777 != 0o400
            || before.len() == 0
            || before.len() > RECEIPT_BYTES
            || !same(&before, &original)
        {
            return Err(BackupError::Invalid(
                "remote original sealed receipt custody",
            ));
        }
        let mut raw = vec![0; usize::try_from(before.len()).map_err(|_| BackupError::Overflow)?];
        file.read_exact(&mut raw)?;
        let mut extra = [0];
        if file.read(&mut extra)? != 0
            || !same(&before, &file.metadata()?)
            || !same(&before, &written.metadata()?)
            || !same(&before, &root.open_file(name)?.metadata()?)
        {
            return Err(BackupError::Invalid(
                "remote original receipt changed during readback",
            ));
        }
        Ok(raw)
    }

    pub(crate) fn publish_original_remote_receipt(
        original: &OriginalRemoteFinalization,
        owner: &RemoteConsumption<'_>,
        witness: DestinationWitness,
    ) -> Result<OriginalRemoteReceipt, BackupError> {
        original.check_deadline()?;
        let destination = original.held_destination_for_receipt(owner)?;
        let trust = original.trust_for_receipt(owner)?;
        let expected = original.statement_for_receipt(owner)?;
        let budget = &destination.lease.budget;
        charge(budget, RECEIPT_BYTES * 32, 16)?;
        require_deployment_verifier_key(trust)?;
        if witness.statement != expected || witness.canonical_bytes()?.len() as u64 > RECEIPT_BYTES
        {
            return Err(BackupError::Invalid(
                "remote original fixed receipt statement",
            ));
        }
        let body = CompleteReceiptBody {
            format_version: WITNESS_FORMAT_VERSION,
            backup_id: expected.backup_id,
            witness,
        };
        let receipt = CompleteReceipt {
            receipt_sha256: digest(&serde_json::to_vec(&body)?),
            body,
        };
        let raw = serde_json::to_vec(&receipt)?;
        checked_receipt(&raw, &expected, trust)?;
        original.check_deadline()?;
        let root = destination.destination();
        let root_identity = root.identity()?;
        let temporary = format!("{}.complete-staging-{}", expected.backup_id, Uuid::new_v4());
        let final_name = format!("{}.complete", expected.backup_id);
        let mut written = root.create_file(&temporary)?;
        original.check_deadline()?;
        charge(budget, raw.len() as u64, 1)?;
        written.write_all(&raw)?;
        original.check_deadline()?;
        written.sync_all()?;
        original.check_deadline()?;
        root.seal_file(&temporary)?;
        original.check_deadline()?;
        // The same concrete guard remains held through all blocking FS work.
        // No timeout task can drop the authority while a writer continues.
        destination.recheck()?;
        root.rename_noreplace_without_sync(&temporary, &final_name)?;
        original.check_deadline()?;
        root.sync()?;
        original.check_deadline()?;
        let actual = read_receipt_held(root, &final_name, &written, budget)?;
        if actual != raw || root.identity()? != root_identity {
            return Err(BackupError::Invalid("remote original receipt replaced"));
        }
        let complete = checked_receipt(&actual, &expected, trust)?;
        // Full package, original eleven-step proof and adjunct readback are
        // repeated under the original held D handles/budgets, never S paths.
        original.recheck_for_receipt(owner)?;
        original.recheck_published_receipt_roster(owner)?;
        if read_receipt_held(root, &final_name, &written, budget)? != actual {
            return Err(BackupError::Invalid(
                "remote original final receipt changed",
            ));
        }
        destination.recheck()?;
        original.check_deadline()?;
        let wire_sha256 = digest(&actual);
        let written_stamp = written.metadata()?;
        Ok(OriginalRemoteReceipt {
            complete,
            raw: actual,
            wire_sha256,
            written,
            written_stamp,
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn original_remote_receipt_readback_rejects_symlink_before_custody() {
            use std::{
                fs,
                os::unix::fs::{PermissionsExt, symlink},
            };
            let path =
                std::env::temp_dir().join(format!("c4-remote-receipt-nofollow-{}", Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            let root = BackupDir::open_private_root(&path).unwrap();
            let mut written = root.create_file("own-staging").unwrap();
            written
                .write_all(b"ordinary negative IO input, not a receipt")
                .unwrap();
            symlink("own-staging", path.join("own.complete")).unwrap();
            let budget = Arc::new(Mutex::new(ScanBudget::default()));
            let result = read_receipt_held(&root, "own.complete", &written, &budget);
            fs::remove_dir_all(&path).unwrap();
            assert!(
                matches!(result, Err(BackupError::Io(error)) if error.raw_os_error() == Some(libc::ELOOP))
            );
            assert!(budget.lock().unwrap().bytes > 0);
        }

        #[test]
        fn original_remote_receipt_parser_rejects_caps_before_json_or_crypto() {
            let trust = VerifierTrustConfig {
                format_version: 2,
                source_root: "/source/pins".into(),
                source_control_root: "/source/control".into(),
                destination_root: "/destination/pins".into(),
                source_host_id: Uuid::new_v4(),
                source_storage_id: Uuid::new_v4(),
                destination_host_id: Uuid::new_v4(),
                destination_storage_id: Uuid::new_v4(),
                verifier_public_key_hex: format!("04{}", "0".repeat(128)),
            };
            let statement = DestinationStatement {
                format_version: 2,
                backup_id: Uuid::new_v4(),
                manifest_sha256: "a".repeat(64),
                source_control_sha256: "b".repeat(64),
                source_root: trust.source_root.clone(),
                source_control_root: trust.source_control_root.clone(),
                source_host_id: trust.source_host_id,
                source_storage_id: trust.source_storage_id,
                destination_host_id: trust.destination_host_id,
                destination_storage_id: trust.destination_storage_id,
                destination_root: trust.destination_root.clone(),
            };
            assert!(matches!(
                checked_receipt(&[], &statement, &trust),
                Err(BackupError::Capacity(_))
            ));
            assert!(matches!(
                checked_receipt(&vec![b' '; 8193], &statement, &trust),
                Err(BackupError::Capacity(_))
            ));
        }
    }
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

// To be merged only by the owner of complete.rs. This stays a pure DATA
// validator. It does not load a file, open a target, qualify D, sign anything,
// create CompleteBackup, or attest the original publication call's outcome.
#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
pub(crate) fn validate_b1_receipt_data(
    raw: &[u8],
    expected: &DestinationStatement,
    pinned_public_key: &[u8],
    pinned_public_key_sha256: &str,
    original_budget: &std::sync::Arc<std::sync::Mutex<crate::registry::ScanBudget>>,
) -> Result<String, BackupError> {
    if raw.is_empty()
        || raw.len() > 8192
        || !valid_digest(pinned_public_key_sha256)
        || pinned_public_key.len() != 65
        || pinned_public_key[0] != 4
    {
        return Err(BackupError::Invalid(
            "B1 receipt DATA bounds or public-key pin",
        ));
    }
    let expected_strings = expected
        .source_root
        .len()
        .checked_add(expected.source_control_root.len())
        .and_then(|n| n.checked_add(expected.destination_root.len()))
        .and_then(|n| n.checked_add(expected.manifest_sha256.len()))
        .and_then(|n| n.checked_add(expected.source_control_sha256.len()))
        .ok_or(BackupError::Overflow)?;
    // Refuse an unbounded caller DATA statement before allocating its old
    // canonical form. The actual caller derives these paths from its retained
    // Source/trust and authenticated HelloD inputs, never from this receipt.
    if expected_strings > 8192 {
        return Err(BackupError::Capacity("B1 expected receipt statement DATA"));
    }
    let reservation = (raw.len() as u64)
        .checked_mul(16)
        .and_then(|n| n.checked_add((expected_strings as u64).checked_mul(16)?))
        .and_then(|n| n.checked_add(16_384))
        .ok_or(BackupError::Overflow)?;
    {
        let mut budget = original_budget
            .lock()
            .map_err(|_| BackupError::Invalid("B1 receipt original scan budget"))?;
        budget.charge(reservation)?;
        budget.entries(64)?;
    }
    if digest(pinned_public_key) != pinned_public_key_sha256 {
        return Err(BackupError::Invalid(
            "B1 receipt DATA public key differs from build pin",
        ));
    }
    expected.canonical_bytes()?;
    let receipt: CompleteReceipt = serde_json::from_slice(raw)?;
    // Preserve v2's existing struct-order byte encodings and embedded body
    // digest. Do not replace them with sorted registry JSON or a new schema.
    if serde_json::to_vec(&receipt)? != raw
        || receipt.body.format_version != WITNESS_FORMAT_VERSION
        || receipt.body.backup_id != expected.backup_id
        || &receipt.body.witness.statement != expected
        || receipt.body.witness.canonical_bytes().is_err()
        || receipt.receipt_sha256 != digest(&serde_json::to_vec(&receipt.body)?)
    {
        return Err(BackupError::Invalid(
            "B1 canonical receipt DATA or expected statement",
        ));
    }
    let signature = hex::decode(&receipt.body.witness.signature_hex)
        .map_err(|_| BackupError::Invalid("B1 receipt signature DATA"))?;
    verify_destination_witness(expected, pinned_public_key, &signature)?;
    // The wire result and ACK use SHA256(full canonical receipt bytes).
    // This is distinct from the old embedded SHA256(receipt.body) above.
    Ok(digest(raw))
}

#[cfg(all(test, target_os = "linux", feature = "offhost-b1"))]
mod b1_receipt_data_tests {
    use super::*;
    use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
    use std::sync::{Arc, Mutex};

    // Ephemeral test-only signature math. No installed key/file/actor or
    // independent host/storage/publication authority is constructed here.
    fn sample() -> (Vec<u8>, DestinationStatement, Vec<u8>, String) {
        let random = ring::rand::SystemRandom::new();
        let pkcs8 =
            EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &random).unwrap();
        let pair =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &random)
                .unwrap();
        let expected = DestinationStatement {
            format_version: 2,
            backup_id: Uuid::new_v4(),
            manifest_sha256: "1".repeat(64),
            source_control_sha256: "2".repeat(64),
            source_root: "/private/source".into(),
            source_control_root: "/private/control".into(),
            source_host_id: Uuid::new_v4(),
            source_storage_id: Uuid::new_v4(),
            destination_host_id: Uuid::new_v4(),
            destination_storage_id: Uuid::new_v4(),
            destination_root: "/private/destination".into(),
        };
        let signature = pair
            .sign(&random, &expected.canonical_bytes().unwrap())
            .unwrap();
        let body = CompleteReceiptBody {
            format_version: 2,
            backup_id: expected.backup_id,
            witness: DestinationWitness {
                statement: expected.clone(),
                signature_hex: hex::encode(signature.as_ref()),
            },
        };
        let body_digest = digest(&serde_json::to_vec(&body).unwrap());
        let raw = serde_json::to_vec(&CompleteReceipt {
            body,
            receipt_sha256: body_digest.clone(),
        })
        .unwrap();
        (
            raw,
            expected,
            pair.public_key().as_ref().to_vec(),
            body_digest,
        )
    }

    #[test]
    fn b1_receipt_data_uses_full_raw_digest_and_preserves_embedded_v2_body_domain() {
        let (raw, expected, public, body_digest) = sample();
        let budget = Arc::new(Mutex::new(crate::registry::ScanBudget::default()));
        let full =
            validate_b1_receipt_data(&raw, &expected, &public, &digest(&public), &budget).unwrap();
        assert_eq!(full, digest(&raw));
        assert_ne!(full, body_digest);
        assert!(budget.lock().unwrap().bytes > raw.len() as u64);
    }

    #[test]
    fn b1_receipt_data_rejects_changed_expected_statement_pin_and_noncanonical_bytes() {
        let (raw, mut expected, public, _) = sample();
        let budget = Arc::new(Mutex::new(crate::registry::ScanBudget::default()));
        assert!(
            validate_b1_receipt_data(&raw, &expected, &public, &"0".repeat(64), &budget).is_err()
        );
        let mut trailing = raw.clone();
        trailing.push(b'\n');
        assert!(
            validate_b1_receipt_data(&trailing, &expected, &public, &digest(&public), &budget)
                .is_err()
        );
        expected.manifest_sha256 = "3".repeat(64);
        assert!(
            validate_b1_receipt_data(&raw, &expected, &public, &digest(&public), &budget).is_err()
        );
        expected.manifest_sha256 = "1".repeat(64);
        let mut extra: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        extra["close"] = serde_json::json!(true);
        assert!(
            validate_b1_receipt_data(
                &serde_json::to_vec(&extra).unwrap(),
                &expected,
                &public,
                &digest(&public),
                &budget
            )
            .is_err()
        );
    }
}

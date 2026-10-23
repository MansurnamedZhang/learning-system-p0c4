//! Destination-only v2 adjunct. Copying never changes the v1 source proof,
//! signs a witness, manufactures CompleteBackup or enables an application.
use crate::{
    BackupError, CapturedSourceProof, EnrolledDestination, ExternalEffectFinding, FileRecord,
    RegistryLease,
    full_restore::jobs::JobInventory,
    source::snapshot_effects::{
        CapturedSnapshotEffectsV2,
        files::{self, Budget, charge, encode},
        verify,
        wire::{self, Capture, Ledger, NamespaceEntry, Prefix, hash, invalid},
    },
};
use learning_assets::{
    SnapshotDirectory, SnapshotJobDirectory,
    backup_fs::{BackupDir, BackupEntryKind},
    root_journal::SharedJournalBudget,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf, sync::Mutex};
use uuid::Uuid;

/// Only the normal copy operation below constructs this type. In particular a
/// JSON capture, current registry or bare destination directory is insufficient.
///
/// ```compile_fail
/// let _: learning_backup::CopiedSnapshotEffectsV2 = serde_json::from_str("{}").unwrap();
/// ```
pub struct CopiedSnapshotEffectsV2 {
    core: SnapshotCopyCore,
}

// Neither variant is caller-created. The private rehearsal wrapper can never
// be converted to the public, production-only copied capability.
pub(crate) enum CheckedSnapshotCopy {
    Production(CopiedSnapshotEffectsV2),
    #[cfg(test)]
    CurrentPin(CurrentPinSnapshotCopy),
}
impl From<CopiedSnapshotEffectsV2> for CheckedSnapshotCopy {
    fn from(copied: CopiedSnapshotEffectsV2) -> Self {
        Self::Production(copied)
    }
}
impl CheckedSnapshotCopy {
    fn core(&self) -> &SnapshotCopyCore {
        match self {
            Self::Production(copied) => &copied.core,
            #[cfg(test)]
            Self::CurrentPin(copied) => &copied.core,
        }
    }
    pub(crate) fn reconcile_with_budget(
        &self,
        target_identity: &Value,
        inventory: &JobInventory,
        budget: &Budget,
    ) -> Result<VerifiedSnapshotFindings, BackupError> {
        self.core()
            .reconcile_with_budget(target_identity, inventory, budget)
    }
    pub(crate) fn restore_deliveries(
        &self,
        target_identity: &Value,
        controlled_root: &BackupDir,
    ) -> Result<RecoverySnapshotCopies, BackupError> {
        self.core()
            .restore_deliveries(target_identity, controlled_root)
    }
}
impl CopiedSnapshotEffectsV2 {
    pub fn backup_id(&self) -> Uuid {
        self.core.record.backup_id
    }
    pub fn manifest_sha256(&self) -> &str {
        &self.core.record.manifest_sha256
    }
}

struct SnapshotCopyCore {
    root: BackupDir,
    directory: BackupDir,
    directory_path: PathBuf,
    origin: CopyOrigin,
    record: Capture,
    records: Vec<FileRecord>,
    inventory: JobInventory,
    namespace: Vec<NamespaceEntry>,
    prefix: Prefix,
    ledger: Ledger,
    source_identity: Value,
    capture_sha256: String,
    payload_sha256: String,
    intent_sha256: String,
    terminal_sha256: String,
    binding_sha256: String,
    budget: Budget,
    journal_budget: SharedJournalBudget,
    recovery_budget: Mutex<Option<Budget>>,
    recovery_identity: Mutex<Option<Value>>,
    attempt: BackupDir,
    attempt_files: Vec<FileRecord>,
}

enum CopyOrigin {
    Production {
        lease: RegistryLease,
        base_identity: super::attempt::CommitIdentity,
    },
    #[cfg(test)]
    CurrentPin(CurrentPinOrigin),
}
impl CopyOrigin {
    fn recheck(&self) -> Result<(), BackupError> {
        match self {
            Self::Production {
                lease,
                base_identity,
            } => {
                lease.recheck()?;
                lease
                    .destination_operation
                    .lock()
                    .map_err(|_| invalid())?
                    .require(base_identity)
            }
            #[cfg(test)]
            Self::CurrentPin(origin) => origin.recheck(),
        }
    }
    fn require_target(&self, _target: &Value) -> Result<(), BackupError> {
        match self {
            Self::Production { .. } => Ok(()),
            #[cfg(test)]
            Self::CurrentPin(origin) => require_current_pin_target(&origin.target, _target),
        }
    }
    fn insert_binding(&self, value: &mut Value, sha256: &str) {
        match self {
            Self::Production { .. } => value["captured_source_proof_sha256"] = json!(sha256),
            #[cfg(test)]
            Self::CurrentPin(_) => {
                value["capability"] = json!("verified_current_pin_snapshot_effect_findings_v2");
                value["current_pin_copy_binding_sha256"] = json!(sha256);
                value["authority_scope"] =
                    json!("PRIVATE_CURRENT_PIN_SINGLE_HOST_REHEARSAL_NOT_COMPLETE");
            }
        }
    }
}

enum CopyBasis<'a> {
    Production {
        destination: &'a EnrolledDestination,
        proof: &'a CapturedSourceProof,
        trust: &'a crate::VerifierTrustConfig,
        base_identity: &'a super::attempt::CommitIdentity,
    },
    #[cfg(test)]
    CurrentPin {
        raw: Vec<u8>,
        target: Value,
        control_identity: (u64, u64),
        package: &'a CurrentPinPackageBinding,
        pin_root: &'a crate::registry::RootRecord,
    },
}
impl CopyBasis<'_> {
    fn binding_raw(&self) -> Result<Vec<u8>, BackupError> {
        match self {
            Self::Production { proof, .. } => crate::registry::canonical(&proof.record),
            #[cfg(test)]
            Self::CurrentPin { raw, .. } => Ok(raw.clone()),
        }
    }
    fn recheck(&self) -> Result<(), BackupError> {
        match self {
            Self::Production { destination, .. } => destination.recheck(),
            #[cfg(test)]
            Self::CurrentPin {
                package, pin_root, ..
            } => {
                crate::registry::matches_root(
                    pin_root,
                    std::path::Path::new(&pin_root.path),
                    &package.root,
                )?;
                package.recheck()
            }
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn statements(
        &self,
        record: &Capture,
        capture_raw: &[u8],
        payload_raw: &[u8],
        binding_raw: &[u8],
        attempt_id: Uuid,
        operation_nonce: Uuid,
        budget: &Budget,
        journal: &SharedJournalBudget,
    ) -> Result<(Value, Value), BackupError> {
        match self {
            Self::Production {
                destination,
                proof,
                trust,
                base_identity,
            } => {
                let (destination_dev, destination_ino) = destination.destination().identity()?;
                let (evidence_dev, evidence_ino) = destination.evidence().identity()?;
                let complete_statement = super::statement(proof, trust);
                let complete_raw = complete_statement.canonical_bytes()?;
                charge(
                    budget,
                    journal,
                    (complete_raw.len() as u64)
                        .checked_mul(4)
                        .ok_or(BackupError::Overflow)?,
                    0,
                )?;
                let witness = json!({"format_version":2,"capability":"snapshot_effect_witness_v2",
                    "backup_id":record.backup_id,"effect_attempt_id":attempt_id,"base_attempt_id":base_identity.intent.attempt_id,
                    "manifest_sha256":record.manifest_sha256,"captured_source_proof_sha256":hash(binding_raw),
                    "capture_sha256":hash(capture_raw),"payload_manifest_sha256":hash(payload_raw),"binding_sha256":record.binding_sha256,
                    "epoch_sha256":record.epoch_sha256,"journal_head_sequence":record.journal_head_sequence,
                    "journal_head_sha256":record.journal_head_sha256,"cut_event_sha256":record.cut_event_sha256,
                    "complete_statement_sha256":hash(&complete_raw),"source_host_id":trust.source_host_id,"source_storage_id":trust.source_storage_id,
                    "destination_host_id":trust.destination_host_id,"destination_storage_id":trust.destination_storage_id,
                    "destination_dev":destination_dev,"destination_ino":destination_ino,"evidence_dev":evidence_dev,"evidence_ino":evidence_ino});
                let intent = json!({"format_version":2,"capability":"snapshot_effect_attempt_v2",
                    "backup_id":record.backup_id,"attempt_id":attempt_id,"base_attempt_id":base_identity.intent.attempt_id,
                    "operation_nonce":operation_nonce,"destination_dev":destination_dev,"destination_ino":destination_ino,
                    "evidence_dev":evidence_dev,"evidence_ino":evidence_ino,"manifest_sha256":record.manifest_sha256,
                    "captured_source_proof_sha256":hash(binding_raw),"capture_sha256":hash(capture_raw),"payload_manifest_sha256":hash(payload_raw)});
                Ok((witness, intent))
            }
            #[cfg(test)]
            Self::CurrentPin {
                target,
                control_identity,
                ..
            } => {
                let witness = json!({"format_version":2,"capability":"current_pin_snapshot_effect_copy_v2",
                    "backup_id":record.backup_id,"effect_attempt_id":attempt_id,"manifest_sha256":record.manifest_sha256,
                    "current_pin_copy_binding_sha256":hash(binding_raw),"capture_sha256":hash(capture_raw),
                    "payload_manifest_sha256":hash(payload_raw),"binding_sha256":record.binding_sha256,
                    "epoch_sha256":record.epoch_sha256,"journal_head_sequence":record.journal_head_sequence,
                    "journal_head_sha256":record.journal_head_sha256,"cut_event_sha256":record.cut_event_sha256,
                    "target":target,"recovery_control_identity":control_identity,
                    "authority_scope":"PRIVATE_CURRENT_PIN_SINGLE_HOST_REHEARSAL_NOT_COMPLETE","complete_backup":false});
                let intent = json!({"format_version":2,"capability":"current_pin_snapshot_effect_attempt_v2",
                    "backup_id":record.backup_id,"attempt_id":attempt_id,"operation_nonce":operation_nonce,
                    "manifest_sha256":record.manifest_sha256,"current_pin_copy_binding_sha256":hash(binding_raw),
                    "capture_sha256":hash(capture_raw),"payload_manifest_sha256":hash(payload_raw),
                    "target":target,"recovery_control_identity":control_identity,
                    "authority_scope":"PRIVATE_CURRENT_PIN_SINGLE_HOST_REHEARSAL_NOT_COMPLETE","complete_backup":false});
                Ok((witness, intent))
            }
        }
    }
}

#[cfg(test)]
pub(crate) struct CurrentPinSnapshotCopy {
    core: SnapshotCopyCore,
}

// No serde, Clone, public constructor, path-only admission or CompleteBackup
// conversion. The importer supplies the original Fresh(pin) and its held roots.
#[cfg(test)]
pub(crate) struct CurrentPinPackageBinding {
    root: BackupDir,
    directory: BackupDir,
    manifest: crate::BackupManifestV1,
    records: Vec<FileRecord>,
    budget: Budget,
    journal: SharedJournalBudget,
}
#[cfg(test)]
impl CurrentPinPackageBinding {
    pub(crate) fn from_fresh(
        pin: &crate::SourceLocalPin,
        root: &BackupDir,
        directory: &BackupDir,
        manifest: &crate::BackupManifestV1,
        budget: &Budget,
        journal: &SharedJournalBudget,
    ) -> Result<Self, BackupError> {
        let _ = files::typed_bytes(manifest, budget, journal)?;
        let _ = files::typed_bytes(pin.manifest(), budget, journal)?;
        if pin.manifest().canonical_bytes()? != manifest.canonical_bytes()?
            || pin.sealed().backup_id() != manifest.backup_id
            || pin.sealed().manifest_sha256() != manifest.canonical_sha256()?
            || root
                .open_dir(&format!("{}.sealed", manifest.backup_id))?
                .identity()?
                != directory.identity()?
        {
            return Err(invalid());
        }
        let mut records = manifest.files.clone();
        let raw = manifest.canonical_bytes()?;
        records.push(FileRecord {
            path: "manifest.json".into(),
            size: raw.len() as u64,
            sha256: hash(&raw),
        });
        records.sort_by(|a, b| a.path.cmp(&b.path));
        let result = Self {
            root: root.try_clone()?,
            directory: directory.try_clone()?,
            manifest: manifest.clone(),
            records,
            budget: budget.clone(),
            journal: journal.clone(),
        };
        result.recheck()?;
        Ok(result)
    }
    fn recheck(&self) -> Result<(), BackupError> {
        if self
            .root
            .open_dir(&format!("{}.sealed", self.manifest.backup_id))?
            .identity()?
            != self.directory.identity()?
        {
            return Err(invalid());
        }
        files::sealed_package_tree(&self.directory, &self.records, &self.budget, &self.journal)?;
        if self
            .root
            .open_dir(&format!("{}.sealed", self.manifest.backup_id))?
            .identity()?
            != self.directory.identity()?
        {
            return Err(invalid());
        }
        Ok(())
    }
}
#[cfg(test)]
struct CurrentPinOrigin {
    lease: RegistryLease,
    control: BackupDir,
    evidence_identity: (u64, u64),
    target: Value,
}
#[cfg(test)]
impl CurrentPinOrigin {
    fn recheck(&self) -> Result<(), BackupError> {
        self.lease.recheck()?;
        if self.control.open_dir("snapshot-effects-copy")?.identity()? != self.evidence_identity {
            return Err(invalid());
        }
        Ok(())
    }
}

/// Only the native Fresh(pin) importer calls this private path, after actual
/// target admission. It never creates the production CopiedSnapshotEffectsV2.
#[cfg(test)]
pub(crate) fn copy_current_pin_snapshot_effects(
    target: &mut crate::full_restore::ImportedTarget,
    package: CurrentPinPackageBinding,
    lease: &RegistryLease,
    captured: CapturedSnapshotEffectsV2,
) -> Result<CurrentPinSnapshotCopy, BackupError> {
    lease.recheck()?;
    package.recheck()?;
    let identity = target.recovery_identity();
    if package.manifest.backup_id != captured.record.backup_id
        || package.manifest.canonical_sha256()? != captured.record.manifest_sha256
        || serde_json::to_value(&package.manifest.source)? != captured.source_identity
        || identity["backup_id"] != captured.record.backup_id.to_string()
        || identity["manifest_sha256"] != captured.record.manifest_sha256
        || identity["source"] != captured.source_identity
    {
        return Err(invalid());
    }
    let source_group = lease
        .groups
        .get(&captured.record.source_group_id.to_string())
        .ok_or_else(invalid)?;
    let crate::registry::Group::Source(source_group) = source_group else {
        return Err(invalid());
    };
    let group_record = captured
        .record
        .files
        .iter()
        .find(|r| r.path == "source-group.json")
        .ok_or_else(invalid)?;
    let raw = files::read(
        &captured.directory,
        group_record,
        &captured.budget,
        &captured.journal_budget,
    )?;
    if raw != encode(source_group, &captured.budget, &captured.journal_budget)? {
        return Err(invalid());
    }
    charge(
        &captured.budget,
        &captured.journal_budget,
        2 * 16_384 * 32,
        2,
    )?;
    let generation_record = captured
        .record
        .files
        .iter()
        .find(|r| r.path == "source-generation.json")
        .ok_or_else(invalid)?;
    let generation_raw = files::read(
        &captured.directory,
        generation_record,
        &captured.budget,
        &captured.journal_budget,
    )?;
    if hash(&generation_raw) != captured.record.source_registry_generation_sha256 {
        return Err(invalid());
    }
    let generation: crate::registry::Generation =
        crate::registry::canonical_record(&generation_raw, 16_384)?;
    let actual_generation = generation
        .generation
        .checked_sub(1)
        .and_then(|n| usize::try_from(n).ok())
        .and_then(|n| lease.generations.get(n))
        .ok_or_else(invalid)?;
    if encode(
        actual_generation,
        &captured.budget,
        &captured.journal_budget,
    )? != generation_raw
    {
        return Err(invalid());
    }
    let mut roots = BTreeMap::new();
    for (name, id) in &source_group.roots {
        let root = lease.roots.get(id).ok_or_else(invalid)?;
        let _ = encode(root, &captured.budget, &captured.journal_budget)?;
        roots.insert(name.clone(), root.clone());
    }
    crate::registry::validate_capture_generation(
        &lease.generations,
        source_group,
        &roots,
        generation.generation,
        &captured.record.source_registry_generation_sha256,
        None,
    )?;
    let pin_root = lease
        .roots
        .get(source_group.roots.get("local_pins").ok_or_else(invalid)?)
        .ok_or_else(invalid)?;
    crate::registry::matches_root(
        pin_root,
        std::path::Path::new(&pin_root.path),
        &package.root,
    )?;
    let control = target.recovery_control()?;
    let evidence = control.create_dir("snapshot-effects-copy")?;
    control.sync()?;
    if evidence.identity()? == captured.directory.identity()?
        || evidence.identity()? == package.directory.identity()?
    {
        return Err(invalid());
    }
    let budget = target.recovery_metadata_budget();
    let binding = json!({"format_version":2,"capability":"current_pin_package_copy_binding_v2",
        "backup_id":package.manifest.backup_id,"manifest_sha256":package.manifest.canonical_sha256()?,
        "source":captured.source_identity,"source_group_id":captured.record.source_group_id,
        "source_registry_generation_sha256":captured.record.source_registry_generation_sha256,
        "source_pin_root_identity":package.root.identity()?,"source_pin_identity":package.directory.identity()?,
        "target":identity,"recovery_control_identity":control.identity()?,"copy_root_identity":evidence.identity()?,
        "authority_scope":"PRIVATE_CURRENT_PIN_SINGLE_HOST_REHEARSAL_NOT_COMPLETE","complete_backup":false});
    let raw = encode(&binding, &budget, &captured.journal_budget)?;
    let control_identity = control.identity()?;
    let origin = CopyOrigin::CurrentPin(CurrentPinOrigin {
        lease: lease.clone_held()?,
        control,
        evidence_identity: evidence.identity()?,
        target: identity.clone(),
    });
    let basis = CopyBasis::CurrentPin {
        raw,
        target: identity,
        control_identity,
        package: &package,
        pin_root,
    };
    let core = write_copy(
        &evidence,
        std::path::Path::new(""),
        captured,
        budget,
        origin,
        basis,
    )?;
    core.recheck()?;
    Ok(CurrentPinSnapshotCopy { core })
}

/// A private observation consumed by the existing normalization planner. The
/// map cannot be constructed, deserialized, cloned or supplied by a caller.
pub(crate) struct VerifiedSnapshotFindings {
    findings: BTreeMap<Uuid, ExternalEffectFinding>,
    evidence: Value,
}
impl VerifiedSnapshotFindings {
    pub(crate) fn finding(&self, job: Uuid) -> ExternalEffectFinding {
        self.findings
            .get(&job)
            .copied()
            .unwrap_or(ExternalEffectFinding::Unknown)
    }
    pub(crate) fn evidence(&self) -> &Value {
        &self.evidence
    }
}

/// An independently read-back destination copy from the original source cut
/// and the original twelve-file proof. Interrupted attempts remain blocking.
pub fn copy_snapshot_effects(
    destination: &EnrolledDestination,
    proof: &CapturedSourceProof,
    captured: CapturedSnapshotEffectsV2,
) -> Result<CopiedSnapshotEffectsV2, BackupError> {
    let fence = super::PublicationErrorFence::new(&destination.lease.destination_operation);
    destination.recheck()?;
    let budget = destination.lease.budget.clone();
    let journal = &captured.journal_budget;
    charge(&budget, journal, 2 * 16_384 * 32, 2)?;
    let trust = super::installed_trust()?;
    let (original_proof, base_identity) =
        super::linux::read_authorized(destination, proof.backup_id(), &trust)?;
    if crate::registry::canonical(&original_proof.record)?
        != crate::registry::canonical(&proof.record)?
    {
        return Err(invalid());
    }
    if proof.record.backup_id != captured.record.backup_id
        || proof.record.manifest_sha256 != captured.record.manifest_sha256
        || proof.record.source_group_id != captured.record.source_group_id.to_string()
        || proof.record.source_registry_generation_sha256
            != captured.record.source_registry_generation_sha256
        || destination.evidence().identity()? == captured.directory.identity()?
    {
        return Err(invalid());
    }
    let evidence_path =
        PathBuf::from(&destination.lease.roots[&destination.group.roots["evidence"]].path);
    let origin = CopyOrigin::Production {
        lease: destination.lease.clone_held()?,
        base_identity: base_identity.clone(),
    };
    let basis = CopyBasis::Production {
        destination,
        proof,
        trust: &trust,
        base_identity: &base_identity,
    };
    let core = write_copy(
        destination.evidence(),
        &evidence_path,
        captured,
        budget,
        origin,
        basis,
    )?;
    let result = super::attempt::finalize_use(
        &destination.lease.destination_operation,
        &base_identity,
        || Ok(CopiedSnapshotEffectsV2 { core }),
    )?;
    fence.complete();
    Ok(result)
}

fn write_copy(
    evidence: &BackupDir,
    evidence_path: &std::path::Path,
    captured: CapturedSnapshotEffectsV2,
    budget: Budget,
    origin: CopyOrigin,
    basis: CopyBasis<'_>,
) -> Result<SnapshotCopyCore, BackupError> {
    let journal = &captured.journal_budget;
    let root = match evidence.kind(".snapshot-effects-v2") {
        Ok(BackupEntryKind::Directory) => evidence.open_dir(".snapshot-effects-v2")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let dir = evidence.create_dir(".snapshot-effects-v2")?;
            evidence.sync()?;
            dir
        }
        _ => return Err(invalid()),
    };
    root.require_private_directory()?;
    let id = captured.record.backup_id.to_string();
    for name in root.list_bounded(wire::MAX_FILES)? {
        if name == id
            || name == format!("{id}.attempt")
            || name.starts_with(&format!("{id}.staging-"))
        {
            return Err(invalid());
        }
    }
    let capture_raw = encode(&captured.record, &budget, journal)?;
    // The original exact-twelve-file proof is capped at 16KiB by its reader.
    // Pay its closed, shallow canonical conversion before touching it.
    charge(&budget, journal, 16_384 * 32, 1)?;
    let source_proof_raw = basis.binding_raw()?;
    if source_proof_raw.len() > 16_384 {
        return Err(invalid());
    }
    let mut records = captured.record.files.clone();
    records.push(FileRecord {
        path: "capture.json".into(),
        size: capture_raw.len() as u64,
        sha256: hash(&capture_raw),
    });
    records.sort_by(|a, b| a.path.cmp(&b.path));
    files::exact_tree(&captured.directory, &records, &captured.budget, journal)?;
    let payload = json!({"format_version":2,"capability":"snapshot_effect_payload_v2","backup_id":captured.record.backup_id,"files":records});
    let payload_raw = encode(&payload, &budget, journal)?;
    let attempt_id = Uuid::new_v4();
    let operation_nonce = Uuid::new_v4();
    let attempt_name = format!("{id}.attempt");
    let attempt = root.create_dir(&attempt_name)?;
    root.sync()?;
    let (effect_statement, intent) = basis.statements(
        &captured.record,
        &capture_raw,
        &payload_raw,
        &source_proof_raw,
        attempt_id,
        operation_nonce,
        &budget,
        journal,
    )?;
    let effect_statement_raw = encode(&effect_statement, &budget, journal)?;
    let mut intent = intent;
    intent["effect_statement_sha256"] = json!(hash(&effect_statement_raw));
    let intent_raw = encode(&intent, &budget, journal)?;
    let intent_file = files::put(&attempt, "intent.json", &intent_raw, &budget, journal)?;
    let mut attempt_files = vec![intent_file];
    #[cfg(test)]
    if matches!(&basis, CopyBasis::CurrentPin { .. }) {
        attempt_files.push(files::put(
            &attempt,
            "current-pin-binding.json",
            &source_proof_raw,
            &budget,
            journal,
        )?);
        attempt_files.push(files::put(
            &attempt,
            "private-copy-statement.json",
            &effect_statement_raw,
            &budget,
            journal,
        )?);
    }
    let staging_name = format!("{id}.staging-{attempt_id}");
    let directory = root.create_dir(&staging_name)?;
    root.sync()?;
    copy_records(
        &captured.directory,
        &directory,
        &records,
        &captured.budget,
        &budget,
        journal,
    )?;
    records.push(files::put(
        &directory,
        "payload-manifest.json",
        &payload_raw,
        &budget,
        journal,
    )?);
    records.sort_by(|a, b| a.path.cmp(&b.path));
    // Independent target bytes, including every package, are checked after the
    // copy and before any no-replace publication or opaque result can exist.
    files::exact_tree(&directory, &records, &budget, journal)?;
    let checked = verify::inventory(
        &captured.inventory,
        &captured.ledger,
        &captured.prefix,
        &captured.namespace,
        &budget,
        journal,
    )?;
    verify_copied_packages(
        &directory,
        &captured.record,
        &checked,
        &captured.inventory,
        &budget,
        journal,
    )?;
    basis.recheck()?;
    origin.recheck()?;
    files::seal_tree(&directory)?;
    root.rename_noreplace_without_sync(&staging_name, &id)?;
    root.sync()?;
    evidence.sync()?;
    if root.open_dir(&id)?.identity()? != directory.identity()? {
        return Err(invalid());
    }
    files::exact_tree(&directory, &records, &budget, journal)?;
    #[allow(unused_mut)]
    let mut terminal = json!({"format_version":2,"capability":"snapshot_effect_commit_v2",
        "backup_id":captured.record.backup_id,"attempt_id":attempt_id,"intent_sha256":hash(&intent_raw)});
    #[cfg(test)]
    if matches!(&basis, CopyBasis::CurrentPin { .. }) {
        terminal["capability"] = json!("current_pin_snapshot_effect_commit_v2");
        terminal["authority_scope"] =
            json!("PRIVATE_CURRENT_PIN_SINGLE_HOST_REHEARSAL_NOT_COMPLETE");
        terminal["complete_backup"] = json!(false);
    }
    let terminal_raw = encode(&terminal, &budget, journal)?;
    let terminal_file = files::put(&attempt, "terminal.json", &terminal_raw, &budget, journal)?;
    attempt.sync()?;
    root.sync()?;
    evidence.sync()?;
    attempt_files.push(terminal_file);
    basis.recheck()?;
    origin.recheck()?;
    Ok(SnapshotCopyCore {
        root,
        directory,
        directory_path: evidence_path.join(".snapshot-effects-v2").join(&id),
        origin,
        record: captured.record,
        records,
        inventory: captured.inventory,
        namespace: captured.namespace,
        prefix: captured.prefix,
        ledger: captured.ledger,
        source_identity: captured.source_identity,
        capture_sha256: hash(&capture_raw),
        payload_sha256: hash(&payload_raw),
        intent_sha256: hash(&intent_raw),
        terminal_sha256: hash(&terminal_raw),
        binding_sha256: hash(&source_proof_raw),
        budget,
        journal_budget: captured.journal_budget,
        recovery_budget: Mutex::new(None),
        recovery_identity: Mutex::new(None),
        attempt,
        attempt_files,
    })
}

fn copy_records(
    source: &BackupDir,
    destination: &BackupDir,
    records: &[FileRecord],
    source_budget: &Budget,
    destination_budget: &Budget,
    journal: &SharedJournalBudget,
) -> Result<(), BackupError> {
    if source.identity()? == destination.identity()? {
        return Err(invalid());
    }
    for record in records {
        let raw = files::read(source, record, source_budget, journal)?;
        if files::put(destination, &record.path, &raw, destination_budget, journal)? != *record {
            return Err(invalid());
        }
    }
    Ok(())
}

#[cfg(test)]
fn require_current_pin_target(original: &Value, target: &Value) -> Result<(), BackupError> {
    if original != target {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
fn current_copy_path_matches(
    control: &BackupDir,
    evidence_identity: (u64, u64),
    root: &BackupDir,
    directory: &BackupDir,
    attempt: &BackupDir,
    backup_id: Uuid,
) -> Result<bool, BackupError> {
    let evidence = control.open_dir("snapshot-effects-copy")?;
    if evidence.identity()? != evidence_identity {
        return Ok(false);
    }
    let reopened = evidence.open_dir(".snapshot-effects-v2")?;
    Ok(reopened.identity()? == root.identity()?
        && reopened.open_dir(&backup_id.to_string())?.identity()? == directory.identity()?
        && reopened
            .open_dir(&format!("{backup_id}.attempt"))?
            .identity()?
            == attempt.identity()?)
}

impl SnapshotCopyCore {
    fn recheck(&self) -> Result<(), BackupError> {
        self.origin.recheck()?;
        let path_matches = match &self.origin {
            CopyOrigin::Production { .. } => {
                BackupDir::open_trusted_private_root(&self.directory_path)?.identity()?
                    == self.directory.identity()?
            }
            #[cfg(test)]
            CopyOrigin::CurrentPin(origin) => current_copy_path_matches(
                &origin.control,
                origin.evidence_identity,
                &self.root,
                &self.directory,
                &self.attempt,
                self.record.backup_id,
            )?,
        };
        if !path_matches
            || self
                .root
                .open_dir(&self.record.backup_id.to_string())?
                .identity()?
                != self.directory.identity()?
            || self
                .root
                .open_dir(&format!("{}.attempt", self.record.backup_id))?
                .identity()?
                != self.attempt.identity()?
        {
            return Err(invalid());
        }
        if let Some(budget) = self.recovery_budget.lock().map_err(|_| invalid())?.as_ref() {
            let bytes = self.records.iter().try_fold(0u64, |sum, record| {
                sum.checked_add(record.size.checked_mul(2).ok_or(BackupError::Overflow)?)
                    .ok_or(BackupError::Overflow)
            })?;
            let mut meter = budget.lock().map_err(|_| invalid())?;
            meter.charge(bytes)?;
            meter.entries(self.records.len())?;
        }
        files::exact_tree(
            &self.directory,
            &self.records,
            &self.budget,
            &self.journal_budget,
        )?;
        files::exact_tree(
            &self.attempt,
            &self.attempt_files,
            &self.budget,
            &self.journal_budget,
        )?;
        Ok(())
    }

    pub(crate) fn reconcile_with_budget(
        &self,
        target_identity: &Value,
        target_inventory: &JobInventory,
        budget: &Budget,
    ) -> Result<VerifiedSnapshotFindings, BackupError> {
        {
            let mut slot = self.recovery_budget.lock().map_err(|_| invalid())?;
            if slot
                .as_ref()
                .is_some_and(|original| !std::sync::Arc::ptr_eq(original, budget))
            {
                return Err(invalid());
            }
            *slot = Some(budget.clone());
        }
        let verified = self.reconcile(target_identity, target_inventory)?;
        let mut identity = self.recovery_identity.lock().map_err(|_| invalid())?;
        if identity
            .as_ref()
            .is_some_and(|original| original != target_identity)
        {
            return Err(invalid());
        }
        *identity = Some(target_identity.clone());
        Ok(verified)
    }

    pub(crate) fn reconcile(
        &self,
        target_identity: &Value,
        target_inventory: &JobInventory,
    ) -> Result<VerifiedSnapshotFindings, BackupError> {
        self.recheck()?;
        self.origin.require_target(target_identity)?;
        if target_identity["capability"] != "held_recovery_target_v1"
            || target_identity["backup_id"] != self.record.backup_id.to_string()
            || target_identity["manifest_sha256"] != self.record.manifest_sha256
            || target_identity["source"] != self.source_identity
            || target_inventory != &self.inventory
            || target_identity["database_oid"]
                .as_u64()
                .is_none_or(|n| n == 0)
            || !crate::valid_digest(
                target_identity["container_id"]
                    .as_str()
                    .ok_or_else(invalid)?,
            )
        {
            return Err(invalid());
        }
        let findings = verify::inventory(
            target_inventory,
            &self.ledger,
            &self.prefix,
            &self.namespace,
            &self.budget,
            &self.journal_budget,
        )?;
        let mut evidence = json!({"format_version":2,"capability":"verified_snapshot_effect_findings_v2",
            "backup_id":self.record.backup_id,"capture_sha256":self.capture_sha256,
            "payload_manifest_sha256":self.payload_sha256,"intent_sha256":self.intent_sha256,
            "terminal_sha256":self.terminal_sha256,
            "target":target_identity,"source_access":false,"complete_authority":false,
            "journal_head_sequence":self.record.journal_head_sequence,"journal_head_sha256":self.record.journal_head_sha256});
        self.origin
            .insert_binding(&mut evidence, &self.binding_sha256);
        encode(&evidence, &self.budget, &self.journal_budget)?;
        Ok(VerifiedSnapshotFindings { findings, evidence })
    }
}

/// A third, actual byte copy owned by the original recovery attempt. It is not
/// a new publication token and is never installed in an application directory.
pub(crate) struct RecoverySnapshotCopies {
    root: BackupDir,
    target: Value,
    records: Vec<FileRecord>,
    packages: BTreeMap<Uuid, (Uuid, String)>,
    receipt_sha256: String,
    budget: Budget,
    journal: SharedJournalBudget,
}
impl RecoverySnapshotCopies {
    pub(crate) fn receipt_sha256(&self) -> &str {
        &self.receipt_sha256
    }
    pub(crate) fn with_package<T>(
        &self,
        job: Uuid,
        visitor: impl FnOnce(&SnapshotDirectory) -> Result<T, BackupError>,
    ) -> Result<T, BackupError> {
        files::exact_tree(&self.root, &self.records, &self.budget, &self.journal)?;
        let (token, manifest) = self.packages.get(&job).ok_or_else(invalid)?;
        let bytes = self
            .records
            .iter()
            .filter(|record| record.path.starts_with(&format!("{job}/{token}.ready/")))
            .try_fold(0u64, |n, record| {
                n.checked_add(record.size).ok_or(BackupError::Overflow)
            })?;
        charge(
            &self.budget,
            &self.journal,
            bytes.checked_mul(16).ok_or(BackupError::Overflow)?,
            8,
        )?;
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_DELIVERY_CHECKPOINT step=PACKAGE_OPEN_EXISTING_BEFORE");
        let reader =
            SnapshotJobDirectory::open_existing_in(&self.root, job).map_err(|_| invalid())?;
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_DELIVERY_CHECKPOINT step=PACKAGE_OPEN_EXISTING_AFTER");
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_DELIVERY_CHECKPOINT step=PACKAGE_REOPEN_BEFORE");
        let package = reader.reopen(*token, manifest).map_err(|_| invalid())?;
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_DELIVERY_CHECKPOINT step=PACKAGE_REOPEN_AFTER");
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_DELIVERY_CHECKPOINT step=PACKAGE_VISITOR_BEFORE");
        let result = visitor(&package)?;
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_DELIVERY_CHECKPOINT step=PACKAGE_VISITOR_AFTER");
        files::exact_tree(&self.root, &self.records, &self.budget, &self.journal)?;
        Ok(result)
    }
    pub(crate) fn target_identity(&self) -> &Value {
        &self.target
    }
}

impl SnapshotCopyCore {
    fn restore_deliveries(
        &self,
        target_identity: &Value,
        controlled_root: &BackupDir,
    ) -> Result<RecoverySnapshotCopies, BackupError> {
        self.recheck()?;
        self.origin.require_target(target_identity)?;
        if self
            .recovery_identity
            .lock()
            .map_err(|_| invalid())?
            .as_ref()
            != Some(target_identity)
        {
            return Err(invalid());
        }
        controlled_root.require_private_directory()?;
        if !controlled_root.list_bounded(wire::MAX_FILES)?.is_empty() {
            return Err(invalid());
        }
        let budget = self
            .recovery_budget
            .lock()
            .map_err(|_| invalid())?
            .as_ref()
            .ok_or_else(invalid)?
            .clone();
        let findings = verify::inventory(
            &self.inventory,
            &self.ledger,
            &self.prefix,
            &self.namespace,
            &budget,
            &self.journal_budget,
        )?;
        let mut packages = BTreeMap::new();
        let mut records = Vec::new();
        for (job, finding) in findings {
            if finding != ExternalEffectFinding::AlreadyCommitted {
                continue;
            }
            let receipt = verify::receipt(&self.inventory, job)?;
            let token = wire::uuid(&receipt["stage_token"])?;
            let manifest = receipt["manifest_sha256"].as_str().ok_or_else(invalid)?;
            let job_root = controlled_root.create_dir(&job.to_string())?;
            let staging_name = format!("{token}.partial-{}", Uuid::new_v4());
            let staging = job_root.create_dir(&staging_name)?;
            let source_prefix = format!("packages/{job}/{token}.ready/");
            let source_files: Vec<_> = self
                .records
                .iter()
                .filter(|record| record.path.starts_with(&source_prefix))
                .collect();
            if source_files.len() != 4 {
                return Err(invalid());
            }
            let mut members = Vec::new();
            for source in source_files {
                let name = source
                    .path
                    .strip_prefix(&source_prefix)
                    .ok_or_else(invalid)?;
                if name.contains('/') {
                    return Err(invalid());
                }
                charge(
                    &budget,
                    &self.journal_budget,
                    source.size.checked_mul(2).ok_or(BackupError::Overflow)?,
                    1,
                )?;
                let raw = files::read(&self.directory, source, &self.budget, &self.journal_budget)?;
                let copied = files::put(&staging, name, &raw, &budget, &self.journal_budget)?;
                if copied.size != source.size || copied.sha256 != source.sha256 {
                    return Err(invalid());
                }
                members.push(copied);
            }
            members.sort_by(|a, b| a.path.cmp(&b.path));
            files::exact_tree(&staging, &members, &budget, &self.journal_budget)?;
            files::seal_tree(&staging)?;
            let ready_name = format!("{token}.ready");
            job_root.rename_noreplace_without_sync(&staging_name, &ready_name)?;
            job_root.sync()?;
            controlled_root.sync()?;
            if job_root.open_dir(&ready_name)?.identity()? != staging.identity()? {
                return Err(invalid());
            }
            for mut record in members {
                record.path = format!("{job}/{ready_name}/{}", record.path);
                records.push(record);
            }
            packages.insert(job, (token, manifest.to_owned()));
        }
        records.sort_by(|a, b| a.path.cmp(&b.path));
        files::exact_tree(controlled_root, &records, &budget, &self.journal_budget)?;
        let receipt = json!({"format_version":2,"capability":"snapshot_recovery_delivery_copy_v2",
            "backup_id":self.record.backup_id,"manifest_sha256":self.record.manifest_sha256,
            "capture_sha256":self.capture_sha256,"payload_manifest_sha256":self.payload_sha256,
            "target":target_identity,"root_identity":controlled_root.identity()?,"files":records,
            "packages":packages.iter().map(|(job,(token,manifest))|json!({"job_id":job,"stage_token":token,"manifest_sha256":manifest})).collect::<Vec<_>>()});
        let raw = encode(&receipt, &budget, &self.journal_budget)?;
        let receipt_sha256 = hash(&raw);
        records.push(files::put(
            controlled_root,
            "copy-receipt.json",
            &raw,
            &budget,
            &self.journal_budget,
        )?);
        records.sort_by(|a, b| a.path.cmp(&b.path));
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_DELIVERY_CHECKPOINT step=RESTORE_FINAL_EXACT_TREE_BEFORE");
        files::exact_tree(controlled_root, &records, &budget, &self.journal_budget)?;
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_DELIVERY_CHECKPOINT step=RESTORE_FINAL_EXACT_TREE_AFTER");
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_DELIVERY_CHECKPOINT step=RESTORE_FINAL_SEAL_BEFORE");
        files::seal_tree(controlled_root)?;
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_DELIVERY_CHECKPOINT step=RESTORE_FINAL_SEAL_AFTER");
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_DELIVERY_CHECKPOINT step=RESTORE_FINAL_RECHECK_BEFORE");
        self.recheck()?;
        #[cfg(test)]
        eprintln!("FULL_REHEARSAL_DELIVERY_CHECKPOINT step=RESTORE_FINAL_RECHECK_AFTER");
        Ok(RecoverySnapshotCopies {
            root: controlled_root.try_clone()?,
            target: target_identity.clone(),
            records,
            packages,
            receipt_sha256,
            budget,
            journal: self.journal_budget.clone(),
        })
    }
}

fn verify_copied_packages(
    directory: &BackupDir,
    capture: &Capture,
    findings: &BTreeMap<Uuid, ExternalEffectFinding>,
    inventory: &JobInventory,
    budget: &Budget,
    journal: &SharedJournalBudget,
) -> Result<(), BackupError> {
    for (job, finding) in findings {
        if *finding != ExternalEffectFinding::AlreadyCommitted {
            continue;
        }
        let receipt = verify::receipt(inventory, *job)?;
        let token = wire::uuid(&receipt["stage_token"])?;
        let prefix = format!("packages/{job}/{token}.ready/");
        let files: Vec<_> = capture
            .files
            .iter()
            .filter(|r| r.path.starts_with(&prefix))
            .collect();
        if files.len() != 4 {
            return Err(invalid());
        }
        let mut names = std::collections::BTreeSet::new();
        for file in files {
            let name = file.path.strip_prefix(&prefix).ok_or_else(invalid)?;
            names.insert(name);
            let bytes = files::read(directory, file, budget, journal)?;
            if name == "manifest.json" {
                if file.sha256 != receipt["manifest_sha256"].as_str().ok_or_else(invalid)? {
                    return Err(invalid());
                }
                let manifest: Value = crate::registry::canonical_record(&bytes, 8 * 1024 * 1024)?;
                if manifest["capability"] != "reading_copy_v1"
                    || manifest["copy_id"] != job.to_string()
                    || manifest["format_version"] != 1
                {
                    return Err(invalid());
                }
                let declared = manifest["files"].as_array().ok_or_else(invalid)?;
                if declared.len() != 3 {
                    return Err(invalid());
                }
                let mut declared_names = std::collections::BTreeSet::new();
                for entry in declared {
                    let name = entry["path"].as_str().ok_or_else(invalid)?;
                    if !matches!(name, "reading.md" | "reading.html" | "validation.json")
                        || !declared_names.insert(name)
                    {
                        return Err(invalid());
                    }
                    let member = capture
                        .files
                        .iter()
                        .find(|r| {
                            r.path == format!("{prefix}{}", entry["path"].as_str().unwrap_or(""))
                        })
                        .ok_or_else(invalid)?;
                    if entry["sha256"] != member.sha256 || entry["size"] != member.size {
                        return Err(invalid());
                    }
                }
            }
        }
        if names
            != std::collections::BTreeSet::from([
                "reading.md",
                "reading.html",
                "validation.json",
                "manifest.json",
            ])
        {
            return Err(invalid());
        }
    }
    Ok(())
}

#[cfg(test)]
mod current_pin_copy_tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::{MetadataExt, PermissionsExt},
        path::Path,
    };

    fn fresh_root() -> (PathBuf, BackupDir) {
        assert_eq!(unsafe { libc::geteuid() }, 0, "actual Root required");
        let parent_path =
            std::env::var_os("TASK6_CUSTODY_TEST_PARENT").expect("fresh Root FS fixture");
        let parent = BackupDir::open_trusted_private_root(Path::new(&parent_path)).unwrap();
        let name = format!("current-pin-copy-{}", Uuid::new_v4());
        let root = parent.create_dir(&name).unwrap();
        (PathBuf::from(parent_path).join(name), root)
    }

    #[test]
    fn private_copy_cannot_rebind_another_target_with_the_same_source() {
        let original = json!({"capability":"held_recovery_target_v1","backup_id":"fixed-backup",
            "manifest_sha256":"a".repeat(64),"source":{"database":"same-source"},
            "database_oid":12345,"container_id":"b".repeat(64),"recovery_attempt_id":"original-attempt",
            "import_attempt_sha256":"c".repeat(64)});
        require_current_pin_target(&original, &original).unwrap();
        for (key, changed) in [
            ("database_oid", json!(12346)),
            ("container_id", json!("d".repeat(64))),
            ("recovery_attempt_id", json!("another-attempt")),
            ("import_attempt_sha256", json!("e".repeat(64))),
            ("source", json!({"database":"another-source"})),
        ] {
            let mut target = original.clone();
            target[key] = changed;
            assert!(
                require_current_pin_target(&original, &target).is_err(),
                "target rebind through {key}"
            );
        }
    }

    #[test]
    #[ignore = "actual Root plus fresh TASK6_CUSTODY_TEST_PARENT; no PG or source authority minted"]
    fn private_copy_reads_back_original_roster_and_rejects_tamper() {
        let (path, root) = fresh_root();
        let source = root.create_dir("source").unwrap();
        let copied = root.create_dir("copied").unwrap();
        let fixture: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/snapshot-effects-p6-capture-v2.json"
        ))
        .unwrap();
        let budget =
            std::sync::Arc::new(std::sync::Mutex::new(crate::registry::ScanBudget::default()));
        let journal = SharedJournalBudget::default();
        let mut records = Vec::new();
        for entry in fixture["files"].as_array().unwrap() {
            let raw = hex::decode(entry["hex"].as_str().unwrap()).unwrap();
            assert_eq!(raw.len() as u64, entry["bytes"].as_u64().unwrap());
            assert_eq!(hash(&raw), entry["sha256"].as_str().unwrap());
            records.push(
                files::put(
                    &source,
                    entry["path"].as_str().unwrap(),
                    &raw,
                    &budget,
                    &journal,
                )
                .unwrap(),
            );
        }
        assert_eq!(records.len(), 14);
        records.sort_by(|a, b| a.path.cmp(&b.path));
        copy_records(&source, &copied, &records, &budget, &budget, &journal).unwrap();
        files::exact_tree(&copied, &records, &budget, &journal).unwrap();
        for record in &records {
            let before = fs::metadata(path.join("source").join(&record.path)).unwrap();
            let after = fs::metadata(path.join("copied").join(&record.path)).unwrap();
            assert_ne!((before.dev(), before.ino()), (after.dev(), after.ino()));
            assert_eq!(after.nlink(), 1);
        }
        assert!(copy_records(&source, &source, &records, &budget, &budget, &journal).is_err());
        let changed = path.join("copied").join(&records[0].path);
        fs::set_permissions(&changed, fs::Permissions::from_mode(0o600)).unwrap();
        let mut raw = fs::read(&changed).unwrap();
        raw[0] ^= 1;
        fs::write(&changed, raw).unwrap();
        fs::set_permissions(&changed, fs::Permissions::from_mode(0o400)).unwrap();
        assert!(files::exact_tree(&copied, &records, &budget, &journal).is_err());
        // Losing an indexed source byte cannot produce a new successful copy.
        fs::remove_file(path.join("source").join(&records[0].path)).unwrap();
        let incomplete = root.create_dir("incomplete").unwrap();
        assert!(copy_records(&source, &incomplete, &records, &budget, &budget, &journal).is_err());
        drop((source, copied, incomplete, root));
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    #[ignore = "actual Root plus fresh TASK6_CUSTODY_TEST_PARENT; identity replacement only"]
    fn private_copy_rejects_detached_evidence_middle_and_backup_directories() {
        for replaced in ["evidence", "middle", "backup", "attempt"] {
            let (path, control) = fresh_root();
            let evidence = control.create_dir("snapshot-effects-copy").unwrap();
            let root = evidence.create_dir(".snapshot-effects-v2").unwrap();
            let id = Uuid::new_v4();
            let directory = root.create_dir(&id.to_string()).unwrap();
            let attempt = root.create_dir(&format!("{id}.attempt")).unwrap();
            let expected = evidence.identity().unwrap();
            assert!(
                current_copy_path_matches(&control, expected, &root, &directory, &attempt, id)
                    .unwrap()
            );
            let original = match replaced {
                "evidence" => path.join("snapshot-effects-copy"),
                "middle" => path.join("snapshot-effects-copy/.snapshot-effects-v2"),
                "attempt" => path
                    .join("snapshot-effects-copy/.snapshot-effects-v2")
                    .join(format!("{id}.attempt")),
                _ => path
                    .join("snapshot-effects-copy/.snapshot-effects-v2")
                    .join(id.to_string()),
            };
            let moved = original.with_extension("detached");
            fs::rename(&original, moved).unwrap();
            fs::create_dir(&original).unwrap();
            fs::set_permissions(&original, fs::Permissions::from_mode(0o700)).unwrap();
            assert!(
                !current_copy_path_matches(&control, expected, &root, &directory, &attempt, id)
                    .unwrap_or(false)
            );
            drop((directory, attempt, root, evidence, control));
            fs::remove_dir_all(path).unwrap();
        }
    }
}

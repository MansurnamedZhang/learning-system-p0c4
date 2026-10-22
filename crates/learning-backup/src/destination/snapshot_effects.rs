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
    root: BackupDir,
    directory: BackupDir,
    directory_path: PathBuf,
    lease: RegistryLease,
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
    source_proof_sha256: String,
    budget: Budget,
    journal_budget: SharedJournalBudget,
    recovery_budget: Mutex<Option<Budget>>,
    recovery_identity: Mutex<Option<Value>>,
    attempt: BackupDir,
    attempt_files: Vec<FileRecord>,
    base_identity: super::attempt::CommitIdentity,
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
    let evidence = destination.evidence();
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
    let source_proof_raw = crate::registry::canonical(&proof.record)?;
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
    let (destination_dev, destination_ino) = destination.destination().identity()?;
    let (evidence_dev, evidence_ino) = destination.evidence().identity()?;
    let complete_statement = super::statement(proof, &trust);
    let complete_raw = complete_statement.canonical_bytes()?;
    charge(
        &budget,
        journal,
        (complete_raw.len() as u64)
            .checked_mul(4)
            .ok_or(BackupError::Overflow)?,
        0,
    )?;
    let effect_statement = json!({"format_version":2,"capability":"snapshot_effect_witness_v2",
        "backup_id":captured.record.backup_id,"effect_attempt_id":attempt_id,"base_attempt_id":base_identity.intent.attempt_id,
        "manifest_sha256":captured.record.manifest_sha256,"captured_source_proof_sha256":hash(&source_proof_raw),
        "capture_sha256":hash(&capture_raw),"payload_manifest_sha256":hash(&payload_raw),"binding_sha256":captured.record.binding_sha256,
        "epoch_sha256":captured.record.epoch_sha256,"journal_head_sequence":captured.record.journal_head_sequence,
        "journal_head_sha256":captured.record.journal_head_sha256,"cut_event_sha256":captured.record.cut_event_sha256,
        "complete_statement_sha256":hash(&complete_raw),"source_host_id":trust.source_host_id,"source_storage_id":trust.source_storage_id,
        "destination_host_id":trust.destination_host_id,"destination_storage_id":trust.destination_storage_id,
        "destination_dev":destination_dev,"destination_ino":destination_ino,"evidence_dev":evidence_dev,"evidence_ino":evidence_ino});
    let effect_statement_raw = encode(&effect_statement, &budget, journal)?;
    let intent = json!({"format_version":2,"capability":"snapshot_effect_attempt_v2",
        "backup_id":captured.record.backup_id,"attempt_id":attempt_id,
        "base_attempt_id":base_identity.intent.attempt_id,"operation_nonce":operation_nonce,
        "destination_dev":destination_dev,"destination_ino":destination_ino,
        "evidence_dev":evidence_dev,"evidence_ino":evidence_ino,
        "manifest_sha256":captured.record.manifest_sha256,"captured_source_proof_sha256":hash(&source_proof_raw),
        "capture_sha256":hash(&capture_raw),"payload_manifest_sha256":hash(&payload_raw),
        "effect_statement_sha256":hash(&effect_statement_raw)});
    let intent_raw = encode(&intent, &budget, journal)?;
    let intent_file = files::put(&attempt, "intent.json", &intent_raw, &budget, journal)?;
    let staging_name = format!("{id}.staging-{attempt_id}");
    let directory = root.create_dir(&staging_name)?;
    root.sync()?;
    for record in &records {
        let raw = files::read(&captured.directory, record, &captured.budget, journal)?;
        if files::put(&directory, &record.path, &raw, &budget, journal)? != *record {
            return Err(invalid());
        }
    }
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
    destination.recheck()?;
    files::seal_tree(&directory)?;
    root.rename_noreplace_without_sync(&staging_name, &id)?;
    root.sync()?;
    evidence.sync()?;
    if root.open_dir(&id)?.identity()? != directory.identity()? {
        return Err(invalid());
    }
    files::exact_tree(&directory, &records, &budget, journal)?;
    let terminal = json!({"format_version":2,"capability":"snapshot_effect_commit_v2",
        "backup_id":captured.record.backup_id,"attempt_id":attempt_id,"intent_sha256":hash(&intent_raw)});
    let terminal_raw = encode(&terminal, &budget, journal)?;
    let terminal_file = files::put(&attempt, "terminal.json", &terminal_raw, &budget, journal)?;
    attempt.sync()?;
    root.sync()?;
    evidence.sync()?;
    destination.recheck()?;
    let evidence_path =
        PathBuf::from(&destination.lease.roots[&destination.group.roots["evidence"]].path);
    let result = super::attempt::finalize_use(
        &destination.lease.destination_operation,
        &base_identity,
        || {
            Ok(CopiedSnapshotEffectsV2 {
                root,
                directory,
                directory_path: evidence_path.join(".snapshot-effects-v2").join(&id),
                lease: destination.lease.clone_held()?,
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
                source_proof_sha256: hash(&source_proof_raw),
                budget,
                journal_budget: captured.journal_budget,
                recovery_budget: Mutex::new(None),
                recovery_identity: Mutex::new(None),
                attempt,
                attempt_files: vec![intent_file, terminal_file],
                base_identity: base_identity.clone(),
            })
        },
    )?;
    fence.complete();
    Ok(result)
}

impl CopiedSnapshotEffectsV2 {
    pub fn backup_id(&self) -> Uuid {
        self.record.backup_id
    }
    pub fn manifest_sha256(&self) -> &str {
        &self.record.manifest_sha256
    }

    fn recheck(&self) -> Result<(), BackupError> {
        self.lease.recheck()?;
        self.lease
            .destination_operation
            .lock()
            .map_err(|_| invalid())?
            .require(&self.base_identity)?;
        let reopened = BackupDir::open_trusted_private_root(&self.directory_path)?;
        if reopened.identity()? != self.directory.identity()?
            || self
                .root
                .open_dir(&self.record.backup_id.to_string())?
                .identity()?
                != self.directory.identity()?
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
        let evidence = json!({"format_version":2,"capability":"verified_snapshot_effect_findings_v2",
            "backup_id":self.record.backup_id,"capture_sha256":self.capture_sha256,
            "payload_manifest_sha256":self.payload_sha256,"intent_sha256":self.intent_sha256,
            "terminal_sha256":self.terminal_sha256,"captured_source_proof_sha256":self.source_proof_sha256,
            "target":target_identity,"source_access":false,"complete_authority":false,
            "journal_head_sequence":self.record.journal_head_sequence,"journal_head_sha256":self.record.journal_head_sha256});
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
        let reader =
            SnapshotJobDirectory::open_existing_in(&self.root, job).map_err(|_| invalid())?;
        let package = reader.reopen(*token, manifest).map_err(|_| invalid())?;
        let result = visitor(&package)?;
        files::exact_tree(&self.root, &self.records, &self.budget, &self.journal)?;
        Ok(result)
    }
    pub(crate) fn target_identity(&self) -> &Value {
        &self.target
    }
}

impl CopiedSnapshotEffectsV2 {
    pub(crate) fn restore_deliveries(
        &self,
        target_identity: &Value,
        controlled_root: &BackupDir,
    ) -> Result<RecoverySnapshotCopies, BackupError> {
        self.recheck()?;
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
        files::exact_tree(controlled_root, &records, &budget, &self.journal_budget)?;
        files::seal_tree(controlled_root)?;
        self.recheck()?;
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

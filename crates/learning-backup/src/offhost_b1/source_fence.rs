//! Permanent negative Source constraints. Disk bytes never reconstruct S.
//!
//! There is deliberately no close, release, GC, TTL, recovery constructor or
//! caller-selected root. Every management writer uses its original registry
//! lock before this separate lock. D-only readers do not call this module.

use super::{BuildPins, Role, policy, profile, wire};
use crate::{
    BackupError, EnrolledSource, RegistryLease, digest,
    registry::{ScanBudget, canonical, canonical_record, prepaid_b1_roster, read_metadata, v4},
    valid_digest,
};
use learning_assets::backup_fs::BackupDir;
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, Metadata},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::{fs::MetadataExt, io::AsRawFd},
    path::{Component, Path},
    sync::{Arc, Mutex},
};

const ROOT: &str = "/var/lib/knowweave-c4-source-fences";
const TOP: [&str; 4] = ["anchor.json", "lock", "operations", "staging"];
const OPEN_CAP: u64 = 16_384;
const ANCHOR_CAP: u64 = 4096;
const OPERATIONS: usize = 256;
type Budget = Arc<Mutex<ScanBudget>>;

fn refused() -> BackupError {
    BackupError::Invalid("B1 Source fence retained, incomplete, replaced or unavailable")
}

fn pay(budget: &Budget, bytes: u64, entries: usize) -> Result<(), BackupError> {
    let mut original = budget.lock().map_err(|_| refused())?;
    original.charge(bytes)?;
    original.entries(entries)
}

fn source_role(input: super::BuildInputs<'_>) -> Result<bool, BackupError> {
    match input.role {
        None | Some("destination") => Ok(false),
        Some("source") => {
            BuildPins::parse(input, Role::Source)?;
            Ok(true)
        }
        Some(_) => Err(refused()),
    }
}

/// Compile selection only. Missing installed facts in a Source build must
/// fail later; absence of a file is never interpreted as a legacy deployment.
pub(crate) fn source_mutation_gate_enabled() -> Result<bool, BackupError> {
    source_role(super::fixed_build_inputs())
}

/// Call only from a concrete Source mutation closure, before its first write.
/// In particular this is NOT a hook in general try_lock/recheck: that would
/// change D-only history/restore when called from a Source-role binary.
pub(crate) fn require_source_mutation_allowed(lease: &RegistryLease) -> Result<(), BackupError> {
    if !source_mutation_gate_enabled()? {
        return Ok(());
    }
    let mut metadata = policy::load_fixed_source_mutation_metadata(lease)?;
    metadata.recheck()?;
    let mut namespace = Namespace::open(lease, metadata.fence_anchor_data()?)?;
    namespace.require_empty()?;
    metadata.recheck()?;
    namespace.require_empty()
}

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
    mtime_nsec: i64,
    ctime: i64,
    ctime_nsec: i64,
}
impl Stamp {
    fn from_metadata(m: &Metadata) -> Self {
        Self {
            dev: m.dev(),
            ino: m.ino(),
            uid: m.uid(),
            gid: m.gid(),
            mode: m.mode(),
            links: m.nlink(),
            len: m.len(),
            mtime: m.mtime(),
            mtime_nsec: m.mtime_nsec(),
            ctime: m.ctime(),
            ctime_nsec: m.ctime_nsec(),
        }
    }
    fn require_record(self, cap: u64, empty: bool) -> Result<(), BackupError> {
        if self.mode & libc::S_IFMT != libc::S_IFREG
            || self.mode & 0o7777 != 0o600
            || self.uid != 0
            || self.links != 1
            || self.len > cap
            || (!empty && self.len == 0)
        {
            return Err(refused());
        }
        Ok(())
    }
}

fn stamp(file: &File, budget: &Budget) -> Result<Stamp, BackupError> {
    pay(budget, 0, 1)?;
    Ok(Stamp::from_metadata(&file.metadata()?))
}

fn read_record(
    file: &mut File,
    budget: &Budget,
    cap: u64,
) -> Result<(Stamp, Vec<u8>), BackupError> {
    let before = stamp(file, budget)?;
    before.require_record(cap, false)?;
    pay(budget, 0, 1)?;
    file.seek(SeekFrom::Start(0))?;
    let raw = read_metadata(file, before.len, budget)?;
    pay(budget, 1, 1)?;
    let mut extra = [0u8; 1];
    if file.read(&mut extra)? != 0 || stamp(file, budget)? != before {
        return Err(refused());
    }
    Ok((before, raw))
}

struct HeldRecord {
    file: File,
    stamp: Stamp,
    raw: Vec<u8>,
    sha256: String,
    cap: u64,
}
impl HeldRecord {
    fn open(
        parent: &BackupDir,
        name: &str,
        budget: &Budget,
        cap: u64,
    ) -> Result<Self, BackupError> {
        pay(budget, 0, 3)?;
        let mut file = parent.open_file(name)?;
        let (stamp, raw) = read_record(&mut file, budget, cap)?;
        pay(budget, raw.len() as u64, 1)?;
        let sha256 = digest(&raw);
        Ok(Self {
            file,
            stamp,
            raw,
            sha256,
            cap,
        })
    }
    fn recheck(
        &mut self,
        parent: &BackupDir,
        name: &str,
        budget: &Budget,
    ) -> Result<(), BackupError> {
        let (now, raw) = read_record(&mut self.file, budget, self.cap)?;
        pay(budget, raw.len() as u64, 1)?;
        if now != self.stamp || raw != self.raw || digest(&raw) != self.sha256 {
            return Err(refused());
        }
        let named = Self::open(parent, name, budget, self.cap)?;
        if named.stamp != self.stamp || named.raw != self.raw || named.sha256 != self.sha256 {
            return Err(refused());
        }
        Ok(())
    }
}

struct Namespace<'a> {
    lease: &'a RegistryLease,
    root: BackupDir,
    root_id: (u64, u64),
    operations: BackupDir,
    operations_id: (u64, u64),
    staging: BackupDir,
    staging_id: (u64, u64),
    lock: File,
    lock_stamp: Stamp,
    anchor: HeldRecord,
}
impl<'a> Namespace<'a> {
    fn open(
        lease: &'a RegistryLease,
        expected: policy::SourceFenceAnchorData<'_>,
    ) -> Result<Self, BackupError> {
        lease.recheck()?;
        let budget = &lease.budget;
        let components = Path::new(ROOT)
            .components()
            .filter(|c| matches!(c, Component::Normal(_)))
            .count();
        pay(budget, 4096, components * 2 + 8)?;
        let root = BackupDir::open_trusted_private_root(Path::new(ROOT))?;
        root.require_private_directory()?;
        let root_id = root.identity()?;
        if root_id != (expected.root_dev, expected.root_ino)
            || expected.deployment_id != lease.authority.deployment_id
            || !v4(expected.source_group_id)
            || !valid_digest(expected.anchor_sha256)
        {
            return Err(refused());
        }
        require_top(&root, budget)?;
        pay(budget, 0, 8)?;
        let operations = root.open_dir("operations")?;
        let staging = root.open_dir("staging")?;
        operations.require_private_directory()?;
        staging.require_private_directory()?;
        let operations_id = operations.identity()?;
        let staging_id = staging.identity()?;
        let lock = root.open_file("lock")?;
        let lock_stamp = stamp(&lock, budget)?;
        lock_stamp.require_record(0, true)?;
        pay(budget, 0, 1)?;
        // Same lock ordering everywhere: the actual original registry lease
        // already owns LOCK_EX before this independent namespace lock.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let anchor = HeldRecord::open(&root, "anchor.json", budget, ANCHOR_CAP)?;
        pay(budget, expected.anchor_raw.len() as u64, 1)?;
        if anchor.raw != expected.anchor_raw || anchor.sha256 != expected.anchor_sha256 {
            return Err(refused());
        }
        let value = Self {
            lease,
            root,
            root_id,
            operations,
            operations_id,
            staging,
            staging_id,
            lock,
            lock_stamp,
            anchor,
        };
        value.require_root_identity()?;
        Ok(value)
    }
    fn require_root_identity(&self) -> Result<(), BackupError> {
        self.lease.recheck()?;
        let budget = &self.lease.budget;
        let components = Path::new(ROOT)
            .components()
            .filter(|c| matches!(c, Component::Normal(_)))
            .count();
        pay(budget, 4096, components * 2 + 22)?;
        self.root.require_private_directory()?;
        self.operations.require_private_directory()?;
        self.staging.require_private_directory()?;
        if self.root.identity()? != self.root_id
            || self.operations.identity()? != self.operations_id
            || self.staging.identity()? != self.staging_id
            || stamp(&self.lock, budget)? != self.lock_stamp
        {
            return Err(refused());
        }
        self.lock_stamp.require_record(0, true)?;
        let named_root = BackupDir::open_trusted_private_root(Path::new(ROOT))?;
        if named_root.identity()? != self.root_id
            || named_root.open_dir("operations")?.identity()? != self.operations_id
            || named_root.open_dir("staging")?.identity()? != self.staging_id
        {
            return Err(refused());
        }
        let named_lock = named_root.open_file("lock")?;
        if stamp(&named_lock, budget)? != self.lock_stamp {
            return Err(refused());
        }
        require_top(&self.root, budget)
    }
    fn require_empty(&mut self) -> Result<(), BackupError> {
        self.require_root_identity()?;
        // Any prior/empty/partial/corrupt operation is already a negative
        // constraint. No successful parse or "close" file can clear it.
        if !prepaid_b1_roster(&self.operations, &self.lease.budget, OPERATIONS)?.is_empty()
            || !prepaid_b1_roster(&self.staging, &self.lease.budget, OPERATIONS)?.is_empty()
        {
            return Err(refused());
        }
        self.anchor
            .recheck(&self.root, "anchor.json", &self.lease.budget)?;
        Ok(())
    }
}

fn require_top(root: &BackupDir, budget: &Budget) -> Result<(), BackupError> {
    require_top_names(&prepaid_b1_roster(root, budget, TOP.len())?)
}

fn require_top_names(names: &[String]) -> Result<(), BackupError> {
    if names.iter().map(String::as_str).ne(TOP) {
        return Err(refused());
    }
    Ok(())
}

/// Borrowed, already authenticated protocol DATA. It does not grant authority
/// and is never deserialized into an original Source or a publication permit.
pub(super) struct FenceDecisionData<'a> {
    pub source: &'a wire::SourceBinding,
    pub destination: &'a wire::DestinationBinding,
    pub ready: &'a wire::DestinationReady,
    pub s_nonce: &'a str,
    pub d_nonce: &'a str,
    pub binding_sha256: &'a str,
    pub source_actor_binding_sha256: &'a str,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenRecord {
    format_version: u32,
    capability: String,
    profile: String,
    operation_id: String,
    backup_id: String,
    deployment_id: String,
    source_group_id: String,
    registry_generation: u64,
    generation_sha256: String,
    manager_release_sha256: String,
    application_build_sha256: String,
    source_binding_sha256: String,
    manifest_sha256: String,
    package_roster_sha256: String,
    captured_roster_sha256: String,
    snapshot_effects_roster_sha256: String,
    protection_snapshot_sha256: String,
    s_nonce: String,
    d_nonce: String,
    binding_sha256: String,
    source_actor_binding_sha256: String,
    destination_attempt_id: String,
    destination_proof_sha256: String,
    snapshot_effects_input_sha256: String,
    snapshot_effects_proof_sha256: String,
    snapshot_effects_intent_sha256: String,
}
impl OpenRecord {
    fn valid_data(&self) -> bool {
        self.format_version == 1
            && self.capability == "task9_b1_source_fence_open_v1"
            && self.profile == profile::PROFILE
            && self.registry_generation > 0
            && [
                &self.operation_id,
                &self.backup_id,
                &self.deployment_id,
                &self.source_group_id,
                &self.destination_attempt_id,
            ]
            .iter()
            .all(|s| v4(s))
            && [
                &self.generation_sha256,
                &self.manager_release_sha256,
                &self.application_build_sha256,
                &self.source_binding_sha256,
                &self.manifest_sha256,
                &self.package_roster_sha256,
                &self.captured_roster_sha256,
                &self.snapshot_effects_roster_sha256,
                &self.protection_snapshot_sha256,
                &self.s_nonce,
                &self.d_nonce,
                &self.binding_sha256,
                &self.source_actor_binding_sha256,
                &self.destination_proof_sha256,
                &self.snapshot_effects_input_sha256,
                &self.snapshot_effects_proof_sha256,
                &self.snapshot_effects_intent_sha256,
            ]
            .iter()
            .all(|s| valid_digest(s))
    }
}

/// Actual negative fact owner. Construction requires the same held lease,
/// enrolled Source and original captured pin view; the fixed SourceFirstS
/// consumer rechecks all its live SQL/Root owners immediately before/after.
/// No disk reader or typed DATA constructor can recreate this object.
pub(super) struct DurableSourceFence<'a> {
    namespace: Namespace<'a>,
    operation: BackupDir,
    operation_id: String,
    operation_identity: (u64, u64),
    written: File,
    record: HeldRecord,
    snapshot_sha256: String,
    actor_binding_sha256: String,
    failed: bool,
}
impl<'a> DurableSourceFence<'a> {
    pub(super) fn create_original(
        lease: &'a RegistryLease,
        anchor: policy::SourceFenceAnchorData<'_>,
        enrolled: &EnrolledSource,
        pin: &crate::source::B1PublicationPinView,
        decision: FenceDecisionData<'_>,
    ) -> Result<Self, BackupError> {
        let budget = &lease.budget;
        if ![
            decision.s_nonce,
            decision.d_nonce,
            decision.binding_sha256,
            decision.source_actor_binding_sha256,
            decision.ready.proof_sha256.as_str(),
            decision.ready.snapshot_effects_input_sha256.as_str(),
            decision.ready.snapshot_effects_proof_sha256.as_str(),
            decision.ready.snapshot_effects_intent_sha256.as_str(),
        ]
        .iter()
        .all(|value| valid_digest(value))
        {
            return Err(refused());
        }
        // Reserve bounded struct/string/canonical/hash/strict parse allocations
        // before constructing them; the large snapshot is hashed by length and
        // never copied into this small metadata namespace.
        pay(budget, 6 * OPEN_CAP, 128)?;
        let s = decision.source;
        let ready = decision.ready;
        let generation = lease.generation();
        let manifest_raw = pin.manifest_raw_bytes();
        if pin.source_records().len() != 12 {
            return Err(refused());
        }
        let record_storage = pin.source_records().iter().try_fold(0u64, |n, r| {
            n.checked_add(r.path.len() as u64)
                .and_then(|n| n.checked_add(512))
                .ok_or(BackupError::Overflow)
        })?;
        pay(
            budget,
            record_storage.checked_mul(8).ok_or(BackupError::Overflow)?,
            13,
        )?;
        let source_raw = canonical(&pin.source_records())?;
        let generation_key = format!("generations/{:020}.json", generation.generation);
        let generation_raw = &lease.metadata.get(&generation_key).ok_or_else(refused)?.0;
        let authority_raw = &lease.metadata.get("authority.json").ok_or_else(refused)?.0;
        let hash_bytes = [
            manifest_raw.len(),
            pin.current_snapshot_bytes().len(),
            source_raw.len(),
            generation_raw.len(),
            authority_raw.len(),
        ]
        .iter()
        .try_fold(0u64, |n, v| {
            n.checked_add(*v as u64).ok_or(BackupError::Overflow)
        })?;
        pay(budget, hash_bytes, 6)?;
        let snapshot_sha256 = digest(pin.current_snapshot_bytes());
        if !s.valid()
            || !decision.destination.valid()
            || s.deployment_id != enrolled.group.deployment_id
            || s.source_group_id != enrolled.group.group_id
            || s.backup_id != pin.manifest().backup_id.to_string()
            || anchor.source_group_id != enrolled.group.group_id
            || s.source_binding_sha256 != enrolled.group.source_binding_sha256
            || s.application_commit != enrolled.group.application_commit
            || s.application_build_sha256 != enrolled.group.application_build_sha256
            || s.registry_generation != generation.generation
            || s.generation_sha256 != digest(generation_raw)
            || s.registry_authority_sha256 != digest(authority_raw)
            || s.manifest_sha256 != digest(manifest_raw)
            || s.package_roster_sha256 != pin.package_roster_sha256(lease)?
            || s.captured_roster_sha256 != digest(&source_raw)
            || ready.attempt_id != decision.destination.attempt_id
            || ready.manifest_sha256 != s.manifest_sha256
            || ready.snapshot_effects_input_sha256 != s.snapshot_effects_input_sha256
            || decision.source_actor_binding_sha256
                != wire::actor_binding_data(
                    &s.source_operation_id,
                    decision.binding_sha256,
                    decision.s_nonce,
                    decision.d_nonce,
                )
                .map_err(|_| refused())?
        {
            return Err(refused());
        }
        if s.roots.len() != enrolled.roots.len() {
            return Err(refused());
        }
        for record in &s.roots {
            let actual = enrolled.roots.get(&record.role).ok_or_else(refused)?;
            let raw = lease
                .metadata
                .get(&format!("roots/{}.json", actual.enrollment_id))
                .ok_or_else(refused)?;
            pay(budget, raw.0.len() as u64, 4)?;
            if record.enrollment_id != actual.enrollment_id
                || record.path != actual.path
                || record.dev != actual.dev
                || record.ino != actual.ino
                || record.uid != actual.uid
                || record.mode != actual.mode
                || record.record_sha256 != digest(&raw.0)
                || enrolled.handle(&record.role).identity()? != (actual.dev, actual.ino)
            {
                return Err(refused());
            }
        }
        let mut namespace = Namespace::open(lease, anchor)?;
        namespace.require_empty()?;
        let open = OpenRecord {
            format_version: 1,
            capability: "task9_b1_source_fence_open_v1".into(),
            profile: profile::PROFILE.into(),
            operation_id: s.source_operation_id.clone(),
            backup_id: s.backup_id.clone(),
            deployment_id: s.deployment_id.clone(),
            source_group_id: s.source_group_id.clone(),
            registry_generation: s.registry_generation,
            generation_sha256: s.generation_sha256.clone(),
            manager_release_sha256: s.manager_release_sha256.clone(),
            application_build_sha256: s.application_build_sha256.clone(),
            source_binding_sha256: s.source_binding_sha256.clone(),
            manifest_sha256: s.manifest_sha256.clone(),
            package_roster_sha256: s.package_roster_sha256.clone(),
            captured_roster_sha256: s.captured_roster_sha256.clone(),
            snapshot_effects_roster_sha256: s.snapshot_effects_roster_sha256.clone(),
            protection_snapshot_sha256: snapshot_sha256.clone(),
            s_nonce: decision.s_nonce.to_owned(),
            d_nonce: decision.d_nonce.to_owned(),
            binding_sha256: decision.binding_sha256.to_owned(),
            source_actor_binding_sha256: decision.source_actor_binding_sha256.to_owned(),
            destination_attempt_id: ready.attempt_id.clone(),
            destination_proof_sha256: ready.proof_sha256.clone(),
            snapshot_effects_input_sha256: ready.snapshot_effects_input_sha256.clone(),
            snapshot_effects_proof_sha256: ready.snapshot_effects_proof_sha256.clone(),
            snapshot_effects_intent_sha256: ready.snapshot_effects_intent_sha256.clone(),
        };
        if !open.valid_data() {
            return Err(refused());
        }
        let raw = canonical(&open)?;
        if raw.is_empty() || raw.len() as u64 > OPEN_CAP {
            return Err(refused());
        }
        let parsed: OpenRecord = canonical_record(&raw, OPEN_CAP)?;
        if !parsed.valid_data() {
            return Err(refused());
        }
        let name = open.operation_id;
        pay(budget, raw.len() as u64, 16)?;
        let operation = namespace.staging.create_dir(&name)?;
        operation.require_private_directory()?;
        let operation_identity = operation.identity()?;
        let mut written = operation.create_file("open.json")?;
        written.write_all(&raw)?;
        written.sync_all()?;
        operation.sync()?;
        namespace.staging.sync()?;
        namespace.root.sync()?;
        namespace.staging.rename_entry_to_noreplace_without_sync(
            &name,
            &namespace.operations,
            &name,
        )?;
        namespace.operations.sync()?;
        namespace.staging.sync()?;
        namespace.root.sync()?;
        let record = HeldRecord::open(&operation, "open.json", budget, OPEN_CAP)?;
        if record.raw != raw || stamp(&written, budget)? != record.stamp {
            return Err(refused());
        }
        namespace
            .anchor
            .recheck(&namespace.root, "anchor.json", budget)?;
        let mut original = Self {
            namespace,
            operation,
            operation_id: name,
            operation_identity,
            written,
            record,
            snapshot_sha256,
            actor_binding_sha256: decision.source_actor_binding_sha256.to_owned(),
            failed: false,
        };
        original.recheck_held()?;
        Ok(original)
    }

    /// This finite view exists only in this actual original. It requires its
    /// exact held directory/file and sole known operation, never an ignore ID.
    pub(super) fn recheck_held(&mut self) -> Result<(), BackupError> {
        if self.failed {
            return Err(refused());
        }
        self.failed = true;
        self.namespace.require_root_identity()?;
        let budget = &self.namespace.lease.budget;
        let names = prepaid_b1_roster(&self.namespace.operations, budget, OPERATIONS)?;
        if names != [self.operation_id.as_str()]
            || !prepaid_b1_roster(&self.namespace.staging, budget, OPERATIONS)?.is_empty()
        {
            return Err(refused());
        }
        pay(budget, 0, 8)?;
        self.operation.require_private_directory()?;
        let named = self.namespace.operations.open_dir(&self.operation_id)?;
        if named.identity()? != self.operation_identity
            || self.operation.identity()? != self.operation_identity
            || prepaid_b1_roster(&self.operation, budget, 1)? != ["open.json"]
        {
            return Err(refused());
        }
        self.record.recheck(&named, "open.json", budget)?;
        if stamp(&self.written, budget)? != self.record.stamp {
            return Err(refused());
        }
        self.namespace
            .anchor
            .recheck(&self.namespace.root, "anchor.json", budget)?;
        self.namespace.lease.recheck()?;
        self.failed = false;
        Ok(())
    }

    pub(super) fn wire_data(&mut self) -> Result<wire::Fence, BackupError> {
        self.recheck_held()?;
        pay(&self.namespace.lease.budget, 1024, 1)?;
        Ok(wire::Fence {
            operation_id: self.operation_id.clone(),
            anchor_sha256: self.namespace.anchor.sha256.clone(),
            open_record_sha256: self.record.sha256.clone(),
            protection_snapshot_sha256: self.snapshot_sha256.clone(),
            source_actor_binding_sha256: self.actor_binding_sha256.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(role: Option<&str>) -> super::super::BuildInputs<'_> {
        super::super::BuildInputs {
            role,
            profile: None,
            source_auth_key: None,
            verifier_key: None,
            commit: None,
            build_input: None,
            source_binding: None,
        }
    }
    #[test]
    fn source_fence_missing_source_pins_cannot_fall_back_to_legacy() {
        assert!(source_role(inputs(Some("source"))).is_err());
        assert!(source_role(inputs(Some("invalid"))).is_err());
    }
    #[test]
    fn source_fence_legacy_and_destination_selection_need_no_source_files() {
        assert!(!source_role(inputs(None)).unwrap());
        assert!(!source_role(inputs(Some("destination"))).unwrap());
    }
    #[test]
    fn source_fence_close_cannot_be_a_top_level_release_credential() {
        let installed: Vec<_> = ["anchor.json", "lock", "operations", "staging"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        require_top_names(&installed).unwrap();
        let mut closed = installed.clone();
        closed.push("close.json".into());
        assert!(require_top_names(&closed).is_err());
        let mut missing = installed.clone();
        missing.remove(2);
        assert!(require_top_names(&missing).is_err());
        let mut replaced = installed;
        replaced[2] = "closed".into();
        assert!(require_top_names(&replaced).is_err());
    }
    #[test]
    fn source_fence_record_is_private_single_link_and_bounded() {
        let good = Stamp {
            dev: 1,
            ino: 2,
            uid: 0,
            gid: 0,
            mode: libc::S_IFREG | 0o600,
            links: 1,
            len: OPEN_CAP,
            mtime: 1,
            mtime_nsec: 0,
            ctime: 1,
            ctime_nsec: 0,
        };
        good.require_record(OPEN_CAP, false).unwrap();
        for bad in [
            Stamp { links: 2, ..good },
            Stamp { uid: 1000, ..good },
            Stamp {
                mode: libc::S_IFREG | 0o644,
                ..good
            },
            Stamp {
                len: OPEN_CAP + 1,
                ..good
            },
            Stamp { len: 0, ..good },
        ] {
            assert!(bad.require_record(OPEN_CAP, false).is_err());
        }
    }
    #[test]
    fn source_fence_budget_failure_is_not_refunded_for_a_new_wrapper() {
        let budget = Arc::new(Mutex::new(ScanBudget::default()));
        pay(&budget, crate::registry::MAX_SCAN_BYTES, 1).unwrap();
        let same = Arc::clone(&budget);
        assert!(pay(&same, 1, 1).is_err());
        assert!(pay(&budget, 1, 1).is_err());
    }
}

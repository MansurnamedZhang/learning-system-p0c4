//! Fixed metadata custody only. No key, ELF, launcher or actor admission.

use super::{
    BuildPins, FENCE_ROOT, RegistryAnchor, Role, SESSION_ROOT, UntrustedDestinationInputs,
    UntrustedSourceInputs, parse_destination_inputs, parse_source_inputs, profile,
};
use crate::{
    BackupError, RegistryLease, digest,
    registry::{REGISTRY_PATH, ScanBudget, read_metadata},
};
use learning_assets::backup_fs::BackupDir;
use std::{
    fs::{File, Metadata},
    io::{Read, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
    path::{Component, Path},
    sync::{Arc, Mutex},
};

const POLICY_ROOT: &str = "/var/lib/knowweave-c4/b1";
const LAUNCH_LEAF: &str = "launch-manifest.json";
const ANCHOR_LEAF: &str = "anchor.json";
// Local loader only: 3 metadata files + 2 held parents, and at most one
// newly traversed parent (transiently two FDs) OR parent+named file at recheck.
// This excludes the caller's registry/actor/runtime FDs and is not a measured
// global 64-FD guarantee. No FD is cloned or sent to another process here.
const RETAINED_METADATA_FDS: usize = 5;
const LOCAL_METADATA_FD_PEAK: usize = 7;

type Budget = Arc<Mutex<ScanBudget>>;

fn refused() -> BackupError {
    BackupError::Invalid("offhost B1 installed metadata custody or replacement")
}

fn entries(budget: &Budget, count: usize) -> Result<(), BackupError> {
    budget.lock().map_err(|_| refused())?.entries(count)
}

fn bytes(budget: &Budget, count: u64) -> Result<(), BackupError> {
    budget.lock().map_err(|_| refused())?.charge(count)
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
    fn from_metadata(value: &Metadata) -> Self {
        Self {
            dev: value.dev(),
            ino: value.ino(),
            uid: value.uid(),
            gid: value.gid(),
            mode: value.mode(),
            links: value.nlink(),
            len: value.len(),
            mtime: value.mtime(),
            mtime_nsec: value.mtime_nsec(),
            ctime: value.ctime(),
            ctime_nsec: value.ctime_nsec(),
        }
    }

    fn require_private_record(self, cap: u64) -> Result<(), BackupError> {
        if self.mode & libc::S_IFMT != libc::S_IFREG
            || self.mode & 0o7777 != 0o600
            || self.uid != 0
            || self.links != 1
            || self.len == 0
            || self.len > cap
        {
            return Err(refused());
        }
        Ok(())
    }
}

fn stat(file: &File, budget: &Budget) -> Result<Stamp, BackupError> {
    entries(budget, 1)?;
    Ok(Stamp::from_metadata(&file.metadata()?))
}

/// The extra byte is reserved before the EOF probe, including at exact cap.
/// It is scan accounting, not permission to accept cap+1 input bytes.
fn read_counted(
    reader: &mut impl Read,
    len: u64,
    cap: u64,
    budget: &Budget,
) -> Result<Vec<u8>, BackupError> {
    if len == 0 || len > cap {
        return Err(refused());
    }
    let raw = read_metadata(reader, len, budget)?;
    bytes(budget, 1)?;
    let mut extra = [0_u8; 1];
    if reader.read(&mut extra)? != 0 {
        return Err(refused());
    }
    Ok(raw)
}

fn read_checked(
    file: &mut File,
    cap: u64,
    budget: &Budget,
) -> Result<(Stamp, Vec<u8>), BackupError> {
    let before = stat(file, budget)?;
    before.require_private_record(cap)?;
    file.seek(SeekFrom::Start(0))?;
    let raw = read_counted(file, before.len, cap, budget)?;
    if stat(file, budget)? != before {
        return Err(refused());
    }
    Ok((before, raw))
}

struct Parent {
    path: &'static str,
    dir: BackupDir,
    identity: (u64, u64),
}

impl Parent {
    fn open(path: &'static str, budget: &Budget) -> Result<Self, BackupError> {
        // Precharge the fixed trusted traversal: open+stat at / and each normal
        // component, final mode check, require_private_directory and identity.
        // Rejections retain the reservation. This depends on the existing
        // BackupDir trusted-traversal implementation, not caller scan counters.
        let components = Path::new(path)
            .components()
            .filter(|c| matches!(c, Component::Normal(_)))
            .count();
        entries(budget, components * 2 + 5)?;
        let dir = BackupDir::open_trusted_private_root(Path::new(path))?;
        dir.require_private_directory()?;
        let identity = dir.identity()?;
        Ok(Self {
            path,
            dir,
            identity,
        })
    }

    fn reopen(&self, budget: &Budget) -> Result<Self, BackupError> {
        entries(budget, 2)?;
        self.dir.require_private_directory()?;
        if self.dir.identity()? != self.identity {
            return Err(refused());
        }
        let current = Self::open(self.path, budget)?;
        if current.identity != self.identity {
            return Err(refused());
        }
        Ok(current)
    }
}

struct Record {
    name: &'static str,
    file: File,
    stamp: Stamp,
    raw: Vec<u8>,
    sha256: String,
    cap: u64,
}

impl Record {
    fn open(
        parent: &BackupDir,
        name: &'static str,
        cap: u64,
        budget: &Budget,
    ) -> Result<Self, BackupError> {
        // openat + secure_dir regular-file stat + BackupDir single-link stat.
        entries(budget, 3)?;
        let mut file = parent.open_file(name)?;
        let (stamp, raw) = read_checked(&mut file, cap, budget)?;
        bytes(budget, raw.len() as u64)?;
        let sha256 = digest(&raw);
        Ok(Self {
            name,
            file,
            stamp,
            raw,
            sha256,
            cap,
        })
    }

    fn recheck(&mut self, named_parent: &BackupDir, budget: &Budget) -> Result<(), BackupError> {
        let (stamp, raw) = read_checked(&mut self.file, self.cap, budget)?;
        bytes(budget, raw.len() as u64)?;
        if stamp != self.stamp || raw != self.raw || digest(&raw) != self.sha256 {
            return Err(refused());
        }
        let named = Self::open(named_parent, self.name, self.cap, budget)?;
        if named.stamp != self.stamp || named.raw != self.raw || named.sha256 != self.sha256 {
            return Err(refused());
        }
        Ok(())
    }
}

enum UntrustedInputs {
    Source(Box<UntrustedSourceInputs>),
    Destination(Box<UntrustedDestinationInputs>),
}

// No Deserialize, Clone, public constructor or authority accessors. The borrow
// prevents these metadata handles from outliving the original registry lease.
pub(super) struct HeldInstalledInputs<'a> {
    policy_parent: Parent,
    anchor_parent: Parent,
    policy: Record,
    anchor: Record,
    launch: Record,
    inputs: UntrustedInputs,
    lease: &'a RegistryLease,
    failed: bool,
}

impl HeldInstalledInputs<'_> {
    pub(super) fn recheck(&mut self) -> Result<(), BackupError> {
        if self.failed {
            return Err(refused());
        }
        // Any error or unwind leaves this data holder terminally rejected.
        self.failed = true;
        self.lease.recheck()?;
        let budget = &self.lease.budget;
        {
            let parent = self.policy_parent.reopen(budget)?;
            self.policy.recheck(&parent.dir, budget)?;
            self.launch.recheck(&parent.dir, budget)?;
        }
        {
            let parent = self.anchor_parent.reopen(budget)?;
            self.anchor.recheck(&parent.dir, budget)?;
        }
        self.require_local_bindings()?;
        self.lease.recheck()?;
        self.failed = false;
        Ok(())
    }

    fn require_local_bindings(&self) -> Result<(), BackupError> {
        let (anchor, deployment, metadata_root) = match &self.inputs {
            UntrustedInputs::Source(value) => (
                &value.policy.registry_anchor,
                &value.policy.deployment_id,
                &value.fence.fence_root,
            ),
            UntrustedInputs::Destination(value) => (
                &value.policy.registry_anchor,
                &value.policy.deployment_id,
                &value.session.session_root,
            ),
        };
        if deployment != &self.lease.authority.deployment_id
            || self.anchor_parent.identity != (metadata_root.dev, metadata_root.ino)
        {
            return Err(refused());
        }
        require_registry_anchor(anchor, self.lease)
    }
}

fn require_registry_anchor(
    anchor: &RegistryAnchor,
    lease: &RegistryLease,
) -> Result<(), BackupError> {
    entries(&lease.budget, 3)?;
    lease.root.require_private_directory()?;
    let root_id = lease.root.identity()?;
    let lock = lease.lock.metadata()?;
    let raw = &lease.metadata.get("authority.json").ok_or_else(refused)?.0;
    bytes(&lease.budget, raw.len() as u64)?;
    if anchor.path != REGISTRY_PATH
        || lease.path != Path::new(REGISTRY_PATH)
        || root_id != (anchor.dev, anchor.ino)
        || (lock.dev(), lock.ino()) != (anchor.lock_dev, anchor.lock_ino)
        || digest(raw) != anchor.authority_sha256
    {
        return Err(refused());
    }
    Ok(())
}

/// Private preparation only; not called by either installed runner. Actual
/// executable/launcher verification, group/PG admission and all actor effects
/// remain separate and absent. No runtime key file is opened by this loader.
pub(super) fn load_installed<'a>(
    pins: &BuildPins<'_>,
    lease: &'a RegistryLease,
) -> Result<HeldInstalledInputs<'a>, BackupError> {
    lease.recheck()?;
    let (policy_name, anchor_path) = match pins.role {
        Role::Source => ("source-policy.json", FENCE_ROOT),
        Role::Destination => ("destination-policy.json", SESSION_ROOT),
    };
    let budget = &lease.budget;
    let policy_parent = Parent::open(POLICY_ROOT, budget)?;
    let anchor_parent = Parent::open(anchor_path, budget)?;
    let policy = Record::open(
        &policy_parent.dir,
        policy_name,
        profile::CONTROL_BYTES,
        budget,
    )?;
    let anchor = Record::open(
        &anchor_parent.dir,
        ANCHOR_LEAF,
        profile::ANCHOR_BYTES,
        budget,
    )?;
    let launch = Record::open(
        &policy_parent.dir,
        LAUNCH_LEAF,
        profile::CONTROL_BYTES,
        budget,
    )?;
    // The existing pure parser hashes policy once and anchor twice; D also
    // fingerprints its 32-byte public key. Charge those repeat hashes too.
    bytes(
        budget,
        policy.raw.len() as u64
            + 2 * anchor.raw.len() as u64
            + if pins.role == Role::Destination {
                32
            } else {
                0
            },
    )?;
    let inputs = match pins.role {
        Role::Source => UntrustedInputs::Source(Box::new(parse_source_inputs(
            &policy.raw,
            &anchor.raw,
            &launch.raw,
            pins,
        )?)),
        Role::Destination => UntrustedInputs::Destination(Box::new(parse_destination_inputs(
            &policy.raw,
            &anchor.raw,
            &launch.raw,
            pins,
        )?)),
    };
    let mut held = HeldInstalledInputs {
        policy_parent,
        anchor_parent,
        policy,
        anchor,
        launch,
        inputs,
        lease,
        failed: false,
    };
    held.recheck()?;
    Ok(held)
}

#[cfg(test)]
mod tests;

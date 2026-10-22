//! OS-only Root custody. These objects prove neither a SQL claim nor a C3
//! write permit, known commit, capture cut, or NoEffect. A trusted broker must
//! establish its own private business authority before consuming this writer.
use super::{
    ExactSnapshotManifest, FsAssetStore, ReadingCopy, ReadingCopyManifest, SNAPSHOT_FORMAT_VERSION,
    SnapshotAssetUse, SnapshotDirectory, SnapshotIoError, SnapshotRow, StageWriter,
    stage_copy_writer, stage_exact_writer,
};
use crate::secure_dir::Dir;
use std::{io, path::Path};
use uuid::Uuid;

#[cfg(target_os = "linux")]
use {
    crate::secure_dir::SNAPSHOT_CUSTODY_MARKER,
    std::{
        fs::File,
        io::Write,
        os::{
            fd::AsRawFd,
            unix::fs::{FileExt, MetadataExt},
        },
        sync::Arc,
    },
};

/// Original live Root-owned directory and control handles. Only a fresh root
/// can be created; there is deliberately no reopen, adoption, or serde API.
#[derive(Debug)]
pub struct RootSnapshotCustody {
    #[cfg(target_os = "linux")]
    state: Arc<CustodyState>,
    #[cfg(not(target_os = "linux"))]
    _unsupported: (),
}

/// Fresh Root OS installation only. This is deliberately not registry/source
/// enrollment, a database claim, or a positive external-effect finding.
/// It has no reopen/import/serde/Clone constructor.
#[cfg(target_os = "linux")]
pub struct InstalledSnapshotBinding {
    parent: Dir,
    root: Dir,
    root_path: std::path::PathBuf,
    broker_directory: Dir,
    _broker_lock: File,
    custody: RootSnapshotCustody,
    journal: Option<crate::root_journal::RootJournalBytes>,
    journal_path: std::path::PathBuf,
    epoch: Uuid,
    binding: Uuid,
    budget: crate::root_journal::SharedJournalBudget,
}

#[cfg(target_os = "linux")]
impl InstalledSnapshotBinding {
    pub const COMPANION_PATH: &str = "/var/lib/knowweave-c4/snapshot-effects-v2";
    pub const EXPORT_PATH: &str = "/var/lib/knowweave-c4/snapshot-export-v2";

    /// Explicit trusted Root management bootstrap. Existing roots, binding
    /// directories, epochs and journals are never adopted or repaired.
    pub fn install_fresh(
        deployment: Uuid,
        epoch: Uuid,
        binding: Uuid,
        writer_contract_sha256: &str,
        budget: crate::root_journal::SharedJournalBudget,
    ) -> Result<Self, SnapshotIoError> {
        Self::install_at(
            (
                Path::new(Self::COMPANION_PATH),
                Path::new(Self::EXPORT_PATH),
            ),
            deployment,
            epoch,
            binding,
            writer_contract_sha256,
            budget,
            None,
        )
    }

    /// Explicit Root management bootstrap of two new, independently observed
    /// empty mounted roots. This consumes their actual directory FDs and never
    /// opens an existing binding, epoch, marker or journal for adoption.
    pub fn install_fresh_empty_mounts(
        companion: File,
        export: File,
        deployment: Uuid,
        epoch: Uuid,
        binding: Uuid,
        writer_contract_sha256: &str,
        budget: crate::root_journal::SharedJournalBudget,
    ) -> Result<Self, SnapshotIoError> {
        Self::install_at(
            (
                Path::new(Self::COMPANION_PATH),
                Path::new(Self::EXPORT_PATH),
            ),
            deployment,
            epoch,
            binding,
            writer_contract_sha256,
            budget,
            Some((companion, export)),
        )
    }

    fn install_at(
        paths: (&Path, &Path),
        deployment: Uuid,
        epoch: Uuid,
        binding: Uuid,
        writer_contract_sha256: &str,
        budget: crate::root_journal::SharedJournalBudget,
        mounts: Option<(File, File)>,
    ) -> Result<Self, SnapshotIoError> {
        let (companion_path, export_path) = paths;
        if unsafe { libc::geteuid() } != 0
            || unsafe { libc::getegid() } != 0
            || [deployment, epoch, binding]
                .iter()
                .any(|id| id.get_version_num() != 4 || id.get_variant() != uuid::Variant::RFC4122)
            || writer_contract_sha256.len() != 64
            || !writer_contract_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid().into());
        }
        budget.charge(16 * 1024, 32)?;
        let parent = Dir::open_trusted_private_root(companion_path.parent().ok_or_else(invalid)?)?;
        let leaf = companion_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(invalid)?;
        let (root, export_root) = if let Some((companion, export)) = mounts {
            let root = Dir::from_opened_private_root(companion)?;
            let export = Dir::from_opened_private_root(export)?;
            let export_parent =
                Dir::open_trusted_private_root(export_path.parent().ok_or_else(invalid)?)?;
            let export_leaf = export_path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(invalid)?;
            if root.identity()? == export.identity()?
                || parent.open_dir(leaf)?.identity()? != root.identity()?
                || export_parent.open_dir(export_leaf)?.identity()? != export.identity()?
                || !root.list_metered(&budget)?.is_empty()
                || !export.list_metered(&budget)?.is_empty()
            {
                return Err(invalid().into());
            }
            (root, Some(export))
        } else {
            (parent.create_dir(leaf)?, None)
        };
        root.require_private_directory()?;
        for name in ["bindings", "captures", "generations", "journals", "staging"] {
            root.create_dir(name)?.sync()?;
        }
        let journals = root.open_dir("journals")?;
        let broker_directory = journals.create_dir(&binding.to_string())?;
        let broker_lock = broker_directory.control_file("broker.lock", true)?;
        broker_lock.try_lock().map_err(|_| invalid())?;
        broker_lock.sync_all()?;
        let epochs = broker_directory.create_dir("epochs")?;
        epochs.create_dir(&epoch.to_string())?.sync()?;
        let journal_path = companion_path
            .join("journals")
            .join(binding.to_string())
            .join("epochs")
            .join(epoch.to_string());
        let journal = crate::root_journal::RootJournalBytes::create_fresh(
            &journal_path,
            crate::root_journal::ByteJournalIdentity::new(epoch, binding)?,
            budget.clone(),
        )?;
        let custody = RootSnapshotCustody::create_fresh_v2(
            export_path,
            deployment,
            binding,
            &broker_directory,
            writer_contract_sha256,
            export_root,
        )?;
        root.sync()?;
        parent.sync()?;
        let this = Self {
            parent,
            root,
            root_path: companion_path.to_path_buf(),
            broker_directory,
            _broker_lock: broker_lock,
            custody,
            journal: Some(journal),
            journal_path,
            epoch,
            binding,
            budget,
        };
        this.revalidate()?;
        Ok(this)
    }

    pub fn epoch_id(&self) -> Uuid {
        self.epoch
    }
    pub fn binding_id(&self) -> Uuid {
        self.binding
    }
    pub fn custody(&self) -> &RootSnapshotCustody {
        &self.custody
    }
    pub fn journal_path(&self) -> &Path {
        &self.journal_path
    }
    pub fn shared_budget(&self) -> &crate::root_journal::SharedJournalBudget {
        &self.budget
    }
    pub fn marker_bytes(&self) -> &[u8] {
        &self.custody.state.marker_bytes
    }

    /// Transfer this installation's original byte owner once; reading its
    /// metadata or recreating UUIDs can never construct a second owner.
    pub fn take_original_journal(
        &mut self,
    ) -> Result<crate::root_journal::RootJournalBytes, SnapshotIoError> {
        self.revalidate()?;
        self.journal.take().ok_or_else(|| invalid().into())
    }

    pub fn identities(&self) -> Result<[(u64, u64); 4], SnapshotIoError> {
        self.revalidate()?;
        let state = &self.custody.state;
        let lock = state.lock.metadata()?;
        Ok([
            self.root.identity()?,
            state.root.identity()?,
            self.broker_directory.identity()?,
            (lock.dev(), lock.ino()),
        ])
    }

    pub fn revalidate(&self) -> Result<(), SnapshotIoError> {
        self.parent.require_private_directory()?;
        self.root.require_private_directory()?;
        let leaf = self
            .root_path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(invalid)?;
        if self.parent.open_dir(leaf)?.identity()? != self.root.identity()? {
            return Err(invalid().into());
        }
        self.broker_directory.require_private_directory()?;
        if self
            .root
            .open_dir("journals")?
            .open_dir(&self.binding.to_string())?
            .identity()?
            != self.broker_directory.identity()?
        {
            return Err(invalid().into());
        }
        same_file(
            &self.broker_directory.control_file("broker.lock", false)?,
            &self._broker_lock,
            0,
        )?;
        self.custody.state.revalidate()?;
        Ok(())
    }
}

/// A move-only OS shared-lock owner, not a database/business write permit.
#[derive(Debug)]
pub struct CustodyWriter {
    guard: CustodyGuard,
}

/// A package with its original OS shared lock. The package can only be borrowed
/// for verified reads; extracting it cannot detach the lock from the operation.
#[derive(Debug)]
pub struct CustodyStaged {
    directory: SnapshotDirectory,
    guard: CustodyGuard,
}
impl CustodyStaged {
    pub fn directory(&self) -> &SnapshotDirectory {
        &self.directory
    }
    pub fn revalidate(&self) -> Result<(), SnapshotIoError> {
        self.guard.revalidate()
    }
}

/// Live exclusive OS lock on the same original effect.lock inode. This alone
/// makes no statement about broker quiescence, SQL state, or a capture cut.
#[derive(Debug)]
pub struct CustodyCaptureGuard {
    guard: CustodyGuard,
}
impl CustodyCaptureGuard {
    pub fn revalidate(&self) -> Result<(), SnapshotIoError> {
        self.guard.revalidate()
    }

    /// Stable, complete namespace visitation from the same EX-held directory.
    /// These observations are not effect classifications or writable handles.
    #[cfg(target_os = "linux")]
    pub fn visit_namespace(
        &self,
        budget: &crate::root_journal::SharedJournalBudget,
        visitor: &mut dyn FnMut(&str, bool, u64) -> io::Result<()>,
    ) -> Result<(), SnapshotIoError> {
        fn walk(
            dir: &Dir,
            prefix: &str,
            depth: usize,
            budget: &crate::root_journal::SharedJournalBudget,
            visitor: &mut dyn FnMut(&str, bool, u64) -> io::Result<()>,
        ) -> Result<(), SnapshotIoError> {
            if depth > 8 {
                return Err(invalid().into());
            }
            let before = dir.root_metadata()?;
            if before.uid() != 0 || before.gid() != 0 || before.mode() & 0o022 != 0 {
                return Err(invalid().into());
            }
            let names = dir.list_metered(budget)?;
            for name in &names {
                budget.charge((prefix.len() + name.len() + 1) as u64, 0)?;
                let path = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}/{name}")
                };
                match dir.kind(name)? {
                    crate::secure_dir::EntryKind::Directory => {
                        let child = dir.open_dir(name)?;
                        visitor(&path, true, 0)?;
                        walk(&child, &path, depth + 1, budget, visitor)?;
                        if dir.open_dir(name)?.identity()? != child.identity()? {
                            return Err(invalid().into());
                        }
                    }
                    crate::secure_dir::EntryKind::File => {
                        let file = dir.open_file(name)?;
                        let m = file.metadata()?;
                        if m.uid() != 0 || m.gid() != 0 || m.nlink() != 1 || m.mode() & 0o022 != 0 {
                            return Err(invalid().into());
                        }
                        visitor(&path, false, m.len())?;
                        let after = dir.open_file(name)?.metadata()?;
                        if (m.dev(), m.ino(), m.len(), m.ctime(), m.ctime_nsec())
                            != (
                                after.dev(),
                                after.ino(),
                                after.len(),
                                after.ctime(),
                                after.ctime_nsec(),
                            )
                        {
                            return Err(invalid().into());
                        }
                    }
                    _ => return Err(invalid().into()),
                }
            }
            if names != dir.list_metered(budget)? {
                return Err(invalid().into());
            }
            let after = dir.root_metadata()?;
            if (
                before.dev(),
                before.ino(),
                before.ctime(),
                before.ctime_nsec(),
            ) != (after.dev(), after.ino(), after.ctime(), after.ctime_nsec())
            {
                return Err(invalid().into());
            }
            Ok(())
        }
        self.revalidate()?;
        walk(&self.guard.state.root, "", 0, budget, visitor)?;
        self.revalidate()
    }

    /// Actual fresh metadata and bounded reads are paid before either verifier
    /// and the single sealed stream used by normal source capture.
    #[cfg(target_os = "linux")]
    pub fn with_verified_package_with_budget<T>(
        &self,
        budget: &crate::root_journal::SharedJournalBudget,
        job: Uuid,
        token: Uuid,
        digest: &str,
        visitor: impl FnOnce(&SnapshotDirectory) -> Result<T, SnapshotIoError>,
    ) -> Result<T, SnapshotIoError> {
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(SnapshotIoError::InvalidPackage);
        }
        budget.charge(256, 4)?;
        self.revalidate()?;
        let directory = SnapshotDirectory {
            dir: self
                .guard
                .state
                .root
                .open_dir(&job.to_string())?
                .open_dir(&format!("{token}.ready"))?,
            manifest_sha256: digest.into(),
            reading_bounds: None,
        };
        let result = directory.with_reading_copy_capture_budget(budget, visitor)?;
        self.revalidate()?;
        Ok(result)
    }

    #[cfg(target_os = "linux")]
    pub fn with_verified_package<T>(
        &self,
        job: Uuid,
        token: Uuid,
        digest: &str,
        visitor: impl FnOnce(&SnapshotDirectory) -> Result<T, SnapshotIoError>,
    ) -> Result<T, SnapshotIoError> {
        self.revalidate()?;
        let directory = SnapshotDirectory {
            dir: self
                .guard
                .state
                .root
                .open_dir(&job.to_string())?
                .open_dir(&format!("{token}.ready"))?,
            manifest_sha256: digest.into(),
            reading_bounds: None,
        };
        super::verify_snapshot(&directory)?;
        let result = visitor(&directory)?;
        self.revalidate()?;
        Ok(result)
    }
}

#[derive(Debug)]
pub(super) struct CustodyGuard {
    #[cfg(target_os = "linux")]
    state: Arc<CustodyState>,
    #[cfg(target_os = "linux")]
    lock: File,
}

impl RootSnapshotCustody {
    #[cfg(target_os = "linux")]
    fn create_fresh_v2(
        path: &Path,
        deployment: Uuid,
        binding: Uuid,
        journal: &Dir,
        writer_contract_sha256: &str,
        opened_root: Option<Dir>,
    ) -> Result<Self, SnapshotIoError> {
        use crate::secure_dir::SNAPSHOT_EXPORT_V2_MARKER;
        let parent = Dir::open_trusted_private_root(path.parent().ok_or_else(invalid)?)?;
        parent.reject_custody_ancestry()?;
        let leaf = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(invalid)?;
        let root = if let Some(root) = opened_root {
            root.require_private_directory()?;
            if parent.open_dir(leaf)?.identity()? != root.identity()?
                || !root.list_bounded(1)?.is_empty()
            {
                return Err(invalid().into());
            }
            root
        } else {
            parent.create_dir(leaf)?
        };
        let lock = root.control_file("effect.lock", true)?;
        lock.sync_all()?;
        let (root_dev, root_ino) = root.identity()?;
        let (journal_dev, journal_ino) = journal.identity()?;
        let m = lock.metadata()?;
        let marker_bytes = serde_json::to_vec(&serde_json::json!({
            "format_version":2,"capability":"snapshot_export_root_v2",
            "binding_id":binding,"deployment_id":deployment,
            "export_root_dev":root_dev,"export_root_ino":root_ino,
            "journal_root_dev":journal_dev,"journal_root_ino":journal_ino,
            "effect_lock_dev":m.dev(),"effect_lock_ino":m.ino(),
            "writer_contract_sha256":writer_contract_sha256
        }))
        .map_err(|_| invalid())?;
        let mut marker = root.control_file(SNAPSHOT_EXPORT_V2_MARKER, true)?;
        marker.write_all(&marker_bytes)?;
        marker.sync_all()?;
        root.sync()?;
        parent.sync()?;
        let state = Arc::new(CustodyState {
            parent,
            leaf: leaf.into(),
            root,
            lock,
            marker,
            marker_bytes,
            marker_name: SNAPSHOT_EXPORT_V2_MARKER,
        });
        state.revalidate()?;
        Ok(Self { state })
    }
    /// Create a new 0700 root below a trusted Root-owned private parent. The
    /// marker is fsynced/read back before any writer can exist. Failure leaves
    /// all created entries in place; existing roots are never adopted.
    pub fn create_fresh(path: &Path) -> Result<Self, SnapshotIoError> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = path;
            Err(SnapshotIoError::InvalidPackage)
        }
        #[cfg(target_os = "linux")]
        {
            if unsafe { libc::geteuid() } != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "snapshot custody requires Root",
                )
                .into());
            }
            if !path.is_absolute() {
                return Err(invalid().into());
            }
            let leaf = path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(invalid)?;
            let parent = Dir::open_trusted_private_root(path.parent().ok_or_else(invalid)?)?;
            parent.reject_custody_ancestry()?;
            let root = parent.create_dir(leaf)?;
            root.require_private_directory()?;
            let lock = root.control_file("effect.lock", true)?;
            lock.sync_all()?;
            root.sync()?;
            let mut marker = root.control_file(SNAPSHOT_CUSTODY_MARKER, true)?;
            let marker_bytes = format!("os-custody-fence-v2:{}\n", Uuid::new_v4()).into_bytes();
            marker.write_all(&marker_bytes)?;
            marker.sync_all()?;
            root.sync()?;
            parent.sync()?;
            let state = Arc::new(CustodyState {
                parent,
                leaf: leaf.into(),
                root,
                lock,
                marker,
                marker_bytes,
                marker_name: SNAPSHOT_CUSTODY_MARKER,
            });
            state.revalidate()?;
            Ok(Self { state })
        }
    }

    /// Independently open the original lock inode and take SH_NB. Closing or
    /// cancelling another caller cannot unlock this open-file-description.
    pub fn writer(&self) -> Result<CustodyWriter, SnapshotIoError> {
        Ok(CustodyWriter {
            guard: self.acquire(false)?,
        })
    }

    /// Only this original live object can acquire capture custody. Busy is an
    /// actual WouldBlock error from flock, never an in-memory writer counter.
    pub fn try_capture(&self) -> Result<CustodyCaptureGuard, SnapshotIoError> {
        Ok(CustodyCaptureGuard {
            guard: self.acquire(true)?,
        })
    }

    fn acquire(&self, exclusive: bool) -> Result<CustodyGuard, SnapshotIoError> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = exclusive;
            Err(SnapshotIoError::InvalidPackage)
        }
        #[cfg(target_os = "linux")]
        {
            self.state.revalidate()?;
            let lock = self.state.root.control_file("effect.lock", false)?;
            same_file(&lock, &self.state.lock, 0)?;
            let kind = if exclusive {
                libc::LOCK_EX
            } else {
                libc::LOCK_SH
            };
            if unsafe { libc::flock(lock.as_raw_fd(), kind | libc::LOCK_NB) } < 0 {
                return Err(io::Error::last_os_error().into());
            }
            let guard = CustodyGuard {
                state: Arc::clone(&self.state),
                lock,
            };
            guard.revalidate()?;
            Ok(guard)
        }
    }
}

impl CustodyWriter {
    /// OS custody only. Before job_directory can mkdir, pay one non-refundable
    /// work reservation on the caller's original cumulative meter for bounded
    /// rendering, all four files, internal verification and one subsequent
    /// broker verify_snapshot. The reservation is not an actual I/O counter.
    pub fn stage_copy_metered(
        self,
        job: Uuid,
        token: Uuid,
        copy: &ReadingCopy,
        budget: crate::root_journal::SharedJournalBudget,
    ) -> Result<CustodyStaged, SnapshotIoError> {
        let prepared = super::copy_meter::prepare(job, copy, &budget)?;
        let dir = self.guard.job_directory(job)?;
        let staged = prepared.stage(StageWriter::in_custody_directory(dir, token, self.guard)?)?;
        Ok(CustodyStaged {
            directory: staged.directory,
            guard: staged.custody.ok_or_else(invalid)?,
        })
    }

    pub fn stage_copy(
        self,
        job: Uuid,
        token: Uuid,
        copy: &ReadingCopy,
    ) -> Result<CustodyStaged, SnapshotIoError> {
        let dir = self.guard.job_directory(job)?;
        let manifest = ReadingCopyManifest {
            format_version: SNAPSHOT_FORMAT_VERSION,
            copy_id: job,
            files: vec![],
        };
        let staged = stage_copy_writer(
            StageWriter::in_custody_directory(dir, token, self.guard)?,
            job,
            &manifest,
            copy,
        )?;
        Ok(CustodyStaged {
            directory: staged.directory,
            guard: staged.custody.ok_or_else(invalid)?,
        })
    }

    pub fn stage_exact(
        self,
        job: Uuid,
        token: Uuid,
        manifest: &ExactSnapshotManifest,
        rows: &[SnapshotRow],
        assets: &[SnapshotAssetUse],
        files: &FsAssetStore,
    ) -> Result<CustodyStaged, SnapshotIoError> {
        let dir = self.guard.job_directory(job)?;
        let staged = stage_exact_writer(
            StageWriter::in_custody_directory(dir, token, self.guard)?,
            manifest,
            rows,
            assets,
            files,
        )?;
        Ok(CustodyStaged {
            directory: staged.directory,
            guard: staged.custody.ok_or_else(invalid)?,
        })
    }
}

impl CustodyGuard {
    pub(super) fn revalidate(&self) -> Result<(), SnapshotIoError> {
        #[cfg(not(target_os = "linux"))]
        {
            Err(SnapshotIoError::InvalidPackage)
        }
        #[cfg(target_os = "linux")]
        {
            self.state.revalidate()?;
            same_file(&self.lock, &self.state.lock, 0)?;
            Ok(())
        }
    }

    fn job_directory(&self, job: Uuid) -> Result<Dir, SnapshotIoError> {
        self.revalidate()?;
        #[cfg(not(target_os = "linux"))]
        {
            let _ = job;
            Err(SnapshotIoError::InvalidPackage)
        }
        #[cfg(target_os = "linux")]
        {
            let dir = super::open_job_directory(&self.state.root, job)?;
            dir.require_private_directory()?;
            self.revalidate()?;
            Ok(dir)
        }
    }
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "snapshot custody identity rejected",
    )
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
struct CustodyState {
    parent: Dir,
    leaf: String,
    root: Dir,
    // Original handles are retained, never flocked, never duplicated for locks.
    lock: File,
    marker: File,
    marker_bytes: Vec<u8>,
    marker_name: &'static str,
}
#[cfg(target_os = "linux")]
impl CustodyState {
    fn revalidate(&self) -> io::Result<()> {
        self.parent.require_private_directory()?;
        self.root.require_private_directory()?;
        let named = self.parent.open_dir(&self.leaf)?;
        named.require_private_directory()?;
        if named.identity()? != self.root.identity()? {
            return Err(invalid());
        }
        let lock = self.root.control_file("effect.lock", false)?;
        same_file(&lock, &self.lock, 0)?;
        let marker = self.root.control_file(self.marker_name, false)?;
        same_file(&marker, &self.marker, self.marker_bytes.len() as u64)?;
        let mut actual = vec![0; self.marker_bytes.len()];
        marker.read_exact_at(&mut actual, 0)?;
        if actual != self.marker_bytes {
            return Err(invalid());
        }
        Ok(())
    }
}
#[cfg(target_os = "linux")]
fn same_file(file: &File, original: &File, length: u64) -> io::Result<()> {
    let a = file.metadata()?;
    let b = original.metadata()?;
    for meta in [&a, &b] {
        if !meta.is_file()
            || meta.uid() != 0
            || meta.mode() & 0o7777 != 0o600
            || meta.nlink() != 1
            || meta.len() != length
        {
            return Err(invalid());
        }
    }
    if (a.dev(), a.ino()) != (b.dev(), b.ino()) {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::{
        fs,
        io::Read,
        os::unix::fs::PermissionsExt,
        sync::mpsc::{self, Receiver, Sender},
        time::Duration,
    };
    #[test]
    #[ignore = "requires Linux UID0 and trusted Root parent; OS component, not enrollment"]
    fn fresh_empty_mount_admission_consumes_original_fds_and_never_adopts_existing_binding() {
        use std::os::unix::fs::OpenOptionsExt;
        assert_eq!(unsafe { libc::geteuid() }, 0);
        let parent = std::env::var_os("TASK6_CUSTODY_TEST_PARENT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| "/root".into());
        let base = parent.join(format!("task6-fresh-mount-{}", Uuid::new_v4()));
        fs::create_dir(&base).unwrap();
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();
        let companion = base.join("companion");
        let export = base.join("export");
        let other = base.join("other");
        for path in [&companion, &export, &other] {
            fs::create_dir(path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let open = |path: &Path| {
            fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(path)
                .unwrap()
        };
        let install = |files: Option<(File, File)>| {
            InstalledSnapshotBinding::install_at(
                (&companion, &export),
                Uuid::new_v4(),
                Uuid::new_v4(),
                Uuid::new_v4(),
                &"a".repeat(64),
                crate::root_journal::SharedJournalBudget::default(),
                files,
            )
        };
        assert!(
            install(None).is_err(),
            "old exclusive fresh-directory install must still refuse existing mount roots"
        );
        assert!(RootSnapshotCustody::create_fresh(&export).is_err());
        assert!(install(Some((open(&companion), open(&companion)))).is_err());
        assert!(install(Some((open(&other), open(&export)))).is_err());
        fs::set_permissions(&companion, fs::Permissions::from_mode(0o750)).unwrap();
        assert!(install(Some((open(&companion), open(&export)))).is_err());
        fs::set_permissions(&companion, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(companion.join("stale"), b"unknown old history").unwrap();
        assert!(install(Some((open(&companion), open(&export)))).is_err());
        fs::remove_file(companion.join("stale")).unwrap();
        assert_eq!(fs::read_dir(&companion).unwrap().count(), 0);
        assert_eq!(fs::read_dir(&export).unwrap().count(), 0);
        let installed = install(Some((open(&companion), open(&export)))).unwrap();
        installed.revalidate().unwrap();
        let writer = installed.custody().writer().unwrap();
        assert!(installed.custody().try_capture().is_err());
        drop(writer);
        installed
            .custody()
            .try_capture()
            .unwrap()
            .revalidate()
            .unwrap();
        assert!(
            install(Some((open(&companion), open(&export)))).is_err(),
            "same mounted inode containing an installed epoch is never adopted"
        );
        assert!(install(None).is_err());
        drop(installed);
        super::super::reading_budget::real_metadata_rejection_cases(&base);
        fs::remove_dir_all(base).unwrap();
    }

    struct PausedRealCopy {
        reads: usize,
        paused: Sender<()>,
        release: Receiver<()>,
    }
    impl Read for PausedRealCopy {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.reads += 1;
            if self.reads <= 16 {
                buf.fill(b'x');
                return Ok(buf.len());
            }
            self.paused.send(()).unwrap();
            self.release.recv_timeout(Duration::from_secs(20)).unwrap();
            Err(io::Error::other("real reader failure after one MiB copied"))
        }
    }

    #[test]
    #[ignore = "requires Linux UID 0 and a trusted root-owned parent; run explicitly"]
    fn real_copy_hash_and_discard_retain_shared_custody_until_writer_drop() {
        assert_eq!(unsafe { libc::geteuid() }, 0, "requires Linux UID 0");
        let parent = std::env::var_os("TASK6_CUSTODY_TEST_PARENT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| "/root".into());
        let base = parent.join(format!("custody-copy-lifetime-{}", Uuid::new_v4()));
        fs::create_dir(&base).unwrap();
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();
        let root = base.join("root");
        let custody = RootSnapshotCustody::create_fresh(&root).unwrap();
        let os_writer = custody.writer().unwrap();
        let job = Uuid::new_v4();
        let token = Uuid::new_v4();
        let dir = os_writer.guard.job_directory(job).unwrap();
        let mut writer = StageWriter::in_custody_directory(dir, token, os_writer.guard).unwrap();
        let (paused_tx, paused_rx) = mpsc::channel();
        let (copy_tx, copy_rx) = mpsc::channel();
        let (cleaned_tx, cleaned_rx) = mpsc::channel();
        let (drop_tx, drop_rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let mut reader = PausedRealCopy {
                reads: 0,
                paused: paused_tx,
                release: copy_rx,
            };
            assert!(writer.write_stream("reading.md", &mut reader).is_err());
            writer.discard();
            cleaned_tx.send(()).unwrap();
            drop_rx.recv_timeout(Duration::from_secs(20)).unwrap();
            // Keep the actual StageWriter and its guard alive through cleanup.
            drop(writer);
        });
        paused_rx.recv_timeout(Duration::from_secs(20)).unwrap();
        let job_root = root.join(job.to_string());
        let partial = fs::read_dir(&job_root)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(
            fs::metadata(partial.join("reading.md")).unwrap().len(),
            1024 * 1024
        );
        assert!(
            matches!(custody.try_capture(), Err(SnapshotIoError::Io(ref e))
            if e.kind() == io::ErrorKind::WouldBlock)
        );
        copy_tx.send(()).unwrap();
        cleaned_rx.recv_timeout(Duration::from_secs(20)).unwrap();
        assert_eq!(fs::read_dir(job_root).unwrap().count(), 0);
        assert!(
            matches!(custody.try_capture(), Err(SnapshotIoError::Io(ref e))
            if e.kind() == io::ErrorKind::WouldBlock)
        );
        drop_tx.send(()).unwrap();
        thread.join().unwrap();
        custody.try_capture().unwrap().revalidate().unwrap();
        drop(custody);
        fs::remove_dir_all(base).unwrap();
    }
}

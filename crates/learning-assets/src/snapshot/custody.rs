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
}

#[derive(Debug)]
pub(super) struct CustodyGuard {
    #[cfg(target_os = "linux")]
    state: Arc<CustodyState>,
    #[cfg(target_os = "linux")]
    lock: File,
}

impl RootSnapshotCustody {
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
        let marker = self.root.control_file(SNAPSHOT_CUSTODY_MARKER, false)?;
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

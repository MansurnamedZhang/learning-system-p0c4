//! Narrow handle-relative filesystem surface for the management backup crate.
//! On non-Linux hosts every operation refuses until equivalent no-follow
//! directory handles are available.
use crate::secure_dir::{Dir, EntryKind};
use std::{fs::File, io, path::Path};

#[derive(Debug)]
pub struct BackupDir(Dir);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupEntryKind {
    File,
    Directory,
    Other,
}

impl BackupDir {
    /// Restore target boundary, including every ancestor from `/`.
    pub fn open_trusted_private_root(path: &Path) -> io::Result<Self> {
        Dir::open_trusted_private_root(path).map(Self)
    }
    pub fn open_private_root(path: &Path) -> io::Result<Self> {
        Dir::open_private_root(path).map(Self)
    }
    pub fn open_dir(&self, name: &str) -> io::Result<Self> {
        self.0.open_dir(name).map(Self)
    }
    pub fn create_dir(&self, name: &str) -> io::Result<Self> {
        self.0.create_dir(name).map(Self)
    }
    pub fn create_file(&self, name: &str) -> io::Result<File> {
        self.0.create_file(name)
    }
    pub fn open_file(&self, name: &str) -> io::Result<File> {
        let file = self.0.open_file(name)?;
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::MetadataExt;
            if file.metadata()?.nlink() != 1 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "hard-linked backup file",
                ));
            }
        }
        Ok(file)
    }
    pub fn kind(&self, name: &str) -> io::Result<BackupEntryKind> {
        self.0.kind(name).map(|kind| match kind {
            EntryKind::File => BackupEntryKind::File,
            EntryKind::Directory => BackupEntryKind::Directory,
            EntryKind::Other => BackupEntryKind::Other,
        })
    }
    pub fn list(&self) -> io::Result<Vec<String>> {
        self.0.list()
    }
    pub fn sync(&self) -> io::Result<()> {
        self.0.sync()
    }
    /// Device identity from the already-open directory handle, avoiding a
    /// second path traversal while checking source/destination separation.
    pub fn device_id(&self) -> io::Result<u64> {
        self.0.device_id()
    }
    /// Device and inode from the already-open directory, for a pinned target.
    pub fn identity(&self) -> io::Result<(u64, u64)> {
        self.0.identity()
    }
    pub fn seal_file(&self, name: &str) -> io::Result<()> {
        self.0.seal_file(name)
    }
    pub fn seal_dir(&self) -> io::Result<()> {
        self.0.chmod(0o500)?;
        self.0.sync()
    }
    pub fn rename_noreplace(&self, old: &str, new: &str) -> io::Result<()> {
        self.0.rename(old, new)
    }
    /// The caller must sync this parent directory after a successful rename.
    /// A sync failure means the publication outcome is indeterminate.
    pub fn rename_noreplace_without_sync(&self, old: &str, new: &str) -> io::Result<()> {
        self.0.rename_without_sync(old, new)
    }
    pub fn try_clone(&self) -> io::Result<Self> {
        self.0.try_clone().map(Self)
    }
}

#[cfg(all(test, target_os = "linux"))]
mod restore_path_tests {
    use super::BackupDir;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::Path;

    #[test]
    fn rejects_world_writable_ancestor_even_for_private_leaf() {
        if unsafe { libc::geteuid() } != 0 {
            return;
        }
        let leaf = std::env::temp_dir().join(format!(
            "knowweave-restore-ancestor-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&leaf).unwrap();
        let opened = BackupDir::open_trusted_private_root(&leaf);
        std::fs::remove_dir(&leaf).unwrap();
        assert!(opened.is_err());
    }

    #[test]
    fn accepts_root_only_chain_and_rejects_symlink_component() {
        if unsafe { libc::geteuid() } != 0 {
            return;
        }
        let base = Path::new("/var/lib");
        let Ok(meta) = base.metadata() else { return };
        if meta.uid() != 0 || meta.mode() & 0o022 != 0 {
            return;
        }
        let parent = base.join(format!("knowweave-restore-{}", uuid::Uuid::new_v4()));
        let real = parent.join("real");
        let alias = parent.join("alias");
        std::fs::create_dir(&parent).unwrap();
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::create_dir(&real).unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        let good = BackupDir::open_trusted_private_root(&real).is_ok();
        let bad = BackupDir::open_trusted_private_root(&alias).is_err();
        std::fs::remove_file(&alias).unwrap();
        std::fs::remove_dir(&real).unwrap();
        std::fs::remove_dir(&parent).unwrap();
        assert!(good && bad);
    }
}

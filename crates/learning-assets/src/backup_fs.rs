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
    pub fn try_clone(&self) -> io::Result<Self> {
        self.0.try_clone().map(Self)
    }
}

//! Handle-relative filesystem operations for the Linux snapshot boundary.
//! Other platforms refuse staging until an equivalent security contract exists.
use std::{fs::File, io, path::Path};

#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) enum EntryKind {
    File,
    Directory,
    Other,
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use std::{
        ffi::{CStr, CString},
        io::{Read, Seek, SeekFrom, Write},
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::{ffi::OsStrExt, fs::MetadataExt},
        },
        path::Component,
    };

    #[derive(Debug)]
    pub(crate) struct Dir {
        file: File,
    }
    impl Dir {
        pub(crate) fn open_private_root(path: &Path) -> io::Result<Self> {
            let current = Self::open_owned_root(path)?;
            let meta = current.file.metadata()?;
            if meta.mode() & 0o777 != 0o700 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "snapshot root must be owner-only 0700",
                ));
            }
            Ok(current)
        }
        pub(crate) fn open_owned_root(path: &Path) -> io::Result<Self> {
            if !path.is_absolute() {
                return Err(invalid());
            }
            let root = CString::new("/").expect("literal");
            let raw = unsafe {
                libc::open(
                    root.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            let mut current = Self::from_raw(raw)?;
            for part in path.components() {
                match part {
                    Component::RootDir => {}
                    Component::Normal(name) => current = current.open_dir_bytes(name.as_bytes())?,
                    _ => return Err(invalid()),
                }
            }
            let meta = current.file.metadata()?;
            if meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o022 != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "snapshot root must be owned and not writable by others",
                ));
            }
            Ok(current)
        }
        fn from_raw(fd: libc::c_int) -> io::Result<Self> {
            if fd < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(Self {
                    file: unsafe { File::from_raw_fd(fd) },
                })
            }
        }
        fn name(name: &str) -> io::Result<CString> {
            if name.is_empty() || name == "." || name == ".." || name.as_bytes().contains(&b'/') {
                return Err(invalid());
            }
            CString::new(name).map_err(|_| invalid())
        }
        fn open_dir_bytes(&self, name: &[u8]) -> io::Result<Self> {
            let name = CString::new(name).map_err(|_| invalid())?;
            let fd = unsafe {
                libc::openat(
                    self.file.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            Self::from_raw(fd)
        }
        pub(crate) fn open_dir(&self, name: &str) -> io::Result<Self> {
            Self::name(name)?;
            self.open_dir_bytes(name.as_bytes())
        }
        pub(crate) fn create_dir(&self, name: &str) -> io::Result<Self> {
            let name = Self::name(name)?;
            if unsafe { libc::mkdirat(self.file.as_raw_fd(), name.as_ptr(), 0o700) } < 0 {
                return Err(io::Error::last_os_error());
            }
            let child = self.open_dir_bytes(name.to_bytes())?;
            child.chmod(0o700)?;
            self.sync()?;
            Ok(child)
        }
        pub(crate) fn create_file(&self, name: &str) -> io::Result<File> {
            let name = Self::name(name)?;
            let fd = unsafe {
                libc::openat(
                    self.file.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_WRONLY
                        | libc::O_CREAT
                        | libc::O_EXCL
                        | libc::O_NOFOLLOW
                        | libc::O_CLOEXEC,
                    0o600,
                )
            };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let file = unsafe { File::from_raw_fd(fd) };
            let meta = file.metadata()?;
            if !meta.is_file() {
                return Err(invalid());
            }
            Ok(file)
        }
        pub(crate) fn open_file(&self, name: &str) -> io::Result<File> {
            let name = Self::name(name)?;
            let fd = unsafe {
                libc::openat(
                    self.file.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let file = unsafe { File::from_raw_fd(fd) };
            if !file.metadata()?.is_file() {
                return Err(invalid());
            }
            Ok(file)
        }
        pub(crate) fn kind(&self, name: &str) -> io::Result<EntryKind> {
            let name = Self::name(name)?;
            let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
            if unsafe {
                libc::fstatat(
                    self.file.as_raw_fd(),
                    name.as_ptr(),
                    stat.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } < 0
            {
                return Err(io::Error::last_os_error());
            }
            let stat = unsafe { stat.assume_init() };
            Ok(match stat.st_mode & libc::S_IFMT {
                libc::S_IFREG => EntryKind::File,
                libc::S_IFDIR => EntryKind::Directory,
                _ => EntryKind::Other,
            })
        }
        pub(crate) fn list(&self) -> io::Result<Vec<String>> {
            let dot = CString::new(".").expect("literal");
            let duplicate = unsafe {
                libc::openat(
                    self.file.as_raw_fd(),
                    dot.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if duplicate < 0 {
                return Err(io::Error::last_os_error());
            }
            let dir = unsafe { libc::fdopendir(duplicate) };
            if dir.is_null() {
                unsafe {
                    libc::close(duplicate);
                }
                return Err(io::Error::last_os_error());
            }
            struct OwnedDir(*mut libc::DIR);
            impl Drop for OwnedDir {
                fn drop(&mut self) {
                    unsafe { libc::closedir(self.0) };
                }
            }
            let dir = OwnedDir(dir);
            let mut names = Vec::new();
            loop {
                unsafe {
                    *libc::__errno_location() = 0;
                }
                let entry = unsafe { libc::readdir(dir.0) };
                if entry.is_null() {
                    let error = io::Error::last_os_error();
                    if error.raw_os_error() == Some(0) {
                        return Ok(names);
                    }
                    return Err(error);
                }
                let bytes = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
                if bytes == b"." || bytes == b".." {
                    continue;
                }
                names.push(String::from_utf8(bytes.to_vec()).map_err(|_| invalid())?);
            }
        }
        pub(crate) fn sync(&self) -> io::Result<()> {
            self.file.sync_all()
        }
        pub(crate) fn chmod(&self, mode: libc::mode_t) -> io::Result<()> {
            if unsafe { libc::fchmod(self.file.as_raw_fd(), mode) } < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
        pub(crate) fn seal_file(&self, name: &str) -> io::Result<()> {
            let file = self.open_file(name)?;
            if unsafe { libc::fchmod(file.as_raw_fd(), 0o400) } < 0 {
                return Err(io::Error::last_os_error());
            }
            file.sync_all()
        }
        pub(crate) fn rename(&self, old: &str, new: &str) -> io::Result<()> {
            let old = Self::name(old)?;
            let new = Self::name(new)?;
            if unsafe {
                libc::renameat2(
                    self.file.as_raw_fd(),
                    old.as_ptr(),
                    self.file.as_raw_fd(),
                    new.as_ptr(),
                    libc::RENAME_NOREPLACE,
                )
            } < 0
            {
                return Err(io::Error::last_os_error());
            }
            self.sync()
        }
        pub(crate) fn remove_tree(&self, name: &str) -> io::Result<()> {
            let child = self.open_dir(name)?;
            child.chmod(0o700)?;
            for entry in child.list()? {
                match child.kind(&entry)? {
                    EntryKind::Directory => child.remove_tree(&entry)?,
                    _ => {
                        let name = Self::name(&entry)?;
                        if unsafe { libc::unlinkat(child.file.as_raw_fd(), name.as_ptr(), 0) } < 0 {
                            return Err(io::Error::last_os_error());
                        }
                    }
                }
            }
            let name = Self::name(name)?;
            if unsafe { libc::unlinkat(self.file.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) }
                < 0
            {
                return Err(io::Error::last_os_error());
            }
            self.sync()
        }
        pub(crate) fn try_clone(&self) -> io::Result<Self> {
            Ok(Self {
                file: self.file.try_clone()?,
            })
        }
    }
    fn invalid() -> io::Error {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid snapshot filesystem entry",
        )
    }

    pub(crate) fn sealed_delivery(
        mut source: File,
        limit: usize,
    ) -> io::Result<(File, String, u64)> {
        use sha2::{Digest, Sha256};
        let name = CString::new("snapshot-delivery").expect("literal");
        let fd = unsafe {
            libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING)
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut sealed = unsafe { File::from_raw_fd(fd) };
        let mut hash = Sha256::new();
        let mut size = 0usize;
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = source.read(&mut buf)?;
            if n == 0 {
                break;
            }
            size = size.checked_add(n).ok_or_else(invalid)?;
            if size > limit {
                return Err(io::Error::new(
                    io::ErrorKind::FileTooLarge,
                    "snapshot file limit",
                ));
            }
            sealed.write_all(&buf[..n])?;
            hash.update(&buf[..n]);
        }
        let flags =
            libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL;
        if unsafe { libc::fcntl(sealed.as_raw_fd(), libc::F_ADD_SEALS, flags) } < 0 {
            return Err(io::Error::last_os_error());
        }
        sealed.seek(SeekFrom::Start(0))?;
        Ok((sealed, format!("{:x}", hash.finalize()), size as u64))
    }
}

#[cfg(not(target_os = "linux"))]
mod platform {
    use super::*;
    #[derive(Debug)]
    pub(crate) struct Dir;
    fn unsupported<T>() -> io::Result<T> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "snapshot staging requires Linux",
        ))
    }
    impl Dir {
        pub(crate) fn open_private_root(_: &Path) -> io::Result<Self> {
            unsupported()
        }
        pub(crate) fn open_owned_root(_: &Path) -> io::Result<Self> {
            unsupported()
        }
        pub(crate) fn open_dir(&self, _: &str) -> io::Result<Self> {
            unsupported()
        }
        pub(crate) fn create_dir(&self, _: &str) -> io::Result<Self> {
            unsupported()
        }
        pub(crate) fn create_file(&self, _: &str) -> io::Result<File> {
            unsupported()
        }
        pub(crate) fn open_file(&self, _: &str) -> io::Result<File> {
            unsupported()
        }
        pub(crate) fn kind(&self, _: &str) -> io::Result<EntryKind> {
            unsupported()
        }
        pub(crate) fn list(&self) -> io::Result<Vec<String>> {
            unsupported()
        }
        pub(crate) fn sync(&self) -> io::Result<()> {
            unsupported()
        }
        pub(crate) fn chmod(&self, _: u32) -> io::Result<()> {
            unsupported()
        }
        pub(crate) fn seal_file(&self, _: &str) -> io::Result<()> {
            unsupported()
        }
        pub(crate) fn rename(&self, _: &str, _: &str) -> io::Result<()> {
            unsupported()
        }
        pub(crate) fn remove_tree(&self, _: &str) -> io::Result<()> {
            unsupported()
        }
        pub(crate) fn try_clone(&self) -> io::Result<Self> {
            unsupported()
        }
    }
    pub(crate) fn sealed_delivery(_: File, _: usize) -> io::Result<(File, String, u64)> {
        unsupported()
    }
}
pub(crate) use platform::{Dir, sealed_delivery};

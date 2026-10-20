use super::*;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    ffi::{CStr, CString},
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Component, Path, PathBuf},
};

fn from_fd(fd: libc::c_int) -> io::Result<File> {
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}
fn cstring(s: &str) -> io::Result<CString> {
    CString::new(s).map_err(|_| invalid())
}
fn trusted(m: &fs::Metadata, mode: Option<u32>, file: bool) -> io::Result<()> {
    if m.uid() != 0
        || m.gid() != 0
        || m.mode() & 0o7022 != 0
        || mode.is_some_and(|v| m.mode() & 0o777 != v)
        || (file && (!m.is_file() || m.nlink() != 1))
        || (!file && !m.is_dir())
    {
        return Err(invalid());
    }
    Ok(())
}
fn identity(m: &fs::Metadata) -> (u64, u64) {
    (m.dev(), m.ino())
}
type StableMetadata = (u64, u64, u64, i64, i64, i64, i64);
fn stable(m: &fs::Metadata) -> StableMetadata {
    (
        m.dev(),
        m.ino(),
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
    )
}
struct Directory(File);
impl Directory {
    fn open(path: &Path) -> io::Result<Self> {
        if !path.is_absolute() {
            return Err(invalid());
        }
        let mut d = Self(from_fd(unsafe {
            libc::open(
                cstring("/")?.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        })?);
        trusted(&d.0.metadata()?, None, false)?;
        for part in path.components() {
            match part {
                Component::RootDir => continue,
                Component::Normal(n) => {
                    let name = CString::new(n.as_bytes()).map_err(|_| invalid())?;
                    d = Self(from_fd(unsafe {
                        libc::openat(
                            d.0.as_raw_fd(),
                            name.as_ptr(),
                            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                        )
                    })?);
                    trusted(&d.0.metadata()?, None, false)?;
                }
                _ => return Err(invalid()),
            }
        }
        trusted(&d.0.metadata()?, Some(0o700), false)?;
        Ok(d)
    }
    fn file(&self, name: &str, create: bool) -> io::Result<File> {
        let mut flags = libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;
        flags |= if create {
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL
        } else {
            libc::O_RDONLY
        };
        let f = from_fd(unsafe {
            libc::openat(self.0.as_raw_fd(), cstring(name)?.as_ptr(), flags, 0o600)
        })?;
        trusted(&f.metadata()?, Some(0o600), true)?;
        Ok(f)
    }
    fn names(&self, budget: &SharedJournalBudget) -> io::Result<BTreeSet<String>> {
        let fd = from_fd(unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                cstring(".")?.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        })?;
        let raw = fd.as_raw_fd();
        let dir = unsafe { libc::fdopendir(raw) };
        if dir.is_null() {
            return Err(io::Error::last_os_error());
        }
        std::mem::forget(fd);
        struct Entries(*mut libc::DIR);
        impl Drop for Entries {
            fn drop(&mut self) {
                unsafe {
                    libc::closedir(self.0);
                }
            }
        }
        let entries = Entries(dir);
        let mut names = BTreeSet::new();
        loop {
            unsafe {
                *libc::__errno_location() = 0;
            }
            let row = unsafe { libc::readdir(entries.0) };
            if row.is_null() {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(0) {
                    return Err(error);
                }
                break;
            }
            let name = unsafe { CStr::from_ptr((*row).d_name.as_ptr()) }.to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            budget.charge(name.len() as u64, 1)?;
            let name = std::str::from_utf8(name).map_err(|_| invalid())?.to_owned();
            if !names.insert(name) {
                return Err(invalid());
            }
        }
        Ok(names)
    }
}

struct OriginalFrame {
    head: ByteJournalHead,
    inode: (u64, u64),
    ctime: (i64, i64),
    size: u64,
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum Boundary {
    FileSync,
    AfterFileSync,
    Rename,
    ParentSync,
    AfterParentSync,
    Readback,
}

/// A fresh original-process Root storage context, never reconstructed from
/// disk. The only public append input is explicitly uninterpreted bytes.
/// Successful receipts and the head are byte-storage observations, not SQL,
/// claim, writer, capture or NoEffect capabilities.
pub struct RootJournalBytes {
    path: PathBuf,
    root: Directory,
    epoch: File,
    epoch_stable: StableMetadata,
    epoch_bytes: Vec<u8>,
    id: ByteJournalIdentity,
    prefix: Vec<OriginalFrame>,
    budget: SharedJournalBudget,
    stored: u64,
    poisoned: bool,
    #[cfg(test)]
    fault: Option<Boundary>,
    #[cfg(test)]
    action: Option<Box<dyn FnMut(Boundary) -> io::Result<()>>>,
}
impl RootJournalBytes {
    /// Existing empty Root0:0/0700 directory under non-writable Root parents.
    /// Any residue, including a complete-looking old epoch, is rejected.
    pub fn create_fresh(
        path: &Path,
        id: ByteJournalIdentity,
        budget: SharedJournalBudget,
    ) -> io::Result<Self> {
        if unsafe { libc::geteuid() } != 0 || unsafe { libc::getegid() } != 0 {
            return Err(invalid());
        }
        let root = Directory::open(path)?;
        if !root.names(&budget)?.is_empty() {
            return Err(invalid());
        }
        let epoch_bytes = id.encode();
        budget.charge(epoch_bytes.len() as u64, 1)?;
        budget.charge(epoch_bytes.len() as u64, 0)?;
        let mut epoch = root.file(".epoch.pending", true)?;
        epoch.write_all(&epoch_bytes)?;
        epoch.sync_all()?;
        rename_new(&root, ".epoch.pending", "epoch.header")?;
        root.0.sync_all()?;
        let epoch_stable = stable(&epoch.metadata()?);
        let mut this = Self {
            path: path.to_path_buf(),
            root,
            epoch,
            epoch_stable,
            epoch_bytes,
            id,
            prefix: Vec::new(),
            budget,
            stored: 40,
            poisoned: true,
            #[cfg(test)]
            fault: None,
            #[cfg(test)]
            action: None,
        };
        this.verify_prefix(None)?;
        this.poisoned = false;
        Ok(this)
    }
    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }
    /// Refuses to expose the former head after any unknown append outcome.
    pub fn known_byte_head(&self) -> io::Result<ByteJournalHead> {
        if self.poisoned {
            return Err(poisoned());
        }
        Ok(self.prefix.last().map_or(
            ByteJournalHead {
                sequence: 0,
                sha256: digest(&self.epoch_bytes),
            },
            |f| f.head,
        ))
    }
    pub fn append_uninterpreted(
        &mut self,
        body: OpaqueJournalBody<'_>,
    ) -> io::Result<ByteJournalHead> {
        let head = self.known_byte_head()?;
        // Before any IO, allocation or callback. Panic/cancellation cannot
        // leave a usable old head after a potentially started append.
        self.poisoned = true;
        let frame = self.append_inner(head, &body)?;
        let next = frame.head;
        self.stored += frame.size;
        self.prefix.push(frame);
        self.poisoned = false;
        Ok(next)
    }
    fn append_inner(
        &mut self,
        head: ByteJournalHead,
        body: &OpaqueJournalBody<'_>,
    ) -> io::Result<OriginalFrame> {
        self.verify_prefix(None)?;
        let size = (HEADER_BYTES + body.0.len()) as u64;
        if self
            .stored
            .checked_add(size)
            .is_none_or(|v| v > MAX_STORED_BYTES)
        {
            return Err(invalid());
        }
        self.budget.charge(size, 1)?;
        let bytes = encode_frame(self.id, head, body)?;
        self.budget.charge(size, 0)?;
        let next = ByteJournalHead {
            sequence: head.sequence.checked_add(1).ok_or_else(invalid)?,
            sha256: digest(&bytes),
        };
        let name = format!("{:020}.frame", next.sequence);
        let pending = format!(".{:020}.pending", next.sequence);
        let mut file = self.root.file(&pending, true)?;
        file.write_all(&bytes)?;
        #[cfg(test)]
        self.fail_at(Boundary::FileSync)?;
        file.sync_all()?;
        #[cfg(test)]
        self.fail_at(Boundary::AfterFileSync)?;
        self.recheck_root()?;
        self.same_named(&pending, &file)?;
        #[cfg(test)]
        self.fail_at(Boundary::Rename)?;
        rename_new(&self.root, &pending, &name)?;
        #[cfg(test)]
        self.fail_at(Boundary::ParentSync)?;
        self.root.0.sync_all()?;
        #[cfg(test)]
        self.fail_at(Boundary::AfterParentSync)?;
        #[cfg(test)]
        self.fail_at(Boundary::Readback)?;
        self.same_named(&name, &file)?;
        let m = file.metadata()?;
        let frame = OriginalFrame {
            head: next,
            inode: identity(&m),
            ctime: (m.ctime(), m.ctime_nsec()),
            size,
        };
        read_frame(&mut file, self.id, head, &frame, &self.budget)?;
        self.verify_prefix(Some(&frame))?;
        Ok(frame)
    }
    #[cfg(test)]
    fn fail_at(&mut self, at: Boundary) -> io::Result<()> {
        if let Some(action) = self.action.as_mut() {
            action(at)?;
        }
        if self.fault == Some(at) {
            Err(io::Error::other("injected real append IO boundary failure"))
        } else {
            Ok(())
        }
    }
    fn recheck_root(&self) -> io::Result<()> {
        let current = Directory::open(&self.path)?;
        trusted(&self.root.0.metadata()?, Some(0o700), false)?;
        if identity(&current.0.metadata()?) != identity(&self.root.0.metadata()?) {
            return Err(invalid());
        }
        Ok(())
    }
    fn same_named(&self, name: &str, held: &File) -> io::Result<()> {
        let named = self.root.file(name, false)?;
        trusted(&held.metadata()?, Some(0o600), true)?;
        if identity(&named.metadata()?) != identity(&held.metadata()?) {
            return Err(invalid());
        }
        Ok(())
    }
    fn verify_prefix(&mut self, extra: Option<&OriginalFrame>) -> io::Result<()> {
        self.recheck_root()?;
        let directory_before = stable(&self.root.0.metadata()?);
        self.same_named("epoch.header", &self.epoch)?;
        let m = self.epoch.metadata()?;
        self.budget.charge(m.len(), 1)?;
        if m.len() != self.epoch_bytes.len() as u64 || stable(&m) != self.epoch_stable {
            return Err(invalid());
        }
        self.epoch.seek(SeekFrom::Start(0))?;
        let mut b = [0; 40];
        self.epoch.read_exact(&mut b)?;
        if b.as_slice() != self.epoch_bytes.as_slice()
            || stable(&self.epoch.metadata()?) != self.epoch_stable
        {
            return Err(invalid());
        }
        let mut expected = BTreeSet::from(["epoch.header".to_owned()]);
        let mut head = ByteJournalHead {
            sequence: 0,
            sha256: digest(&self.epoch_bytes),
        };
        for frame in self.prefix.iter().chain(extra) {
            let name = format!("{:020}.frame", frame.head.sequence);
            expected.insert(name.clone());
            let mut f = self.root.file(&name, false)?;
            read_frame(&mut f, self.id, head, frame, &self.budget)?;
            head = frame.head;
        }
        if self.root.names(&self.budget)? != expected {
            return Err(invalid());
        }
        if stable(&self.root.0.metadata()?) != directory_before {
            return Err(invalid());
        }
        self.recheck_root()?;
        Ok(())
    }
}

fn rename_new(root: &Directory, old: &str, new: &str) -> io::Result<()> {
    let result = unsafe {
        libc::renameat2(
            root.0.as_raw_fd(),
            cstring(old)?.as_ptr(),
            root.0.as_raw_fd(),
            cstring(new)?.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
fn read_frame(
    file: &mut File,
    id: ByteJournalIdentity,
    previous: ByteJournalHead,
    frame: &OriginalFrame,
    budget: &SharedJournalBudget,
) -> io::Result<()> {
    let before = file.metadata()?;
    trusted(&before, Some(0o600), true)?;
    if identity(&before) != frame.inode
        || (before.ctime(), before.ctime_nsec()) != frame.ctime
        || before.len() != frame.size
        || before.len() > MAX_FRAME_BYTES as u64
    {
        return Err(invalid());
    }
    budget.charge(before.len(), 1)?;
    file.seek(SeekFrom::Start(0))?;
    let mut header = [0; HEADER_BYTES];
    file.read_exact(&mut header)?;
    let parsed = FrameHeader::parse(&header)?;
    if parsed.identity != id
        || parsed.sequence != previous.sequence.checked_add(1).ok_or_else(invalid)?
        || parsed.sequence != frame.head.sequence
        || parsed.previous != previous.sha256
        || parsed.body_len + HEADER_BYTES as u64 != before.len()
    {
        return Err(invalid());
    }
    let mut whole = Sha256::new();
    whole.update(header);
    let mut body = Sha256::new();
    let mut left = parsed.body_len;
    let mut buffer = [0; 65536];
    while left > 0 {
        let count = file.read(&mut buffer[..left.min(65536) as usize])?;
        if count == 0 {
            return Err(invalid());
        }
        whole.update(&buffer[..count]);
        body.update(&buffer[..count]);
        left -= count as u64;
    }
    if <[u8; 32]>::from(whole.finalize()) != frame.head.sha256
        || <[u8; 32]>::from(body.finalize()) != parsed.body_sha256
        || stable(&before) != stable(&file.metadata()?)
    {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            assert_eq!(unsafe { libc::geteuid() }, 0, "explicit Root-only test");
            let p = Path::new("/run").join(format!("kw-journal-{}", Uuid::new_v4()));
            fs::create_dir(&p).unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(0o700)).unwrap();
            Self(p)
        }
        fn journal(&self) -> RootJournalBytes {
            RootJournalBytes::create_fresh(
                &self.0,
                ByteJournalIdentity::new(Uuid::new_v4(), Uuid::new_v4()).unwrap(),
                SharedJournalBudget::default(),
            )
            .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            assert!(
                self.0.parent() == Some(Path::new("/run"))
                    && self
                        .0
                        .file_name()
                        .unwrap()
                        .as_bytes()
                        .starts_with(b"kw-journal-")
            );
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn append(j: &mut RootJournalBytes, b: &[u8]) -> io::Result<ByteJournalHead> {
        j.append_uninterpreted(OpaqueJournalBody::from_uninterpreted_bytes(b).unwrap())
    }
    #[test]
    #[ignore = "requires actual Linux Root /run private fixture"]
    fn root_real_append_chain_is_fresh_only_and_missing_history_poisoned() {
        let f = Fixture::new();
        let mut j = f.journal();
        let first = append(&mut j, b"opaque first").unwrap();
        assert_eq!(first.sequence, 1);
        assert_eq!(append(&mut j, b"opaque second").unwrap().sequence, 2);
        assert!(
            RootJournalBytes::create_fresh(&f.0, j.id, SharedJournalBudget::default()).is_err()
        );
        fs::remove_file(f.0.join("00000000000000000001.frame")).unwrap();
        assert!(append(&mut j, b"third").is_err());
        assert!(j.is_poisoned() && j.known_byte_head().is_err());
        assert!(append(&mut j, b"cannot retry").is_err());
        assert!(!f.0.join("00000000000000000003.frame").exists());
    }
    #[test]
    #[ignore = "requires actual Linux Root /run private fixture"]
    fn root_real_io_boundaries_keep_residue_and_never_advance_or_adopt() {
        for phase in [
            Boundary::FileSync,
            Boundary::Rename,
            Boundary::ParentSync,
            Boundary::AfterParentSync,
            Boundary::Readback,
        ] {
            let f = Fixture::new();
            let mut j = f.journal();
            j.fault = Some(phase);
            assert!(append(&mut j, b"unconfirmed").is_err());
            assert!(j.is_poisoned() && j.prefix.is_empty() && j.known_byte_head().is_err());
            assert!(append(&mut j, b"do not overwrite").is_err());
            assert!(
                f.0.join(".00000000000000000001.pending").exists()
                    || f.0.join("00000000000000000001.frame").exists()
            );
            drop(j);
            assert!(
                RootJournalBytes::create_fresh(
                    &f.0,
                    ByteJournalIdentity::new(Uuid::new_v4(), Uuid::new_v4()).unwrap(),
                    SharedJournalBudget::default()
                )
                .is_err()
            );
        }
    }
    #[test]
    #[ignore = "requires actual Linux Root /run private fixture"]
    fn root_real_noreplace_collision_preserves_original_bytes() {
        let f = Fixture::new();
        let j = f.journal();
        fs::write(f.0.join("left"), b"new").unwrap();
        fs::write(f.0.join("right"), b"original").unwrap();
        assert!(rename_new(&j.root, "left", "right").is_err());
        assert_eq!(fs::read(f.0.join("right")).unwrap(), b"original");
        assert_eq!(fs::read(f.0.join("left")).unwrap(), b"new");
    }
    #[test]
    #[ignore = "requires actual Linux Root /run private fixture"]
    fn root_real_path_replacement_symlink_and_pending_are_refused() {
        let f = Fixture::new();
        let mut j = f.journal();
        let moved = f.0.with_extension("moved");
        fs::rename(&f.0, &moved).unwrap();
        fs::create_dir(&f.0).unwrap();
        fs::set_permissions(&f.0, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(append(&mut j, b"wrong root").is_err());
        assert!(j.is_poisoned());
        fs::remove_dir_all(&moved).unwrap();
        let alias = f.0.join("alias");
        symlink(&f.0, &alias).unwrap();
        assert!(
            RootJournalBytes::create_fresh(&alias, j.id, SharedJournalBudget::default()).is_err()
        );
        fs::remove_file(alias).unwrap();
        let mut next = f.journal();
        fs::write(f.0.join(".unexpected.pending"), b"unknown").unwrap();
        assert!(append(&mut next, b"reject residue").is_err());
        assert!(next.is_poisoned());
    }
    #[test]
    #[ignore = "requires actual Linux Root /run private fixture"]
    fn root_real_directory_race_after_file_sync_poison_preserves_pending() {
        let f = Fixture::new();
        let mut j = f.journal();
        let original = f.0.clone();
        let displaced = f.0.with_extension("displaced");
        let saved = displaced.clone();
        j.action = Some(Box::new(move |phase| {
            if phase == Boundary::AfterFileSync {
                fs::rename(&original, &displaced)?;
                fs::create_dir(&original)?;
                fs::set_permissions(&original, fs::Permissions::from_mode(0o700))?;
            }
            Ok(())
        }));
        assert!(append(&mut j, b"race").is_err());
        assert!(j.is_poisoned() && j.known_byte_head().is_err());
        assert!(saved.join(".00000000000000000001.pending").exists());
        assert!(!saved.join("00000000000000000001.frame").exists());
        fs::remove_dir_all(saved).unwrap();
    }
    #[test]
    #[ignore = "requires actual Linux Root /run private fixture"]
    fn root_real_prefix_replacement_and_shared_aggregate_budget_fence() {
        let f = Fixture::new();
        let mut j = f.journal();
        append(&mut j, b"one").unwrap();
        let p = f.0.join("00000000000000000001.frame");
        let bytes = fs::read(&p).unwrap();
        let _old_inode = File::open(&p).unwrap();
        fs::remove_file(&p).unwrap();
        fs::write(&p, &bytes).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(append(&mut j, b"same bytes new inode").is_err());
        let g = Fixture::new();
        let budget = SharedJournalBudget::limited(1000, 100);
        let mut k = RootJournalBytes::create_fresh(&g.0, j.id, budget.clone()).unwrap();
        append(&mut k, b"first").unwrap();
        assert!(append(&mut k, b"second aggregate exceeds").is_err());
        assert!(budget.usage().unwrap().exhausted && k.is_poisoned());
    }

    #[test]
    #[ignore = "requires actual Linux Root /run private fixture"]
    fn root_real_tamper_hardlink_and_directory_mode_changes_poison() {
        for change in 0..3 {
            let f = Fixture::new();
            let mut j = f.journal();
            append(&mut j, b"verified opaque body").unwrap();
            let path = f.0.join("00000000000000000001.frame");
            match change {
                0 => {
                    let mut file = fs::OpenOptions::new().write(true).open(&path).unwrap();
                    file.seek(SeekFrom::Start(HEADER_BYTES as u64)).unwrap();
                    file.write_all(b"tampered").unwrap();
                    file.sync_all().unwrap();
                }
                1 => fs::hard_link(&path, f.0.join("alias")).unwrap(),
                _ => fs::set_permissions(&f.0, fs::Permissions::from_mode(0o770)).unwrap(),
            }
            assert!(append(&mut j, b"must not append").is_err());
            assert!(j.is_poisoned() && j.known_byte_head().is_err());
            assert!(!f.0.join("00000000000000000002.frame").exists());
        }
    }

    #[test]
    #[ignore = "requires actual Linux Root /run private fixture"]
    fn root_real_same_inode_same_bytes_epoch_rewrite_poisoned() {
        let f = Fixture::new();
        let mut j = f.journal();
        let path = f.0.join("epoch.header");
        let before = fs::metadata(&path).unwrap();
        let original = fs::read(&path).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let mut file = fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.write_all(&original).unwrap();
        file.sync_all().unwrap();
        let after = file.metadata().unwrap();
        assert_eq!(identity(&before), identity(&after));
        assert_eq!(before.len(), after.len());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_ne!(
            (before.ctime(), before.ctime_nsec()),
            (after.ctime(), after.ctime_nsec())
        );
        assert_ne!(stable(&before), stable(&after));
        assert!(append(&mut j, b"epoch was rewritten").is_err());
        assert!(j.is_poisoned() && j.known_byte_head().is_err());
        assert!(j.prefix.is_empty());
        assert!(append(&mut j, b"no replay").is_err());
        assert!(!f.0.join("00000000000000000001.frame").exists());
    }
}

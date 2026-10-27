//! Real Linux observation primitives. None returns a qualified process ctx.
use super::{Error, Result, job};
use learning_assets::root_journal::SharedJournalBudget;
use sha2::{Digest, Sha256};
use std::{
    ffi::CString,
    fs::File,
    io::{Read, Seek, SeekFrom},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
        unix::fs::MetadataExt,
    },
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};

/// No reset. The future zero-argument factory must call this first, before
/// compile-pin parsing, environment/FD operations or any other fallible IO.
static PROCESS_ATTEMPT: AtomicBool = AtomicBool::new(false);
pub(super) fn burn_process_attempt() -> Result<()> {
    start_process_attempt().map(|_| ())
}
fn start_process_attempt() -> Result<SharedJournalBudget> {
    let scan = burn_attempt_before_io()?;
    scan.charge(0, 2).map_err(|_| Error::Capacity)?;
    require_actual_root()?;
    Ok(scan)
}
fn burn_attempt_before_io() -> Result<SharedJournalBudget> {
    PROCESS_ATTEMPT
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| Error::Poisoned)?;
    Ok(SharedJournalBudget::default())
}
pub(super) fn require_actual_root() -> Result<()> {
    let (mut r, mut e, mut s) = (u32::MAX, u32::MAX, u32::MAX);
    let (mut gr, mut ge, mut gs) = (u32::MAX, u32::MAX, u32::MAX);
    if unsafe { libc::getresuid(&mut r, &mut e, &mut s) } != 0
        || unsafe { libc::getresgid(&mut gr, &mut ge, &mut gs) } != 0
    {
        return Err(Error::Io);
    }
    if (r, e, s, gr, ge, gs) != (0, 0, 0, 0, 0, 0) {
        return Err(Error::Identity);
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Stamp {
    pub(super) dev: u64,
    pub(super) ino: u64,
    pub(super) uid: u32,
    pub(super) gid: u32,
    pub(super) mode: u32,
    pub(super) links: u64,
    pub(super) bytes: u64,
    pub(super) mtime_ns: u64,
    pub(super) ctime_ns: u64,
}
impl Stamp {
    pub(super) fn observe(file: &File, scan: &SharedJournalBudget) -> Result<Self> {
        scan.charge(0, 1).map_err(|_| Error::Capacity)?;
        let m = file.metadata().map_err(|_| Error::Io)?;
        let nanos = |s: i64, n: i64| -> Result<u64> {
            u64::try_from(s)
                .ok()
                .and_then(|s| s.checked_mul(1_000_000_000))
                .and_then(|s| {
                    u64::try_from(n)
                        .ok()
                        .filter(|v| *v < 1_000_000_000)
                        .and_then(|n| s.checked_add(n))
                })
                .ok_or(Error::Identity)
        };
        Ok(Self {
            dev: m.dev(),
            ino: m.ino(),
            uid: m.uid(),
            gid: m.gid(),
            mode: m.mode(),
            links: m.nlink(),
            bytes: m.len(),
            mtime_ns: nanos(m.mtime(), m.mtime_nsec())?,
            ctime_ns: nanos(m.ctime(), m.ctime_nsec())?,
        })
    }
    pub(super) fn same_declared(&self, d: &job::FileStamp) -> bool {
        self.dev == d.dev
            && self.ino == d.ino
            && u64::from(self.uid) == d.uid
            && u64::from(self.gid) == d.gid
            && u64::from(self.mode) == d.mode
            && self.links == d.nlink
            && self.bytes == d.bytes
            && self.mtime_ns == d.mtime_ns
            && self.ctime_ns == d.ctime_ns
    }
    fn root_file(&self, maximum: u64) -> bool {
        self.dev > 0
            && self.ino > 0
            && self.uid == 0
            && self.links == 1
            && self.mode & libc::S_IFMT == libc::S_IFREG
            && self.mode & 0o022 == 0
            && self.bytes > 0
            && self.bytes <= maximum
    }
    fn same_directory_identity(&self, other: &Self) -> bool {
        self.dev == other.dev
            && self.ino == other.ino
            && self.uid == other.uid
            && self.gid == other.gid
            && self.mode == other.mode
            && self.mode & libc::S_IFMT == libc::S_IFDIR
            && self.uid == 0
            && self.mode & 0o022 == 0
    }
}

/// Openat ancestry remains held. A named reopen cannot adopt a new inode.
pub(super) struct HeldNamed {
    file: File,
    parents: Vec<(File, Stamp)>,
    leaf: CString,
    stamp: Stamp,
    path: String,
}
impl HeldNamed {
    pub(super) fn open(path: &str, scan: &SharedJournalBudget) -> Result<Self> {
        if !job::path(path) {
            return Err(Error::Shape);
        }
        let segments: Vec<_> = path[1..].split('/').collect();
        let root = CString::new("/").map_err(|_| Error::Shape)?;
        scan.charge(1, 1).map_err(|_| Error::Capacity)?;
        let raw = unsafe {
            libc::open(
                root.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if raw < 0 {
            return Err(Error::Io);
        }
        let mut directory = unsafe { File::from_raw_fd(raw) };
        let mut parents = Vec::new();
        for segment in &segments[..segments.len() - 1] {
            let stamp = Stamp::observe(&directory, scan)?;
            if stamp.uid != 0
                || stamp.mode & libc::S_IFMT != libc::S_IFDIR
                || stamp.mode & 0o022 != 0
            {
                return Err(Error::Identity);
            }
            let name = CString::new(*segment).map_err(|_| Error::Shape)?;
            scan.charge(segment.len() as u64, 1)
                .map_err(|_| Error::Capacity)?;
            let next = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                )
            };
            if next < 0 {
                return Err(Error::Io);
            }
            parents.push((directory, stamp));
            directory = unsafe { File::from_raw_fd(next) };
        }
        let stamp = Stamp::observe(&directory, scan)?;
        if stamp.uid != 0 || stamp.mode & libc::S_IFMT != libc::S_IFDIR || stamp.mode & 0o022 != 0 {
            return Err(Error::Identity);
        }
        let leaf = CString::new(*segments.last().ok_or(Error::Shape)?).map_err(|_| Error::Shape)?;
        scan.charge(leaf.as_bytes().len() as u64, 1)
            .map_err(|_| Error::Capacity)?;
        let raw = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                leaf.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            )
        };
        if raw < 0 {
            return Err(Error::Io);
        }
        let file = unsafe { File::from_raw_fd(raw) };
        parents.push((directory, stamp));
        let stamp = Stamp::observe(&file, scan)?;
        Ok(Self {
            file,
            parents,
            leaf,
            stamp,
            path: path.to_owned(),
        })
    }
    fn revalidate(&self, scan: &SharedJournalBudget) -> Result<()> {
        let named = Self::open(&self.path, scan)?;
        if named.parents.len() != self.parents.len()
            || named.stamp != self.stamp
            || !named
                .parents
                .iter()
                .zip(&self.parents)
                .all(|((_, a), (_, b))| a.same_directory_identity(b))
        {
            return Err(Error::Identity);
        }
        for (parent, stamp) in &self.parents {
            if !Stamp::observe(parent, scan)?.same_directory_identity(stamp) {
                return Err(Error::Identity);
            }
        }
        if Stamp::observe(&self.file, scan)? != self.stamp {
            return Err(Error::Identity);
        }
        let parent = &self.parents.last().ok_or(Error::Identity)?.0;
        scan.charge(self.leaf.as_bytes().len() as u64, 1)
            .map_err(|_| Error::Capacity)?;
        let raw = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                self.leaf.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            )
        };
        if raw < 0 {
            return Err(Error::Identity);
        }
        let reopened = unsafe { File::from_raw_fd(raw) };
        if Stamp::observe(&reopened, scan)? != self.stamp {
            return Err(Error::Identity);
        }
        scan.charge(0, 1).map_err(|_| Error::Capacity)?;
        require_cloexec(self.file.as_raw_fd())?;
        Ok(())
    }
    fn full_read_and_hash(
        &mut self,
        maximum: u64,
        scan: &SharedJournalBudget,
    ) -> Result<(Vec<u8>, [u8; 32])> {
        if !self.stamp.root_file(maximum) {
            return Err(Error::Identity);
        }
        self.revalidate(scan)?;
        // Actual read and full hash are separately charged before allocation.
        scan.charge(self.stamp.bytes, 1)
            .map_err(|_| Error::Capacity)?;
        scan.charge(self.stamp.bytes, 1)
            .map_err(|_| Error::Capacity)?;
        let length = usize::try_from(self.stamp.bytes).map_err(|_| Error::Capacity)?;
        let mut bytes = vec![0; length];
        self.file.seek(SeekFrom::Start(0)).map_err(|_| Error::Io)?;
        self.file.read_exact(&mut bytes).map_err(|_| Error::Io)?;
        let mut extra = [0];
        if self.file.read(&mut extra).map_err(|_| Error::Io)? != 0 {
            return Err(Error::Identity);
        }
        let digest = Sha256::digest(&bytes).into();
        self.revalidate(scan)?;
        Ok((bytes, digest))
    }
}

pub(super) struct HeldCode {
    held: HeldNamed,
    digest: [u8; 32],
}
impl HeldCode {
    pub(super) fn open(
        declared: &job::CodeFile,
        manager: bool,
        scan: &SharedJournalBudget,
    ) -> Result<Self> {
        let mut held = HeldNamed::open(&declared.path, scan)?;
        if !held.stamp.same_declared(&declared.stamp)
            || manager && held.stamp.mode & 0o7777 != 0o500
        {
            return Err(Error::Identity);
        }
        let (bytes, digest) = held.full_read_and_hash(128 * 1024 * 1024, scan)?;
        if !bytes.starts_with(b"\x7fELF") || encode_hex(&digest) != declared.sha256 {
            return Err(Error::Identity);
        }
        Ok(Self { held, digest })
    }
    pub(super) fn revalidate(&self, scan: &SharedJournalBudget) -> Result<()> {
        self.held.revalidate(scan)
    }
    pub(super) fn matches_actual_exe(&self, pid: u32, scan: &SharedJournalBudget) -> Result<()> {
        if pid == 0 {
            return Err(Error::Shape);
        }
        let view = ProcView::open(scan)?;
        let actual = view.open_checked_magic(&format!("{pid}/exe"), scan)?;
        // This actual current-task/parent ELF is joined to the fixed held file;
        // the proc magic link never substitutes a named-code declaration.
        if Stamp::observe(&actual, scan)? != self.held.stamp {
            return Err(Error::Identity);
        }
        Ok(())
    }
}
pub(super) struct HeldRecord<T> {
    held: HeldNamed,
    raw: Vec<u8>,
    digest: [u8; 32],
    data: T,
}
impl<T: job::Record + serde::de::DeserializeOwned> HeldRecord<T> {
    pub(super) fn open(path: &str, maximum: u64, scan: &SharedJournalBudget) -> Result<Self> {
        let mut held = HeldNamed::open(path, scan)?;
        if held.stamp.mode & 0o7777 != 0o600 {
            return Err(Error::Identity);
        }
        if held.parents.last().ok_or(Error::Identity)?.1.mode & 0o7777 != 0o700 {
            return Err(Error::Identity);
        }
        let (raw, digest) = held.full_read_and_hash(maximum, scan)?;
        let data = job::parse(&raw, usize::try_from(maximum).map_err(|_| Error::Capacity)?)?;
        Ok(Self {
            held,
            raw,
            digest,
            data,
        })
    }
    pub(super) fn revalidate(&mut self, scan: &SharedJournalBudget) -> Result<()> {
        let (raw, digest) = self.held.full_read_and_hash(self.raw.len() as u64, scan)?;
        if raw != self.raw || digest != self.digest {
            return Err(Error::Identity);
        }
        Ok(())
    }
}

pub(super) fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub(super) fn require_cloexec(fd: RawFd) -> Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 || flags & libc::FD_CLOEXEC == 0 {
        return Err(Error::Identity);
    }
    Ok(())
}
pub(super) fn mark_cloexec_nonblocking(fd: RawFd) -> Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(Error::Identity);
    }
    let status = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if status < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, status | libc::O_NONBLOCK) } < 0 {
        return Err(Error::Identity);
    }
    require_cloexec(fd)
}
pub(super) fn poll_fd(fd: RawFd, events: i16, deadline: Instant) -> Result<()> {
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or(Error::Deadline)?;
        let milliseconds = remaining
            .as_millis()
            .saturating_add(1)
            .min(i32::MAX as u128) as i32;
        let mut p = libc::pollfd {
            fd,
            events,
            revents: 0,
        };
        let code = unsafe { libc::poll(&mut p, 1, milliseconds) };
        if code < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        if code < 0 {
            return Err(Error::Io);
        }
        if code == 0 {
            return Err(Error::Deadline);
        }
        if p.revents & libc::POLLNVAL != 0 {
            return Err(Error::Identity);
        }
        if p.revents & events != 0 {
            return Ok(());
        }
        if p.revents & (libc::POLLERR | libc::POLLHUP) != 0 {
            return Err(Error::Eof);
        }
        return Err(Error::Io);
    }
}
pub(super) struct ParentPid {
    fd: OwnedFd,
    pid: u32,
    start: u64,
}
impl ParentPid {
    pub(super) fn open(pid: u32, start: u64, scan: &SharedJournalBudget) -> Result<Self> {
        if pid == 0 || pid > i32::MAX as u32 || start == 0 {
            return Err(Error::Shape);
        }
        scan.charge(0, 1).map_err(|_| Error::Capacity)?;
        let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        if raw < 0 {
            return Err(Error::Identity);
        }
        let fd = unsafe { OwnedFd::from_raw_fd(i32::try_from(raw).map_err(|_| Error::Io)?) };
        let result = Self { fd, pid, start };
        result.revalidate(scan)?;
        Ok(result)
    }
    pub(super) fn revalidate(&self, scan: &SharedJournalBudget) -> Result<()> {
        scan.charge(0, 2).map_err(|_| Error::Capacity)?;
        require_cloexec(self.fd.as_raw_fd())?;
        let mut p = libc::pollfd {
            fd: self.fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut p, 1, 0) } != 0 || actual_start(self.pid, scan)? != self.start {
            return Err(Error::Identity);
        }
        Ok(())
    }
}
pub(super) fn parse_start(stat: &[u8], pid: u32) -> Result<u64> {
    let text = std::str::from_utf8(stat).map_err(|_| Error::Shape)?;
    let (head, tail) = text.rsplit_once(')').ok_or(Error::Shape)?;
    if !head.starts_with(&format!("{pid} (")) {
        return Err(Error::Identity);
    }
    let value = tail.split_ascii_whitespace().nth(19).ok_or(Error::Shape)?;
    let ticks = value.parse::<u64>().map_err(|_| Error::Shape)?;
    if ticks == 0 || ticks.to_string() != value {
        return Err(Error::Shape);
    }
    Ok(ticks)
}
pub(super) fn actual_start(pid: u32, scan: &SharedJournalBudget) -> Result<u64> {
    let bytes = ProcView::open(scan)?.read_relative(&format!("{pid}/stat"), 4096, scan)?;
    parse_start(&bytes, pid)
}
pub(super) fn source_socket_facts(fd: RawFd) -> Result<(u32, u64)> {
    require_cloexec(fd)?;
    let mut domain = 0_i32;
    let mut domain_size = std::mem::size_of_val(&domain) as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_DOMAIN,
            (&mut domain as *mut i32).cast(),
            &mut domain_size,
        )
    } != 0
        || domain != libc::AF_UNIX
        || domain_size as usize != std::mem::size_of_val(&domain)
    {
        return Err(Error::Identity);
    }
    let mut peer = libc::ucred {
        pid: 0,
        uid: u32::MAX,
        gid: u32::MAX,
    };
    let mut size = std::mem::size_of_val(&peer) as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut peer as *mut libc::ucred).cast(),
            &mut size,
        )
    } != 0
        || size as usize != std::mem::size_of_val(&peer)
        || peer.uid != 0
        || peer.gid != 0
        || peer.pid <= 0
    {
        return Err(Error::Identity);
    }
    let mut cookie = 0_u64;
    let mut size = std::mem::size_of_val(&cookie) as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_COOKIE,
            (&mut cookie as *mut u64).cast(),
            &mut size,
        )
    } != 0
        || size as usize != std::mem::size_of_val(&cookie)
        || cookie == 0
    {
        return Err(Error::Identity);
    }
    let mut kind = 0_i32;
    let mut size = std::mem::size_of_val(&kind) as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_TYPE,
            (&mut kind as *mut i32).cast(),
            &mut size,
        )
    } != 0
        || size as usize != std::mem::size_of_val(&kind)
        || kind != libc::SOCK_STREAM
    {
        return Err(Error::Identity);
    }
    Ok((peer.pid as u32, cookie))
}

// Conservative local-view rejection only. No initial-userns or C-installation
// proof is inferred from maps, PID1, filesystem magic, namespace inode or EPERM.
// C must establish the real host anchor and original M4 task/proc closure.
const PROC_MAGIC: libc::c_long = 0x9fa0;
const NSFS_MAGIC: libc::c_long = 0x6e736673;
const NS_GET_NSTYPE: libc::c_ulong = 0xb703;
const RESOLVE_NO_XDEV: u64 = 0x01;
const RESOLVE_NO_SYMLINKS: u64 = 0x04;
const RESOLVE_BENEATH: u64 = 0x08;
#[repr(C)]
struct OpenHow {
    flags: u64,
    mode: u64,
    resolve: u64,
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct NativeObject {
    dev: u64,
    ino: u64,
    kind: u32,
}
fn actual_object(file: &File, scan: &SharedJournalBudget) -> Result<NativeObject> {
    scan.charge(0, 1).map_err(|_| Error::Capacity)?;
    let m = file.metadata().map_err(|_| Error::Io)?;
    Ok(NativeObject {
        dev: m.dev(),
        ino: m.ino(),
        kind: m.mode() & libc::S_IFMT,
    })
}
fn require_filesystem(file: &File, kind: libc::c_long, scan: &SharedJournalBudget) -> Result<()> {
    scan.charge(0, 1).map_err(|_| Error::Capacity)?;
    let mut fs = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::fstatfs(file.as_raw_fd(), fs.as_mut_ptr()) } != 0 {
        return Err(Error::Identity);
    }
    if unsafe { fs.assume_init() }.f_type != kind {
        return Err(Error::Identity);
    }
    Ok(())
}
fn mount_id(file: &File, scan: &SharedJournalBudget) -> Result<u64> {
    scan.charge(0, 1).map_err(|_| Error::Capacity)?;
    let mut st = std::mem::MaybeUninit::<libc::statx>::zeroed();
    if unsafe {
        libc::statx(
            file.as_raw_fd(),
            c"".as_ptr(),
            libc::AT_EMPTY_PATH | libc::AT_SYMLINK_NOFOLLOW,
            libc::STATX_MNT_ID,
            st.as_mut_ptr(),
        )
    } != 0
    {
        return Err(Error::Identity);
    }
    let st = unsafe { st.assume_init() };
    if st.stx_mask & libc::STATX_MNT_ID == 0 || st.stx_mnt_id == 0 {
        return Err(Error::Identity);
    }
    Ok(st.stx_mnt_id)
}
pub(super) fn require_same_current_pid(original: u32, scan: &SharedJournalBudget) -> Result<()> {
    scan.charge(0, 1).map_err(|_| Error::Capacity)?;
    let current = unsafe { libc::getpid() };
    if original == 0 || current <= 0 || current as u32 != original {
        return Err(Error::Identity);
    }
    Ok(())
}
/// DATA parser only. Linux pads these decimal columns with spaces.
pub(super) fn supported_identity_mapping(raw: &[u8]) -> Result<()> {
    if raw.is_empty() || raw.len() > 256 || !raw.is_ascii() {
        return Err(Error::Identity);
    }
    let text = std::str::from_utf8(raw).map_err(|_| Error::Identity)?;
    let text = text.strip_suffix('\n').unwrap_or(text);
    if text.contains(['\n', '\r'])
        || text
            .bytes()
            .any(|b| b != b' ' && b != b'\t' && !b.is_ascii_digit())
    {
        return Err(Error::Identity);
    }
    let fields: Vec<_> = text.split_ascii_whitespace().collect();
    if fields.len() != 3 {
        return Err(Error::Identity);
    }
    let mut values = [0_u64; 3];
    for (index, value) in fields.iter().enumerate() {
        values[index] = value.parse().map_err(|_| Error::Identity)?;
        if values[index].to_string() != *value {
            return Err(Error::Identity);
        }
    }
    if values != [0, 0, u64::from(u32::MAX)] {
        return Err(Error::Identity);
    }
    Ok(())
}
pub(super) fn proc_root_mount_descriptor(raw: &[u8], wanted: u64, dev: u64) -> Result<String> {
    if raw.is_empty() || !raw.is_ascii() {
        return Err(Error::Identity);
    }
    let text = std::str::from_utf8(raw).map_err(|_| Error::Identity)?;
    let mut found = None;
    for line in text.lines() {
        let (left, right) = line.split_once(" - ").ok_or(Error::Identity)?;
        let a: Vec<_> = left.split_ascii_whitespace().collect();
        let b: Vec<_> = right.split_ascii_whitespace().collect();
        if a.len() < 6 || b.len() != 3 {
            return Err(Error::Identity);
        }
        let id = a[0].parse::<u64>().map_err(|_| Error::Identity)?;
        if a[4] == "/proc" || id == wanted {
            let (major, minor) = a[2].split_once(':').ok_or(Error::Identity)?;
            let major = major.parse::<u32>().map_err(|_| Error::Identity)?;
            let minor = minor.parse::<u32>().map_err(|_| Error::Identity)?;
            if found.is_some()
                || id != wanted
                || a[3] != "/"
                || a[4] != "/proc"
                || b[0] != "proc"
                || major != libc::major(dev)
                || minor != libc::minor(dev)
            {
                return Err(Error::Identity);
            }
            found = Some(line.to_owned());
        }
    }
    found.ok_or(Error::Identity)
}
/// A held, genuine proc mount with kernel-generated `self` pointing to this
/// actual current PID. Numeric task/leaf opens refuse every child mount/bind.
/// This does not identify the global initial user namespace or its custodian.
pub(super) struct ProcView {
    root: File,
    object: NativeObject,
    mount: u64,
    own_pid: u32,
    descriptor: String,
}
impl ProcView {
    pub(super) fn open(scan: &SharedJournalBudget) -> Result<Self> {
        scan.charge(5, 1).map_err(|_| Error::Capacity)?;
        let raw = unsafe {
            libc::open(
                c"/proc".as_ptr(),
                libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if raw < 0 {
            return Err(Error::Identity);
        }
        let root = unsafe { File::from_raw_fd(raw) };
        require_filesystem(&root, PROC_MAGIC, scan)?;
        let object = actual_object(&root, scan)?;
        if object.kind != libc::S_IFDIR {
            return Err(Error::Identity);
        }
        let mount = mount_id(&root, scan)?;
        scan.charge(0, 1).map_err(|_| Error::Capacity)?;
        let own_pid = u32::try_from(unsafe { libc::getpid() }).map_err(|_| Error::Identity)?;
        if own_pid == 0 {
            return Err(Error::Identity);
        }
        let mut view = Self {
            root,
            object,
            mount,
            own_pid,
            descriptor: String::new(),
        };
        if view.read_link_relative("self", scan)? != own_pid.to_string() {
            return Err(Error::Identity);
        }
        view.descriptor = proc_root_mount_descriptor(
            &view.read_relative(&format!("{own_pid}/mountinfo"), 1024 * 1024, scan)?,
            mount,
            object.dev,
        )?;
        view.revalidate_root(scan)?;
        Ok(view)
    }
    fn open_relative(
        &self,
        relative: &str,
        flags: i32,
        scan: &SharedJournalBudget,
    ) -> Result<File> {
        if relative.is_empty()
            || relative.starts_with('/')
            || relative
                .split('/')
                .any(|p| p.is_empty() || p == "." || p == "..")
        {
            return Err(Error::Identity);
        }
        let path = CString::new(relative).map_err(|_| Error::Identity)?;
        let how = OpenHow {
            flags: (flags | libc::O_CLOEXEC) as u64,
            mode: 0,
            resolve: RESOLVE_BENEATH | RESOLVE_NO_XDEV | RESOLVE_NO_SYMLINKS,
        };
        scan.charge(path.as_bytes().len() as u64, 1)
            .map_err(|_| Error::Capacity)?;
        let raw = unsafe {
            libc::syscall(
                libc::SYS_openat2,
                self.root.as_raw_fd(),
                path.as_ptr(),
                &how,
                std::mem::size_of::<OpenHow>(),
            )
        };
        if raw < 0 {
            return Err(Error::Identity);
        }
        let file = unsafe { File::from_raw_fd(i32::try_from(raw).map_err(|_| Error::Identity)?) };
        require_filesystem(&file, PROC_MAGIC, scan)?;
        if mount_id(&file, scan)? != self.mount {
            return Err(Error::Identity);
        }
        Ok(file)
    }
    pub(super) fn read_relative(
        &self,
        relative: &str,
        maximum: usize,
        scan: &SharedJournalBudget,
    ) -> Result<Vec<u8>> {
        let mut file = self.open_relative(relative, libc::O_RDONLY | libc::O_NOFOLLOW, scan)?;
        if actual_object(&file, scan)?.kind != libc::S_IFREG {
            return Err(Error::Identity);
        }
        let cap = maximum.checked_add(1).ok_or(Error::Capacity)?;
        scan.charge(cap as u64, 1).map_err(|_| Error::Capacity)?;
        let mut raw = Vec::new();
        file.by_ref()
            .take(cap as u64)
            .read_to_end(&mut raw)
            .map_err(|_| Error::Io)?;
        if raw.len() > maximum {
            return Err(Error::Capacity);
        }
        Ok(raw)
    }
    fn link(&self, relative: &str, scan: &SharedJournalBudget) -> Result<File> {
        let link = self.open_relative(relative, libc::O_PATH | libc::O_NOFOLLOW, scan)?;
        if actual_object(&link, scan)?.kind != libc::S_IFLNK {
            return Err(Error::Identity);
        }
        Ok(link)
    }
    fn link_value(link: &File, scan: &SharedJournalBudget) -> Result<String> {
        scan.charge(4097, 1).map_err(|_| Error::Capacity)?;
        let mut raw = [0_u8; 4097];
        let len = unsafe {
            libc::readlinkat(
                link.as_raw_fd(),
                c"".as_ptr(),
                raw.as_mut_ptr().cast(),
                raw.len(),
            )
        };
        if len <= 0 || len as usize >= raw.len() {
            return Err(Error::Identity);
        }
        String::from_utf8(raw[..len as usize].to_vec()).map_err(|_| Error::Identity)
    }
    fn read_link_relative(&self, relative: &str, scan: &SharedJournalBudget) -> Result<String> {
        Self::link_value(&self.link(relative, scan)?, scan)
    }
    fn relative_path(&self, path: &str, scan: &SharedJournalBudget) -> Result<String> {
        let path = path.strip_prefix("/proc/").ok_or(Error::Identity)?;
        if let Some(tail) = path.strip_prefix("self/") {
            require_same_current_pid(self.own_pid, scan)?;
            Ok(format!("{}/{tail}", self.own_pid))
        } else {
            Ok(path.to_owned())
        }
    }
    fn require_single_task(&self, scan: &SharedJournalBudget) -> Result<()> {
        let directory = self.open_relative(
            &format!("{}/task", self.own_pid),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW,
            scan,
        )?;
        let mut count = 0_usize;
        let mut buffer = [0_u8; 4096];
        loop {
            scan.charge(buffer.len() as u64, 1)
                .map_err(|_| Error::Capacity)?;
            let size = unsafe {
                libc::syscall(
                    libc::SYS_getdents64,
                    directory.as_raw_fd(),
                    buffer.as_mut_ptr(),
                    buffer.len(),
                )
            };
            if size < 0 {
                return Err(Error::Identity);
            }
            if size == 0 {
                break;
            }
            let size = usize::try_from(size).map_err(|_| Error::Identity)?;
            if size > buffer.len() {
                return Err(Error::Identity);
            }
            let mut offset = 0;
            while offset < size {
                if size - offset < 20 {
                    return Err(Error::Identity);
                }
                let length = usize::from(u16::from_ne_bytes([
                    buffer[offset + 16],
                    buffer[offset + 17],
                ]));
                if length < 20 || length > size - offset {
                    return Err(Error::Identity);
                }
                let raw = &buffer[offset + 19..offset + length];
                let end = raw
                    .iter()
                    .position(|byte| *byte == 0)
                    .ok_or(Error::Identity)?;
                let name = std::str::from_utf8(&raw[..end]).map_err(|_| Error::Identity)?;
                scan.charge(name.len() as u64, 1)
                    .map_err(|_| Error::Capacity)?;
                if name != "." && name != ".." {
                    if name != self.own_pid.to_string() || count != 0 {
                        return Err(Error::Identity);
                    }
                    count += 1;
                }
                offset += length;
            }
        }
        if count != 1 {
            return Err(Error::Identity);
        }
        self.revalidate_root(scan)
    }
    fn open_checked_magic(&self, relative: &str, scan: &SharedJournalBudget) -> Result<File> {
        let (parent, name) = relative.rsplit_once('/').ok_or(Error::Identity)?;
        let parent = self.open_relative(
            parent,
            libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW,
            scan,
        )?;
        let before = self.link(relative, scan)?;
        let identity = actual_object(&before, scan)?;
        let name = CString::new(name).map_err(|_| Error::Identity)?;
        scan.charge(name.as_bytes().len() as u64, 1)
            .map_err(|_| Error::Capacity)?;
        // Only this final, previously held genuine proc magic link is followed.
        // The parent was opened NO_XDEV; the link and mount are checked again.
        let raw = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC,
            )
        };
        if raw < 0 {
            return Err(Error::Identity);
        }
        let target = unsafe { File::from_raw_fd(raw) };
        if actual_object(&self.link(relative, scan)?, scan)? != identity
            || Self::link_value(&before, scan)? != self.read_link_relative(relative, scan)?
        {
            return Err(Error::Identity);
        }
        self.revalidate_root(scan)?;
        Ok(target)
    }
    fn revalidate_root(&self, scan: &SharedJournalBudget) -> Result<()> {
        require_same_current_pid(self.own_pid, scan)?;
        scan.charge(5, 1).map_err(|_| Error::Capacity)?;
        let raw = unsafe {
            libc::open(
                c"/proc".as_ptr(),
                libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if raw < 0 {
            return Err(Error::Identity);
        }
        let named = unsafe { File::from_raw_fd(raw) };
        require_filesystem(&named, PROC_MAGIC, scan)?;
        require_filesystem(&self.root, PROC_MAGIC, scan)?;
        if actual_object(&named, scan)? != self.object
            || actual_object(&self.root, scan)? != self.object
            || mount_id(&named, scan)? != self.mount
            || mount_id(&self.root, scan)? != self.mount
            || self.read_link_relative("self", scan)? != self.own_pid.to_string()
        {
            return Err(Error::Identity);
        }
        let descriptor = proc_root_mount_descriptor(
            &self.read_relative(&format!("{}/mountinfo", self.own_pid), 1024 * 1024, scan)?,
            self.mount,
            self.object.dev,
        )?;
        if descriptor != self.descriptor {
            return Err(Error::Identity);
        }
        Ok(())
    }
    pub(super) fn user_namespace(
        &self,
        pid: u32,
        scan: &SharedJournalBudget,
    ) -> Result<NamespaceObject> {
        if pid == 0 {
            return Err(Error::Identity);
        }
        let relative = format!("{pid}/ns/user");
        let link = self.link(&relative, scan)?;
        let target = Self::link_value(&link, scan)?;
        let value = target
            .strip_prefix("user:[")
            .and_then(|s| s.strip_suffix(']'))
            .ok_or(Error::Identity)?;
        let expected = value.parse::<u64>().map_err(|_| Error::Identity)?;
        if expected == 0 || expected.to_string() != value {
            return Err(Error::Identity);
        }
        let file = self.open_checked_magic(&relative, scan)?;
        require_user_namespace_fd(&file, scan)?;
        let object = actual_object(&file, scan)?;
        if object.ino != expected {
            return Err(Error::Identity);
        }
        Ok(NamespaceObject { file, object })
    }
}
pub(super) fn require_user_namespace_fd(file: &File, scan: &SharedJournalBudget) -> Result<()> {
    require_filesystem(file, NSFS_MAGIC, scan)?;
    scan.charge(0, 2).map_err(|_| Error::Capacity)?;
    require_cloexec(file.as_raw_fd())?;
    if unsafe { libc::ioctl(file.as_raw_fd(), NS_GET_NSTYPE) } != libc::CLONE_NEWUSER {
        return Err(Error::Identity);
    }
    Ok(())
}
pub(super) struct NamespaceObject {
    file: File,
    object: NativeObject,
}
impl NamespaceObject {
    fn revalidate(&self, scan: &SharedJournalBudget) -> Result<()> {
        require_user_namespace_fd(&self.file, scan)?;
        if actual_object(&self.file, scan)? != self.object {
            return Err(Error::Identity);
        }
        Ok(())
    }
    pub(super) fn same_object(&self, other: &Self) -> bool {
        self.object == other.object
    }
}
// Fixed boot-ID compatibility only. This DATA parser does not attest a native
// view; ProcSysView independently opens and joins every returned mount object.
struct NativeSysMount {
    mount: u64,
    descriptor: String,
}
enum ProcSysShapeKind {
    Direct,
    NativeReadOnly(NativeSysMount),
}
pub(super) struct ProcSysShape {
    kind: ProcSysShapeKind,
}
impl ProcSysShape {
    pub(super) fn direct_data(&self) -> bool {
        matches!(&self.kind, ProcSysShapeKind::Direct)
    }
    pub(super) fn native_ro_mount_data(&self) -> Option<u64> {
        match &self.kind {
            ProcSysShapeKind::Direct => None,
            ProcSysShapeKind::NativeReadOnly(row) => Some(row.mount),
        }
    }
}
pub(super) fn parse_proc_sys_shape(
    raw: &[u8],
    proc_mount: u64,
    proc_dev: u64,
) -> Result<ProcSysShape> {
    if raw.is_empty() || raw.len() > 1024 * 1024 || !raw.is_ascii() || proc_mount == 0 {
        return Err(Error::Identity);
    }
    let text = std::str::from_utf8(raw).map_err(|_| Error::Identity)?;
    if !text.ends_with('\n') || text.contains('\r') {
        return Err(Error::Identity);
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut selected = None;
    let mut proc_seen = 0;
    for line in text.lines() {
        let (left, right) = line.split_once(" - ").ok_or(Error::Identity)?;
        let a: Vec<_> = left.split_ascii_whitespace().collect();
        let b: Vec<_> = right.split_ascii_whitespace().collect();
        if a.len() < 6 || b.len() != 3 {
            return Err(Error::Identity);
        }
        let decimal = |s: &str| -> Result<u64> {
            let value = s.parse::<u64>().map_err(|_| Error::Identity)?;
            if value == 0 || value.to_string() != s {
                return Err(Error::Identity);
            }
            Ok(value)
        };
        let id = decimal(a[0])?;
        let parent = decimal(a[1])?;
        if !ids.insert(id) {
            return Err(Error::Identity);
        }
        let target = a[4];
        if target == "/proc" {
            proc_seen += 1;
            let (major, minor) = a[2].split_once(':').ok_or(Error::Identity)?;
            let major = major.parse::<u32>().map_err(|_| Error::Identity)?;
            let minor = minor.parse::<u32>().map_err(|_| Error::Identity)?;
            if id != proc_mount
                || a[3] != "/"
                || b[0] != "proc"
                || major != libc::major(proc_dev)
                || minor != libc::minor(proc_dev)
            {
                return Err(Error::Identity);
            }
        }
        if target.starts_with("/proc/") && target.contains('\\') {
            return Err(Error::Identity);
        }
        if target.starts_with("/proc/sys/") {
            return Err(Error::Identity);
        }
        if target == "/proc/sys" {
            let (major, minor) = a[2].split_once(':').ok_or(Error::Identity)?;
            let major = major.parse::<u32>().map_err(|_| Error::Identity)?;
            let minor = minor.parse::<u32>().map_err(|_| Error::Identity)?;
            let options: Vec<_> = a[5].split(',').collect();
            let flags_unique = options
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == options.len();
            if selected.is_some()
                || id == proc_mount
                || parent != proc_mount
                || a[3] != "/sys"
                || b[0] != "proc"
                || major != libc::major(proc_dev)
                || minor != libc::minor(proc_dev)
                || !flags_unique
                || options.contains(&"rw")
                || !["ro", "nosuid", "nodev", "noexec"]
                    .iter()
                    .all(|v| options.contains(v))
            {
                return Err(Error::Identity);
            }
            // Superblock options b[2] may be rw for a native RO bind; only the
            // actual selected mount's per-mount options determine this gate.
            selected = Some(NativeSysMount {
                mount: id,
                descriptor: line.to_owned(),
            });
        }
    }
    if proc_seen != 1 {
        return Err(Error::Identity);
    }
    Ok(ProcSysShape {
        kind: selected.map_or(ProcSysShapeKind::Direct, ProcSysShapeKind::NativeReadOnly),
    })
}
enum ProcSysKind {
    Direct,
    NativeReadOnly {
        root: File,
        object: NativeObject,
        row: NativeSysMount,
    },
}
struct ProcSysView {
    kind: ProcSysKind,
}
impl ProcSysView {
    fn mountinfo(proc: &ProcView, scan: &SharedJournalBudget) -> Result<Vec<u8>> {
        proc.read_relative(&format!("{}/mountinfo", proc.own_pid), 1024 * 1024, scan)
    }
    fn open_sys_root(proc: &ProcView, scan: &SharedJournalBudget) -> Result<File> {
        let how = OpenHow {
            flags: (libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) as u64,
            mode: 0,
            resolve: RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS,
        };
        scan.charge(3, 1).map_err(|_| Error::Capacity)?;
        // The one intentionally permitted crossing is the literal component
        // sys. No generic proc opener loses NO_XDEV, and no caller path exists.
        let raw = unsafe {
            libc::syscall(
                libc::SYS_openat2,
                proc.root.as_raw_fd(),
                c"sys".as_ptr(),
                &how,
                std::mem::size_of::<OpenHow>(),
            )
        };
        if raw < 0 {
            return Err(Error::Identity);
        }
        let root = unsafe { File::from_raw_fd(i32::try_from(raw).map_err(|_| Error::Identity)?) };
        require_filesystem(&root, PROC_MAGIC, scan)?;
        let object = actual_object(&root, scan)?;
        if object.kind != libc::S_IFDIR || object.dev != proc.object.dev {
            return Err(Error::Identity);
        }
        Ok(root)
    }
    fn require_ro(root: &File, scan: &SharedJournalBudget) -> Result<()> {
        scan.charge(0, 1).map_err(|_| Error::Capacity)?;
        let mut st = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
        if unsafe { libc::fstatvfs(root.as_raw_fd(), st.as_mut_ptr()) } != 0 {
            return Err(Error::Identity);
        }
        let st = unsafe { st.assume_init() };
        if st.f_flag & libc::ST_RDONLY == 0 || st.f_flag & libc::ST_NOSUID == 0 {
            return Err(Error::Identity);
        }
        Ok(())
    }
    fn observe(proc: &ProcView, scan: &SharedJournalBudget) -> Result<Self> {
        proc.revalidate_root(scan)?;
        let shape =
            parse_proc_sys_shape(&Self::mountinfo(proc, scan)?, proc.mount, proc.object.dev)?;
        let kind = match shape.kind {
            ProcSysShapeKind::Direct => ProcSysKind::Direct,
            ProcSysShapeKind::NativeReadOnly(row) => {
                let root = Self::open_sys_root(proc, scan)?;
                let object = actual_object(&root, scan)?;
                if mount_id(&root, scan)? != row.mount {
                    return Err(Error::Identity);
                }
                Self::require_ro(&root, scan)?;
                ProcSysKind::NativeReadOnly { root, object, row }
            }
        };
        let view = Self { kind };
        view.revalidate(proc, scan)?;
        Ok(view)
    }
    fn revalidate(&self, proc: &ProcView, scan: &SharedJournalBudget) -> Result<()> {
        proc.revalidate_root(scan)?;
        let now = parse_proc_sys_shape(&Self::mountinfo(proc, scan)?, proc.mount, proc.object.dev)?;
        match (&self.kind, now.kind) {
            (ProcSysKind::Direct, ProcSysShapeKind::Direct) => {}
            (
                ProcSysKind::NativeReadOnly { root, object, row },
                ProcSysShapeKind::NativeReadOnly(now),
            ) => {
                if now.mount != row.mount || now.descriptor != row.descriptor {
                    return Err(Error::Identity);
                }
                let named = Self::open_sys_root(proc, scan)?;
                require_filesystem(root, PROC_MAGIC, scan)?;
                if actual_object(root, scan)? != *object
                    || actual_object(&named, scan)? != *object
                    || mount_id(root, scan)? != row.mount
                    || mount_id(&named, scan)? != row.mount
                {
                    return Err(Error::Identity);
                }
                Self::require_ro(root, scan)?;
                Self::require_ro(&named, scan)?;
            }
            _ => return Err(Error::Identity),
        }
        proc.revalidate_root(scan)
    }
    fn boot_id(&self, proc: &ProcView, scan: &SharedJournalBudget) -> Result<String> {
        self.revalidate(proc, scan)?;
        let raw = match &self.kind {
            ProcSysKind::Direct => proc.read_relative("sys/kernel/random/boot_id", 37, scan)?,
            ProcSysKind::NativeReadOnly { root, object, row } => {
                let how = OpenHow {
                    flags: (libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC) as u64,
                    mode: 0,
                    resolve: RESOLVE_BENEATH | RESOLVE_NO_XDEV | RESOLVE_NO_SYMLINKS,
                };
                scan.charge(21, 1).map_err(|_| Error::Capacity)?;
                let raw = unsafe {
                    libc::syscall(
                        libc::SYS_openat2,
                        root.as_raw_fd(),
                        c"kernel/random/boot_id".as_ptr(),
                        &how,
                        std::mem::size_of::<OpenHow>(),
                    )
                };
                if raw < 0 {
                    return Err(Error::Identity);
                }
                let mut leaf =
                    unsafe { File::from_raw_fd(i32::try_from(raw).map_err(|_| Error::Identity)?) };
                require_filesystem(&leaf, PROC_MAGIC, scan)?;
                let before = actual_object(&leaf, scan)?;
                if before.kind != libc::S_IFREG
                    || before.dev != object.dev
                    || mount_id(&leaf, scan)? != row.mount
                {
                    return Err(Error::Identity);
                }
                scan.charge(38, 1).map_err(|_| Error::Capacity)?;
                let mut raw = Vec::new();
                leaf.by_ref()
                    .take(38)
                    .read_to_end(&mut raw)
                    .map_err(|_| Error::Io)?;
                if raw.len() > 37
                    || actual_object(&leaf, scan)? != before
                    || mount_id(&leaf, scan)? != row.mount
                {
                    return Err(Error::Identity);
                }
                self.revalidate(proc, scan)?;
                raw
            }
        };
        self.revalidate(proc, scan)?;
        let value = std::str::from_utf8(&raw)
            .map_err(|_| Error::Identity)?
            .strip_suffix('\n')
            .ok_or(Error::Identity)?;
        if !job::uuid(value, false) {
            return Err(Error::Identity);
        }
        Ok(value.to_owned())
    }
}
struct TaskNamespace {
    object: NamespaceObject,
    pid: u32,
    start: u64,
    uid_map: Vec<u8>,
    gid_map: Vec<u8>,
}
impl TaskNamespace {
    fn observe(proc: &ProcView, pid: u32, scan: &SharedJournalBudget) -> Result<Self> {
        let start = parse_start(
            &proc.read_relative(&format!("{pid}/stat"), 4096, scan)?,
            pid,
        )?;
        let object = proc.user_namespace(pid, scan)?;
        let uid_map = proc.read_relative(&format!("{pid}/uid_map"), 256, scan)?;
        let gid_map = proc.read_relative(&format!("{pid}/gid_map"), 256, scan)?;
        supported_identity_mapping(&uid_map)?;
        supported_identity_mapping(&gid_map)?;
        let value = Self {
            object,
            pid,
            start,
            uid_map,
            gid_map,
        };
        value.revalidate(proc, scan)?;
        Ok(value)
    }
    fn revalidate(&self, proc: &ProcView, scan: &SharedJournalBudget) -> Result<()> {
        self.object.revalidate(scan)?;
        if parse_start(
            &proc.read_relative(&format!("{}/stat", self.pid), 4096, scan)?,
            self.pid,
        )? != self.start
        {
            return Err(Error::Identity);
        }
        let before = proc.user_namespace(self.pid, scan)?;
        if !before.same_object(&self.object) {
            return Err(Error::Identity);
        }
        for (leaf, baseline) in [("uid_map", &self.uid_map), ("gid_map", &self.gid_map)] {
            let raw = proc.read_relative(&format!("{}/{leaf}", self.pid), 256, scan)?;
            supported_identity_mapping(&raw)?;
            if raw != *baseline {
                return Err(Error::Identity);
            }
        }
        let after = proc.user_namespace(self.pid, scan)?;
        if !after.same_object(&self.object) || !before.same_object(&after) {
            return Err(Error::Identity);
        }
        Ok(())
    }
}
struct OwnNativeView {
    proc: ProcView,
    sys: ProcSysView,
    task: TaskNamespace,
}
impl OwnNativeView {
    fn observe(scan: &SharedJournalBudget) -> Result<Self> {
        let proc = ProcView::open(scan)?;
        let sys = ProcSysView::observe(&proc, scan)?;
        let task = TaskNamespace::observe(&proc, proc.own_pid, scan)?;
        let view = Self { proc, sys, task };
        view.revalidate(scan)?;
        Ok(view)
    }
    fn revalidate(&self, scan: &SharedJournalBudget) -> Result<()> {
        require_same_current_pid(self.task.pid, scan)?;
        self.proc.revalidate_root(scan)?;
        self.task.revalidate(&self.proc, scan)?;
        self.sys.revalidate(&self.proc, scan)
    }
}
enum NativeRoleView {
    Source {
        own: OwnNativeView,
        parent: TaskNamespace,
    },
    Capture {
        own: OwnNativeView,
    },
}
/// Private process-only native guard; neither branch establishes initial-host
/// Root or the missing original M4 issuance predicate. No PG-proof Option/bool.
pub(super) struct SupportedNativeView {
    role: NativeRoleView,
}
impl SupportedNativeView {
    fn observe_source(parent_pid: u32, scan: &SharedJournalBudget) -> Result<Self> {
        let own = OwnNativeView::observe(scan)?;
        let parent = TaskNamespace::observe(&own.proc, parent_pid, scan)?;
        if !own.task.object.same_object(&parent.object) {
            return Err(Error::Identity);
        }
        let view = Self {
            role: NativeRoleView::Source { own, parent },
        };
        view.revalidate(scan)?;
        Ok(view)
    }
    pub(super) fn observe_capture(scan: &SharedJournalBudget) -> Result<Self> {
        // The compiled capture branch observes only its actual own userns.
        // PG PID1 native/nonce/record gates stay in CaptureNative. The original
        // M4 producer must later perform its own exact Root/PG/capture userns
        // join before issuance and critical rechecks; it is not implemented or
        // converted from an unknown to true here.
        let view = Self {
            role: NativeRoleView::Capture {
                own: OwnNativeView::observe(scan)?,
            },
        };
        view.revalidate(scan)?;
        Ok(view)
    }
    pub(super) fn revalidate(&self, scan: &SharedJournalBudget) -> Result<()> {
        match &self.role {
            NativeRoleView::Source { own, parent } => {
                own.revalidate(scan)?;
                parent.revalidate(&own.proc, scan)?;
                if !own.task.object.same_object(&parent.object) {
                    return Err(Error::Identity);
                }
                own.revalidate(scan)
            }
            NativeRoleView::Capture { own } => own.revalidate(scan),
        }
    }
    pub(super) fn boot_id(&self, scan: &SharedJournalBudget) -> Result<String> {
        let own = match &self.role {
            NativeRoleView::Source { own, .. } | NativeRoleView::Capture { own } => own,
        };
        own.revalidate(scan)?;
        let value = own.sys.boot_id(&own.proc, scan)?;
        own.revalidate(scan)?;
        Ok(value)
    }
}

const ENV_KEYS: [&[u8]; 7] = [
    b"PATH",
    b"HOME",
    b"LANG",
    b"PGHOST",
    b"PGPORT",
    b"PGUSER",
    b"PGDATABASE",
];
const CONSTRUCTOR_QUERIES: [&str; 12] = [
    "PGPORT",
    "PGHOSTADDR",
    "PGHOST",
    "PGUSER",
    "PGDATABASE",
    "PGPASSWORD",
    "PGSSLROOTCERT",
    "PGSSLCERT",
    "PGSSLKEY",
    "PGSSLMODE",
    "PGAPPNAME",
    "PGOPTIONS",
];
unsafe extern "C" {
    static mut environ: *mut *mut libc::c_char;
}
/// Caller must provide an actual C environment pointer array terminated at its
/// eighth pointer at latest. Unknown key rejection never dereferences its value.
pub(super) unsafe fn scan_environment(
    entries: *const *const libc::c_char,
    expected_host: &str,
    expected_db: &str,
    scan: &SharedJournalBudget,
) -> Result<()> {
    if entries.is_null() || !job::database(expected_db) {
        return Err(Error::Shape);
    }
    let expected = [
        "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
        "/root",
        "C",
        expected_host,
        "5432",
        "learning_admin",
        expected_db,
    ];
    let mut seen = [false; 7];
    for index in 0..=7 {
        scan.charge(std::mem::size_of::<*const libc::c_char>() as u64, 1)
            .map_err(|_| Error::Capacity)?;
        let entry = unsafe { *entries.add(index) };
        if entry.is_null() {
            return if index == 7 && seen.into_iter().all(|v| v) {
                Ok(())
            } else {
                Err(Error::Identity)
            };
        }
        if index == 7 {
            return Err(Error::Identity);
        }
        let mut key = [0_u8; 11];
        let mut length = None;
        for (offset, slot) in key.iter_mut().enumerate() {
            scan.charge(1, 0).map_err(|_| Error::Capacity)?;
            let byte = unsafe { *entry.add(offset) } as u8;
            if byte == b'=' {
                length = Some(offset);
                break;
            }
            if byte == 0 || offset == 10 {
                return Err(Error::Identity);
            }
            *slot = byte;
        }
        let length = length.ok_or(Error::Identity)?;
        let slot = ENV_KEYS
            .iter()
            .position(|allowed| *allowed == &key[..length])
            .ok_or(Error::Identity)?;
        if seen[slot] {
            return Err(Error::Identity);
        }
        seen[slot] = true;
        // Only after matching the allowlisted key, inspect the exact fixed value.
        let value = expected[slot].as_bytes();
        for offset in 0..=value.len() {
            scan.charge(1, 0).map_err(|_| Error::Capacity)?;
            let byte = unsafe { *entry.add(length + 1 + offset) } as u8;
            if byte != value.get(offset).copied().unwrap_or(0) {
                return Err(Error::Identity);
            }
        }
    }
    Err(Error::Identity)
}
pub(super) fn recheck_actual_environment(
    role: job::ProcessRole,
    database: &str,
    scan: &SharedJournalBudget,
) -> Result<()> {
    let host = match role {
        job::ProcessRole::Source => "/run/knowweave-c4-source-pg",
        job::ProcessRole::Capture => "/var/run/postgresql",
    };
    // Exact fixed ELF closure must independently prove no environment mutation.
    // No vars()/vars_os()/getenv or proc-environ bulk values are copied here.
    unsafe { scan_environment(environ.cast(), host, database, scan) }
}
pub(super) fn precharge_fixed_options_environment(
    role: job::ProcessRole,
    database: &str,
    scan: &SharedJournalBudget,
) -> Result<()> {
    recheck_actual_environment(role, database, scan)?;
    for key in CONSTRUCTOR_QUERIES {
        // Keys, absence lookups and the known nonsecret values are observations.
        let value = match key {
            "PGPORT" => 4,
            "PGHOST" => match role {
                job::ProcessRole::Source => "/run/knowweave-c4-source-pg".len(),
                job::ProcessRole::Capture => "/var/run/postgresql".len(),
            },
            "PGUSER" => "learning_admin".len(),
            "PGDATABASE" => database.len(),
            _ => 0,
        };
        scan.charge((key.len() + value) as u64, 1)
            .map_err(|_| Error::Capacity)?;
    }
    Ok(())
}

const OBSERVATION_ROOT: &str = "/var/lib/knowweave-c4/b1-source-observation";
const PROFILE: &str = "task9-b1-limited-v1";

struct CompilePins {
    role: job::ProcessRole,
    manager_commit: &'static str,
    manager_build: &'static str,
    application_commit: &'static str,
    application_build: &'static str,
    source_auth: &'static str,
    verifier: &'static str,
    source_binding: &'static str,
}
impl CompilePins {
    fn read() -> Result<Self> {
        let role = match option_env!("KNOWWEAVE_C4_B1_SOURCE_PROCESS_ROLE") {
            Some("publication-source") => job::ProcessRole::Source,
            Some("capture") => job::ProcessRole::Capture,
            // RootLauncher is never accepted by this DB process factory.
            _ => return Err(Error::Identity),
        };
        if option_env!("KNOWWEAVE_C4_B1_ROLE") != Some("source")
            || option_env!("KNOWWEAVE_C4_B1_PROFILE") != Some(PROFILE)
        {
            return Err(Error::Identity);
        }
        let pin = |value: Option<&'static str>, length: usize| {
            value.filter(|v| job::hex(v, length)).ok_or(Error::Identity)
        };
        Ok(Self {
            role,
            manager_commit: pin(option_env!("KNOWWEAVE_C4_B1_MANAGER_CODE_COMMIT"), 40)?,
            manager_build: pin(
                option_env!("KNOWWEAVE_C4_B1_MANAGER_BUILD_INPUT_SHA256"),
                64,
            )?,
            application_commit: pin(option_env!("KNOWWEAVE_SOURCE_COMMIT"), 40)?,
            application_build: pin(option_env!("KNOWWEAVE_BUILD_ID_SHA256"), 64)?,
            source_auth: pin(option_env!("KNOWWEAVE_C4_B1_SOURCE_AUTH_KEY_SHA256"), 64)?,
            verifier: pin(option_env!("KNOWWEAVE_C4_VERIFIER_KEY_SHA256"), 64)?,
            source_binding: pin(
                option_env!("KNOWWEAVE_C4_SOURCE_CONTROL_BINDING_SHA256"),
                64,
            )?,
        })
    }
    fn catalog_path(&self) -> &'static str {
        match self.role {
            job::ProcessRole::Source => "/var/lib/knowweave-c4/b1/source-peer-execution.json",
            job::ProcessRole::Capture => "/opt/knowweave-c4/b1-capture/source-peer-execution.json",
        }
    }
    fn manifest_path(&self) -> &'static str {
        match self.role {
            job::ProcessRole::Source => "/var/lib/knowweave-c4/b1/launch-manifest.json",
            job::ProcessRole::Capture => {
                "/opt/knowweave-c4/b1-capture/capture-launch-manifest.json"
            }
        }
    }
    fn row(&self) -> &'static str {
        match self.role {
            job::ProcessRole::Source => "PublicationSource",
            job::ProcessRole::Capture => "Capture",
        }
    }
    fn join(&self, catalog: &job::ExecutionCatalog, manifest: &job::LaunchManifest) -> Result<()> {
        let own_image = image(catalog, self.row())?;
        let launcher = image(catalog, "RootLauncher")?;
        let installed = &catalog.installed_source;
        if own_image.code_commit != self.manager_commit
            || own_image.build_input_sha256 != self.manager_build
            || installed.application_commit != self.application_commit
            || installed.application_build_sha256 != self.application_build
            || installed.source_auth_public_key_sha256 != self.source_auth
            || installed.source_control_binding_sha256 != self.source_binding
            || catalog.capture_recipe_sha256
                != catalog.installed_runtime.capture_mount_recipe_sha256
            || manifest.code_commit != self.manager_commit
            || manifest.build_input_sha256 != self.manager_build
            || manifest.binary_sha256 != own_image.executable.sha256
            || manifest.launcher_sha256 != launcher.executable.sha256
            || manifest.source_auth_public_key_sha256 != self.source_auth
            || manifest.destination_verifier_public_key_sha256 != self.verifier
            || manifest.launch_id != catalog.launch_id
        {
            return Err(Error::Identity);
        }
        Ok(())
    }
}
fn image<'a>(catalog: &'a job::ExecutionCatalog, role: &str) -> Result<&'a job::RoleImage> {
    let mut matches = catalog.images.iter().filter(|i| i.role == role);
    let row = matches.next().ok_or(Error::Identity)?;
    if matches.next().is_some() {
        return Err(Error::Identity);
    }
    Ok(row)
}
fn declared_process_image(record: &job::ProcessRef, image: &job::RoleImage) -> Result<()> {
    if record.exe_sha256 != image.executable.sha256
        || record.exe_dev != image.executable.stamp.dev
        || record.exe_ino != image.executable.stamp.ino
    {
        return Err(Error::Identity);
    }
    Ok(())
}

fn bounded_kernel_read(path: &str, maximum: usize, scan: &SharedJournalBudget) -> Result<Vec<u8>> {
    let view = ProcView::open(scan)?;
    view.read_relative(&view.relative_path(path, scan)?, maximum, scan)
}
fn kernel_link(path: &str, scan: &SharedJournalBudget) -> Result<String> {
    let view = ProcView::open(scan)?;
    view.read_link_relative(&view.relative_path(path, scan)?, scan)
}

fn actual_root_status(pid: u32, scan: &SharedJournalBudget) -> Result<()> {
    let bytes = bounded_kernel_read(&format!("/proc/{pid}/status"), 65536, scan)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| Error::Identity)?;
    let mut count = 0;
    for line in text
        .lines()
        .filter(|l| l.starts_with("Uid:") || l.starts_with("Gid:"))
    {
        let values: Vec<_> = line.split_ascii_whitespace().skip(1).collect();
        if values != ["0", "0", "0", "0"] {
            return Err(Error::Identity);
        }
        count += 1;
    }
    if count != 2 {
        return Err(Error::Identity);
    }
    Ok(())
}
fn require_single_thread(scan: &SharedJournalBudget) -> Result<()> {
    ProcView::open(scan)?.require_single_task(scan)
}

#[derive(PartialEq, Eq)]
pub(super) struct ExecutableMapping {
    line: String,
}
pub(super) fn parse_executable_mappings(
    raw: &[u8],
    files: &[(&str, Stamp)],
) -> Result<Vec<ExecutableMapping>> {
    let text = std::str::from_utf8(raw).map_err(|_| Error::Identity)?;
    let mut mappings = Vec::new();
    let mut vdso = false;
    let mut vsyscall = false;
    for line in text.lines() {
        let fields: Vec<_> = line.split_ascii_whitespace().collect();
        if fields.len() < 5 {
            return Err(Error::Identity);
        }
        if fields[1].len() != 4 {
            return Err(Error::Identity);
        }
        if fields[1].as_bytes()[2] != b'x' {
            continue;
        }
        if fields.len() != 6 || fields[1].as_bytes()[1] == b'w' || fields[1].as_bytes()[3] != b'p' {
            return Err(Error::Identity);
        }
        let (begin, end) = fields[0].split_once('-').ok_or(Error::Identity)?;
        let begin = u64::from_str_radix(begin, 16).map_err(|_| Error::Identity)?;
        let end = u64::from_str_radix(end, 16).map_err(|_| Error::Identity)?;
        if begin == 0 || end <= begin {
            return Err(Error::Identity);
        }
        match fields[5] {
            "[vdso]" => {
                if vdso {
                    return Err(Error::Identity);
                }
                vdso = true;
            }
            "[vsyscall]" => {
                if vsyscall {
                    return Err(Error::Identity);
                }
                vsyscall = true;
            }
            path => {
                let (_, stamp) = files
                    .iter()
                    .find(|(p, _)| *p == path)
                    .ok_or(Error::Identity)?;
                let (major, minor) = fields[3].split_once(':').ok_or(Error::Identity)?;
                let major = u32::from_str_radix(major, 16).map_err(|_| Error::Identity)?;
                let minor = u32::from_str_radix(minor, 16).map_err(|_| Error::Identity)?;
                let inode = fields[4].parse::<u64>().map_err(|_| Error::Identity)?;
                let offset = u64::from_str_radix(fields[2], 16).map_err(|_| Error::Identity)?;
                if major != libc::major(stamp.dev)
                    || minor != libc::minor(stamp.dev)
                    || inode != stamp.ino
                    || offset >= stamp.bytes
                {
                    return Err(Error::Identity);
                }
            }
        }
        mappings.push(ExecutableMapping {
            line: line.to_owned(),
        });
    }
    if mappings.is_empty() {
        return Err(Error::Identity);
    }
    Ok(mappings)
}
struct MeasuredImage {
    files: Vec<(String, HeldCode)>,
    mappings: Vec<ExecutableMapping>,
    pid: u32,
    start: u64,
    pid_ns: String,
    mnt_ns: String,
}
impl MeasuredImage {
    fn open(row: &job::RoleImage, pid: u32, scan: &SharedJournalBudget) -> Result<Self> {
        let mut files = Vec::new();
        files.push((
            row.executable.path.clone(),
            HeldCode::open(&row.executable, true, scan)?,
        ));
        for code in &row.libraries {
            files.push((code.path.clone(), HeldCode::open(code, false, scan)?));
        }
        files[0].1.matches_actual_exe(pid, scan)?;
        let start = actual_start(pid, scan)?;
        let pid_ns = kernel_link(&format!("/proc/{pid}/ns/pid"), scan)?;
        let mnt_ns = kernel_link(&format!("/proc/{pid}/ns/mnt"), scan)?;
        let roster: Vec<_> = files
            .iter()
            .map(|(p, c)| (p.as_str(), c.held.stamp))
            .collect();
        let mappings = parse_executable_mappings(
            &bounded_kernel_read(&format!("/proc/{pid}/maps"), 1024 * 1024, scan)?,
            &roster,
        )?;
        let value = Self {
            files,
            mappings,
            pid,
            start,
            pid_ns,
            mnt_ns,
        };
        value.revalidate(scan)?;
        Ok(value)
    }
    fn revalidate(&self, scan: &SharedJournalBudget) -> Result<()> {
        actual_root_status(self.pid, scan)?;
        if actual_start(self.pid, scan)? != self.start
            || kernel_link(&format!("/proc/{}/ns/pid", self.pid), scan)? != self.pid_ns
            || kernel_link(&format!("/proc/{}/ns/mnt", self.pid), scan)? != self.mnt_ns
        {
            return Err(Error::Identity);
        }
        for (_, code) in &self.files {
            code.revalidate(scan)?;
        }
        self.files[0].1.matches_actual_exe(self.pid, scan)?;
        let roster: Vec<_> = self
            .files
            .iter()
            .map(|(p, c)| (p.as_str(), c.held.stamp))
            .collect();
        if parse_executable_mappings(
            &bounded_kernel_read(&format!("/proc/{}/maps", self.pid), 1024 * 1024, scan)?,
            &roster,
        )? != self.mappings
        {
            return Err(Error::Identity);
        }
        Ok(())
    }
    fn join(&self, record: &job::ProcessRef, host: bool) -> Result<()> {
        let code = &self.files[0].1;
        if (host && record.host_pid != u64::from(self.pid))
            || record.namespace_pid != u64::from(self.pid)
            || record.start_ticks != self.start
            || record.pid_namespace != self.pid_ns
            || record.mount_namespace != self.mnt_ns
            || record.exe_dev != code.held.stamp.dev
            || record.exe_ino != code.held.stamp.ino
            || record.exe_sha256 != encode_hex(&code.digest)
        {
            return Err(Error::Identity);
        }
        Ok(())
    }
}

struct JobLock {
    held: HeldNamed,
}
impl JobLock {
    fn open(path: &str, dev: u64, ino: u64, scan: &SharedJournalBudget) -> Result<Self> {
        let held = HeldNamed::open(path, scan)?;
        let s = held.stamp;
        if s.dev != dev
            || s.ino != ino
            || s.uid != 0
            || s.links != 1
            || s.bytes != 0
            || s.mode & libc::S_IFMT != libc::S_IFREG
            || s.mode & 0o7777 != 0o600
            || held.parents.last().ok_or(Error::Identity)?.1.mode & 0o7777 != 0o700
        {
            return Err(Error::Identity);
        }
        // O/jobs/J are fixed private roots. System ancestors remain root-owned
        // and nonwritable, but legitimate immutable appends can change dir times.
        for (_, parent) in held.parents.iter().skip(4) {
            if parent.mode & 0o7777 != 0o700 {
                return Err(Error::Identity);
            }
        }
        let value = Self { held };
        value.revalidate(scan)?;
        Ok(value)
    }
    fn revalidate(&self, scan: &SharedJournalBudget) -> Result<()> {
        self.held.revalidate(scan)?;
        // Reserve probe and possible immediate unlock together. Cleanup after
        // observing an absent owner never needs a fresh budget reservation.
        scan.charge(0, 2).map_err(|_| Error::Capacity)?;
        if unsafe { libc::flock(self.held.file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            let _ = unsafe { libc::flock(self.held.file.as_raw_fd(), libc::LOCK_UN) };
            return Err(Error::RootLost);
        }
        if std::io::Error::last_os_error().raw_os_error() != Some(libc::EWOULDBLOCK) {
            return Err(Error::RootLost);
        }
        Ok(())
    }
}

struct CaptureNative {
    postmaster_start: u64,
    data: (u64, u64),
    socket: (u64, u64),
    pid_ns: String,
    mnt_ns: String,
}
impl CaptureNative {
    fn observe(scan: &SharedJournalBudget) -> Result<Self> {
        let pid_ns = kernel_link("/proc/self/ns/pid", scan)?;
        let mnt_ns = kernel_link("/proc/self/ns/mnt", scan)?;
        scan.charge(0, 1).map_err(|_| Error::Capacity)?;
        if std::fs::symlink_metadata("/proc/1")
            .map_err(|_| Error::Io)?
            .uid()
            != 999
            || bounded_kernel_read("/proc/1/comm", 64, scan)? != b"postgres\n"
        {
            return Err(Error::Identity);
        }
        let raw = bounded_kernel_read("/proc/1/stat", 4096, scan)?;
        let text = std::str::from_utf8(&raw).map_err(|_| Error::Identity)?;
        let tail = text.strip_prefix("1 (postgres) ").ok_or(Error::Identity)?;
        if matches!(
            tail.split_ascii_whitespace().next(),
            Some("Z" | "X" | "x") | None
        ) {
            return Err(Error::Identity);
        }
        let postmaster_start = parse_start(&raw, 1)?;
        scan.charge(0, 2).map_err(|_| Error::Capacity)?;
        let data = std::fs::symlink_metadata("/var/lib/postgresql").map_err(|_| Error::Io)?;
        let socket = std::fs::symlink_metadata("/var/run/postgresql/.s.PGSQL.5432")
            .map_err(|_| Error::Io)?;
        if !data.is_dir() || socket.mode() & libc::S_IFMT != libc::S_IFSOCK || socket.uid() != 999 {
            return Err(Error::Identity);
        }
        Ok(Self {
            postmaster_start,
            data: (data.dev(), data.ino()),
            socket: (socket.dev(), socket.ino()),
            pid_ns,
            mnt_ns,
        })
    }
    fn join(&self, catalog: &job::ExecutionCatalog, record: &job::CaptureStart) -> Result<()> {
        let r = &catalog.installed_runtime;
        let n = &record.native;
        let endpoint = &catalog.peer_endpoint;
        if self.pid_ns != r.pg_pid_namespace
            || self.mnt_ns != r.pg_mount_namespace
            || self.postmaster_start != r.postmaster_start_ticks
            || n.postmaster_start_ticks != self.postmaster_start
            || (n.data_dev, n.data_ino) != self.data
            || (n.socket_dev, n.socket_ino) != self.socket
            || (endpoint.socket_dev, endpoint.socket_ino) != self.socket
        {
            return Err(Error::Identity);
        }
        Ok(())
    }
    fn revalidate(&self, scan: &SharedJournalBudget) -> Result<()> {
        let now = Self::observe(scan)?;
        if now.postmaster_start != self.postmaster_start
            || now.data != self.data
            || now.socket != self.socket
            || now.pid_ns != self.pid_ns
            || now.mnt_ns != self.mnt_ns
        {
            return Err(Error::Identity);
        }
        Ok(())
    }
}

/// Enumerates only this already bound J/B, never selects a latest job/backup.
/// Parent directory timestamps may legitimately change on immutable appends.
fn directory_roster(
    file: &File,
    maximum: usize,
    scan: &SharedJournalBudget,
) -> Result<Vec<(String, bool)>> {
    let m = Stamp::observe(file, scan)?;
    if m.uid != 0 || m.mode & libc::S_IFMT != libc::S_IFDIR || m.mode & 0o7777 != 0o700 {
        return Err(Error::Identity);
    }
    scan.charge(0, 1).map_err(|_| Error::Capacity)?;
    let mut result = Vec::new();
    for entry in
        std::fs::read_dir(format!("/proc/self/fd/{}", file.as_raw_fd())).map_err(|_| Error::Io)?
    {
        scan.charge(0, 2).map_err(|_| Error::Capacity)?;
        if result.len() >= maximum {
            return Err(Error::Identity);
        }
        let entry = entry.map_err(|_| Error::Io)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| Error::Identity)?;
        let kind = entry.file_type().map_err(|_| Error::Io)?;
        if !kind.is_file() && !kind.is_dir() {
            return Err(Error::Identity);
        }
        result.push((name, kind.is_dir()));
    }
    result.sort();
    Ok(result)
}
fn child_directory(parent: &File, name: &str, scan: &SharedJournalBudget) -> Result<File> {
    if name.is_empty() || name.contains('/') || name == "." || name == ".." {
        return Err(Error::Shape);
    }
    let name = CString::new(name).map_err(|_| Error::Shape)?;
    scan.charge(name.as_bytes().len() as u64, 1)
        .map_err(|_| Error::Capacity)?;
    let raw = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if raw < 0 {
        return Err(Error::Identity);
    }
    Ok(unsafe { File::from_raw_fd(raw) })
}
fn check_job_roster(
    start: &HeldRecord<job::SourceStart>,
    control: &job::OriginalControlOwner,
    backup: Option<&str>,
    scan: &SharedJournalBudget,
) -> Result<()> {
    let root = &start.held.parents.last().ok_or(Error::Identity)?.0;
    let entries = directory_roster(root, 4, scan)?;
    if !entries.iter().any(|(n, d)| n == "source-start.json" && !*d)
        || !entries.iter().any(|(n, d)| n == "job.lock" && !*d)
    {
        return Err(Error::Identity);
    }
    for (name, dir) in entries {
        match (name.as_str(), dir) {
            ("source-start.json" | "job.lock", false) => {}
            ("backups", true) => {
                let root = child_directory(root, "backups", scan)?;
                let backups = directory_roster(&root, 1, scan)?;
                for (name, dir) in backups {
                    if !dir || !job::uuid(&name, true) || backup.is_some_and(|b| b != name) {
                        return Err(Error::Identity);
                    }
                    // A host has not adopted the observed ID. Before Spawn it
                    // may not observe an existing child; afterward Root alone
                    // owns creation and a later exact DATA joins this ID.
                    if backup.is_none() && control.phase_data() < 2 {
                        return Err(Error::Identity);
                    }
                    let b = child_directory(&root, &name, scan)?;
                    for (leaf, is_dir) in directory_roster(&b, 6, scan)? {
                        let allowed = match (leaf.as_str(), is_dir) {
                            ("capture-start.json", false) => true,
                            ("install", true) => control.phase_data() >= 4,
                            ("capture", true) => control.phase_data() >= 7,
                            ("captured-data.json", false) => control.phase_data() >= 9,
                            ("capture-settlement.json", false) => false,
                            _ => false,
                        };
                        if !allowed {
                            return Err(Error::Identity);
                        }
                    }
                }
            }
            // hold, partial files or unknown names permanently reject.
            _ => return Err(Error::RootLost),
        }
    }
    Ok(())
}
fn fresh_capture_ids(job: &str, scan: &SharedJournalBudget) -> Result<(String, String, String)> {
    let mut bytes = [0_u8; 64];
    scan.charge(64, 1).map_err(|_| Error::Capacity)?;
    if unsafe { libc::getrandom(bytes.as_mut_ptr().cast(), bytes.len(), 0) } != bytes.len() as isize
    {
        return Err(Error::Io);
    }
    let mut backup: [u8; 16] = bytes[..16].try_into().map_err(|_| Error::Identity)?;
    let mut request: [u8; 16] = bytes[16..32].try_into().map_err(|_| Error::Identity)?;
    for id in [&mut backup, &mut request] {
        id[6] = (id[6] & 0x0f) | 0x40;
        id[8] = (id[8] & 0x3f) | 0x80;
    }
    let backup = uuid::Uuid::from_bytes(backup).hyphenated().to_string();
    let request = uuid::Uuid::from_bytes(request).hyphenated().to_string();
    if backup == job || request == job || backup == request {
        return Err(Error::Identity);
    }
    Ok((backup, encode_hex(&bytes[32..]), request))
}
fn match_record_reference(
    reference: &job::RecordRef,
    record_bytes: &[u8],
    digest: &[u8; 32],
) -> Result<()> {
    if reference.bytes != record_bytes.len() as u64 || reference.sha256 != encode_hex(digest) {
        return Err(Error::Identity);
    }
    Ok(())
}
fn validate_capture_start(
    catalog: &job::ExecutionCatalog,
    start: &job::SourceStart,
    source_digest: &[u8; 32],
    record: &job::CaptureStart,
    backup: &str,
    nonce: &str,
    request: &str,
) -> Result<()> {
    let runtime = &catalog.installed_runtime;
    let exec_recipe = job::canonical(&catalog.capture_exec_recipe)?;
    if record.job_id != start.job_id
        || record.launch_id != start.launch_id
        || record.catalog_sha256 != start.catalog_sha256
        || record.boot_id != start.boot_id
        || record.root_cookie != start.root_cookie
        || record.backup_id != backup
        || record.capture_nonce != nonce
        || record.capture_request_id != request
        || record.source_start_sha256 != encode_hex(source_digest)
        || record.container_id != runtime.pg_container_id
        || record.image_id != runtime.admitted_pg_image_id
        || record.image_ref != runtime.pg_image_ref
        || record.image_digest != runtime.pg_image_digest
        || record.mount_recipe_sha256 != runtime.capture_mount_recipe_sha256
        || record.mount_recipe_sha256 != catalog.capture_recipe_sha256
        || record.capture_exec_recipe_sha256 != encode_hex(&Sha256::digest(&exec_recipe))
        || record.exec_process_config.entrypoint != catalog.capture_exec_recipe.entrypoint
        || record.exec_process_config.arguments != catalog.capture_exec_recipe.arguments
    {
        return Err(Error::Identity);
    }
    Ok(())
}
fn source_endpoint(
    catalog: &job::ExecutionCatalog,
    role: job::ProcessRole,
    scan: &SharedJournalBudget,
) -> Result<()> {
    let endpoint = &catalog.peer_endpoint;
    let directory = match role {
        job::ProcessRole::Source => "/run/knowweave-c4-source-pg",
        job::ProcessRole::Capture => "/var/run/postgresql",
    };
    scan.charge(directory.len() as u64, 2)
        .map_err(|_| Error::Capacity)?;
    let dir = std::fs::symlink_metadata(directory).map_err(|_| Error::Identity)?;
    let socket = std::fs::symlink_metadata(format!("{directory}/.s.PGSQL.5432"))
        .map_err(|_| Error::Identity)?;
    if !dir.is_dir()
        || dir.uid() != 999
        || socket.uid() != 999
        || socket.mode() & libc::S_IFMT != libc::S_IFSOCK
        || (dir.dev(), dir.ino()) != (endpoint.directory_dev, endpoint.directory_ino)
        || (socket.dev(), socket.ino()) != (endpoint.socket_dev, endpoint.socket_ino)
    {
        return Err(Error::Identity);
    }
    Ok(())
}
fn require_capture_readonly_roots(scan: &SharedJournalBudget) -> Result<()> {
    let raw = bounded_kernel_read("/proc/self/mountinfo", 1024 * 1024, scan)?;
    let text = std::str::from_utf8(&raw).map_err(|_| Error::Identity)?;
    for root in [OBSERVATION_ROOT, "/opt/knowweave-c4/b1-capture"] {
        let mut count = 0;
        for line in text.lines() {
            let fields: Vec<_> = line.split_ascii_whitespace().collect();
            if fields.len() < 10 {
                return Err(Error::Identity);
            }
            let target = fields[4];
            if target == root {
                count += 1;
                if !fields[5].split(',').any(|v| v == "ro") {
                    return Err(Error::Identity);
                }
            }
            if target
                .strip_prefix(root)
                .is_some_and(|suffix| suffix.starts_with('/'))
                && fields[5].split(',').any(|v| v == "rw")
            {
                return Err(Error::Identity);
            }
        }
        if count != 1 {
            return Err(Error::Identity);
        }
    }
    Ok(())
}

enum OriginalRoleOwner {
    Source {
        parent: ParentPid,
        parent_image: MeasuredImage,
        parent_manifest: Box<HeldRecord<job::LaunchManifest>>,
        parent_pid: u32,
        socket_cookie: u64,
    },
    Capture {
        capture: Box<HeldRecord<job::CaptureStart>>,
        native: CaptureNative,
        backup: String,
        nonce: String,
        request: String,
    },
}
struct OriginalState {
    pins: CompilePins,
    catalog: HeldRecord<job::ExecutionCatalog>,
    manifest: HeldRecord<job::LaunchManifest>,
    own_image: MeasuredImage,
    source_start: HeldRecord<job::SourceStart>,
    lock: JobLock,
    native_view: SupportedNativeView,
    control: job::OriginalControlOwner,
    role: OriginalRoleOwner,
}
impl OriginalState {
    fn revalidate(&mut self, scan: &SharedJournalBudget) -> Result<()> {
        // A fork inheritor must fail before any saved-PID, UID or native gate.
        require_same_current_pid(self.own_image.pid, scan)?;
        self.own_image.files[0]
            .1
            .matches_actual_exe(self.own_image.pid, scan)?;
        scan.charge(0, 2).map_err(|_| Error::Capacity)?;
        require_actual_root()?;
        self.native_view.revalidate(scan)?;
        self.catalog.revalidate(scan)?;
        self.manifest.revalidate(scan)?;
        self.pins.join(&self.catalog.data, &self.manifest.data)?;
        self.own_image.revalidate(scan)?;
        recheck_actual_environment(
            self.pins.role,
            &self.catalog.data.installed_source.database,
            scan,
        )?;
        self.control.require_idle_no_extra()?;
        self.source_start.revalidate(scan)?;
        self.lock.revalidate(scan)?;
        let start = &self.source_start.data;
        if start.catalog_sha256 != encode_hex(&self.catalog.digest)
            || start.launch_id != self.catalog.data.launch_id
            || start.boot_id != self.native_view.boot_id(scan)?
        {
            return Err(Error::Identity);
        }
        declared_process_image(
            &start.source,
            image(&self.catalog.data, "PublicationSource")?,
        )?;
        declared_process_image(&start.launcher, image(&self.catalog.data, "RootLauncher")?)?;
        match &mut self.role {
            OriginalRoleOwner::Source {
                parent,
                parent_image,
                parent_manifest,
                parent_pid,
                socket_cookie,
            } => {
                scan.charge(0, 1).map_err(|_| Error::Capacity)?;
                if unsafe { libc::getppid() } != *parent_pid as i32
                    || self.control.source_facts()? != (*parent_pid, *socket_cookie)
                    || start.source_socket_cookie != *socket_cookie
                {
                    return Err(Error::Identity);
                }
                parent.revalidate(scan)?;
                parent_image.revalidate(scan)?;
                parent_manifest.revalidate(scan)?;
                parent_image.join(&start.launcher, true)?;
                self.own_image.join(&start.source, true)?;
                match_record_reference(
                    &image(&self.catalog.data, "RootLauncher")?.launch_manifest,
                    &parent_manifest.raw,
                    &parent_manifest.digest,
                )?;
                check_job_roster(&self.source_start, &self.control, None, scan)?;
            }
            OriginalRoleOwner::Capture {
                capture,
                native,
                backup,
                nonce,
                request,
            } => {
                require_capture_readonly_roots(scan)?;
                capture.revalidate(scan)?;
                native.revalidate(scan)?;
                validate_capture_start(
                    &self.catalog.data,
                    start,
                    &self.source_start.digest,
                    &capture.data,
                    backup,
                    nonce,
                    request,
                )?;
                self.own_image.join(&capture.data.capture, false)?;
                native.join(&self.catalog.data, &capture.data)?;
                check_job_roster(&self.source_start, &self.control, Some(backup), scan)?;
            }
        }
        source_endpoint(&self.catalog.data, self.pins.role, scan)?;
        self.native_view.revalidate(scan)?;
        Ok(())
    }
}

/// Sole qualified owner, not serializable/Clone/Default/Debug and not a SQL or S
/// authority. Actual install/SQL/native-session gates remain M2b's responsibility.
pub(super) struct PeerOriginal {
    scan: SharedJournalBudget,
    state: std::sync::Mutex<OriginalState>,
    failed: AtomicBool,
}
impl PeerOriginal {
    pub(super) fn revalidate(&self) -> Result<()> {
        if self.failed.load(Ordering::Acquire) {
            return Err(Error::Poisoned);
        }
        let result = self
            .state
            .lock()
            .map_err(|_| Error::Poisoned)
            .and_then(|mut state| state.revalidate(&self.scan));
        if result.is_err() {
            self.failed.store(true, Ordering::Release);
        }
        result
    }
    pub(super) fn scan(&self) -> SharedJournalBudget {
        self.scan.clone()
    }
    pub(super) fn capture_backup_id_data(&self) -> Result<uuid::Uuid> {
        self.revalidate()?;
        let state = self.state.lock().map_err(|_| Error::Poisoned)?;
        match &state.role {
            OriginalRoleOwner::Capture { backup, .. } => {
                uuid::Uuid::parse_str(backup).map_err(|_| Error::Identity)
            }
            _ => Err(Error::Phase),
        }
    }
}

/// No parameters, callbacks, caller connection, budget, role, clock or DTO.
/// The only private finish is below the actual kernel/held-record/channel chain.
pub(super) fn admit_original_process() -> Result<PeerOriginal> {
    let scan = burn_attempt_before_io()?;
    // Pure compiled constants only: no getenv/UID/native/metadata observation.
    let pins = CompilePins::read()?;
    // Restore and retain the same original descriptors before native/UID IO.
    let mut control = job::OriginalControlOwner::take_installed(pins.role, scan.clone())?;
    scan.charge(0, 2).map_err(|_| Error::Capacity)?;
    require_actual_root()?;
    let early_source_parent = if pins.role == job::ProcessRole::Source {
        let (pid, cookie) = control.source_facts()?;
        scan.charge(0, 1).map_err(|_| Error::Capacity)?;
        if unsafe { libc::getppid() } != pid as i32 {
            return Err(Error::Identity);
        }
        let start = actual_start(pid, &scan)?;
        Some((pid, cookie, ParentPid::open(pid, start, &scan)?))
    } else {
        require_capture_readonly_roots(&scan)?;
        None
    };
    let native_view = match pins.role {
        job::ProcessRole::Source => SupportedNativeView::observe_source(
            early_source_parent.as_ref().ok_or(Error::Identity)?.0,
            &scan,
        )?,
        job::ProcessRole::Capture => SupportedNativeView::observe_capture(&scan)?,
    };
    require_single_thread(&scan)?;
    let mut catalog = HeldRecord::<job::ExecutionCatalog>::open(pins.catalog_path(), 16384, &scan)?;
    let mut manifest = HeldRecord::<job::LaunchManifest>::open(pins.manifest_path(), 4096, &scan)?;
    pins.join(&catalog.data, &manifest.data)?;
    let own_row = image(&catalog.data, pins.row())?;
    match_record_reference(&own_row.launch_manifest, &manifest.raw, &manifest.digest)?;
    let own_image = MeasuredImage::open(own_row, std::process::id(), &scan)?;
    recheck_actual_environment(pins.role, &catalog.data.installed_source.database, &scan)?;
    require_single_thread(&scan)?;
    let native = if pins.role == job::ProcessRole::Capture {
        Some(CaptureNative::observe(&scan)?)
    } else {
        None
    };
    let bootstrap = control.read_frame()?;
    let (job_id, cookie, source_hash) = match (&pins.role, bootstrap) {
        (
            job::ProcessRole::Source,
            job::ControlData::SourceBootstrap {
                job_id,
                root_cookie,
                source_start_sha256,
                ..
            },
        )
        | (
            job::ProcessRole::Capture,
            job::ControlData::CaptureBootstrap {
                job_id,
                root_cookie,
                source_start_sha256,
                ..
            },
        ) => (job_id, root_cookie, source_start_sha256),
        _ => return Err(Error::Phase),
    };
    let job_path = format!("{OBSERVATION_ROOT}/jobs/{job_id}");
    let mut source_start = HeldRecord::<job::SourceStart>::open(
        &format!("{job_path}/source-start.json"),
        4096,
        &scan,
    )?;
    let start = &source_start.data;
    if encode_hex(&source_start.digest) != source_hash
        || start.job_id != job_id
        || start.root_cookie != cookie
        || start.catalog_sha256 != encode_hex(&catalog.digest)
        || start.launch_id != catalog.data.launch_id
        || start.boot_id != native_view.boot_id(&scan)?
    {
        return Err(Error::Identity);
    }
    declared_process_image(&start.source, image(&catalog.data, "PublicationSource")?)?;
    declared_process_image(&start.launcher, image(&catalog.data, "RootLauncher")?)?;
    let lock = JobLock::open(
        &format!("{job_path}/job.lock"),
        start.job_lock_dev,
        start.job_lock_ino,
        &scan,
    )?;
    check_job_roster(&source_start, &control, None, &scan)?;
    let role = match pins.role {
        job::ProcessRole::Source => {
            let (parent_pid, socket_cookie, parent) = early_source_parent.ok_or(Error::Identity)?;
            scan.charge(0, 1).map_err(|_| Error::Capacity)?;
            if unsafe { libc::getppid() } != parent_pid as i32
                || control.source_facts()? != (parent_pid, socket_cookie)
                || start.source_socket_cookie != socket_cookie
            {
                return Err(Error::Identity);
            }
            own_image.join(&start.source, true)?;
            let parent_row = image(&catalog.data, "RootLauncher")?;
            let parent_image = MeasuredImage::open(parent_row, parent_pid, &scan)?;
            parent_image.join(&start.launcher, true)?;
            if parent_image.start != parent.start {
                return Err(Error::Identity);
            }
            parent.revalidate(&scan)?;
            let parent_manifest = HeldRecord::<job::LaunchManifest>::open(
                "/var/lib/knowweave-c4/b1/launcher-launch-manifest.json",
                4096,
                &scan,
            )?;
            match_record_reference(
                &parent_row.launch_manifest,
                &parent_manifest.raw,
                &parent_manifest.digest,
            )?;
            let pm = &parent_manifest.data;
            if pm.launch_id != catalog.data.launch_id
                || pm.binary_sha256 != parent_row.executable.sha256
                || pm.launcher_sha256 != parent_row.executable.sha256
                || pm.code_commit != parent_row.code_commit
                || pm.build_input_sha256 != parent_row.build_input_sha256
                || pm.source_auth_public_key_sha256 != pins.source_auth
                || pm.destination_verifier_public_key_sha256 != pins.verifier
            {
                return Err(Error::Identity);
            }
            OriginalRoleOwner::Source {
                parent,
                parent_image,
                parent_manifest: Box::new(parent_manifest),
                parent_pid,
                socket_cookie,
            }
        }
        job::ProcessRole::Capture => {
            let (backup, nonce, request) = fresh_capture_ids(&job_id, &scan)?;
            control.write_frame(&job::ControlData::CaptureInit {
                schema_version: 1,
                job_id: job_id.clone(),
                root_cookie: cookie.clone(),
                sequence: 0,
                backup_id: backup.clone(),
                capture_nonce: nonce.clone(),
                request_id: request.clone(),
            })?;
            let ack = control.read_frame()?;
            let record_hash = match ack {
                job::ControlData::CaptureStartBound {
                    backup_id,
                    capture_start_sha256,
                    ..
                } if backup_id == backup => capture_start_sha256,
                _ => return Err(Error::Identity),
            };
            let capture = HeldRecord::<job::CaptureStart>::open(
                &format!("{job_path}/backups/{backup}/capture-start.json"),
                16384,
                &scan,
            )?;
            if encode_hex(&capture.digest) != record_hash {
                return Err(Error::Identity);
            }
            validate_capture_start(
                &catalog.data,
                start,
                &source_start.digest,
                &capture.data,
                &backup,
                &nonce,
                &request,
            )?;
            own_image.join(&capture.data.capture, false)?;
            let native = native.ok_or(Error::Identity)?;
            native.join(&catalog.data, &capture.data)?;
            OriginalRoleOwner::Capture {
                capture: Box::new(capture),
                native,
                backup,
                nonce,
                request,
            }
        }
    };
    // Full immutable bytes were checked before either branch, and remain the
    // same held originals at this only finish point. No disk-adopt constructor.
    catalog.revalidate(&scan)?;
    manifest.revalidate(&scan)?;
    source_start.revalidate(&scan)?;
    let mut state = OriginalState {
        pins,
        catalog,
        manifest,
        own_image,
        source_start,
        lock,
        native_view,
        control,
        role,
    };
    state.revalidate(&scan)?;
    Ok(PeerOriginal {
        scan,
        state: std::sync::Mutex::new(state),
        failed: AtomicBool::new(false),
    })
}

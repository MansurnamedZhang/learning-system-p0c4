//! Original installed D session. It owns no Source PG or live Source path.
//! Registry, native image, stdio and signer owners are rooted before async IO.

use super::{BuildPins, Role, fixed_build_inputs, policy, profile, wire};
use crate::{
    BackupError, EnrolledDestination, FileRecord, ManagementRegistry, RegistryLease,
    destination::{OperationAuthority, OriginalRemoteFinalization},
    registry::ScanBudget,
};
use learning_assets::backup_fs::{BackupDir, BackupEntryKind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use uuid::Uuid;

const RECEIVER_PATH: &str = "/usr/local/libexec/knowweave-c4-b1-receiver";
const SESSION_ROOT: &str = "/var/lib/knowweave-c4-b1-sessions";
const PROC_BYTES: u64 = 1_048_576;

static ENTRY_ATTEMPTED: AtomicBool = AtomicBool::new(false);
static ENTRY_REJECTED: AtomicBool = AtomicBool::new(false);
static ORIGINAL_ENTRY: OnceLock<OriginalDEntryScope> = OnceLock::new();

fn refused() -> BackupError {
    BackupError::Invalid("original installed destination custody or phase")
}

#[derive(Clone, Copy)]
enum FixedFdWork {
    Observe,
    Native,
    Runtime,
    FlowLayout,
    PayloadCreate,
    Readback,
    EffectsCopy,
}

/// Process-original custody and accounting, not D publication authority. The
/// sole fixed entry constructs this before its first real native/stdio IO.
/// Unknown/cancellation cannot clear its actual owned slots, locks or images.
pub(crate) struct OriginalDEntryScope {
    pins: BuildPins<'static>,
    pid: u32,
    handshake_deadline: Instant,
    deadline: Instant,
    journal: learning_assets::root_journal::SharedJournalBudget,
    scan: Arc<Mutex<ScanBudget>>,
    failed: Arc<AtomicBool>,
    registry_attempted: AtomicBool,
    signer_attempted: AtomicBool,
    lease: OnceLock<RegistryLease>,
    native: OnceLock<Arc<NativeDestinationLocalCustody>>,
    signer: OnceLock<Arc<crate::destination::B1DestinationSignerOriginal>>,
    pipes: OnceLock<&'static wire::OriginalPipePair>,
    metadata: OnceLock<Mutex<policy::HeldDestinationInputs<'static>>>,
    destination: OnceLock<Arc<EnrolledDestination>>,
    io: tokio::sync::Mutex<Option<wire::OriginalPipeIo>>,
    session: tokio::sync::Mutex<Option<DestinationSessionState>>,
    namespace: OnceLock<Mutex<SessionNamespace>>,
    runtime: OnceLock<tokio::runtime::Runtime>,
}

impl OriginalDEntryScope {
    fn begin_fixed() -> Result<&'static Self, BackupError> {
        if ENTRY_ATTEMPTED.swap(true, Ordering::AcqRel) {
            ENTRY_REJECTED.store(true, Ordering::Release);
            if let Some(original) = ORIGINAL_ENTRY.get() {
                original.reject();
            }
            return Err(refused());
        }
        let journal = learning_assets::root_journal::SharedJournalBudget::default();
        // Only the unavoidable first ledger bootstrap precedes its own meter.
        // Pay all following Arc/Mutex/failed-flag/Book fixed control storage
        // before creating it; later scopes reuse this exact same backing.
        let fixed_bytes = (std::mem::size_of::<Self>() as u64)
            .checked_add(std::mem::size_of::<ScanBudget>() as u64)
            .and_then(|n| n.checked_add(std::mem::size_of::<Mutex<ScanBudget>>() as u64))
            .and_then(|n| n.checked_add(std::mem::size_of::<Arc<AtomicBool>>() as u64))
            .and_then(|n| n.checked_add(4096))
            .ok_or(BackupError::Overflow)?;
        journal.charge(fixed_bytes, 5)?;
        let scan = Arc::new(Mutex::new(ScanBudget::from_b1_scan(journal.clone())));
        // Clock and actual kernel identity observations also use this one
        // meter. A later registry open must reuse it, never repay from zero.
        scan.lock().map_err(|_| refused())?.entries(5)?;
        let start = Instant::now();
        let pins = match BuildPins::parse(fixed_build_inputs(), Role::Destination) {
            Ok(pins) => pins,
            Err(error) => {
                ENTRY_REJECTED.store(true, Ordering::Release);
                return Err(error);
            }
        };
        if unsafe { libc::getuid() } != 0
            || unsafe { libc::geteuid() } != 0
            || unsafe { libc::getgid() } != 0
            || unsafe { libc::getegid() } != 0
        {
            ENTRY_REJECTED.store(true, Ordering::Release);
            return Err(refused());
        }
        let original = Self {
            pins,
            pid: std::process::id(),
            handshake_deadline: start + Duration::from_secs(profile::HANDSHAKE_SECONDS),
            deadline: start + Duration::from_secs(profile::OPERATION_SECONDS),
            journal,
            scan,
            failed: Arc::new(AtomicBool::new(false)),
            registry_attempted: AtomicBool::new(false),
            signer_attempted: AtomicBool::new(false),
            lease: OnceLock::new(),
            native: OnceLock::new(),
            signer: OnceLock::new(),
            pipes: OnceLock::new(),
            metadata: OnceLock::new(),
            destination: OnceLock::new(),
            io: tokio::sync::Mutex::new(None),
            session: tokio::sync::Mutex::new(None),
            namespace: OnceLock::new(),
            runtime: OnceLock::new(),
        };
        if ORIGINAL_ENTRY.set(original).is_err() {
            ENTRY_REJECTED.store(true, Ordering::Release);
            return Err(refused());
        }
        let original = ORIGINAL_ENTRY.get().ok_or_else(refused)?;
        original.recheck()?;
        original.require_fixed_fd_room(FixedFdWork::Native)?;
        Ok(original)
    }

    pub(crate) fn recheck(&self) -> Result<(), BackupError> {
        if !ORIGINAL_ENTRY
            .get()
            .is_some_and(|held| std::ptr::eq(held, self))
            || ENTRY_REJECTED.load(Ordering::Acquire)
            || self.failed.load(Ordering::Acquire)
        {
            return Err(refused());
        }
        self.check_local_process()?;
        Ok(())
    }

    fn check_local_process(&self) -> Result<(), BackupError> {
        // Local sticky rejection survives Final consumption. Only the remote
        // parent/Source liveness obligation ends at that transition.
        if !ORIGINAL_ENTRY
            .get()
            .is_some_and(|held| std::ptr::eq(held, self))
            || ENTRY_REJECTED.load(Ordering::Acquire)
            || self.failed.load(Ordering::Acquire)
        {
            return Err(refused());
        }
        self.prepay_wire_work(0, 5)?;
        if std::process::id() != self.pid
            || Instant::now() >= self.deadline
            || unsafe { libc::getuid() } != 0
            || unsafe { libc::geteuid() } != 0
            || unsafe { libc::getgid() } != 0
            || unsafe { libc::getegid() } != 0
        {
            return Err(refused());
        }
        Ok(())
    }

    pub(crate) fn reject(&self) {
        self.failed.store(true, Ordering::Release);
        ENTRY_REJECTED.store(true, Ordering::Release);
    }

    pub(crate) fn prepay_wire_work(&self, bytes: u64, entries: usize) -> Result<(), BackupError> {
        let result = (|| {
            let mut meter = self.scan.lock().map_err(|_| refused())?;
            meter.charge(bytes)?;
            meter.entries(entries)
        })();
        if result.is_err() {
            self.reject();
        }
        result
    }

    fn require_fixed_fd_room(&self, work: FixedFdWork) -> Result<(), BackupError> {
        let extra = match work {
            FixedFdWork::Observe => 0,
            FixedFdWork::Native => 6,
            FixedFdWork::Runtime => 8,
            FixedFdWork::FlowLayout => 12,
            FixedFdWork::PayloadCreate => 6,
            FixedFdWork::Readback => 4,
            FixedFdWork::EffectsCopy => 16,
        };
        self.check_local_process()?;
        self.prepay_wire_work(
            profile::PRODUCT_FDS * 256,
            profile::PRODUCT_FDS as usize + 2,
        )?;
        let mut current = 0u64;
        for item in std::fs::read_dir("/proc/self/fd")? {
            if item?
                .file_name()
                .to_str()
                .is_none_or(|n| n.parse::<u32>().is_err())
            {
                return Err(refused());
            }
            current = current.checked_add(1).ok_or(BackupError::Overflow)?;
            // Includes the finite observer, conservatively leaving its slot
            // unused by productive work. No counter credit, rlimit or close.
            if current
                .checked_add(extra)
                .is_none_or(|n| n > profile::PRODUCT_FDS)
            {
                return Err(refused());
            }
        }
        Ok(())
    }

    pub(crate) fn take_fixed_registry_scope(
        &self,
    ) -> Result<(Arc<Mutex<ScanBudget>>, Arc<AtomicBool>), BackupError> {
        self.recheck()?;
        if self.registry_attempted.swap(true, Ordering::AcqRel) {
            self.reject();
            return Err(refused());
        }
        Ok((Arc::clone(&self.scan), Arc::clone(&self.failed)))
    }

    pub(crate) fn operation_deadline_data(&self) -> Instant {
        self.deadline
    }

    pub(crate) fn handshake_deadline_data(&self) -> Instant {
        self.handshake_deadline.min(self.deadline)
    }

    pub(crate) fn take_fixed_signer_scope(&self) -> Result<Arc<Mutex<ScanBudget>>, BackupError> {
        self.recheck()?;
        self.native.get().ok_or_else(refused)?.recheck_pending()?;
        if self.signer_attempted.swap(true, Ordering::AcqRel) {
            self.reject();
            return Err(refused());
        }
        Ok(Arc::clone(&self.scan))
    }

    pub(crate) fn verifier_public_key_pin_data(&self) -> &str {
        self.pins.verifier_key
    }
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
    mtime_ns: i64,
    ctime: i64,
    ctime_ns: i64,
}
impl Stamp {
    fn from(metadata: &std::fs::Metadata) -> Self {
        Self {
            dev: metadata.dev(),
            ino: metadata.ino(),
            uid: metadata.uid(),
            gid: metadata.gid(),
            mode: metadata.mode(),
            links: metadata.nlink(),
            len: metadata.len(),
            mtime: metadata.mtime(),
            mtime_ns: metadata.mtime_nsec(),
            ctime: metadata.ctime(),
            ctime_ns: metadata.ctime_nsec(),
        }
    }
    fn require_code(self) -> Result<(), BackupError> {
        if self.mode & libc::S_IFMT != libc::S_IFREG
            || self.uid != 0
            || self.gid != 0
            || self.mode & 0o022 != 0
            || self.links != 1
            || self.len == 0
            || self.len > 64 * 1024 * 1024
        {
            return Err(refused());
        }
        Ok(())
    }
}

struct HeldNativeCode {
    path: PathBuf,
    file: File,
    stamp: Stamp,
    sha256: String,
}

impl HeldNativeCode {
    fn open_kernel_or_fixed(
        path: &Path,
        original: &'static OriginalDEntryScope,
    ) -> Result<Self, BackupError> {
        original.require_fixed_fd_room(FixedFdWork::Native)?;
        trusted_code_parents(path, original)?;
        original.prepay_wire_work(0, 3)?;
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(path)?;
        let stamp = Stamp::from(&file.metadata()?);
        stamp.require_code()?;
        original.prepay_wire_work(
            stamp.len.checked_mul(2).ok_or(BackupError::Overflow)? + 65_536,
            1,
        )?;
        let mut hash = sha2::Sha256::new();
        let mut remaining = stamp.len;
        let mut buffer = [0u8; 65_536];
        let mut magic = [0u8; 4];
        file.read_exact(&mut magic)?;
        if magic != *b"\x7fELF" {
            return Err(refused());
        }
        file.seek(SeekFrom::Start(0))?;
        use sha2::Digest;
        while remaining > 0 {
            original.check_local_process()?;
            let length =
                usize::try_from(remaining.min(buffer.len() as u64)).map_err(|_| refused())?;
            let count = match file.read(&mut buffer[..length]) {
                Ok(count) => count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            };
            if count == 0 {
                return Err(refused());
            }
            hash.update(&buffer[..count]);
            remaining -= count as u64;
        }
        let mut extra = [0u8];
        if file.read(&mut extra)? != 0 || Stamp::from(&file.metadata()?) != stamp {
            return Err(refused());
        }
        let code = Self {
            path: path.to_owned(),
            file,
            stamp,
            sha256: format!("{:x}", hash.finalize()),
        };
        code.recheck(original)?;
        Ok(code)
    }

    fn recheck(&self, original: &OriginalDEntryScope) -> Result<(), BackupError> {
        trusted_code_parents(&self.path, original)?;
        original.prepay_wire_work(0, 3)?;
        let named = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(&self.path)?;
        if Stamp::from(&self.file.metadata()?) != self.stamp
            || Stamp::from(&named.metadata()?) != self.stamp
        {
            return Err(refused());
        }
        Ok(())
    }
}

fn trusted_code_parents(path: &Path, original: &OriginalDEntryScope) -> Result<(), BackupError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
    {
        return Err(refused());
    }
    let parent = path.parent().ok_or_else(refused)?;
    original.prepay_wire_work(
        parent.as_os_str().len() as u64,
        parent.components().count() * 2,
    )?;
    let mut current = PathBuf::from("/");
    for part in parent.components() {
        match part {
            Component::RootDir => (),
            Component::Normal(name) => current.push(name),
            _ => return Err(refused()),
        }
        let metadata = std::fs::symlink_metadata(&current)?;
        if !metadata.is_dir()
            || metadata.uid() != 0
            || metadata.gid() != 0
            || metadata.mode() & 0o022 != 0
        {
            return Err(refused());
        }
    }
    Ok(())
}

fn proc_bytes(path: &Path, original: &OriginalDEntryScope) -> Result<Vec<u8>, BackupError> {
    original.require_fixed_fd_room(FixedFdWork::Native)?;
    original.prepay_wire_work(0, 3)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)?;
    let mut raw = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        original.check_local_process()?;
        let allowed = (PROC_BYTES + 1)
            .checked_sub(raw.len() as u64)
            .ok_or_else(refused)?;
        if allowed == 0 {
            return Err(refused());
        }
        let length = usize::try_from(allowed.min(buffer.len() as u64)).map_err(|_| refused())?;
        original.prepay_wire_work(length as u64, 1)?;
        let count = match file.read(&mut buffer[..length]) {
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        };
        if count == 0 {
            break;
        }
        let wanted = raw.len().checked_add(count).ok_or(BackupError::Overflow)?;
        if wanted as u64 > PROC_BYTES {
            return Err(refused());
        }
        if wanted > raw.capacity() {
            let capacity = wanted
                .checked_next_power_of_two()
                .ok_or(BackupError::Overflow)?;
            original.prepay_wire_work(
                (capacity as u64)
                    .checked_mul(2)
                    .ok_or(BackupError::Overflow)?,
                1,
            )?;
            raw.reserve_exact(capacity - raw.len());
        }
        raw.extend_from_slice(&buffer[..count]);
    }
    if raw.is_empty() {
        return Err(refused());
    }
    // Prepay the bounded token/path projections before any parser allocates.
    original.prepay_wire_work(
        (raw.len() as u64)
            .checked_mul(8)
            .ok_or(BackupError::Overflow)?,
        1,
    )?;
    Ok(raw)
}

#[derive(PartialEq, Eq)]
struct ProcessData {
    pid: u32,
    parent: u32,
    start_ticks: u64,
}
fn process_stat_data(raw: &[u8], expected: u32) -> Result<ProcessData, BackupError> {
    let text = std::str::from_utf8(raw).map_err(|_| refused())?;
    let (head, tail) = text.rsplit_once(") ").ok_or_else(refused)?;
    let (pid, _) = head.split_once(" (").ok_or_else(refused)?;
    let fields = tail.split_ascii_whitespace().collect::<Vec<_>>();
    if pid.parse::<u32>().ok() != Some(expected) || fields.len() < 20 {
        return Err(refused());
    }
    let parent = fields[1].parse::<u32>().map_err(|_| refused())?;
    let start_ticks = fields[19].parse::<u64>().map_err(|_| refused())?;
    if expected == 0 || parent == 0 || start_ticks == 0 {
        return Err(refused());
    }
    Ok(ProcessData {
        pid: expected,
        parent,
        start_ticks,
    })
}

fn root_process_status(raw: &[u8]) -> Result<(), BackupError> {
    let text = std::str::from_utf8(raw).map_err(|_| refused())?;
    let mut uid = None;
    let mut gid = None;
    let mut tracer = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("Uid:") {
            if uid
                .replace(value.split_ascii_whitespace().collect::<Vec<_>>())
                .is_some()
            {
                return Err(refused());
            }
        } else if let Some(value) = line.strip_prefix("Gid:") {
            if gid
                .replace(value.split_ascii_whitespace().collect::<Vec<_>>())
                .is_some()
            {
                return Err(refused());
            }
        } else if let Some(value) = line.strip_prefix("TracerPid:")
            && tracer.replace(value.trim()).is_some()
        {
            return Err(refused());
        }
    }
    if uid.as_deref() != Some(&["0", "0", "0", "0"][..])
        || gid.as_deref() != Some(&["0", "0", "0", "0"][..])
        || tracer != Some("0")
    {
        return Err(refused());
    }
    Ok(())
}

#[derive(PartialEq, Eq)]
struct ExecutableMappingData {
    range: String,
    offset: u64,
    device: String,
    inode: u64,
    path: PathBuf,
}

fn executable_mapping_data(raw: &[u8]) -> Result<Vec<ExecutableMappingData>, BackupError> {
    let text = std::str::from_utf8(raw).map_err(|_| refused())?;
    let mut mappings = Vec::new();
    for line in text.lines() {
        let fields = line.split_ascii_whitespace().collect::<Vec<_>>();
        if fields.len() < 5 {
            return Err(refused());
        }
        let permissions = fields[1].as_bytes();
        if permissions.len() != 4 {
            return Err(refused());
        }
        if permissions[2] != b'x' {
            continue;
        }
        if permissions[1] == b'w' || fields.len() != 6 {
            return Err(refused());
        }
        let path = fields[5];
        if path == "[vdso]" || path == "[vsyscall]" {
            if fields[4] != "0" {
                return Err(refused());
            }
        } else if !path.starts_with('/') || path.contains('\\') || path.ends_with("(deleted)") {
            return Err(refused());
        }
        let offset = u64::from_str_radix(fields[2], 16).map_err(|_| refused())?;
        let inode = fields[4].parse::<u64>().map_err(|_| refused())?;
        mappings.push(ExecutableMappingData {
            range: fields[0].to_owned(),
            offset,
            device: fields[3].to_owned(),
            inode,
            path: PathBuf::from(path),
        });
        if mappings.len() as u64 > profile::PRODUCT_FDS {
            return Err(refused());
        }
    }
    if mappings.is_empty() {
        return Err(refused());
    }
    Ok(mappings)
}

/// Concrete actual local custody retained separately from transport liveness.
/// Only its real fixed native admission can create it; after Consuming the
/// original parent image stays held, but S/sshd remaining alive is not a veto.
pub(crate) struct NativeDestinationLocalCustody {
    entry: &'static OriginalDEntryScope,
    self_data: ProcessData,
    parent_data: ProcessData,
    executable: HeldNativeCode,
    launcher: HeldNativeCode,
    mappings: Vec<ExecutableMappingData>,
    mapped_files: Vec<HeldNativeCode>,
}

impl NativeDestinationLocalCustody {
    fn admit_fixed(
        entry: &'static OriginalDEntryScope,
        metadata: &policy::DestinationMetadataData<'_>,
    ) -> Result<Self, BackupError> {
        entry.recheck()?;
        let self_data =
            process_stat_data(&proc_bytes(Path::new("/proc/self/stat"), entry)?, entry.pid)?;
        root_process_status(&proc_bytes(Path::new("/proc/self/status"), entry)?)?;
        let parent_data = process_stat_data(
            &proc_bytes(
                &PathBuf::from(format!("/proc/{}/stat", self_data.parent)),
                entry,
            )?,
            self_data.parent,
        )?;
        root_process_status(&proc_bytes(
            &PathBuf::from(format!("/proc/{}/status", self_data.parent)),
            entry,
        )?)?;
        entry.prepay_wire_work(8192, 4)?;
        if std::fs::read_link("/proc/self/exe")? != Path::new(RECEIVER_PATH)
            || proc_bytes(Path::new("/proc/self/cmdline"), entry)?
                != format!("{RECEIVER_PATH}\0").as_bytes()
        {
            return Err(refused());
        }
        let executable = HeldNativeCode::open_kernel_or_fixed(Path::new(RECEIVER_PATH), entry)?;
        if executable.stamp.mode & 0o7777 != 0o500
            || executable.sha256 != metadata.binary_sha256_data()
            || metadata.launch_commit_data() != entry.pins.commit
            || metadata.launch_build_data() != entry.pins.build_input
        {
            return Err(refused());
        }
        let self_exe = File::open("/proc/self/exe")?;
        if Stamp::from(&self_exe.metadata()?) != executable.stamp {
            return Err(refused());
        }
        let launcher_path = std::fs::read_link(format!("/proc/{}/exe", self_data.parent))?;
        let launcher = HeldNativeCode::open_kernel_or_fixed(&launcher_path, entry)?;
        if launcher.sha256 != metadata.launcher_sha256_data() {
            return Err(refused());
        }
        let parent_exe = File::open(format!("/proc/{}/exe", self_data.parent))?;
        if Stamp::from(&parent_exe.metadata()?) != launcher.stamp {
            return Err(refused());
        }
        let mappings = executable_mapping_data(&proc_bytes(Path::new("/proc/self/maps"), entry)?)?;
        let mut mapped_files = Vec::new();
        for mapping in &mappings {
            if !mapping.path.is_absolute() {
                continue;
            }
            if mapped_files
                .iter()
                .any(|file: &HeldNativeCode| file.path == mapping.path)
            {
                continue;
            }
            let held = HeldNativeCode::open_kernel_or_fixed(&mapping.path, entry)?;
            if held.stamp.ino != mapping.inode
                || !device_matches(held.stamp.dev, &mapping.device)
                || mapping.offset >= held.stamp.len
            {
                return Err(refused());
            }
            mapped_files.push(held);
        }
        if mapped_files.len() as u64 + 16 > profile::PRODUCT_FDS {
            return Err(refused());
        }
        let original = Self {
            entry,
            self_data,
            parent_data,
            executable,
            launcher,
            mappings,
            mapped_files,
        };
        original.recheck_pending()?;
        Ok(original)
    }

    fn recheck_local_facts(&self) -> Result<(), BackupError> {
        self.entry.require_fixed_fd_room(FixedFdWork::Native)?;
        self.entry.check_local_process()?;
        let now = process_stat_data(
            &proc_bytes(Path::new("/proc/self/stat"), self.entry)?,
            self.entry.pid,
        )?;
        if now.pid != self.self_data.pid || now.start_ticks != self.self_data.start_ticks {
            return Err(refused());
        }
        root_process_status(&proc_bytes(Path::new("/proc/self/status"), self.entry)?)?;
        self.executable.recheck(self.entry)?;
        self.launcher.recheck(self.entry)?;
        let exe = File::open("/proc/self/exe")?;
        if Stamp::from(&exe.metadata()?) != self.executable.stamp
            || std::fs::read_link("/proc/self/exe")? != Path::new(RECEIVER_PATH)
            || executable_mapping_data(&proc_bytes(Path::new("/proc/self/maps"), self.entry)?)?
                != self.mappings
        {
            return Err(refused());
        }
        for file in &self.mapped_files {
            file.recheck(self.entry)?;
        }
        // Count actual process descriptors (including this bounded observer),
        // rather than treating an inventory length as proof of the FD limit.
        self.entry.prepay_wire_work(
            profile::PRODUCT_FDS * 256,
            profile::PRODUCT_FDS as usize + 2,
        )?;
        let mut count = 0u64;
        for item in std::fs::read_dir("/proc/self/fd")? {
            let item = item?;
            if item
                .file_name()
                .to_str()
                .is_none_or(|n| n.parse::<u32>().is_err())
            {
                return Err(refused());
            }
            count = count.checked_add(1).ok_or(BackupError::Overflow)?;
            if count > profile::PRODUCT_FDS {
                return Err(refused());
            }
        }
        Ok(())
    }

    fn recheck_pending(&self) -> Result<(), BackupError> {
        self.entry.recheck()?;
        self.recheck_local_facts()?;
        let self_now = process_stat_data(
            &proc_bytes(Path::new("/proc/self/stat"), self.entry)?,
            self.entry.pid,
        )?;
        let parent = process_stat_data(
            &proc_bytes(
                &PathBuf::from(format!("/proc/{}/stat", self.parent_data.pid)),
                self.entry,
            )?,
            self.parent_data.pid,
        )?;
        if self_now.parent != self.parent_data.pid || parent != self.parent_data {
            return Err(refused());
        }
        root_process_status(&proc_bytes(
            &PathBuf::from(format!("/proc/{}/status", self.parent_data.pid)),
            self.entry,
        )?)?;
        // PID/start/Root credentials survive exec. Pending must also observe
        // the parent's CURRENT kernel executable, not only the old named ELF.
        // This belongs solely to Pending; Consuming/Committed use local D
        // custody and do not reintroduce remote/SSHD-parent liveness.
        self.entry.prepay_wire_work(16_384, 12)?;
        let executable = PathBuf::from(format!("/proc/{}/exe", self.parent_data.pid));
        let held_kernel = File::open(&executable)?;
        let before = Stamp::from(&held_kernel.metadata()?);
        if before != self.launcher.stamp
            || std::fs::read_link(&executable)? != self.launcher.path
            || Stamp::from(&held_kernel.metadata()?) != before
            || Stamp::from(&File::open(&executable)?.metadata()?) != before
        {
            return Err(refused());
        }
        Ok(())
    }

    pub(crate) fn require_operation_original(
        &self,
        operation: &Arc<Mutex<OperationAuthority>>,
    ) -> Result<(), BackupError> {
        let lease = self.entry.lease.get().ok_or_else(refused)?;
        if !Arc::ptr_eq(&lease.destination_operation, operation) {
            return Err(refused());
        }
        Ok(())
    }

    pub(crate) fn recheck_consuming_original(
        &self,
        original: &OriginalRemoteFinalization,
        owner: &crate::destination::RemoteConsumption<'_>,
    ) -> Result<(), BackupError> {
        original.require_own_local_native(owner, self)?;
        // Preserve actual local code/metadata/deadline checks. Do not require
        // the remote transport/launcher process to remain alive after consume.
        self.recheck_local_facts()?;
        let destination = original.held_destination_for_receipt(owner)?;
        self.entry
            .metadata
            .get()
            .ok_or_else(refused)?
            .lock()
            .map_err(|_| refused())?
            .require_destination(destination)?;
        Ok(())
    }
}

fn device_matches(device: u64, raw: &str) -> bool {
    let Some((major, minor)) = raw.split_once(':') else {
        return false;
    };
    u32::from_str_radix(major, 16).ok() == Some(libc::major(device))
        && u32::from_str_radix(minor, 16).ok() == Some(libc::minor(device))
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionAdmission {
    format_version: u32,
    capability: String,
    profile: String,
    backup_id: Uuid,
    operation_id: Uuid,
    destination_attempt_id: Uuid,
    binding_sha256: String,
    source_auth_frame_sha256: String,
    s_nonce: String,
    d_nonce: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionBindingData {
    source: wire::SourceBinding,
    destination: wire::DestinationBinding,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EffectsTransportIndex {
    schema_version: u32,
    profile: String,
    backup_id: Uuid,
    source_capture_sha256: String,
    files: Vec<FileRecord>,
}

fn transport_index_shape_data(
    value: &EffectsTransportIndex,
    backup: Uuid,
) -> Result<(), BackupError> {
    if value.schema_version != 1
        || value.profile != profile::PROFILE
        || value.backup_id != backup
        || !crate::registry::v4(&value.backup_id.to_string())
        || !crate::valid_digest(&value.source_capture_sha256)
        || value.files.is_empty()
        || value.files.len() > profile::FILES as usize
        || value.files.windows(2).any(|w| w[0].path >= w[1].path)
        || value.files.iter().any(|r| {
            !crate::source::snapshot_effects::wire::path(&r.path)
                || r.size == 0
                || r.size > crate::source::snapshot_effects::wire::MAX_FILE
                || !crate::valid_digest(&r.sha256)
                || matches!(
                    r.path.as_str(),
                    "payload-manifest.json" | "snapshot-effects-transport-index.json"
                )
        })
        || value
            .files
            .iter()
            .filter(|r| r.path == "capture.json" && r.sha256 == value.source_capture_sha256)
            .count()
            != 1
        || value
            .files
            .iter()
            .try_fold(0u64, |n, r| n.checked_add(r.size))
            .is_none_or(|n| n > profile::EVIDENCE_BYTES)
    {
        return Err(refused());
    }
    Ok(())
}

struct HeldMetadataOriginal {
    file: File,
    stamp: Stamp,
    raw: Vec<u8>,
}
impl HeldMetadataOriginal {
    fn read_first(
        dir: &BackupDir,
        name: &str,
        cap: u64,
        entry: &OriginalDEntryScope,
    ) -> Result<Self, BackupError> {
        entry.prepay_wire_work(0, 4)?;
        let mut file = dir.open_file(name)?;
        let stamp = Stamp::from(&file.metadata()?);
        if stamp.mode & libc::S_IFMT != libc::S_IFREG
            || stamp.mode & 0o7777 != 0o600
            || stamp.uid != 0
            || stamp.gid != 0
            || stamp.links != 1
            || stamp.len == 0
            || stamp.len > cap
        {
            return Err(refused());
        }
        entry.prepay_wire_work(
            stamp
                .len
                .checked_mul(4)
                .and_then(|n| n.checked_add(1))
                .ok_or(BackupError::Overflow)?,
            3,
        )?;
        let mut raw = Vec::with_capacity(stamp.len as usize + 1);
        (&mut file).take(stamp.len + 1).read_to_end(&mut raw)?;
        if raw.len() as u64 != stamp.len
            || Stamp::from(&file.metadata()?) != stamp
            || Stamp::from(&dir.open_file(name)?.metadata()?) != stamp
        {
            return Err(refused());
        }
        Ok(Self { file, stamp, raw })
    }
    fn recheck(
        &self,
        dir: &BackupDir,
        name: &str,
        entry: &OriginalDEntryScope,
    ) -> Result<(), BackupError> {
        entry.prepay_wire_work(
            self.stamp.len.checked_mul(3).ok_or(BackupError::Overflow)?,
            5,
        )?;
        if Stamp::from(&self.file.metadata()?) != self.stamp
            || Stamp::from(&dir.open_file(name)?.metadata()?) != self.stamp
        {
            return Err(refused());
        }
        let mut file = &self.file;
        file.seek(SeekFrom::Start(0))?;
        let mut actual = vec![0; self.stamp.len as usize];
        file.read_exact(&mut actual)?;
        let mut extra = [0];
        if file.read(&mut extra)? != 0
            || actual != self.raw
            || Stamp::from(&file.metadata()?) != self.stamp
        {
            return Err(refused());
        }
        Ok(())
    }
}

struct OriginalSessionOperation {
    backup: Uuid,
    directory: BackupDir,
    admission: Option<HeldMetadataOriginal>,
    binding: Option<HeldMetadataOriginal>,
    transport_index: Option<HeldMetadataOriginal>,
}
struct SessionNamespace {
    entry: &'static OriginalDEntryScope,
    root: BackupDir,
    root_identity: (u64, u64),
    lock: Option<File>,
    lock_stamp: Option<Stamp>,
    operations: Option<BackupDir>,
    staging: Option<BackupDir>,
    anchor: Option<HeldMetadataOriginal>,
    history: BTreeMap<String, Vec<FileRecord>>,
    current: Option<OriginalSessionOperation>,
}
impl SessionNamespace {
    fn retain_root(
        entry: &'static OriginalDEntryScope,
        expected: (u64, u64),
    ) -> Result<(), BackupError> {
        entry.prepay_wire_work(0, 12)?;
        let root = BackupDir::open_trusted_private_root(Path::new(SESSION_ROOT))?;
        if root.identity()? != expected {
            return Err(refused());
        }
        let state = Self {
            entry,
            root,
            root_identity: expected,
            lock: None,
            lock_stamp: None,
            operations: None,
            staging: None,
            anchor: None,
            history: BTreeMap::new(),
            current: None,
        };
        if entry.namespace.set(Mutex::new(state)).is_err() {
            return Err(refused());
        }
        let mut state = entry
            .namespace
            .get()
            .ok_or_else(refused)?
            .lock()
            .map_err(|_| refused())?;
        state.open_existing()
    }
    fn names(&self, dir: &BackupDir, maximum: usize) -> Result<Vec<String>, BackupError> {
        crate::registry::prepaid_b1_roster(dir, &self.entry.scan, maximum)
    }
    fn open_existing(&mut self) -> Result<(), BackupError> {
        if self.names(&self.root, 4)? != ["anchor.json", "lock", "operations", "staging"] {
            return Err(refused());
        }
        self.entry.prepay_wire_work(0, 12)?;
        self.lock = Some(self.root.open_file("lock")?);
        let lock = self.lock.as_ref().ok_or_else(refused)?;
        let stamp = Stamp::from(&lock.metadata()?);
        if stamp.uid != 0
            || stamp.gid != 0
            || stamp.mode & 0o7777 != 0o600
            || stamp.links != 1
            || stamp.len != 0
            || stamp.mode & libc::S_IFMT != libc::S_IFREG
            || unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0
        {
            return Err(refused());
        }
        self.lock_stamp = Some(stamp);
        self.anchor = Some(HeldMetadataOriginal::read_first(
            &self.root,
            "anchor.json",
            profile::ANCHOR_BYTES,
            self.entry,
        )?);
        self.operations = Some(self.root.open_dir("operations")?);
        self.staging = Some(self.root.open_dir("staging")?);
        // No new process may adopt an uncertain retained staging subtree.
        if !self
            .names(self.staging.as_ref().ok_or_else(refused)?, 0)?
            .is_empty()
        {
            return Err(refused());
        }
        let names = self.names(
            self.operations.as_ref().ok_or_else(refused)?,
            profile::SESSION_OPERATIONS as usize,
        )?;
        let mut bytes = profile::ANCHOR_BYTES;
        let mut entries = 4usize;
        for name in names {
            if !crate::registry::v4(&name) {
                return Err(refused());
            }
            let directory = self
                .operations
                .as_ref()
                .ok_or_else(refused)?
                .open_dir(&name)?;
            let records = self.inspect_closed_operation(&directory, &name)?;
            bytes = records.iter().try_fold(bytes, |n, r| {
                n.checked_add(r.size).ok_or(BackupError::Overflow)
            })?;
            entries = entries
                .checked_add(1 + records.len())
                .ok_or(BackupError::Overflow)?;
            if entries as u64 > profile::SESSION_ENTRIES || bytes > profile::SESSION_METADATA_BYTES
            {
                return Err(refused());
            }
            self.entry.prepay_wire_work(
                (name.len() as u64) + (records.len() as u64) * 256,
                records.len(),
            )?;
            self.history.insert(name, records);
        }
        self.recheck_retained()
    }
    fn inspect_closed_operation(
        &self,
        dir: &BackupDir,
        name: &str,
    ) -> Result<Vec<FileRecord>, BackupError> {
        dir.require_private_directory()?;
        if self.names(dir, 3)?
            != [
                "admission.json",
                "binding.json",
                "snapshot-effects-transport-index.json",
            ]
        {
            return Err(refused());
        }
        let admission = HeldMetadataOriginal::read_first(
            dir,
            "admission.json",
            profile::SESSION_ADMISSION_BYTES,
            self.entry,
        )?;
        let binding = HeldMetadataOriginal::read_first(
            dir,
            "binding.json",
            profile::SESSION_BINDING_BYTES,
            self.entry,
        )?;
        let transport = HeldMetadataOriginal::read_first(
            dir,
            "snapshot-effects-transport-index.json",
            profile::SESSION_TRANSPORT_INDEX_BYTES,
            self.entry,
        )?;
        self.entry.prepay_wire_work(
            (admission.raw.len() as u64 + binding.raw.len() as u64 + transport.raw.len() as u64)
                .checked_mul(32)
                .ok_or(BackupError::Overflow)?,
            3,
        )?;
        crate::registry::prepay_current_b1_json(&self.entry.scan, &admission.raw)?;
        crate::registry::prepay_current_b1_json(&self.entry.scan, &binding.raw)?;
        crate::registry::prepay_current_b1_json(&self.entry.scan, &transport.raw)?;
        let a: SessionAdmission =
            crate::registry::canonical_record(&admission.raw, profile::SESSION_ADMISSION_BYTES)?;
        let b: SessionBindingData =
            crate::registry::canonical_record(&binding.raw, profile::SESSION_BINDING_BYTES)?;
        let t: EffectsTransportIndex = crate::registry::canonical_record(
            &transport.raw,
            profile::SESSION_TRANSPORT_INDEX_BYTES,
        )?;
        transport_index_shape_data(&t, a.backup_id)?;
        if a.format_version != 1
            || a.capability != "task9_b1_session_admission_v1"
            || a.profile != profile::PROFILE
            || a.backup_id.to_string() != name
            || !crate::registry::v4(&a.operation_id.to_string())
            || !crate::registry::v4(&a.destination_attempt_id.to_string())
            || !b.source.valid()
            || !b.destination.valid()
            || b.source.backup_id != name
            || b.destination.attempt_id != a.destination_attempt_id.to_string()
            || a.binding_sha256 != crate::digest(&binding.raw)
            || [
                &a.binding_sha256,
                &a.source_auth_frame_sha256,
                &a.s_nonce,
                &a.d_nonce,
            ]
            .iter()
            .any(|v| !crate::valid_digest(v))
            || t.schema_version != 1
            || t.profile != profile::PROFILE
            || t.backup_id != a.backup_id
            || crate::digest(&transport.raw) != b.source.snapshot_effects_transport_index_sha256
        {
            return Err(refused());
        }
        Ok([
            ("admission.json", admission.raw),
            ("binding.json", binding.raw),
            ("snapshot-effects-transport-index.json", transport.raw),
        ]
        .into_iter()
        .map(|(path, raw)| FileRecord {
            path: path.into(),
            size: raw.len() as u64,
            sha256: crate::digest(&raw),
        })
        .collect())
    }
    fn recheck_retained(&self) -> Result<(), BackupError> {
        self.entry.prepay_wire_work(0, 14)?;
        self.root.require_private_directory()?;
        if self.root.identity()? != self.root_identity
            || BackupDir::open_trusted_private_root(Path::new(SESSION_ROOT))?.identity()?
                != self.root_identity
            || self.names(&self.root, 4)? != ["anchor.json", "lock", "operations", "staging"]
        {
            return Err(refused());
        }
        let ops = self.operations.as_ref().ok_or_else(refused)?;
        if self.root.open_dir("operations")?.identity()? != ops.identity()?
            || self.root.open_dir("staging")?.identity()?
                != self.staging.as_ref().ok_or_else(refused)?.identity()?
            || !self
                .names(self.staging.as_ref().ok_or_else(refused)?, 0)?
                .is_empty()
        {
            return Err(refused());
        }
        let lock = self.lock.as_ref().ok_or_else(refused)?;
        if Some(Stamp::from(&lock.metadata()?)) != self.lock_stamp
            || Some(Stamp::from(&self.root.open_file("lock")?.metadata()?)) != self.lock_stamp
        {
            return Err(refused());
        }
        self.anchor
            .as_ref()
            .ok_or_else(refused)?
            .recheck(&self.root, "anchor.json", self.entry)?;
        let mut expected = self.history.keys().cloned().collect::<Vec<_>>();
        if let Some(current) = &self.current {
            expected.push(current.backup.to_string());
        }
        expected.sort();
        if self.names(ops, profile::SESSION_OPERATIONS as usize)? != expected {
            return Err(refused());
        }
        for (name, expected) in &self.history {
            if self.inspect_closed_operation(&ops.open_dir(name)?, name)? != *expected {
                return Err(refused());
            }
        }
        if let Some(current) = &self.current {
            if ops.open_dir(&current.backup.to_string())?.identity()?
                != current.directory.identity()?
            {
                return Err(refused());
            }
            let mut wanted = vec!["admission.json".to_owned(), "binding.json".to_owned()];
            if current.transport_index.is_some() {
                wanted.push("snapshot-effects-transport-index.json".into());
            }
            if self.names(&current.directory, 3)? != wanted {
                return Err(refused());
            }
            current.admission.as_ref().ok_or_else(refused)?.recheck(
                &current.directory,
                "admission.json",
                self.entry,
            )?;
            current.binding.as_ref().ok_or_else(refused)?.recheck(
                &current.directory,
                "binding.json",
                self.entry,
            )?;
            if let Some(t) = &current.transport_index {
                t.recheck(
                    &current.directory,
                    "snapshot-effects-transport-index.json",
                    self.entry,
                )?;
            }
        }
        Ok(())
    }
    fn admit_original(
        &mut self,
        auth: &wire::SourceAuth,
        source_auth_frame: &str,
        operation: Uuid,
    ) -> Result<(), BackupError> {
        self.recheck_retained()?;
        let backup = Uuid::parse_str(&auth.source.backup_id).map_err(|_| refused())?;
        if self.current.is_some()
            || self.history.contains_key(&backup.to_string())
            || self.history.len() as u64 >= profile::SESSION_OPERATIONS
        {
            return Err(refused());
        }
        self.entry
            .prepay_wire_work(2 * profile::SESSION_BINDING_BYTES * 32, 12)?;
        let binding = SessionBindingData {
            source: auth.source.clone(),
            destination: auth.destination.clone(),
        };
        let binding_raw = crate::registry::canonical(&binding)?;
        if crate::digest(&binding_raw) != auth.binding_sha256
            || binding_raw.len() as u64 > profile::SESSION_BINDING_BYTES
        {
            return Err(refused());
        }
        let admission = SessionAdmission {
            format_version: 1,
            capability: "task9_b1_session_admission_v1".into(),
            profile: profile::PROFILE.into(),
            backup_id: backup,
            operation_id: operation,
            destination_attempt_id: Uuid::parse_str(&auth.destination.attempt_id)
                .map_err(|_| refused())?,
            binding_sha256: auth.binding_sha256.clone(),
            source_auth_frame_sha256: source_auth_frame.into(),
            s_nonce: auth.s_nonce.clone(),
            d_nonce: auth.d_nonce.clone(),
        };
        let admission_raw = crate::registry::canonical(&admission)?;
        if admission_raw.len() as u64 > profile::SESSION_ADMISSION_BYTES {
            return Err(refused());
        }
        let reserved_bytes = self
            .history
            .values()
            .flatten()
            .try_fold(profile::ANCHOR_BYTES, |n, r| {
                n.checked_add(r.size).ok_or(BackupError::Overflow)
            })?
            .checked_add(admission_raw.len() as u64)
            .and_then(|n| n.checked_add(binding_raw.len() as u64))
            .and_then(|n| n.checked_add(profile::SESSION_TRANSPORT_INDEX_BYTES))
            .ok_or(BackupError::Overflow)?;
        if reserved_bytes > profile::SESSION_METADATA_BYTES
            || (self.history.len() as u64 + 1) * 4 + 4 > profile::SESSION_ENTRIES
        {
            return Err(refused());
        }
        let ops = self.operations.as_ref().ok_or_else(refused)?;
        self.current = Some(OriginalSessionOperation {
            backup,
            directory: ops.create_dir(&backup.to_string())?,
            admission: None,
            binding: None,
            transport_index: None,
        });
        ops.sync()?;
        let current = self.current.as_mut().ok_or_else(refused)?;
        for (name, raw, cap, which) in [
            (
                "admission.json",
                admission_raw,
                profile::SESSION_ADMISSION_BYTES,
                true,
            ),
            (
                "binding.json",
                binding_raw,
                profile::SESSION_BINDING_BYTES,
                false,
            ),
        ] {
            let mut file = current.directory.create_file(name)?;
            file.write_all(&raw)?;
            file.sync_all()?;
            current.directory.sync()?;
            let held = HeldMetadataOriginal::read_first(&current.directory, name, cap, self.entry)?;
            if held.raw != raw {
                return Err(refused());
            }
            if which {
                current.admission = Some(held);
            } else {
                current.binding = Some(held);
            }
        }
        ops.sync()?;
        self.root.sync()?;
        self.recheck_retained()
    }
    fn store_original_transport(&mut self, raw: &[u8]) -> Result<(), BackupError> {
        self.recheck_retained()?;
        if raw.is_empty() || raw.len() as u64 > profile::SESSION_TRANSPORT_INDEX_BYTES {
            return Err(refused());
        }
        self.entry.prepay_wire_work(
            (raw.len() as u64)
                .checked_mul(4)
                .ok_or(BackupError::Overflow)?,
            12,
        )?;
        let current = self.current.as_mut().ok_or_else(refused)?;
        if current.transport_index.is_some() {
            return Err(refused());
        }
        let name = "snapshot-effects-transport-index.json";
        let mut file = current.directory.create_file(name)?;
        file.write_all(raw)?;
        file.sync_all()?;
        current.directory.sync()?;
        let held = HeldMetadataOriginal::read_first(
            &current.directory,
            name,
            profile::SESSION_TRANSPORT_INDEX_BYTES,
            self.entry,
        )?;
        if held.raw != raw {
            return Err(refused());
        }
        current.transport_index = Some(held);
        self.operations.as_ref().ok_or_else(refused)?.sync()?;
        self.root.sync()?;
        self.recheck_retained()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OriginalPhase {
    Hello,
    Receiving,
    Ready,
    Consuming,
    Committed,
    Acknowledged,
    Ended,
    Failed,
}

struct DestinationSessionState {
    destination: Arc<EnrolledDestination>,
    operation_label: Uuid,
    attempt: Uuid,
    phase: OriginalPhase,
    source_key: [u8; 32],
    source_key_sha256: String,
    authenticated: Option<Arc<OriginalAuthenticatedSourceInD>>,
    transfer: Option<IncomingOriginalFiles>,
    prepared: Option<Arc<crate::destination::PreparedRemoteDestinationOriginal>>,
    committed: Option<crate::destination::CommittedRemoteOriginal>,
}

/// A borrow of the one actual static Book. Dropping/cancelling this borrow does
/// not destroy the native/key/lease/stdio/in-flight IO or payload-file owners.
pub(super) struct OriginalDestinationSession<'book> {
    entry: &'static OriginalDEntryScope,
    state: tokio::sync::MutexGuard<'book, Option<DestinationSessionState>>,
}
impl Drop for OriginalDestinationSession<'_> {
    fn drop(&mut self) {
        if let Some(state) = self.state.as_mut()
            && !matches!(
                state.phase,
                OriginalPhase::Committed | OriginalPhase::Acknowledged | OriginalPhase::Ended
            )
        {
            state.phase = OriginalPhase::Failed;
            self.entry.reject();
        }
        // Static Book slots, including OriginalPipeIo offsets and real FD
        // owners, remain in place. No clear/recreate, detached task or kill.
    }
}

pub(super) fn run_fixed_destination_entry() -> Result<(), BackupError> {
    let entry = OriginalDEntryScope::begin_fixed()?;
    let prepared = (|| {
        let pipes = wire::retain_original_destination_stdio(entry)?;
        entry.pipes.set(pipes).map_err(|_| refused())?;
        let registry = ManagementRegistry::open_installed_destination_original(entry)?;
        entry
            .lease
            .set(registry.try_lock()?)
            .map_err(|_| refused())?;
        let lease = entry.lease.get().ok_or_else(refused)?;
        let held = policy::load_fixed_destination_metadata(lease)?;
        entry
            .metadata
            .set(Mutex::new(held))
            .map_err(|_| refused())?;
        let mut held = entry
            .metadata
            .get()
            .ok_or_else(refused)?
            .lock()
            .map_err(|_| refused())?;
        held.recheck()?;
        let data = held.destination_data()?;
        let native = NativeDestinationLocalCustody::admit_fixed(entry, &data)?;
        entry.native.set(Arc::new(native)).map_err(|_| refused())?;
        let data = held.destination_data()?;
        let signature = data.signature_data_pins();
        let (_, group_id, _, _) = signature.destination_data();
        let group = match lease.groups.get(group_id) {
            Some(crate::registry::Group::Destination(group)) => group,
            _ => return Err(refused()),
        };
        let path = &lease.roots[&group.roots["destination"]].path;
        entry.prepay_wire_work(0, 24)?;
        let destination = Arc::new(lease.admit_destination(Path::new(path))?);
        held.require_destination(&destination)?;
        let data = held.destination_data()?;
        let signature = data.signature_data_pins();
        let (key_hex, key_sha) = signature.public_key_data();
        entry.prepay_wire_work(512, 3)?;
        let key: [u8; 32] = hex::decode(key_hex)
            .map_err(|_| refused())?
            .try_into()
            .map_err(|_| refused())?;
        if crate::digest(&key) != key_sha {
            return Err(refused());
        }
        let key_sha = key_sha.to_owned();
        let namespace_id = data.session_root_identity_data();
        let anchor_hash = data.anchor_sha256_data().to_owned();
        entry
            .destination
            .set(Arc::clone(&destination))
            .map_err(|_| refused())?;
        drop(held);
        let signer =
            crate::destination::B1DestinationSignerOriginal::open_fixed_for_original(entry)?;
        entry.signer.set(signer).map_err(|_| refused())?;
        SessionNamespace::retain_root(entry, namespace_id)?;
        {
            let namespace = entry
                .namespace
                .get()
                .ok_or_else(refused)?
                .lock()
                .map_err(|_| refused())?;
            if crate::digest(&namespace.anchor.as_ref().ok_or_else(refused)?.raw) != anchor_hash {
                return Err(refused());
            }
        }
        entry.prepay_wire_work(128, 3)?;
        let operation_label = Uuid::new_v4();
        let attempt = Uuid::new_v4();
        Ok(DestinationSessionState {
            destination,
            operation_label,
            attempt,
            phase: OriginalPhase::Hello,
            source_key: key,
            source_key_sha256: key_sha,
            authenticated: None,
            transfer: None,
            prepared: None,
            committed: None,
        })
    })();
    let state = match prepared {
        Ok(state) => state,
        Err(error) => {
            entry.reject();
            return Err(error);
        }
    };
    let mut slot = entry.session.try_lock().map_err(|_| refused())?;
    if slot.is_some() {
        entry.reject();
        return Err(refused());
    }
    *slot = Some(state);
    let mut original = OriginalDestinationSession { entry, state: slot };
    original.recheck()?;
    // The only runtime is created after native/stdio/lease/key admission and
    // after its actual kernel/allocator work was charged to the original meter.
    // Its owned reactor/timer descriptors also stay in the original static Book.
    entry.prepay_wire_work(1_048_576, 64)?;
    entry.require_fixed_fd_room(FixedFdWork::Runtime)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    entry.runtime.set(runtime).map_err(|_| refused())?;
    entry.require_fixed_fd_room(FixedFdWork::Observe)?;
    entry
        .runtime
        .get()
        .ok_or_else(refused)?
        .block_on(wire::run_original_destination(&mut original))
}

impl<'book> OriginalDestinationSession<'book> {
    fn current(&self) -> Result<&DestinationSessionState, BackupError> {
        self.state.as_ref().ok_or_else(refused)
    }
    fn current_mut(&mut self) -> Result<&mut DestinationSessionState, BackupError> {
        self.state.as_mut().ok_or_else(refused)
    }
    pub(super) fn original_wire_io_slot(
        &self,
    ) -> &'static tokio::sync::Mutex<Option<wire::OriginalPipeIo>> {
        &self.entry.io
    }
    pub(super) fn original_pipe_pair(
        &self,
    ) -> Result<&'static wire::OriginalPipePair, BackupError> {
        self.entry.pipes.get().copied().ok_or_else(refused)
    }
    pub(super) fn recheck(&mut self) -> Result<(), BackupError> {
        let state = self.current()?;
        if matches!(
            state.phase,
            OriginalPhase::Failed | OriginalPhase::Consuming
        ) {
            return Err(refused());
        }
        if matches!(
            state.phase,
            OriginalPhase::Committed | OriginalPhase::Acknowledged | OriginalPhase::Ended
        ) {
            return state
                .committed
                .as_ref()
                .ok_or_else(refused)?
                .recheck_local_committed();
        }
        self.entry.recheck()?;
        self.entry
            .native
            .get()
            .ok_or_else(refused)?
            .recheck_pending()?;
        let mut held = self
            .entry
            .metadata
            .get()
            .ok_or_else(refused)?
            .lock()
            .map_err(|_| refused())?;
        held.require_destination(&state.destination)?;
        state.destination.recheck()?;
        if let Some(authenticated) = &state.authenticated {
            authenticated.recheck_live()?;
        }
        Ok(())
    }
    pub(super) fn hello_data(&mut self) -> Result<wire::DestinationBinding, BackupError> {
        self.recheck()?;
        let state = self.current()?;
        if state.phase != OriginalPhase::Hello {
            return Err(refused());
        }
        self.entry.prepay_wire_work(2 * 16_384 * 32, 12)?;
        let mut roots = Vec::with_capacity(2);
        for role in ["destination", "evidence"] {
            let record = &state.destination.lease.roots[&state.destination.group.roots[role]];
            let raw = crate::registry::canonical(record)?;
            roots.push(wire::RootRecord {
                role: role.into(),
                enrollment_id: record.enrollment_id.clone(),
                path: record.path.clone(),
                dev: record.dev,
                ino: record.ino,
                uid: record.uid,
                mode: record.mode,
                record_sha256: crate::digest(&raw),
            });
        }
        Ok(wire::DestinationBinding {
            deployment_id: state.destination.group.deployment_id.clone(),
            destination_group_id: state.destination.group.group_id.clone(),
            attempt_id: state.attempt.to_string(),
            registry_authority_sha256: crate::digest(&crate::registry::canonical(
                &state.destination.lease.authority,
            )?),
            registry_generation: state.destination.lease.generation().generation,
            generation_sha256: crate::digest(&crate::registry::canonical(
                state.destination.lease.generation(),
            )?),
            receiver_release_sha256: self
                .entry
                .native
                .get()
                .ok_or_else(refused)?
                .executable
                .sha256
                .clone(),
            roots,
        })
    }
    pub(super) fn wire_identity(
        &mut self,
    ) -> Result<OriginalDWireIdentity<'_, 'book>, BackupError> {
        self.recheck()?;
        let mut metadata = self
            .entry
            .metadata
            .get()
            .ok_or_else(refused)?
            .lock()
            .map_err(|_| refused())?;
        metadata.require_destination(&self.current()?.destination)?;
        Ok(OriginalDWireIdentity {
            owner: self,
            metadata: Some(metadata),
        })
    }
    pub(super) fn admit_authenticated_source(
        &mut self,
        verified: wire::VerifiedSourceAuthInOriginalD,
    ) -> Result<(), BackupError> {
        verified.require_original(self)?;
        self.recheck()?;
        if self.current()?.phase != OriginalPhase::Hello {
            return Err(refused());
        }
        self.entry
            .prepay_wire_work(profile::CONTROL_BYTES * 32, 10)?;
        let auth = verified.control_data();
        if auth.destination != self.hello_data()? {
            return Err(refused());
        }
        let source_raw = crate::registry::canonical(&auth.source)?;
        let source: wire::SourceBinding = serde_json::from_slice(&source_raw)?;
        let destination_raw = crate::registry::canonical(&auth.destination)?;
        let destination: wire::DestinationBinding = serde_json::from_slice(&destination_raw)?;
        let operation = self.current()?.operation_label;
        let mut namespace = self
            .entry
            .namespace
            .get()
            .ok_or_else(refused)?
            .lock()
            .map_err(|_| refused())?;
        // Durable negative admission precedes the first package/evidence write.
        namespace.admit_original(auth, verified.frame_sha256_data(), operation)?;
        let original = Arc::new(OriginalAuthenticatedSourceInD {
            entry: self.entry,
            destination: Arc::clone(&self.current()?.destination),
            source,
            destination_binding: destination,
            s_nonce: auth.s_nonce.clone(),
            d_nonce: auth.d_nonce.clone(),
            binding_sha256: auth.binding_sha256.clone(),
            source_auth_frame_sha256: verified.frame_sha256_data().to_owned(),
            _verified: verified,
        });
        drop(namespace);
        let transfer = IncomingOriginalFiles::start(Arc::clone(&original))?;
        let state = self.current_mut()?;
        state.authenticated = Some(original);
        state.transfer = Some(transfer);
        state.phase = OriginalPhase::Receiving;
        Ok(())
    }
    pub(super) fn ready_witness_bytes(&self) -> Result<&[u8], BackupError> {
        self.current()?
            .prepared
            .as_ref()
            .ok_or_else(refused)?
            .ready_witness_bytes()
    }
    pub(super) fn ready_intent_bytes(&self) -> Result<&[u8], BackupError> {
        self.current()?
            .prepared
            .as_ref()
            .ok_or_else(refused)?
            .ready_intent_bytes()
    }
    pub(super) fn committed_receipt_bytes(&self) -> Result<&[u8], BackupError> {
        Ok(self
            .current()?
            .committed
            .as_ref()
            .ok_or_else(refused)?
            .receipt_data())
    }
}

/// Metadata-only view over actual held inputs and the actual operation Arc.
/// It has no constructor outside this module and supplies no signing authority.
pub(super) struct OriginalDWireIdentity<'view, 'book> {
    owner: &'view mut OriginalDestinationSession<'book>,
    metadata: Option<std::sync::MutexGuard<'static, policy::HeldDestinationInputs<'static>>>,
}
impl OriginalDWireIdentity<'_, '_> {
    pub(super) fn recheck(&mut self) -> Result<(), BackupError> {
        // Only release the lock borrow, never the original held metadata FDs.
        // The common actual owner then selects Pending or private Committed
        // local custody from its real phase; no caller-controlled bypass flag.
        drop(self.metadata.take());
        self.owner.recheck()?;
        let mut metadata = self
            .owner
            .entry
            .metadata
            .get()
            .ok_or_else(refused)?
            .lock()
            .map_err(|_| refused())?;
        metadata.require_destination(&self.owner.current()?.destination)?;
        self.metadata = Some(metadata);
        Ok(())
    }
    pub(super) fn operation_original(&self) -> &Arc<Mutex<OperationAuthority>> {
        &self
            .owner
            .current()
            .expect("original Book state")
            .destination
            .lease
            .destination_operation
    }
    pub(super) fn source_verifier_public_key_data(&self) -> &[u8; 32] {
        &self
            .owner
            .current()
            .expect("original Book state")
            .source_key
    }
    pub(super) fn source_auth_key_sha256_data(&self) -> &str {
        &self
            .owner
            .current()
            .expect("original Book state")
            .source_key_sha256
    }
    pub(super) fn signature_policy_data(
        &self,
    ) -> Result<policy::CandidateDestinationSignaturePins<'_>, BackupError> {
        Ok(self
            .metadata
            .as_ref()
            .ok_or_else(refused)?
            .destination_data()?
            .signature_data_pins())
    }
    pub(super) fn entry_original(&self) -> &'static OriginalDEntryScope {
        self.owner.entry
    }
    pub(super) fn operation_deadline_data(&self) -> Instant {
        self.owner.entry.deadline
    }
    pub(super) fn prepay_wire_work(&self, bytes: u64, entries: usize) -> Result<(), BackupError> {
        self.owner.entry.prepay_wire_work(bytes, entries)
    }
}

/// Only the same native D consumes the opaque cryptographically verified token
/// and makes this owner after its durable exclusive negative admission. A DTO,
/// policy hash, copied local proof or current registry cannot construct it.
pub(crate) struct OriginalAuthenticatedSourceInD {
    entry: &'static OriginalDEntryScope,
    destination: Arc<EnrolledDestination>,
    source: wire::SourceBinding,
    destination_binding: wire::DestinationBinding,
    s_nonce: String,
    d_nonce: String,
    binding_sha256: String,
    source_auth_frame_sha256: String,
    _verified: wire::VerifiedSourceAuthInOriginalD,
}
impl OriginalAuthenticatedSourceInD {
    pub(crate) fn recheck_live(&self) -> Result<(), BackupError> {
        self.entry.recheck()?;
        self.entry
            .native
            .get()
            .ok_or_else(refused)?
            .recheck_pending()?;
        self.destination.recheck()?;
        self.entry
            .namespace
            .get()
            .ok_or_else(refused)?
            .lock()
            .map_err(|_| refused())?
            .recheck_retained()?;
        Ok(())
    }
    pub(crate) fn require_destination(
        &self,
        destination: &EnrolledDestination,
    ) -> Result<(), BackupError> {
        self.recheck_live()?;
        if !std::ptr::eq(self.destination.as_ref(), destination)
            || !Arc::ptr_eq(
                &self.destination.lease.destination_operation,
                &destination.lease.destination_operation,
            )
            || !Arc::ptr_eq(&self.entry.scan, &destination.lease.budget)
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(crate) fn destination_original(&self) -> &EnrolledDestination {
        &self.destination
    }
    pub(crate) fn prepay_original_work(
        &self,
        bytes: u64,
        entries: usize,
    ) -> Result<(), BackupError> {
        self.entry.prepay_wire_work(bytes, entries)
    }
    pub(crate) fn backup_id_data(&self) -> Result<Uuid, BackupError> {
        Uuid::parse_str(&self.source.backup_id).map_err(|_| refused())
    }
    pub(crate) fn destination_attempt_data(&self) -> Result<Uuid, BackupError> {
        Uuid::parse_str(&self.destination_binding.attempt_id).map_err(|_| refused())
    }
    pub(crate) fn snapshot_effects_input_sha256_data(&self) -> &str {
        &self.source.snapshot_effects_input_sha256
    }
}

fn package_member_path_data(record: &FileRecord) -> bool {
    matches!(
        record.path.as_str(),
        "asset-index.json" | "database.dump" | "roles.json"
    ) || crate::valid_digest(&record.sha256)
        && record.path == format!("assets/sha256/{}/{}", &record.sha256[..2], record.sha256)
}

struct CurrentIncomingFile {
    number: u32,
    role: wire::FileRole,
    index: u32,
    parent: BackupDir,
    name: String,
    file: File,
    identity: (u64, u64),
    record: FileRecord,
    offset: u64,
    hash: sha2::Sha256,
}

/// Actual inbound FD owners retained in the original Book across every await.
/// Its names come only from fixed roles, the exact manifest, authenticated root
/// IDs or the validated source-only transport index; no peer path is opened.
struct IncomingOriginalFiles {
    original: Arc<OriginalAuthenticatedSourceInD>,
    package: BackupDir,
    input_root: BackupDir,
    input_directory: BackupDir,
    source_directory: BackupDir,
    effects_directory: BackupDir,
    package_stage: String,
    input_stage: String,
    manifest: Option<crate::BackupManifestV1>,
    manifest_raw: Option<Vec<u8>>,
    transport: Option<EffectsTransportIndex>,
    transport_raw: Option<Vec<u8>>,
    package_records: Vec<FileRecord>,
    source_records: Vec<FileRecord>,
    effect_records: Vec<FileRecord>,
    source_names: Vec<String>,
    current: Option<CurrentIncomingFile>,
    file_count: u32,
    package_bytes: u64,
    evidence_bytes: u64,
}
impl IncomingOriginalFiles {
    fn start(original: Arc<OriginalAuthenticatedSourceInD>) -> Result<Self, BackupError> {
        original.recheck_live()?;
        original
            .entry
            .require_fixed_fd_room(FixedFdWork::FlowLayout)?;
        original.prepay_original_work(16_384, 32)?;
        let id = original.backup_id_data()?;
        let attempt = original.destination_attempt_data()?;
        let destination = original.destination_original();
        let names = crate::registry::prepaid_b1_roster(
            destination.destination(),
            &original.entry.scan,
            crate::registry::MAX_ENTRIES,
        )?;
        if names.iter().any(|n| {
            n == &format!("{id}.sealed")
                || n == &id.to_string()
                || n.starts_with(&format!("{id}.staging-"))
        }) {
            return Err(refused());
        }
        let package_stage = format!("{id}.staging-{attempt}");
        let package = destination.destination().create_dir(&package_stage)?;
        destination.destination().sync()?;
        let input_root = match destination.evidence().kind(".b1-inputs") {
            Ok(BackupEntryKind::Directory) => destination.evidence().open_dir(".b1-inputs")?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let dir = destination.evidence().create_dir(".b1-inputs")?;
                destination.evidence().sync()?;
                dir
            }
            _ => return Err(refused()),
        };
        let names = crate::registry::prepaid_b1_roster(
            &input_root,
            &original.entry.scan,
            crate::registry::MAX_ENTRIES,
        )?;
        if names
            .iter()
            .any(|n| n == &id.to_string() || n.starts_with(&format!("{id}.")))
        {
            return Err(refused());
        }
        let input_stage = format!("{id}.staging-{attempt}");
        let input_directory = input_root.create_dir(&input_stage)?;
        input_root.sync()?;
        let source_directory = input_directory.create_dir("source")?;
        let effects_directory = input_directory.create_dir("effects")?;
        input_directory.sync()?;
        let mut source_names = [
            "protection-catalog.json",
            "protection-pending.json",
            "protection-retained.json",
            "protection-snapshot.json",
            "source-authority.json",
            "source-generation.json",
            "source-group.json",
            "source-released.json",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
        source_names.extend(
            original
                .source
                .roots
                .iter()
                .map(|r| format!("roots/{}.json", r.enrollment_id)),
        );
        source_names.sort();
        if source_names.len() != 12 || source_names.windows(2).any(|w| w[0] >= w[1]) {
            return Err(refused());
        }
        Ok(Self {
            original,
            package,
            input_root,
            input_directory,
            source_directory,
            effects_directory,
            package_stage,
            input_stage,
            manifest: None,
            manifest_raw: None,
            transport: None,
            transport_raw: None,
            package_records: Vec::new(),
            source_records: Vec::new(),
            effect_records: Vec::new(),
            source_names,
            current: None,
            file_count: 0,
            package_bytes: 0,
            evidence_bytes: 0,
        })
    }
    fn stage_parent(
        root: &BackupDir,
        path: &str,
        original: &OriginalAuthenticatedSourceInD,
    ) -> Result<(BackupDir, String), BackupError> {
        if !crate::source::snapshot_effects::wire::path(path) {
            return Err(refused());
        }
        original.prepay_original_work((path.len() as u64) * 3, path.split('/').count() * 4 + 1)?;
        let mut parent = root.try_clone()?;
        let mut parts = path.split('/').peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                return Ok((parent, part.to_owned()));
            }
            parent = match parent.kind(part) {
                Ok(BackupEntryKind::Directory) => parent.open_dir(part)?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let dir = parent.create_dir(part)?;
                    parent.sync()?;
                    dir
                }
                _ => return Err(refused()),
            };
            parent.require_private_directory()?;
        }
        Err(refused())
    }
    fn expected_path(
        &self,
        c: &wire::FileBegin,
    ) -> Result<(String, Option<FileRecord>), BackupError> {
        use wire::FileRole;
        let index = usize::try_from(c.roster_index).map_err(|_| refused())?;
        match c.role {
            FileRole::PackageManifest
                if self.file_count == 0 && index == 0 && self.manifest.is_none() =>
            {
                Ok(("manifest.json".into(), None))
            }
            FileRole::PackageMember
                if self.source_records.is_empty() && self.transport.is_none() =>
            {
                let manifest = self.manifest.as_ref().ok_or_else(refused)?;
                if index + 1 != self.package_records.len() {
                    return Err(refused());
                }
                let record = manifest.files.get(index).ok_or_else(refused)?;
                Ok((record.path.clone(), Some(record.clone())))
            }
            FileRole::SourceEvidenceMember
                if self.transport.is_none()
                    && self.package_records.len()
                        == self.manifest.as_ref().ok_or_else(refused)?.files.len() + 1
                    && index == self.source_records.len() =>
            {
                Ok((
                    self.source_names.get(index).ok_or_else(refused)?.clone(),
                    None,
                ))
            }
            FileRole::SnapshotEffectsTransportIndex
                if index == 0 && self.source_records.len() == 12 && self.transport.is_none() =>
            {
                Ok(("snapshot-effects-transport-index.json".into(), None))
            }
            FileRole::SnapshotEffectsMember
                if self.transport.is_some() && index == self.effect_records.len() =>
            {
                let record = self
                    .transport
                    .as_ref()
                    .ok_or_else(refused)?
                    .files
                    .get(index)
                    .ok_or_else(refused)?;
                Ok((record.path.clone(), Some(record.clone())))
            }
            _ => Err(refused()),
        }
    }
    fn begin(&mut self, c: &wire::FileBegin) -> Result<(), BackupError> {
        self.original.recheck_live()?;
        self.original
            .entry
            .require_fixed_fd_room(FixedFdWork::PayloadCreate)?;
        if self.current.is_some()
            || c.file_no != self.file_count
            || c.file_no >= self.original.source.file_count
            || u64::from(c.file_no) >= profile::FILES
            || c.size == 0
            || !crate::valid_digest(&c.sha256)
        {
            return Err(refused());
        }
        self.original.prepay_original_work(4096, 16)?;
        let (path, expected) = self.expected_path(c)?;
        // Preserve the old exact12 per-name cap before opening/writing input.
        if c.role == wire::FileRole::SourceEvidenceMember
            && c.size > crate::destination::source_evidence_file_cap_data(&path)
        {
            return Err(refused());
        }
        if expected
            .as_ref()
            .is_some_and(|r| r.size != c.size || r.sha256 != c.sha256)
        {
            return Err(refused());
        }
        let package = matches!(
            c.role,
            wire::FileRole::PackageManifest | wire::FileRole::PackageMember
        );
        let size = if package {
            &mut self.package_bytes
        } else {
            &mut self.evidence_bytes
        };
        let total = size.checked_add(c.size).ok_or(BackupError::Overflow)?;
        let max = if package {
            self.original
                .source
                .package_bytes
                .min(profile::PACKAGE_BYTES)
        } else {
            self.original
                .source
                .evidence_bytes
                .min(profile::EVIDENCE_BYTES)
        };
        if total > max {
            return Err(refused());
        }
        if !package && c.size > crate::source::snapshot_effects::wire::MAX_FILE
            || c.role == wire::FileRole::SnapshotEffectsTransportIndex
                && c.size > profile::SESSION_TRANSPORT_INDEX_BYTES
            || c.role == wire::FileRole::PackageManifest && c.size > 64 * 1024 * 1024
        {
            return Err(refused());
        }
        *size = total;
        let root = match c.role {
            wire::FileRole::PackageManifest | wire::FileRole::PackageMember => &self.package,
            wire::FileRole::SourceEvidenceMember => &self.source_directory,
            wire::FileRole::SnapshotEffectsTransportIndex => &self.input_directory,
            wire::FileRole::SnapshotEffectsMember => &self.effects_directory,
        };
        let (parent, name) = Self::stage_parent(root, &path, &self.original)?;
        let file = parent.create_file(&name)?;
        let m = file.metadata()?;
        if m.uid() != 0
            || m.gid() != 0
            || m.nlink() != 1
            || m.mode() & 0o7777 != 0o600
            || m.len() != 0
        {
            return Err(refused());
        }
        self.current = Some(CurrentIncomingFile {
            number: c.file_no,
            role: c.role,
            index: c.roster_index,
            parent,
            name,
            file,
            identity: (m.dev(), m.ino()),
            record: FileRecord {
                path,
                size: c.size,
                sha256: c.sha256.clone(),
            },
            offset: 0,
            hash: sha2::Sha256::default(),
        });
        Ok(())
    }
    fn chunk(&mut self, c: &wire::FileChunk, body: &[u8]) -> Result<(), BackupError> {
        use sha2::Digest;
        self.original.entry.recheck()?;
        self.original
            .entry
            .require_fixed_fd_room(FixedFdWork::Readback)?;
        if body.is_empty() || body.len() as u64 > profile::CHUNK_BODY_BYTES {
            return Err(refused());
        }
        self.original.prepay_original_work(
            (body.len() as u64)
                .checked_mul(2)
                .ok_or(BackupError::Overflow)?,
            3,
        )?;
        let current = self.current.as_mut().ok_or_else(refused)?;
        let end = current
            .offset
            .checked_add(body.len() as u64)
            .ok_or(BackupError::Overflow)?;
        let m = current.file.metadata()?;
        if current.number != c.file_no
            || current.offset != c.offset
            || end > current.record.size
            || (m.dev(), m.ino()) != current.identity
            || m.nlink() != 1
            || m.uid() != 0
            || m.gid() != 0
            || m.mode() & 0o7777 != 0o600
            || m.len() != current.offset
            || Stamp::from(&current.parent.open_file(&current.name)?.metadata()?) != Stamp::from(&m)
        {
            return Err(refused());
        }
        current.file.write_all(body)?;
        current.hash.update(body);
        current.offset = end;
        Ok(())
    }
    fn finish(&mut self, c: &wire::FileEnd) -> Result<(), BackupError> {
        use sha2::Digest;
        self.original.recheck_live()?;
        let current = self.current.as_mut().ok_or_else(refused)?;
        if c.file_no != current.number
            || c.size != current.record.size
            || c.sha256 != current.record.sha256
            || current.offset != current.record.size
            || format!("{:x}", current.hash.clone().finalize()) != current.record.sha256
        {
            return Err(refused());
        }
        self.original.prepay_original_work(
            current
                .record
                .size
                .checked_mul(2)
                .and_then(|n| n.checked_add(65_536))
                .ok_or(BackupError::Overflow)?,
            14,
        )?;
        current.file.sync_all()?;
        current.parent.seal_file(&current.name)?;
        current.parent.sync()?;
        let before = Stamp::from(&current.file.metadata()?);
        if before.mode & 0o7777 != 0o400
            || before.uid != 0
            || before.gid != 0
            || before.links != 1
            || before.len != current.record.size
            || (before.dev, before.ino) != current.identity
        {
            return Err(refused());
        }
        // create_file owns an O_WRONLY descriptor. Keep that original writer
        // in Book; independently reopen only through its actual held parent,
        // and require the exact same completed inode/stamp before any read.
        self.original
            .entry
            .require_fixed_fd_room(FixedFdWork::Readback)?;
        let mut readback = current.parent.open_file(&current.name)?;
        if Stamp::from(&readback.metadata()?) != before {
            return Err(refused());
        }
        let mut check = sha2::Sha256::new();
        let mut left = current.record.size;
        let mut buf = [0; 65_536];
        while left != 0 {
            self.original.entry.check_local_process()?;
            let n = usize::try_from(left.min(buf.len() as u64)).map_err(|_| refused())?;
            readback.read_exact(&mut buf[..n])?;
            check.update(&buf[..n]);
            left -= n as u64;
        }
        let mut extra = [0];
        if readback.read(&mut extra)? != 0
            || format!("{:x}", check.finalize()) != current.record.sha256
            || Stamp::from(&readback.metadata()?) != before
            || Stamp::from(&current.file.metadata()?) != before
            || Stamp::from(&current.parent.open_file(&current.name)?.metadata()?) != before
        {
            return Err(refused());
        }
        let completed = self.current.take().ok_or_else(refused)?;
        self.original
            .entry
            .require_fixed_fd_room(FixedFdWork::Observe)?;
        let record = completed.record;
        match completed.role {
            wire::FileRole::PackageManifest => {
                if record.size > 64 * 1024 * 1024
                    || record.sha256 != self.original.source.manifest_sha256
                {
                    return Err(refused());
                }
                self.original.prepay_original_work(
                    record.size.checked_mul(32).ok_or(BackupError::Overflow)?,
                    4,
                )?;
                let raw = Self::read_completed(&self.package, &record, &self.original)?;
                crate::registry::prepay_current_b1_json(&self.original.entry.scan, &raw)?;
                let manifest: crate::BackupManifestV1 = serde_json::from_slice(&raw)?;
                if manifest.canonical_bytes()? != raw
                    || manifest.backup_id != self.original.backup_id_data()?
                    || manifest.files.len() + 1 > profile::FILES as usize
                    || manifest.files.windows(2).any(|w| w[0].path >= w[1].path)
                    || manifest.files.iter().any(|r| {
                        !crate::source::snapshot_effects::wire::path(&r.path)
                            || !package_member_path_data(r)
                            || r.size == 0
                            || !crate::valid_digest(&r.sha256)
                    })
                {
                    return Err(refused());
                }
                self.manifest = Some(manifest);
                self.manifest_raw = Some(raw);
                self.package_records.push(record);
            }
            wire::FileRole::PackageMember => {
                self.package_records.push(record);
            }
            wire::FileRole::SourceEvidenceMember => {
                self.source_records.push(record);
            }
            wire::FileRole::SnapshotEffectsTransportIndex => {
                self.original.prepay_original_work(
                    record.size.checked_mul(32).ok_or(BackupError::Overflow)?,
                    profile::FILES as usize,
                )?;
                let raw = Self::read_completed(&self.input_directory, &record, &self.original)?;
                crate::registry::prepay_current_b1_json(&self.original.entry.scan, &raw)?;
                let t: EffectsTransportIndex = crate::registry::canonical_record(
                    &raw,
                    profile::SESSION_TRANSPORT_INDEX_BYTES,
                )?;
                transport_index_shape_data(&t, self.original.backup_id_data()?)?;
                if crate::digest(&raw)
                    != self.original.source.snapshot_effects_transport_index_sha256
                    || t.schema_version != 1
                    || t.profile != profile::PROFILE
                    || t.backup_id != self.original.backup_id_data()?
                    || !crate::valid_digest(&t.source_capture_sha256)
                    || t.files.is_empty()
                    || t.files.len() > profile::FILES as usize
                    || t.files.windows(2).any(|w| w[0].path >= w[1].path)
                    || t.files.iter().any(|r| {
                        !crate::source::snapshot_effects::wire::path(&r.path)
                            || r.size == 0
                            || r.size > crate::source::snapshot_effects::wire::MAX_FILE
                            || !crate::valid_digest(&r.sha256)
                            || r.path == "payload-manifest.json"
                            || r.path == "snapshot-effects-transport-index.json"
                    })
                    || t.files.iter().filter(|r| r.path == "capture.json").count() != 1
                    || crate::digest(&crate::registry::canonical(&t.files)?)
                        != self.original.source.snapshot_effects_roster_sha256
                {
                    return Err(refused());
                }
                self.original
                    .entry
                    .namespace
                    .get()
                    .ok_or_else(refused)?
                    .lock()
                    .map_err(|_| refused())?
                    .store_original_transport(&raw)?;
                self.transport = Some(t);
                self.transport_raw = Some(raw);
            }
            wire::FileRole::SnapshotEffectsMember => self.effect_records.push(record),
        }
        self.file_count = self
            .file_count
            .checked_add(1)
            .ok_or(BackupError::Overflow)?;
        Ok(())
    }
    fn verify_and_seal_package(&self, records: &[FileRecord]) -> Result<(), BackupError> {
        use sha2::Digest;
        let mut directories = BTreeSet::from([String::new()]);
        let mut by_path = BTreeMap::new();
        for record in records {
            if !crate::source::snapshot_effects::wire::path(&record.path)
                || by_path.insert(record.path.as_str(), record).is_some()
            {
                return Err(refused());
            }
            let mut cursor = record.path.as_str();
            while let Some((parent, _)) = cursor.rsplit_once('/') {
                directories.insert(parent.to_owned());
                cursor = parent;
            }
        }
        self.original.prepay_original_work(
            (directories.len() as u64 + records.len() as u64) * 1024,
            directories.len() + records.len(),
        )?;
        fn directory_at(
            root: &BackupDir,
            path: &str,
            original: &OriginalAuthenticatedSourceInD,
        ) -> Result<BackupDir, BackupError> {
            original
                .prepay_original_work((path.len() as u64) * 2, path.split('/').count() * 4 + 2)?;
            let mut current = root.try_clone()?;
            if !path.is_empty() {
                for part in path.split('/') {
                    current = current.open_dir(part)?;
                }
            }
            Ok(current)
        }
        let mut actual = BTreeSet::new();
        // Iterative walk holds at most two new directory descriptors, avoiding
        // recursion/one-FD-per-depth for a 512-byte canonical peer inventory.
        for path in &directories {
            let directory = directory_at(&self.package, path, &self.original)?;
            directory.require_private_directory()?;
            let identity = directory.identity()?;
            let names = crate::registry::prepaid_b1_roster(
                &directory,
                &self.original.entry.scan,
                profile::FILES as usize,
            )?;
            for name in &names {
                let joined = if path.is_empty() {
                    name.clone()
                } else {
                    format!("{path}/{name}")
                };
                self.original
                    .prepay_original_work((joined.len() as u64) * 2, 2)?;
                match directory.kind(name)? {
                    BackupEntryKind::Directory if directories.contains(&joined) => (),
                    BackupEntryKind::File if by_path.contains_key(joined.as_str()) => {
                        if !actual.insert(joined.clone()) {
                            return Err(refused());
                        }
                        let record = by_path[joined.as_str()];
                        self.original.prepay_original_work(
                            record
                                .size
                                .checked_mul(2)
                                .and_then(|n| n.checked_add(65_537))
                                .ok_or(BackupError::Overflow)?,
                            5,
                        )?;
                        let mut file = directory.open_file(name)?;
                        let before = Stamp::from(&file.metadata()?);
                        if before.uid != 0
                            || before.gid != 0
                            || before.mode & 0o7777 != 0o400
                            || before.links != 1
                            || before.len != record.size
                        {
                            return Err(refused());
                        }
                        let mut hash = sha2::Sha256::new();
                        let mut left = record.size;
                        let mut buffer = [0; 65_536];
                        while left != 0 {
                            self.original.entry.check_local_process()?;
                            let n = left.min(buffer.len() as u64) as usize;
                            file.read_exact(&mut buffer[..n])?;
                            hash.update(&buffer[..n]);
                            left -= n as u64;
                        }
                        let mut extra = [0];
                        if file.read(&mut extra)? != 0
                            || format!("{:x}", hash.finalize()) != record.sha256
                            || Stamp::from(&file.metadata()?) != before
                            || Stamp::from(&directory.open_file(name)?.metadata()?) != before
                        {
                            return Err(refused());
                        }
                    }
                    _ => return Err(refused()),
                }
            }
            if crate::registry::prepaid_b1_roster(
                &directory,
                &self.original.entry.scan,
                profile::FILES as usize,
            )? != names
                || directory.identity()? != identity
            {
                return Err(refused());
            }
        }
        if actual.iter().map(String::as_str).collect::<BTreeSet<_>>()
            != by_path.keys().copied().collect()
        {
            return Err(refused());
        }
        let mut sealing = directories.into_iter().collect::<Vec<_>>();
        sealing.sort_by_key(|path| std::cmp::Reverse(path.len()));
        for path in sealing {
            let directory = directory_at(&self.package, &path, &self.original)?;
            directory.seal_dir()?;
            directory.require_sealed_directory()?;
        }
        Ok(())
    }

    fn read_completed(
        root: &BackupDir,
        record: &FileRecord,
        original: &OriginalAuthenticatedSourceInD,
    ) -> Result<Vec<u8>, BackupError> {
        if record.size == 0 || record.size > crate::source::snapshot_effects::wire::MAX_FILE {
            return Err(refused());
        }
        crate::source::snapshot_effects::files::read(
            root,
            record,
            &original.entry.scan,
            &original.entry.journal,
        )
    }
    fn finish_source_data(
        &mut self,
        end: &wire::SourceDataEnd,
    ) -> Result<OriginalReceivedFiles<'_>, BackupError> {
        self.original.recheck_live()?;
        let source = &self.original.source;
        if self.current.is_some()
            || self.file_count != source.file_count
            || end.files != self.file_count
            || self.package_bytes != source.package_bytes
            || self.evidence_bytes != source.evidence_bytes
            || end.bytes
                != self
                    .package_bytes
                    .checked_add(self.evidence_bytes)
                    .ok_or(BackupError::Overflow)?
            || self.source_records.len() != 12
            || self.effect_records != self.transport.as_ref().ok_or_else(refused)?.files
        {
            return Err(refused());
        }
        self.original
            .prepay_original_work(16_384 * 64, profile::FILES as usize)?;
        let mut package_records = self.package_records.clone();
        package_records.sort_by(|a, b| a.path.cmp(&b.path));
        let mut source_records = self.source_records.clone();
        source_records.sort_by(|a, b| a.path.cmp(&b.path));
        if crate::digest(&crate::registry::canonical(&package_records)?)
            != source.package_roster_sha256
            || crate::digest(&crate::registry::canonical(&source_records)?)
                != source.captured_roster_sha256
            || end.package_roster_sha256 != source.package_roster_sha256
            || end.captured_roster_sha256 != source.captured_roster_sha256
            || end.snapshot_effects_roster_sha256 != source.snapshot_effects_roster_sha256
        {
            return Err(refused());
        }
        let manifest = self.manifest.as_ref().ok_or_else(refused)?;
        if package_records.len() != manifest.files.len() + 1 {
            return Err(refused());
        }
        let index = self
            .package_records
            .iter()
            .find(|r| r.path == "asset-index.json")
            .ok_or_else(refused)?;
        let index_raw = Self::read_completed(&self.package, index, &self.original)?;
        crate::registry::prepay_current_b1_json(&self.original.entry.scan, &index_raw)?;
        manifest.validate_with_index(&index_raw)?;
        let f = &crate::source::snapshot_effects::files::exact_tree;
        self.verify_and_seal_package(&package_records)?;
        f(
            &self.source_directory,
            &source_records,
            &self.original.entry.scan,
            &self.original.entry.journal,
        )?;
        f(
            &self.effects_directory,
            &self.effect_records,
            &self.original.entry.scan,
            &self.original.entry.journal,
        )?;

        let destination = self.original.destination_original();
        destination.destination().rename_noreplace_without_sync(
            &self.package_stage,
            &format!("{}.sealed", self.original.backup_id_data()?),
        )?;
        destination.destination().sync()?;
        if destination
            .destination()
            .open_dir(&format!("{}.sealed", self.original.backup_id_data()?))?
            .identity()?
            != self.package.identity()?
        {
            return Err(refused());
        }
        self.original.recheck_live()?;
        Ok(OriginalReceivedFiles { incoming: self })
    }
}

/// Only the completed actual stream above lends these retained handles. This
/// cannot be deserialized, cloned, created from a directory, or promoted from a
/// local CapturedSnapshotEffectsV2. Concrete D readers still validate all bytes.
pub(crate) struct OriginalReceivedFiles<'a> {
    incoming: &'a IncomingOriginalFiles,
}
impl OriginalReceivedFiles<'_> {
    pub(crate) fn original(&self) -> &Arc<OriginalAuthenticatedSourceInD> {
        &self.incoming.original
    }
    pub(crate) fn manifest_raw(&self) -> &[u8] {
        self.incoming
            .manifest_raw
            .as_deref()
            .expect("actual completed manifest")
    }
    pub(crate) fn source_records(&self) -> &[FileRecord] {
        &self.incoming.source_records
    }
    pub(crate) fn effect_records(&self) -> &[FileRecord] {
        &self.incoming.effect_records
    }
    pub(crate) fn source_directory(&self) -> &BackupDir {
        &self.incoming.source_directory
    }
    pub(crate) fn effects_directory(&self) -> &BackupDir {
        &self.incoming.effects_directory
    }
    pub(crate) fn transport_raw(&self) -> &[u8] {
        self.incoming
            .transport_raw
            .as_deref()
            .expect("actual completed transport index")
    }
    pub(crate) fn recheck(&self) -> Result<(), BackupError> {
        self.incoming.original.recheck_live()
    }
}

impl OriginalAuthenticatedSourceInD {
    pub(crate) fn require_original_effects_fd_room(&self) -> Result<(), BackupError> {
        self.require_destination(&self.destination)?;
        self.entry.require_fixed_fd_room(FixedFdWork::EffectsCopy)
    }
    pub(crate) fn entry_original(&self) -> &'static OriginalDEntryScope {
        self.entry
    }
    pub(crate) fn destination_arc_original(&self) -> &Arc<EnrolledDestination> {
        &self.destination
    }
    pub(crate) fn signer_original(
        &self,
    ) -> Result<&Arc<crate::destination::B1DestinationSignerOriginal>, BackupError> {
        self.entry.signer.get().ok_or_else(refused)
    }
    pub(crate) fn native_original(
        &self,
    ) -> Result<&Arc<NativeDestinationLocalCustody>, BackupError> {
        self.entry.native.get().ok_or_else(refused)
    }
    pub(crate) fn journal_original(&self) -> &learning_assets::root_journal::SharedJournalBudget {
        &self.entry.journal
    }
    pub(crate) fn require_source_binding_files(
        &self,
        files: &BTreeMap<String, Vec<u8>>,
    ) -> Result<(), BackupError> {
        self.require_destination(&self.destination)?;
        let total = files.values().try_fold(0u64, |n, v| {
            n.checked_add(v.len() as u64).ok_or(BackupError::Overflow)
        })?;
        if total > profile::EVIDENCE_BYTES {
            return Err(refused());
        }
        self.prepay_original_work(total.checked_mul(32).ok_or(BackupError::Overflow)?, 32)?;
        for (name, raw) in files {
            if raw.is_empty()
                || raw.len() as u64 > crate::destination::source_evidence_file_cap_data(name)
            {
                return Err(refused());
            }
            crate::registry::prepay_current_b1_json(&self.entry.scan, raw)?;
        }
        let field = |name: &str| files.get(name).ok_or_else(refused);
        let authority: crate::registry::Authority =
            crate::registry::canonical_record(field("source-authority.json")?, 16_384)?;
        let generation: crate::registry::Generation =
            crate::registry::canonical_record(field("source-generation.json")?, 524_288)?;
        let group: crate::registry::SourceGroup =
            crate::registry::canonical_record(field("source-group.json")?, 16_384)?;
        if files.len() != 12
            || crate::digest(field("source-authority.json")?)
                != self.source.registry_authority_sha256
            || crate::digest(field("source-generation.json")?) != self.source.generation_sha256
            || authority.deployment_id != self.source.deployment_id
            || generation.deployment_id != self.source.deployment_id
            || generation.generation != self.source.registry_generation
            || group.deployment_id != self.source.deployment_id
            || group.group_id != self.source.source_group_id
            || group.source_binding_sha256 != self.source.source_binding_sha256
            || group.application_commit != self.source.application_commit
            || group.application_build_sha256 != self.source.application_build_sha256
            || crate::digest(field("source-released.json")?) != self.source.source_control_sha256
        {
            return Err(refused());
        }
        for root in &self.source.roots {
            let name = format!("roots/{}.json", root.enrollment_id);
            let raw = field(&name)?;
            let record: crate::registry::RootRecord =
                crate::registry::canonical_record(raw, 16_384)?;
            if group.roots.get(&root.role) != Some(&root.enrollment_id)
                || record.deployment_id != self.source.deployment_id
                || record.group_id != self.source.source_group_id
                || record.enrollment_id != root.enrollment_id
                || record.path != root.path
                || record.dev != root.dev
                || record.ino != root.ino
                || record.uid != root.uid
                || record.mode != root.mode
                || crate::digest(raw) != root.record_sha256
            {
                return Err(refused());
            }
        }
        Ok(())
    }
}
impl OriginalReceivedFiles<'_> {
    pub(crate) fn read_source_files_data(&self) -> Result<BTreeMap<String, Vec<u8>>, BackupError> {
        self.recheck()?;
        let original = self.original();
        original.prepay_original_work(
            self.source_records()
                .iter()
                .try_fold(0u64, |n, r| {
                    n.checked_add(r.size).ok_or(BackupError::Overflow)
                })?
                .checked_mul(2)
                .ok_or(BackupError::Overflow)?,
            12,
        )?;
        let mut files = BTreeMap::new();
        for record in self.source_records() {
            files.insert(
                record.path.clone(),
                IncomingOriginalFiles::read_completed(self.source_directory(), record, original)?,
            );
        }
        original.require_source_binding_files(&files)?;
        Ok(files)
    }
    pub(crate) fn manifest_data(&self) -> &crate::BackupManifestV1 {
        self.incoming
            .manifest
            .as_ref()
            .expect("completed original manifest")
    }
    pub(crate) fn transport_capture_sha256_data(&self) -> &str {
        &self
            .incoming
            .transport
            .as_ref()
            .expect("completed original transport index")
            .source_capture_sha256
    }
}

impl OriginalDestinationSession<'_> {
    pub(super) fn begin_fixed_file(&mut self, begin: &wire::FileBegin) -> Result<(), BackupError> {
        self.recheck()?;
        let state = self.current_mut()?;
        if state.phase != OriginalPhase::Receiving {
            return Err(refused());
        }
        state.transfer.as_mut().ok_or_else(refused)?.begin(begin)
    }
    pub(super) fn write_file_bytes(
        &mut self,
        chunk: &wire::FileChunk,
        body: &[u8],
    ) -> Result<(), BackupError> {
        let state = self.current_mut()?;
        if state.phase != OriginalPhase::Receiving {
            return Err(refused());
        }
        state
            .transfer
            .as_mut()
            .ok_or_else(refused)?
            .chunk(chunk, body)
    }
    pub(super) fn finish_fixed_file(&mut self, end: &wire::FileEnd) -> Result<(), BackupError> {
        self.recheck()?;
        let state = self.current_mut()?;
        if state.phase != OriginalPhase::Receiving {
            return Err(refused());
        }
        state.transfer.as_mut().ok_or_else(refused)?.finish(end)
    }
    pub(super) fn finish_source_data_and_prepare_ready(
        &mut self,
        end: &wire::SourceDataEnd,
    ) -> Result<wire::DestinationReady, BackupError> {
        self.recheck()?;
        let state = self.current_mut()?;
        if state.phase != OriginalPhase::Receiving || state.prepared.is_some() {
            return Err(refused());
        }
        let received = state
            .transfer
            .as_mut()
            .ok_or_else(refused)?
            .finish_source_data(end)?;
        let prepared = crate::destination::prepare_original_remote_destination(&received)?;
        state.prepared = Some(prepared);
        state.phase = OriginalPhase::Ready;
        self.recheck()?;
        self.entry.prepay_wire_work(16_384 * 4, 12)?;
        let state = self.current()?;
        let prepared = state.prepared.as_ref().ok_or_else(refused)?;
        prepared.recheck_original_ready()?;
        let auth = state.authenticated.as_ref().ok_or_else(refused)?;
        // Common envelope DATA is replaced only by the fixed wire owner using
        // its current transcript. These fields alone are neither a frame nor
        // evidence that the original writer naturally reached Ready.
        Ok(wire::DestinationReady {
            schema_version: 1,
            profile: profile::PROFILE.into(),
            kind: "DestinationReady".into(),
            direction: wire::Direction::DestinationToSource,
            sequence: 0,
            s_nonce: auth.s_nonce.clone(),
            d_nonce: auth.d_nonce.clone(),
            prev_s: String::new(),
            prev_d: String::new(),
            binding_sha256: auth.binding_sha256.clone(),
            attempt_id: state.attempt.to_string(),
            proof_sha256: prepared.proof_sha256_data().to_owned(),
            snapshot_effects_proof_sha256: prepared.ready_proof_sha256_data(),
            manifest_sha256: prepared.manifest_sha256_data().to_owned(),
            snapshot_effects_input_sha256: prepared.input_sha256_data().to_owned(),
            snapshot_effects_intent_sha256: prepared.ready_intent_sha256_data(),
        })
    }
    pub(super) fn require_original_final_data(
        &mut self,
        verified: &wire::VerifiedFinalInOriginalD,
    ) -> Result<
        (
            Arc<Mutex<OperationAuthority>>,
            crate::destination::RemoteFinalBindingData,
        ),
        BackupError,
    > {
        verified.require_original(self)?;
        self.recheck()?;
        self.entry.prepay_wire_work(16_384 * 8, 16)?;
        let state = self.current()?;
        if state.phase != OriginalPhase::Ready {
            return Err(refused());
        }
        let prepared = state.prepared.as_ref().ok_or_else(refused)?;
        prepared.recheck_original_ready()?;
        let auth = state.authenticated.as_ref().ok_or_else(refused)?;
        let final_data = verified.control_data();
        let source_operation =
            Uuid::parse_str(&auth.source.source_operation_id).map_err(|_| refused())?;
        let actor_data = serde_json::json!({"source_operation_id":source_operation,"binding_sha256":auth.binding_sha256,
            "s_nonce":auth.s_nonce,"d_nonce":auth.d_nonce});
        let mut actor_bytes = b"KnowWeave/Task9/B1/source-actor/v1\0".to_vec();
        actor_bytes.extend(crate::registry::canonical(&actor_data)?);
        if final_data.attempt_id != state.attempt.to_string()
            || final_data.proof_sha256 != prepared.proof_sha256_data()
            || final_data.snapshot_effects_proof_sha256 != prepared.ready_proof_sha256_data()
            || final_data.snapshot_effects_input_sha256 != prepared.input_sha256_data()
            || final_data.snapshot_effects_intent_sha256 != prepared.ready_intent_sha256_data()
            || final_data.s_nonce != auth.s_nonce
            || final_data.d_nonce != auth.d_nonce
            || final_data.binding_sha256 != auth.binding_sha256
            || final_data.source_auth_key_sha256 != state.source_key_sha256
            || final_data.fence.operation_id != source_operation.to_string()
            || final_data.fence.source_actor_binding_sha256 != crate::digest(&actor_bytes)
            || [
                &final_data.fence.anchor_sha256,
                &final_data.fence.open_record_sha256,
                &final_data.fence.protection_snapshot_sha256,
            ]
            .iter()
            .any(|v| !crate::valid_digest(v))
        {
            return Err(refused());
        }
        // Final's snapshot is the first-S full post-Capture protection scan.
        // The old exact12 snapshot predates creation of the export mirror, so
        // equality with that historical hash would incorrectly reject S.
        let binding = crate::destination::RemoteFinalBindingData {
            backup_id: auth.backup_id_data()?,
            attempt_id: state.attempt,
            source_operation_id: source_operation,
            session_binding_sha256: auth.binding_sha256.clone(),
            source_auth_frame_sha256: auth.source_auth_frame_sha256.clone(),
            source_auth_key_sha256: state.source_key_sha256.clone(),
            s_nonce: auth.s_nonce.clone(),
            d_nonce: auth.d_nonce.clone(),
            manifest_sha256: prepared.manifest_sha256_data().to_owned(),
            proof_sha256: prepared.proof_sha256_data().to_owned(),
            snapshot_effects_proof_sha256: prepared.ready_proof_sha256_data(),
            snapshot_effects_input_sha256: prepared.input_sha256_data().to_owned(),
            snapshot_effects_intent_sha256: prepared.ready_intent_sha256_data(),
            fence_anchor_sha256: final_data.fence.anchor_sha256.clone(),
            protection_snapshot_sha256: final_data.fence.protection_snapshot_sha256.clone(),
        };
        binding.validate_data()?;
        Ok((
            Arc::clone(&state.destination.lease.destination_operation),
            binding,
        ))
    }
    pub(super) fn consume_authenticated_final(
        &mut self,
        verified: wire::VerifiedFinalInOriginalD,
    ) -> Result<wire::CommittedResult, BackupError> {
        let decision = super::finalize::AuthenticatedFinalInOriginalD::from_original_destination(
            self, &verified,
        )?;
        self.entry.prepay_wire_work(32_768, 12)?;
        let prepared = Arc::clone(self.current()?.prepared.as_ref().ok_or_else(refused)?);
        // The actual Ready owner remains in the original static Book during
        // all fallible consumption/sign/receipt work and any worker unwind.
        self.current_mut()?.phase = OriginalPhase::Consuming;
        let committed = match prepared.finalize_fixed(decision) {
            Ok(committed) => committed,
            Err(error) => {
                self.current_mut()?.phase = OriginalPhase::Failed;
                self.entry.reject();
                return Err(error);
            }
        };
        let state = self.current_mut()?;
        state.committed = Some(committed);
        state.phase = OriginalPhase::Committed;
        let auth = state.authenticated.as_ref().ok_or_else(refused)?;
        let committed = state.committed.as_ref().ok_or_else(refused)?;
        Ok(wire::CommittedResult {
            schema_version: 1,
            profile: profile::PROFILE.into(),
            kind: "CommittedResult".into(),
            direction: wire::Direction::DestinationToSource,
            sequence: 0,
            s_nonce: auth.s_nonce.clone(),
            d_nonce: auth.d_nonce.clone(),
            prev_s: String::new(),
            prev_d: String::new(),
            binding_sha256: auth.binding_sha256.clone(),
            attempt_id: state.attempt.to_string(),
            proof_sha256: prepared.proof_sha256_data().to_owned(),
            receipt_sha256: committed.receipt_sha256_data().to_owned(),
            receipt_size: u32::try_from(committed.receipt_data().len()).map_err(|_| refused())?,
        })
    }
    pub(super) fn acknowledge_result(
        &mut self,
        ack: &wire::SourceResultAck,
    ) -> Result<(), BackupError> {
        self.recheck()?;
        self.entry.prepay_wire_work(4096, 4)?;
        let state = self.current_mut()?;
        if state.phase != OriginalPhase::Committed {
            return Err(refused());
        }
        let auth = state.authenticated.as_ref().ok_or_else(refused)?;
        let committed = state.committed.as_ref().ok_or_else(refused)?;
        if ack.attempt_id != state.attempt.to_string()
            || ack.receipt_sha256 != committed.receipt_sha256_data()
            || ack.s_nonce != auth.s_nonce
            || ack.d_nonce != auth.d_nonce
            || ack.binding_sha256 != auth.binding_sha256
        {
            return Err(refused());
        }
        state.phase = OriginalPhase::Acknowledged;
        Ok(())
    }
    pub(super) fn finish_original_end(
        &mut self,
        verified: wire::VerifiedOriginalDestinationEnd,
    ) -> Result<(), BackupError> {
        verified.require_original(self)?;
        self.entry.check_local_process()?;
        let state = self.current_mut()?;
        if state.phase != OriginalPhase::Acknowledged || state.committed.is_none() {
            return Err(refused());
        }
        state.phase = OriginalPhase::Ended;
        Ok(())
    }
}

impl NativeDestinationLocalCustody {
    pub(crate) fn recheck_committed_original(
        &self,
        committed: &crate::destination::CommittedRemoteOriginal,
    ) -> Result<(), BackupError> {
        committed.require_own_committed_native(self)?;
        self.recheck_local_facts()?;
        self.entry
            .metadata
            .get()
            .ok_or_else(refused)?
            .lock()
            .map_err(|_| refused())?
            .require_destination(committed.held_destination_for_committed())?;
        Ok(())
    }
}
impl OriginalAuthenticatedSourceInD {
    pub(crate) fn recheck_committed_namespace(
        &self,
        committed: &crate::destination::CommittedRemoteOriginal,
    ) -> Result<(), BackupError> {
        committed.require_own_committed_native(self.entry.native.get().ok_or_else(refused)?)?;
        if !std::ptr::eq(
            committed.held_destination_for_committed(),
            self.destination.as_ref(),
        ) {
            return Err(refused());
        }
        self.entry
            .namespace
            .get()
            .ok_or_else(refused)?
            .lock()
            .map_err(|_| refused())?
            .recheck_retained()
    }
}

#[cfg(test)]
mod original_destination_data_tests {
    use super::*;
    // These fixtures exercise bounded DATA predicates, never native/installed
    // admission, an original session, key custody, a lease or Final authority.
    #[test]
    fn original_destination_proc_status_rejects_duplicate_or_foreign_credentials() {
        let root = b"Name: receiver\nUid:\t0\t0\t0\t0\nGid:\t0\t0\t0\t0\nTracerPid:\t0\n";
        assert!(root_process_status(root).is_ok());
        let mut duplicate = root.to_vec();
        duplicate.extend(b"Uid:\t0\t0\t0\t0\n");
        assert!(root_process_status(&duplicate).is_err());
        assert!(root_process_status(b"Uid: 0 1000 0 0\nGid: 0 0 0 0\nTracerPid: 0\n").is_err());
        assert!(root_process_status(b"Uid: 0 0 0 0\nGid: 0 0 0 0\nTracerPid: 1\n").is_err());
    }
    #[test]
    fn original_destination_process_data_keeps_kernel_pid_parent_and_start_join() {
        let mut tail = vec!["0"; 20];
        tail[0] = "S";
        tail[1] = "42";
        tail[19] = "7000";
        let raw = format!("81 (name ) with spaces) {}\n", tail.join(" "));
        let parsed = process_stat_data(raw.as_bytes(), 81).unwrap();
        assert_eq!(
            (parsed.pid, parsed.parent, parsed.start_ticks),
            (81, 42, 7000)
        );
        assert!(process_stat_data(raw.as_bytes(), 82).is_err());
        tail[19] = "0";
        assert!(
            process_stat_data(format!("81 (receiver) {}", tail.join(" ")).as_bytes(), 81).is_err()
        );
    }
    #[test]
    fn original_destination_exec_mapping_data_rejects_writable_deleted_or_anonymous_code() {
        assert!(
            executable_mapping_data(b"1000-2000 r-xp 00000000 08:01 7 /usr/lib/libc.so\n").is_ok()
        );
        for raw in [
            b"1000-2000 rwxp 00000000 08:01 7 /usr/lib/libc.so\n".as_slice(),
            b"1000-2000 r-xp 00000000 08:01 7 /usr/lib/libc.so (deleted)\n".as_slice(),
            b"1000-2000 r-xp 00000000 00:00 0 [anon]\n".as_slice(),
        ] {
            assert!(executable_mapping_data(raw).is_err());
        }
    }
    #[test]
    fn original_destination_package_roster_cannot_select_peer_paths() {
        let mut r = FileRecord {
            path: "database.dump".into(),
            size: 1,
            sha256: "a".repeat(64),
        };
        assert!(package_member_path_data(&r));
        r.path = format!("assets/sha256/aa/{}", r.sha256);
        assert!(package_member_path_data(&r));
        for path in [
            "../database.dump",
            "arbitrary/config",
            "assets/sha256/bb/mismatch",
            "destination.pk8",
        ] {
            r.path = path.into();
            assert!(!package_member_path_data(&r));
        }
    }
    #[test]
    fn original_destination_transport_metadata_rejects_extra_scope_and_duplicate_capture() {
        let id = Uuid::new_v4();
        let mut t = EffectsTransportIndex {
            schema_version: 1,
            profile: profile::PROFILE.into(),
            backup_id: id,
            source_capture_sha256: "a".repeat(64),
            files: vec![FileRecord {
                path: "capture.json".into(),
                size: 4,
                sha256: "a".repeat(64),
            }],
        };
        transport_index_shape_data(&t, id).unwrap();
        t.files.push(t.files[0].clone());
        assert!(transport_index_shape_data(&t, id).is_err());
        t.files.pop();
        t.files[0].path = "../capture.json".into();
        assert!(transport_index_shape_data(&t, id).is_err());
        t.files[0].path = "capture.json".into();
        t.files[0].sha256 = "b".repeat(64);
        assert!(transport_index_shape_data(&t, id).is_err());
        t.files[0].sha256 = "a".repeat(64);
        t.schema_version = 2;
        assert!(transport_index_shape_data(&t, id).is_err());
    }
    #[test]
    fn original_destination_admission_metadata_never_tolerates_extra_or_duplicate_fields() {
        let a = SessionAdmission {
            format_version: 1,
            capability: "task9_b1_session_admission_v1".into(),
            profile: profile::PROFILE.into(),
            backup_id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            destination_attempt_id: Uuid::new_v4(),
            binding_sha256: "1".repeat(64),
            source_auth_frame_sha256: "2".repeat(64),
            s_nonce: "3".repeat(64),
            d_nonce: "4".repeat(64),
        };
        let mut value = serde_json::to_value(&a).unwrap();
        value["authority"] = serde_json::json!(true);
        assert!(
            crate::registry::canonical_record::<SessionAdmission>(
                &crate::registry::canonical(&value).unwrap(),
                4096
            )
            .is_err()
        );
        let raw = crate::registry::canonical(&a).unwrap();
        let duplicate = format!(
            "{{\"format_version\":1,{}",
            &std::str::from_utf8(&raw).unwrap()[1..]
        );
        assert!(
            crate::registry::canonical_record::<SessionAdmission>(duplicate.as_bytes(), 4096)
                .is_err()
        );
    }
}

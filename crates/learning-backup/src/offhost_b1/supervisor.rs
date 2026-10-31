//! The original Source owns its fixed SSH process, every descriptor, signer
//! and IO registration before any cancellable work. No public command/FD/key
//! constructor exists. Timeouts observe this owner; they do not kill or replace.
use super::{
    source_actor::{SourceFirstS, SourceFrontOriginal},
    wire,
};
use crate::BackupError;
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Component, Path},
    process::{Child, Command, Stdio},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
fn refused() -> BackupError {
    BackupError::Invalid("original Source SSH custody, fixed key or lifecycle")
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct FileStamp {
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
impl FileStamp {
    fn of(m: &std::fs::Metadata) -> Self {
        Self {
            dev: m.dev(),
            ino: m.ino(),
            uid: m.uid(),
            gid: m.gid(),
            mode: m.mode(),
            links: m.nlink(),
            len: m.len(),
            mtime: m.mtime(),
            mtime_ns: m.mtime_nsec(),
            ctime: m.ctime(),
            ctime_ns: m.ctime_nsec(),
        }
    }
}
/// Each slot exists in the Book before its real file is opened. There is no
/// generic external path reader; all uses below are fixed compiled leaves.
pub(super) struct SourceFixedFile {
    parent: Option<File>,
    file: Option<File>,
    path: &'static str,
    stamp: Option<FileStamp>,
    raw: Vec<u8>,
    executable: bool,
}
impl SourceFixedFile {
    fn empty(path: &'static str, executable: bool) -> Self {
        Self {
            parent: None,
            file: None,
            path,
            stamp: None,
            raw: Vec::new(),
            executable,
        }
    }
    fn load_in_place(
        &mut self,
        original: &SourceFrontOriginal,
        cap: usize,
    ) -> Result<(), BackupError> {
        if self.file.is_some() || self.parent.is_some() {
            return Err(refused());
        }
        require_fixed_source_fd_room(original)?;
        let path = Path::new(self.path);
        let parent = path.parent().ok_or_else(refused)?;
        original.prepay_wire_work(
            parent.as_os_str().len() as u64,
            parent.components().count() * 3 + 6,
        )?;
        let held_parent = open_fixed_parent(parent)?;
        self.parent = Some(held_parent); // retain actual directory before file open
        let name = std::ffi::CString::new(
            path.file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(refused)?,
        )
        .map_err(|_| refused())?;
        let fd = unsafe {
            libc::openat(
                self.parent.as_ref().ok_or_else(refused)?.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            )
        };
        if fd < 0 {
            return Err(refused());
        }
        self.file = Some(unsafe { File::from_raw_fd(fd) }); // before metadata/read/parser
        let file = self.file.as_mut().ok_or_else(refused)?;
        original.prepay_wire_work(0, 2)?;
        let stamp = FileStamp::of(&file.metadata()?);
        let mode = if self.executable { 0o755 } else { 0o600 };
        if stamp.mode & libc::S_IFMT != libc::S_IFREG
            || stamp.mode & 0o7777 != mode
            || stamp.uid != 0
            || stamp.gid != 0
            || stamp.links != 1
            || stamp.len == 0
            || stamp.len > cap as u64
        {
            return Err(refused());
        }
        original.prepay_wire_work(stamp.len.checked_mul(4).ok_or(BackupError::Overflow)?, 2)?;
        self.raw.resize(stamp.len as usize, 0);
        file.read_exact(&mut self.raw)?;
        let mut extra = [0u8; 1];
        if file.read(&mut extra)? != 0 || FileStamp::of(&file.metadata()?) != stamp {
            return Err(refused());
        }
        self.stamp = Some(stamp);
        if self.executable && !self.raw.starts_with(b"\x7fELF") {
            return Err(refused());
        }
        recheck_source_fd_count(original)?;
        self.recheck(original)
    }
    fn recheck(&self, original: &SourceFrontOriginal) -> Result<(), BackupError> {
        require_fixed_source_fd_room(original)?;
        original.prepay_wire_work(
            (self.path.len() as u64)
                .checked_mul(4)
                .and_then(|n| n.checked_add(1024))
                .ok_or(BackupError::Overflow)?,
            Path::new(self.path).components().count() * 4 + 12,
        )?;
        let stamp = self.stamp.ok_or_else(refused)?;
        let original_file = self.file.as_ref().ok_or_else(refused)?;
        if FileStamp::of(&original_file.metadata()?) != stamp {
            return Err(refused());
        }
        let path = Path::new(self.path);
        let parent = open_fixed_parent(path.parent().ok_or_else(refused)?)?;
        if FileStamp::of(&parent.metadata()?).dev
            != FileStamp::of(&self.parent.as_ref().ok_or_else(refused)?.metadata()?).dev
            || FileStamp::of(&parent.metadata()?).ino
                != FileStamp::of(&self.parent.as_ref().ok_or_else(refused)?.metadata()?).ino
        {
            return Err(refused());
        }
        let name = std::ffi::CString::new(
            path.file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(refused)?,
        )
        .map_err(|_| refused())?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            )
        };
        if fd < 0 {
            return Err(refused());
        }
        let named = unsafe { File::from_raw_fd(fd) };
        if FileStamp::of(&named.metadata()?) != stamp {
            return Err(refused());
        }
        // Executable metadata is immutable to unprivileged peers; C's trusted
        // root/OS installation constraint remains necessary, not inferred here.
        if !self.executable {
            use std::os::unix::fs::FileExt;
            original.prepay_wire_work(stamp.len.checked_mul(2).ok_or(BackupError::Overflow)?, 2)?;
            let mut actual = vec![0u8; stamp.len as usize];
            let mut offset = 0;
            while offset < actual.len() {
                let n = original_file.read_at(&mut actual[offset..], offset as u64)?;
                if n == 0 {
                    return Err(refused());
                }
                offset += n;
            }
            if actual != self.raw
                || FileStamp::of(&original_file.metadata()?) != stamp
                || FileStamp::of(&named.metadata()?) != stamp
            {
                return Err(refused());
            }
        }
        recheck_source_fd_count(original)?;
        Ok(())
    }
}
fn open_fixed_parent(path: &Path) -> Result<File, BackupError> {
    if !path.is_absolute() {
        return Err(refused());
    }
    let mut dir = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open("/")?;
    let root = dir.metadata()?;
    if !root.is_dir() || root.uid() != 0 || root.mode() & 0o022 != 0 {
        return Err(refused());
    }
    for part in path.components() {
        let Component::Normal(part) = part else {
            if part == Component::RootDir {
                continue;
            }
            return Err(refused());
        };
        let name = std::ffi::CString::new(part.as_encoded_bytes()).map_err(|_| refused())?;
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            return Err(refused());
        }
        dir = unsafe { File::from_raw_fd(fd) };
        let m = dir.metadata()?;
        if !m.is_dir() || m.uid() != 0 || m.mode() & 0o022 != 0 {
            return Err(refused());
        }
    }
    Ok(dir)
}
struct SourceInstalledFiles {
    ssh: SourceFixedFile,
    known_hosts: SourceFixedFile,
    transport: SourceFixedFile,
    signer: SourceFixedFile,
    trust: SourceFixedFile,
    signing_pair: Option<Ed25519KeyPair>,
    trust_data: Option<crate::VerifierTrustConfig>,
    ready: bool,
}
impl SourceInstalledFiles {
    fn empty() -> Self {
        Self {
            ssh: SourceFixedFile::empty("/usr/bin/ssh", true),
            known_hosts: SourceFixedFile::empty("/var/lib/knowweave-c4/b1/known-hosts", false),
            transport: SourceFixedFile::empty("/var/lib/knowweave-c4/b1/transport-key", false),
            signer: SourceFixedFile::empty("/var/lib/knowweave-c4/b1/source-auth.pk8", false),
            trust: SourceFixedFile::empty(crate::destination::DESTINATION_TRUST_PATH, false),
            signing_pair: None,
            trust_data: None,
            ready: false,
        }
    }
    fn load(&mut self, first: &SourceFirstS<'_>) -> Result<(), BackupError> {
        if self.ready {
            return Err(refused());
        }
        let original = first.original_owner();
        self.ssh.load_in_place(original, 32 * 1024 * 1024)?;
        self.known_hosts.load_in_place(original, 4096)?;
        self.transport.load_in_place(original, 4096)?;
        self.signer.load_in_place(original, 4096)?;
        self.trust.load_in_place(original, 4096)?;
        original.prepay_wire_work(
            (self.transport.raw.len() as u64)
                .checked_mul(32)
                .ok_or(BackupError::Overflow)?
                + 262144,
            32,
        )?;
        let metadata = first.fence_metadata()?;
        let public_blob = checked_openssh_transport_public_blob(&self.transport.raw)?;
        if crate::digest(&public_blob) != metadata.transport_public_key_sha256_data() {
            return Err(refused());
        }
        self.signing_pair =
            Some(Ed25519KeyPair::from_pkcs8(&self.signer.raw).map_err(|_| refused())?);
        if crate::digest(
            self.signing_pair
                .as_ref()
                .ok_or_else(refused)?
                .public_key()
                .as_ref(),
        ) != metadata.source_auth_sha256_data()
        {
            return Err(refused());
        }
        // The two fingerprint domains differ even for an identical key.
        // Purpose separation therefore compares the actual parsed key bytes.
        require_distinct_ed25519_material(
            self.signing_pair
                .as_ref()
                .ok_or_else(refused)?
                .public_key()
                .as_ref(),
            &public_blob,
        )?;
        self.trust_data = Some(serde_json::from_slice(&self.trust.raw)?);
        let trust = self.trust_data.as_ref().ok_or_else(refused)?;
        if trust.canonical_bytes()? != self.trust.raw {
            return Err(refused());
        }
        crate::complete::require_deployment_verifier_key(trust)?;
        let key = hex::decode(&trust.verifier_public_key_hex).map_err(|_| refused())?;
        if crate::digest(&key) != metadata.verifier_public_key_sha256_data() {
            return Err(refused());
        }
        let source = first.source_original()?;
        let pin = first.pin()?;
        if trust.source_root
            != first
                .lease_original()
                .roots
                .get(source.group.roots.get("local_pins").ok_or_else(refused)?)
                .ok_or_else(refused)?
                .path
            || trust.source_control_root
                != first
                    .lease_original()
                    .roots
                    .get(source.group.roots.get("control").ok_or_else(refused)?)
                    .ok_or_else(refused)?
                    .path
            || pin.manifest().backup_id.is_nil()
        {
            return Err(refused());
        }
        self.ready = true;
        self.recheck(first)
    }
    fn recheck(&self, first: &SourceFirstS<'_>) -> Result<(), BackupError> {
        if !self.ready {
            return Err(refused());
        }
        for file in [
            &self.ssh,
            &self.known_hosts,
            &self.transport,
            &self.signer,
            &self.trust,
        ] {
            file.recheck(first.original_owner())?;
        }
        Ok(())
    }
}
struct StderrTask {
    handle: tokio::task::JoinHandle<Result<StderrObserved, BackupError>>,
    observed: bool,
    result_ok: Option<bool>,
}
impl StderrTask {
    /// Only this borrowed observer has a deadline. A timed-out handle remains
    /// unconsumed; a naturally returned JoinErr or inner Err is recorded before
    /// propagation, so a negative observer can never poll it a second time.
    async fn observe_original_once(
        &mut self,
        deadline: Instant,
    ) -> Result<StderrObserved, BackupError> {
        if self.observed {
            return Err(refused());
        }
        let settled =
            tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), &mut self.handle)
                .await
                .map_err(|_| refused())?;
        self.observed = true;
        self.result_ok = Some(settled.as_ref().is_ok_and(|value| value.is_ok()));
        settled.map_err(|_| refused())?
    }
}
struct StderrObserved {
    bytes: u64,
    sha256: String,
}
struct NegativeWaitTask {
    handle: tokio::task::JoinHandle<Result<(), BackupError>>,
    observed: bool,
    result_ok: Option<bool>,
}
/// The actual child/keys/pipes/registrations are never moved into a caller's
/// cancellable future. Its static owner belongs to the original Source Book.
pub(super) struct OriginalSshOwner {
    original: &'static SourceFrontOriginal,
    files: Mutex<SourceInstalledFiles>,
    child: Mutex<Option<Child>>,
    stderr: OnceLock<OwnedFd>,
    pipes: OnceLock<&'static wire::OriginalPipePair>,
    pipe_transfer: AtomicBool,
    spawn_attempted: AtomicBool,
    stderr_task: tokio::sync::Mutex<Option<StderrTask>>,
    io: tokio::sync::Mutex<Option<wire::OriginalPipeIo>>,
    stream: Mutex<Option<(File, FileStamp)>>,
    negative_wait: tokio::sync::Mutex<Option<NegativeWaitTask>>,
}
impl OriginalSshOwner {
    pub(super) fn empty(original: &'static SourceFrontOriginal) -> Self {
        Self {
            original,
            files: Mutex::new(SourceInstalledFiles::empty()),
            child: Mutex::new(None),
            stderr: OnceLock::new(),
            pipes: OnceLock::new(),
            pipe_transfer: AtomicBool::new(false),
            spawn_attempted: AtomicBool::new(false),
            stderr_task: tokio::sync::Mutex::new(None),
            io: tokio::sync::Mutex::new(None),
            stream: Mutex::new(None),
            negative_wait: tokio::sync::Mutex::new(None),
        }
    }
    pub(super) fn require_original(
        &self,
        original: &'static SourceFrontOriginal,
    ) -> Result<(), BackupError> {
        if !std::ptr::eq(self.original, original) {
            return Err(refused());
        }
        original.require_m6_owner(self)?;
        original.revalidate()
    }
    pub(super) fn take_original_read_pipe(&self) -> Result<OwnedFd, BackupError> {
        self.require_original(self.original)?;
        if self.pipe_transfer.swap(true, Ordering::AcqRel) {
            return Err(refused());
        }
        Ok(self
            .child
            .lock()
            .map_err(|_| refused())?
            .as_mut()
            .ok_or_else(refused)?
            .stdout
            .take()
            .ok_or_else(refused)?
            .into())
    }
    pub(super) fn take_original_write_pipe(&self) -> Result<OwnedFd, BackupError> {
        self.require_original(self.original)?;
        if !self.pipe_transfer.load(Ordering::Acquire) {
            return Err(refused());
        }
        Ok(self
            .child
            .lock()
            .map_err(|_| refused())?
            .as_mut()
            .ok_or_else(refused)?
            .stdin
            .take()
            .ok_or_else(refused)?
            .into())
    }
    fn spawn_fixed(&'static self, first: &SourceFirstS<'_>) -> Result<(), BackupError> {
        self.require_original(first.original_owner())?;
        if self.spawn_attempted.swap(true, Ordering::AcqRel) {
            return Err(refused());
        }
        self.original.prepay_wire_work(65536, 128)?;
        self.files.lock().map_err(|_| refused())?.load(first)?;
        let metadata = first.fence_metadata()?;
        let (host, port, user) = metadata.ssh_endpoint_data();
        let mut command = Command::new("/usr/bin/ssh");
        command
            .env_clear()
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .env("PATH", "/usr/bin:/bin");
        command.args([
            "-F",
            "/dev/null",
            "-T",
            "-x",
            "-a",
            "-o",
            "BatchMode=yes",
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            "GlobalKnownHostsFile=/dev/null",
            "-o",
            "UserKnownHostsFile=/var/lib/knowweave-c4/b1/known-hosts",
            "-o",
            "IdentityAgent=none",
            "-o",
            "IdentitiesOnly=yes",
            "-o",
            "PubkeyAuthentication=yes",
            "-o",
            "PasswordAuthentication=no",
            "-o",
            "KbdInteractiveAuthentication=no",
            "-o",
            "PreferredAuthentications=publickey",
            "-o",
            "ClearAllForwardings=yes",
            "-o",
            "ForwardAgent=no",
            "-o",
            "ForwardX11=no",
            "-o",
            "PermitLocalCommand=no",
            "-o",
            "ProxyCommand=none",
            "-o",
            "ProxyJump=none",
            "-o",
            "RequestTTY=no",
            "-o",
            "ControlMaster=no",
            "-o",
            "ControlPath=none",
            "-o",
            "ConnectionAttempts=1",
            "-o",
            "ConnectTimeout=30",
            "-i",
            "/var/lib/knowweave-c4/b1/transport-key",
            "-p",
        ]);
        command
            .arg(port.to_string())
            .arg("-l")
            .arg(user)
            .arg("--")
            .arg(host)
            .arg("/usr/local/libexec/knowweave-c4-b1-receiver");
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // This preserves stdlib's own CLOEXEC exec-error pipe. It never
        // blindly close_range()s that error channel or clears original FD3.
        require_no_extra_inheritable_fds(self.original)?;
        self.files.lock().map_err(|_| refused())?.recheck(first)?;
        let mut child_slot = self.child.lock().map_err(|_| refused())?;
        if child_slot.is_some() {
            return Err(refused());
        }
        require_no_extra_inheritable_fds(self.original)?;
        *child_slot = Some(command.spawn()?); // lock obtained before spawn, then immediate retention
        drop(child_slot);
        recheck_source_fd_count(self.original)?;
        let stderr: OwnedFd = self
            .child
            .lock()
            .map_err(|_| refused())?
            .as_mut()
            .ok_or_else(refused)?
            .stderr
            .take()
            .ok_or_else(refused)?
            .into();
        if self.stderr.set(stderr).is_err() {
            return Err(refused());
        }
        let pair = wire::retain_original_source_pipes(self.original, self)?;
        self.pipes.set(pair).map_err(|_| refused())?;
        self.files.lock().map_err(|_| refused())?.recheck(first)
    }
    pub(super) fn recheck_installed(&self, first: &SourceFirstS<'_>) -> Result<(), BackupError> {
        self.require_original(first.original_owner())?;
        require_fixed_source_fd_room(self.original)?;
        self.files.lock().map_err(|_| refused())?.recheck(first)?;
        recheck_source_fd_count(self.original)
    }
    pub(super) fn begin_original_file(
        &self,
        first: &SourceFirstS<'_>,
        role: wire::FileRole,
        index: usize,
    ) -> Result<(), BackupError> {
        self.recheck_installed(first)?;
        let mut slot = self.stream.lock().map_err(|_| refused())?;
        if slot.is_some() {
            return Err(refused());
        }
        require_fixed_source_fd_room(self.original)?;
        let file = match role {
            wire::FileRole::PackageMember => first
                .pin()?
                .open_package_member_original(index, first.lease_original())?,
            wire::FileRole::SnapshotEffectsMember => first
                .pin()?
                .open_effect_member_original(index, first.lease_original())?,
            _ => return Err(refused()),
        };
        // The fixed no-follow member is stored before the next metadata/read.
        *slot = Some((
            file,
            FileStamp {
                dev: 0,
                ino: 0,
                uid: 0,
                gid: 0,
                mode: 0,
                links: 0,
                len: 0,
                mtime: 0,
                mtime_ns: 0,
                ctime: 0,
                ctime_ns: 0,
            },
        ));
        let (file, stamp) = slot.as_mut().ok_or_else(refused)?;
        self.original.prepay_wire_work(0, 1)?;
        *stamp = FileStamp::of(&file.metadata()?);
        recheck_source_fd_count(self.original)
    }
    pub(super) fn read_original_file(&self, buffer: &mut [u8]) -> Result<usize, BackupError> {
        self.original.first_s_clock()?.check()?;
        recheck_source_fd_count(self.original)?;
        self.original.prepay_wire_work(buffer.len() as u64, 1)?;
        let mut slot = self.stream.lock().map_err(|_| refused())?;
        let (file, stamp) = slot.as_mut().ok_or_else(refused)?;
        if FileStamp::of(&file.metadata()?) != *stamp {
            return Err(refused());
        }
        let count = file.read(buffer)?;
        if FileStamp::of(&file.metadata()?) != *stamp {
            return Err(refused());
        }
        self.original.first_s_clock()?.check()?;
        recheck_source_fd_count(self.original)?;
        Ok(count)
    }
    pub(super) fn finish_original_file(&self) -> Result<(), BackupError> {
        let mut probe = [0u8; 1];
        if self.read_original_file(&mut probe)? != 0 {
            return Err(refused());
        }
        let mut slot = self.stream.lock().map_err(|_| refused())?;
        let (file, stamp) = slot.as_ref().ok_or_else(refused)?;
        if FileStamp::of(&file.metadata()?) != *stamp {
            return Err(refused());
        }
        drop(slot.take());
        recheck_source_fd_count(self.original)
    }
    pub(super) fn trust_data(&self) -> Result<crate::VerifierTrustConfig, BackupError> {
        self.original.prepay_wire_work(32768, 8)?;
        Ok(self
            .files
            .lock()
            .map_err(|_| refused())?
            .trust_data
            .as_ref()
            .ok_or_else(refused)?
            .clone())
    }
    pub(super) fn sign_original_auth(
        &self,
        first: &SourceFirstS<'_>,
        control: &mut wire::SourceAuth,
        io: &wire::OriginalPipeIo,
    ) -> Result<(), BackupError> {
        self.recheck_installed(first)?;
        self.original.prepay_wire_work(131072, 16)?;
        wire::sign_source_auth_with_original_key(
            first,
            control,
            io,
            self.files
                .lock()
                .map_err(|_| refused())?
                .signing_pair
                .as_ref()
                .ok_or_else(refused)?,
        )
    }
    pub(super) fn sign_original_final(
        &self,
        first: &SourceFirstS<'_>,
        control: &mut wire::SourceFinalDecision,
        io: &wire::OriginalPipeIo,
        fence: &mut super::source_fence::DurableSourceFence<'_>,
    ) -> Result<(), BackupError> {
        self.recheck_installed(first)?;
        self.original.prepay_wire_work(131072, 16)?;
        wire::sign_source_final_with_original_key(
            first,
            control,
            io,
            fence,
            self.files
                .lock()
                .map_err(|_| refused())?
                .signing_pair
                .as_ref()
                .ok_or_else(refused)?,
        )
    }
    async fn begin_stderr(&'static self) -> Result<(), BackupError> {
        let mut slot = self.stderr_task.lock().await;
        if slot.is_some() {
            return Err(refused());
        }
        self.original.prepay_wire_work(16384, 8)?;
        let (send, receive) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            receive.await.map_err(|_| refused())?;
            self.drain_original_stderr().await
        });
        *slot = Some(StderrTask {
            handle,
            observed: false,
            result_ok: None,
        });
        send.send(()).map_err(|_| refused())
    }
    async fn drain_original_stderr(&'static self) -> Result<StderrObserved, BackupError> {
        use sha2::{Digest, Sha256};
        struct StderrBorrow(&'static OwnedFd);
        impl AsRawFd for StderrBorrow {
            fn as_raw_fd(&self) -> i32 {
                self.0.as_raw_fd()
            }
        }
        let fd = self.stderr.get().ok_or_else(refused)?;
        self.original.prepay_wire_work(8192, 8)?;
        let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
        let dflags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) };
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        if flags < 0
            || dflags < 0
            || dflags & libc::FD_CLOEXEC == 0
            || flags & libc::O_ACCMODE != libc::O_RDONLY
            || unsafe { libc::fstat(fd.as_raw_fd(), &mut stat) } != 0
            || stat.st_mode & libc::S_IFMT != libc::S_IFIFO
            || unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
        {
            return Err(refused());
        }
        require_fixed_source_fd_room(self.original)?;
        let ready = tokio::io::unix::AsyncFd::new(StderrBorrow(fd))?;
        recheck_source_fd_count(self.original)?;
        let mut raw = [0u8; 8192];
        let mut hash = Sha256::new();
        let mut bytes = 0u64;
        loop {
            // This worker's original handle is retained. Observation timeout
            // does not cancel it or close the FD/SSH child it is draining.
            let mut guard = ready.readable().await?;
            self.original.prepay_wire_work(raw.len() as u64, 1)?;
            let count = match guard.try_io(|fd| {
                let n = unsafe {
                    libc::read(fd.get_ref().as_raw_fd(), raw.as_mut_ptr().cast(), raw.len())
                };
                if n < 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(n as usize)
                }
            }) {
                Ok(Ok(n)) => n,
                Ok(Err(e)) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Ok(Err(e)) => return Err(e.into()),
                Err(_) => continue,
            };
            if count == 0 {
                return Ok(StderrObserved {
                    bytes,
                    sha256: hex::encode(hash.finalize()),
                });
            }
            bytes = bytes
                .checked_add(count as u64)
                .ok_or(BackupError::Overflow)?;
            if bytes > 1_048_576 {
                self.original.reject();
                return Err(BackupError::Capacity("original SSH stderr"));
            }
            hash.update(&raw[..count]);
        }
    }
    pub(super) async fn observe_negative_after_original_worker(
        &'static self,
    ) -> Result<(), BackupError> {
        self.original.require_negative_m6_custody(self)?;
        {
            let mut io = self.io.try_lock().map_err(|_| refused())?;
            if let Some(io) = io.as_mut() {
                io.close_output_negative_after_original_worker(self)?;
            }
        }
        wire::close_original_source_output_negative(self, self.original)?;
        {
            let mut child = self.child.lock().map_err(|_| refused())?;
            if let Some(child) = child.as_mut() {
                drop(child.stdin.take());
            } else {
                return Ok(());
            }
        }

        // This fixed negative waiter storage was prepaid with the SSH Book.
        // The actual Child remains in its slot; no kill or replacement occurs.
        let deadline = Instant::now() + Duration::from_secs(super::profile::SETTLEMENT_SECONDS);
        let mut wait = tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            self.negative_wait.lock(),
        )
        .await
        .map_err(|_| refused())?;
        if wait.is_none() {
            let (start, started) = std::sync::mpsc::sync_channel::<()>(1);
            let handle = tokio::task::spawn_blocking(move || {
                started.recv().map_err(|_| refused())?;
                let mut child = self.child.lock().map_err(|_| refused())?;
                child.as_mut().ok_or_else(refused)?.wait()?;
                Ok(())
            });
            *wait = Some(NegativeWaitTask {
                handle,
                observed: false,
                result_ok: None,
            });
            start.send(()).map_err(|_| refused())?;
        }
        // Only the borrowed observer is timed out. The original started
        // blocking waiter and Child are still retained in Book on unknown.
        let actual = wait.as_mut().ok_or_else(refused)?;
        if !actual.observed {
            let result = tokio::time::timeout_at(
                tokio::time::Instant::from_std(deadline),
                &mut actual.handle,
            )
            .await
            .map_err(|_| refused())?;
            actual.observed = true;
            actual.result_ok = Some(result.is_ok_and(|value| value.is_ok()));
        }
        if actual.result_ok != Some(true) {
            return Err(refused());
        }
        drop(wait);
        let mut stderr = tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            self.stderr_task.lock(),
        )
        .await
        .map_err(|_| refused())?;
        if let Some(task) = stderr.as_mut() {
            if !task.observed {
                task.observe_original_once(deadline).await?;
            }
            // Natural error is known negative, not an EOF/success receipt. The
            // consumed JoinHandle is never repolled by this later observer.
            if task.result_ok != Some(true) {
                return Err(refused());
            }
        }
        Ok(())
    }
    async fn observe_natural_exit(&'static self) -> Result<(), BackupError> {
        let deadline = self.original.first_s_clock()?.deadline()?;
        loop {
            self.original.prepay_wire_work(0, 1)?;
            let status = self
                .child
                .lock()
                .map_err(|_| refused())?
                .as_mut()
                .ok_or_else(refused)?
                .try_wait()?;
            if let Some(status) = status {
                if !status.success() {
                    return Err(refused());
                }
                break;
            }
            if Instant::now() >= deadline {
                return Err(BackupError::Invalid("original SSH exit unconfirmed"));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let mut slot = self.stderr_task.lock().await;
        let task = slot.as_mut().ok_or_else(refused)?;
        if task.observed {
            return Err(refused());
        }
        let actual = task.observe_original_once(deadline).await?;
        if actual.bytes > 1_048_576 || !crate::valid_digest(&actual.sha256) {
            return Err(refused());
        }
        Ok(())
    }
}
// Fixed original-owner guards, never a caller-selected quota or FD authority.
// Six stdio pipe ends plus the stdlib exec-error pipe need eight temporary
// descriptors; the fixed sixteen-slot margin also conservatively covers the
// fixed metadata/pin clone-and-walk checkpoints and runtime setup. Each
// retained metadata/protection/pin stage is guarded separately so its newly
// retained descriptors are counted before the next fixed stage, not hidden
// inside one initial census. Recursive/native external peaks remain subject
// to the C installation inventory obligation below.
// The census includes its own observer FD. This enforces local checkpoints;
// C's actual whole-process/runtime/SSH inventory and peak proof remain OPEN.
#[derive(Clone, Copy)]
enum SourceFdProbe {
    Current,
    FixedWork,
    BeforeExec,
}
fn fd_capacity_data(observed: usize, reserved: usize) -> Result<(), BackupError> {
    let limit = usize::try_from(super::profile::PRODUCT_FDS).map_err(|_| refused())?;
    if observed
        .checked_add(reserved)
        .is_none_or(|total| total > limit)
    {
        return Err(BackupError::Capacity(
            "Source actual FD roster and fixed headroom",
        ));
    }
    Ok(())
}
fn inventory_original_source_fds(
    original: &SourceFrontOriginal,
    probe: SourceFdProbe,
) -> Result<(), BackupError> {
    let observed = (|| {
        // Prepay the bounded observer/entries/metadata before opening read_dir.
        original.prepay_wire_work(65536, super::profile::PRODUCT_FDS as usize + 2)?;
        let reserved = match probe {
            SourceFdProbe::Current => 0,
            SourceFdProbe::FixedWork | SourceFdProbe::BeforeExec => 16,
        };
        let mut count = 0usize;
        for entry in std::fs::read_dir("/proc/self/fd")? {
            count = count.checked_add(1).ok_or(BackupError::Overflow)?;
            fd_capacity_data(count, reserved)?;
            let name = entry?.file_name();
            let fd = name
                .to_str()
                .and_then(|s| s.parse::<i32>().ok())
                .ok_or_else(refused)?;
            if matches!(probe, SourceFdProbe::BeforeExec) && fd >= 3 {
                let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
                if flags < 0 || flags & libc::FD_CLOEXEC == 0 {
                    return Err(refused());
                }
            }
        }
        Ok(())
    })();
    if observed.is_err() {
        original.reject();
    }
    observed
}
pub(super) fn require_fixed_source_fd_room(
    original: &SourceFrontOriginal,
) -> Result<(), BackupError> {
    inventory_original_source_fds(original, SourceFdProbe::FixedWork)
}
pub(super) fn recheck_source_fd_count(original: &SourceFrontOriginal) -> Result<(), BackupError> {
    inventory_original_source_fds(original, SourceFdProbe::Current)
}
fn require_no_extra_inheritable_fds(original: &SourceFrontOriginal) -> Result<(), BackupError> {
    inventory_original_source_fds(original, SourceFdProbe::BeforeExec)
}

pub(super) async fn run_source_m6_in_first_s(
    first: &mut SourceFirstS<'_>,
) -> Result<(), BackupError> {
    first.recheck().await?;
    let original = first.retain_m6_original()?;
    original.spawn_fixed(first)?;
    original.begin_stderr().await?;
    first.recheck().await?;
    let pair = *original.pipes.get().ok_or_else(refused)?;
    let mut slot = original.io.lock().await;
    if slot.is_some() {
        first.reject();
        return Err(refused());
    }
    require_fixed_source_fd_room(first.original_owner())?;
    *slot = Some(wire::OriginalPipeIo::for_original_source(pair, first)?);
    recheck_source_fd_count(first.original_owner())?;
    let result =
        wire::run_original_source(first, original, slot.as_mut().ok_or_else(refused)?).await;
    if result.is_err() {
        first.reject();
        return result;
    }
    original.observe_natural_exit().await?;
    first.recheck().await
}

/// The one standard OpenSSH SHA256 fingerprint domain: the canonical SSH
/// public-key wire blob itself, excluding an outer string length or .pub text.
/// The initial noninteractive profile admits an unencrypted Ed25519 transport
/// key only. This parses DATA; it never installs, generates or signs with it.
fn checked_openssh_transport_public_blob(raw: &[u8]) -> Result<Vec<u8>, BackupError> {
    const BEGIN: &[u8] = b"-----BEGIN OPENSSH PRIVATE KEY-----\n";
    const END: &[u8] = b"-----END OPENSSH PRIVATE KEY-----\n";
    if raw.len() > 4096 || !raw.starts_with(BEGIN) || !raw.ends_with(END) {
        return Err(refused());
    }
    let text = &raw[BEGIN.len()..raw.len() - END.len()];
    let mut encoded = Vec::with_capacity(text.len());
    for line in text.split(|b| *b == b'\n') {
        if line.is_empty() {
            continue;
        }
        if line.len() > 76
            || line
                .iter()
                .any(|b| !b.is_ascii_alphanumeric() && !b"+/=".contains(b))
        {
            return Err(refused());
        }
        encoded.extend_from_slice(line);
    }
    let decoded = strict_base64(&encoded)?;
    let magic = b"openssh-key-v1\0";
    if !decoded.starts_with(magic) {
        return Err(refused());
    }
    let mut input = &decoded[magic.len()..];
    if ssh_string(&mut input)? != b"none"
        || ssh_string(&mut input)? != b"none"
        || !ssh_string(&mut input)?.is_empty()
        || ssh_u32(&mut input)? != 1
    {
        return Err(refused());
    }
    let public = ssh_string(&mut input)?;
    let private = ssh_string(&mut input)?;
    if !input.is_empty() {
        return Err(refused());
    }
    let mut public_fields = public;
    if ssh_string(&mut public_fields)? != b"ssh-ed25519" {
        return Err(refused());
    }
    let key = ssh_string(&mut public_fields)?;
    if key.len() != 32 || !public_fields.is_empty() {
        return Err(refused());
    }
    let mut private_fields = private;
    if ssh_u32(&mut private_fields)? != ssh_u32(&mut private_fields)?
        || ssh_string(&mut private_fields)? != b"ssh-ed25519"
        || ssh_string(&mut private_fields)? != key
    {
        return Err(refused());
    }
    let secret = ssh_string(&mut private_fields)?;
    if secret.len() != 64 || &secret[32..] != key {
        return Err(refused());
    }
    let pair =
        Ed25519KeyPair::from_seed_and_public_key(&secret[..32], key).map_err(|_| refused())?;
    if pair.public_key().as_ref() != key
        || ssh_string(&mut private_fields)?.len() > 1024
        || private_fields.is_empty()
        || private_fields.len() > 8
        || private_fields
            .iter()
            .enumerate()
            .any(|(i, b)| usize::from(*b) != i + 1)
    {
        return Err(refused());
    }
    Ok(public.to_vec())
}
fn require_distinct_ed25519_material(
    source_auth: &[u8],
    transport_blob: &[u8],
) -> Result<(), BackupError> {
    if source_auth.len() != 32 {
        return Err(refused());
    }
    let mut fields = transport_blob;
    if ssh_string(&mut fields)? != b"ssh-ed25519" {
        return Err(refused());
    }
    let transport_public = ssh_string(&mut fields)?;
    if transport_public.len() != 32 || !fields.is_empty() || source_auth == transport_public {
        return Err(refused());
    }
    Ok(())
}
fn ssh_u32(input: &mut &[u8]) -> Result<u32, BackupError> {
    let head = input.get(..4).ok_or_else(refused)?;
    let value = u32::from_be_bytes(head.try_into().map_err(|_| refused())?);
    *input = &input[4..];
    Ok(value)
}
fn ssh_string<'a>(input: &mut &'a [u8]) -> Result<&'a [u8], BackupError> {
    let len = usize::try_from(ssh_u32(input)?).map_err(|_| refused())?;
    let data = input.get(..len).ok_or_else(refused)?;
    *input = &input[len..];
    Ok(data)
}
fn strict_base64(raw: &[u8]) -> Result<Vec<u8>, BackupError> {
    if raw.is_empty() || !raw.len().is_multiple_of(4) || raw.len() > 4096 {
        return Err(refused());
    }
    fn digit(byte: u8) -> Result<u8, BackupError> {
        match byte {
            b'A'..=b'Z' => Ok(byte - b'A'),
            b'a'..=b'z' => Ok(byte - b'a' + 26),
            b'0'..=b'9' => Ok(byte - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err(refused()),
        }
    }
    let mut output = Vec::with_capacity(raw.len() / 4 * 3);
    for (i, chunk) in raw.chunks_exact(4).enumerate() {
        let a = digit(chunk[0])?;
        let b = digit(chunk[1])?;
        let last = i + 1 == raw.len() / 4;
        output.push((a << 2) | (b >> 4));
        if chunk[2] == b'=' {
            if !last || chunk[3] != b'=' || b & 15 != 0 {
                return Err(refused());
            }
            continue;
        }
        let c = digit(chunk[2])?;
        output.push((b << 4) | (c >> 2));
        if chunk[3] == b'=' {
            if !last || c & 3 != 0 {
                return Err(refused());
            }
            continue;
        }
        output.push((c << 6) | digit(chunk[3])?);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_transport_base64_rejects_nonzero_pad_bits_and_embedded_padding() {
        assert_eq!(strict_base64(b"TQ==").unwrap(), b"M");
        assert_eq!(strict_base64(b"TWE=").unwrap(), b"Ma");
        for raw in [
            b"TR==".as_slice(),
            b"TWF=".as_slice(),
            b"TQ==TQ==".as_slice(),
            b"TQ=\n".as_slice(),
        ] {
            assert!(strict_base64(raw).is_err());
        }
    }
    #[test]
    fn transport_public_blob_requires_actual_private_envelope_not_pub_text() {
        for raw in [
            b"ssh-ed25519 AAAA comment\n".as_slice(),
            b"-----BEGIN OPENSSH PRIVATE KEY-----\nTQ==\n-----END OPENSSH PRIVATE KEY-----\n"
                .as_slice(),
        ] {
            assert!(checked_openssh_transport_public_blob(raw).is_err());
        }
    }
    fn base64_fixture(raw: &[u8]) -> Vec<u8> {
        let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut output = Vec::new();
        for chunk in raw.chunks(3) {
            let a = chunk[0];
            let b = *chunk.get(1).unwrap_or(&0);
            let c = *chunk.get(2).unwrap_or(&0);
            output.push(alphabet[(a >> 2) as usize]);
            output.push(alphabet[(((a & 3) << 4) | (b >> 4)) as usize]);
            output.push(if chunk.len() > 1 {
                alphabet[(((b & 15) << 2) | (c >> 6)) as usize]
            } else {
                b'='
            });
            output.push(if chunk.len() > 2 {
                alphabet[(c & 63) as usize]
            } else {
                b'='
            });
        }
        output
    }
    fn openssh_rfc8032_fixture(cipher: &[u8], break_private: bool) -> Vec<u8> {
        fn string(output: &mut Vec<u8>, raw: &[u8]) {
            output.extend_from_slice(&(raw.len() as u32).to_be_bytes());
            output.extend_from_slice(raw);
        }
        // RFC8032 public test vector, never an installed or generated key.
        let public =
            hex::decode("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")
                .unwrap();
        let mut seed =
            hex::decode("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")
                .unwrap();
        if break_private {
            seed[0] ^= 1;
        }
        let mut blob = Vec::new();
        string(&mut blob, b"ssh-ed25519");
        string(&mut blob, &public);
        let mut private = Vec::new();
        private.extend_from_slice(&0x12345678u32.to_be_bytes());
        private.extend_from_slice(&0x12345678u32.to_be_bytes());
        string(&mut private, b"ssh-ed25519");
        string(&mut private, &public);
        seed.extend_from_slice(&public);
        string(&mut private, &seed);
        string(&mut private, b"");
        let padding = 8 - private.len() % 8;
        for byte in 1..=padding {
            private.push(byte as u8);
        }
        let mut data = b"openssh-key-v1\0".to_vec();
        string(&mut data, cipher);
        string(&mut data, b"none");
        string(&mut data, b"");
        data.extend_from_slice(&1u32.to_be_bytes());
        string(&mut data, &blob);
        string(&mut data, &private);
        let encoded = base64_fixture(&data);
        let mut pem = b"-----BEGIN OPENSSH PRIVATE KEY-----\n".to_vec();
        for line in encoded.chunks(70) {
            pem.extend_from_slice(line);
            pem.push(b'\n');
        }
        pem.extend_from_slice(b"-----END OPENSSH PRIVATE KEY-----\n");
        pem
    }
    #[test]
    fn openssh_transport_fingerprint_uses_only_the_standard_public_wire_blob() {
        let pem = openssh_rfc8032_fixture(b"none", false);
        let public = checked_openssh_transport_public_blob(&pem).unwrap();
        assert_eq!(public.len(), 51);
        assert_eq!(
            crate::digest(&public),
            "6db5e9b8a1bace1cdd9a7c6adb9e9396acc5073465d9fe8e3a0ef6d9c60d6d4f"
        );
        assert_ne!(crate::digest(&public), crate::digest(&public[19..]));
        let mut outer = (public.len() as u32).to_be_bytes().to_vec();
        outer.extend_from_slice(&public);
        assert_ne!(crate::digest(&public), crate::digest(&outer));
        assert_ne!(crate::digest(&public), crate::digest(&pem));
    }
    #[test]
    fn transport_parser_rejects_encryption_and_private_public_mismatch() {
        assert!(
            checked_openssh_transport_public_blob(&openssh_rfc8032_fixture(b"aes256-ctr", false))
                .is_err()
        );
        assert!(
            checked_openssh_transport_public_blob(&openssh_rfc8032_fixture(b"none", true)).is_err()
        );
    }

    #[test]
    fn source_auth_and_transport_compare_material_not_incompatible_hash_domains() {
        let pem = openssh_rfc8032_fixture(b"none", false);
        let transport = checked_openssh_transport_public_blob(&pem).unwrap();
        let same = hex::decode("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")
            .unwrap();
        assert_ne!(crate::digest(&same), crate::digest(&transport));
        assert!(require_distinct_ed25519_material(&same, &transport).is_err());
        let other = hex::decode("3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c")
            .unwrap();
        assert!(require_distinct_ed25519_material(&other, &transport).is_ok());
        assert!(require_distinct_ed25519_material(&other[..31], &transport).is_err());
        let mut extra = transport.clone();
        extra.push(0);
        assert!(require_distinct_ed25519_material(&other, &extra).is_err());
    }

    #[test]
    fn transport_and_host_trust_profile_never_raise_the_four_kib_key_limit() {
        let oversized = vec![b'A'; 4097];
        assert!(checked_openssh_transport_public_blob(&oversized).is_err());
        assert!(strict_base64(&[b'A'; 4100]).is_err());
    }
}

#[cfg(test)]
mod original_resource_component_tests {
    use super::*;
    #[test]
    fn source_fd_headroom_data_rejects_the_peak_before_the_fixed_work() {
        assert!(fd_capacity_data(48, 16).is_ok());
        assert!(fd_capacity_data(49, 16).is_err());
        assert!(fd_capacity_data(64, 0).is_ok());
        assert!(fd_capacity_data(65, 0).is_err());
        assert!(fd_capacity_data(usize::MAX, 1).is_err());
    }
    #[tokio::test]
    async fn stderr_natural_inner_error_is_recorded_before_later_negative_observation() {
        let handle =
            tokio::spawn(async { Err(BackupError::Invalid("ordinary stderr component error")) });
        let mut task = StderrTask {
            handle,
            observed: false,
            result_ok: None,
        };
        let deadline = Instant::now() + Duration::from_secs(2);
        assert!(task.observe_original_once(deadline).await.is_err());
        assert!(task.observed);
        assert_eq!(task.result_ok, Some(false));
        assert!(task.observe_original_once(deadline).await.is_err());
        assert!(task.observed);
        assert_eq!(task.result_ok, Some(false));
    }
    #[tokio::test]
    async fn stderr_natural_join_error_is_never_polled_again() {
        async fn original_component_panics() -> Result<StderrObserved, BackupError> {
            panic!("ordinary stderr component panic")
        }
        let handle = tokio::spawn(original_component_panics());
        let mut task = StderrTask {
            handle,
            observed: false,
            result_ok: None,
        };
        let deadline = Instant::now() + Duration::from_secs(2);
        assert!(task.observe_original_once(deadline).await.is_err());
        assert!(task.observed);
        assert_eq!(task.result_ok, Some(false));
        assert!(task.observe_original_once(deadline).await.is_err());
    }
    #[tokio::test]
    async fn stderr_observer_timeout_keeps_original_worker_for_natural_return() {
        let (send, receive) = tokio::sync::oneshot::channel::<()>();
        let handle = tokio::spawn(async move {
            receive.await.map_err(|_| refused())?;
            Ok(StderrObserved {
                bytes: 0,
                sha256: crate::digest(b""),
            })
        });
        let mut task = StderrTask {
            handle,
            observed: false,
            result_ok: None,
        };
        assert!(
            task.observe_original_once(Instant::now() + Duration::from_millis(10))
                .await
                .is_err()
        );
        assert!(!task.observed);
        assert_eq!(task.result_ok, None);
        send.send(()).unwrap();
        let actual = task
            .observe_original_once(Instant::now() + Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(actual.bytes, 0);
        assert_eq!(actual.sha256, crate::digest(b""));
        assert!(task.observed);
        assert_eq!(task.result_ok, Some(true));
        assert!(
            task.observe_original_once(Instant::now() + Duration::from_secs(2))
                .await
                .is_err()
        );
    }
}

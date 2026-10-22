use super::{
    CLIENT_GID, CLIENT_UID, ClientConfig, DISABLED, DisabledReply, Signal, SignalDisposition,
    invalid, wire,
};
use std::{
    ffi::CString,
    fs::{self, File},
    io::{self, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{FileTypeExt, MetadataExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::{Component, Path, PathBuf},
};
use uuid::Uuid;

/// An optional native-controller start barrier. It carries no job authority.
/// The same process must receive one fixed marker and EOF within five seconds.
pub fn wait_for_stdin_start() -> io::Result<()> {
    wire::read_start_gate(libc::STDIN_FILENO, std::time::Duration::from_secs(5))
}

fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "untrusted snapshot broker custody or peer",
    )
}
fn from_fd(raw: libc::c_int) -> io::Result<File> {
    if raw < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(raw) })
    }
}
fn name(value: &str) -> io::Result<CString> {
    if value.is_empty() || value.contains('/') || value == "." || value == ".." {
        return Err(invalid());
    }
    CString::new(value).map_err(|_| invalid())
}
fn trusted(metadata: &fs::Metadata, mode: Option<u32>) -> io::Result<()> {
    trusted_group(metadata, mode, 0)
}
fn trusted_group(metadata: &fs::Metadata, mode: Option<u32>, group: u32) -> io::Result<()> {
    if metadata.uid() != 0
        || metadata.gid() != group
        || metadata.mode() & 0o022 != 0
        || mode.is_some_and(|expected| metadata.mode() & 0o777 != expected)
    {
        return Err(denied());
    }
    Ok(())
}

struct Directory(File);
impl Directory {
    fn open(path: &Path, mode: u32, group: u32, public: bool) -> io::Result<Self> {
        if !path.is_absolute() {
            return Err(invalid());
        }
        let root = CString::new("/").expect("literal");
        let mut directory = Self(from_fd(unsafe {
            libc::open(
                root.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        })?);
        trusted(&directory.0.metadata()?, None)?;
        let components: Vec<_> = path.components().collect();
        for (index, component) in components.iter().enumerate() {
            match component {
                Component::RootDir => continue,
                Component::Normal(part) => {
                    let part = CString::new(part.as_bytes()).map_err(|_| invalid())?;
                    directory = directory.open_directory(&part)?;
                }
                _ => return Err(invalid()),
            }
            let metadata = directory.0.metadata()?;
            let last = index == components.len() - 1;
            trusted_group(&metadata, None, if last { group } else { 0 })?;
            if public && !last && metadata.mode() & 0o001 == 0 {
                return Err(denied());
            }
        }
        trusted_group(&directory.0.metadata()?, Some(mode), group)?;
        Ok(directory)
    }

    fn open_directory(&self, part: &CString) -> io::Result<Self> {
        Ok(Self(from_fd(unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                part.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        })?))
    }

    fn create_directory(&self, value: &str, mode: u32, allow_existing: bool) -> io::Result<Self> {
        let value = name(value)?;
        let result = unsafe { libc::mkdirat(self.0.as_raw_fd(), value.as_ptr(), mode) };
        if result != 0 {
            let error = io::Error::last_os_error();
            if !allow_existing || error.kind() != io::ErrorKind::AlreadyExists {
                return Err(error);
            }
        }
        let child = self.open_directory(&value)?;
        if result == 0 {
            child.0.set_permissions(fs::Permissions::from_mode(mode))?;
        }
        trusted(&child.0.metadata()?, Some(mode))?;
        self.0.sync_all()?;
        child.0.sync_all()?;
        Ok(child)
    }

    fn file(&self, value: &str, exclusive: bool) -> io::Result<File> {
        let value = name(value)?;
        let flags = libc::O_RDWR
            | libc::O_CREAT
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | if exclusive { libc::O_EXCL } else { 0 };
        let file =
            from_fd(unsafe { libc::openat(self.0.as_raw_fd(), value.as_ptr(), flags, 0o600) })?;
        let metadata = file.metadata()?;
        trusted(&metadata, Some(0o600))?;
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Err(denied());
        }
        self.0.sync_all()?;
        Ok(file)
    }

    fn same_file(&self, value: &str, held: &File) -> io::Result<()> {
        let value = name(value)?;
        let reopened = from_fd(unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                value.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        })?;
        let before = held.metadata()?;
        let after = reopened.metadata()?;
        trusted(&after, Some(0o600))?;
        if (before.dev(), before.ino()) != (after.dev(), after.ino())
            || !after.is_file()
            || after.nlink() != 1
            || after.len() != 0
        {
            return Err(denied());
        }
        Ok(())
    }
}

/// Optional Root controller companion. There is deliberately no database pool,
/// claim method, export root, writer permit or user delivery endpoint in S1.
/// Private File fields are never cloned or handed to worker clients. Closing
/// this original object releases broker.lock; no Drop issues LOCK_UN early.
pub struct DisabledBroker {
    custody_path: PathBuf,
    root: Directory,
    broker_lock: File,
    _epoch_directory: Directory,
    _effect_lock: File,
    _journal: Directory,
    ipc_parent_path: PathBuf,
    ipc_parent: Directory,
    socket_leaf: Directory,
    listener: UnixListener,
    socket: PathBuf,
    socket_identity: (u64, u64),
    epoch: Uuid,
}
impl DisabledBroker {
    /// Both existing roots must be managed by Root: private custody 0700,
    /// private IPC host parent 0700. Only the 0750 root:65532 epoch leaf is
    /// mounted read-only into a client namespace. Epoch leaves
    /// are fresh and are never unlinked/rebound, including after restart.
    pub fn bind(custody_root: &Path, ipc_parent: &Path, epoch: Uuid) -> io::Result<Self> {
        if unsafe { libc::geteuid() } != 0 || unsafe { libc::getegid() } != 0 || epoch.is_nil() {
            return Err(denied());
        }
        let root = Directory::open(custody_root, 0o700, 0, false)?;
        let ipc_parent_handle = Directory::open(ipc_parent, 0o700, 0, false)?;
        let broker_lock = root.file("broker.lock", false)?;
        if broker_lock.metadata()?.len() != 0 {
            return Err(denied());
        }
        if unsafe { libc::flock(broker_lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let epochs = root.create_directory("epochs", 0o700, true)?;
        let epoch_directory = epochs.create_directory(&epoch.to_string(), 0o700, false)?;
        let effect_lock = epoch_directory.file("effect.lock", true)?;
        effect_lock.sync_all()?;
        let journal = epoch_directory.create_directory("journal", 0o700, false)?;
        let mut state = epoch_directory.file("epoch.json", true)?;
        let bytes = serde_json::to_vec(&serde_json::json!({
            "format_version":2,"epoch":epoch,"state":"S1_WRITER_DISABLED_NO_DATABASE",
            "client_uid":CLIENT_UID,"client_gid":CLIENT_GID,"effects":"UNKNOWN"
        }))
        .map_err(|_| invalid())?;
        state.write_all(&bytes)?;
        state.sync_all()?;
        epoch_directory.0.sync_all()?;
        // No old leaf is removed on bind failure or Drop. A subsequent broker
        // must use a new epoch, rather than revive authority from disk files.
        let socket_leaf = ipc_parent_handle.create_directory(&epoch.to_string(), 0o750, false)?;
        if unsafe { libc::fchown(socket_leaf.0.as_raw_fd(), 0, CLIENT_GID) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let socket = ipc_parent.join(epoch.to_string()).join("work.sock");
        let listener = UnixListener::bind(&socket)?;
        let socket_name = name("work.sock")?;
        if unsafe {
            libc::fchownat(
                socket_leaf.0.as_raw_fd(),
                socket_name.as_ptr(),
                0,
                CLIENT_GID,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o660))?;
        let metadata = fs::symlink_metadata(&socket)?;
        if !metadata.file_type().is_socket() || metadata.uid() != 0 || metadata.gid() != CLIENT_GID
        {
            return Err(denied());
        }
        socket_leaf.0.sync_all()?;
        let broker = Self {
            custody_path: custody_root.to_path_buf(),
            root,
            broker_lock,
            _epoch_directory: epoch_directory,
            _effect_lock: effect_lock,
            _journal: journal,
            ipc_parent_path: ipc_parent.to_path_buf(),
            ipc_parent: ipc_parent_handle,
            socket_leaf,
            listener,
            socket,
            socket_identity: (metadata.dev(), metadata.ino()),
            epoch,
        };
        broker.recheck()?;
        Ok(broker)
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket
    }

    fn recheck(&self) -> io::Result<()> {
        trusted(&self.root.0.metadata()?, Some(0o700))?;
        trusted(&self.ipc_parent.0.metadata()?, Some(0o700))?;
        // A held descriptor survives a whole-directory rename. Rebind both
        // configured paths without following links before trusting that held
        // custody, so a replacement root cannot leave the old broker serving.
        for (path, held) in [
            (&self.custody_path, &self.root),
            (&self.ipc_parent_path, &self.ipc_parent),
        ] {
            let current = Directory::open(path, 0o700, 0, false)?;
            let current_meta = current.0.metadata()?;
            let held_meta = held.0.metadata()?;
            if (current_meta.dev(), current_meta.ino()) != (held_meta.dev(), held_meta.ino()) {
                return Err(denied());
            }
        }
        self.root.same_file("broker.lock", &self.broker_lock)?;
        let current = self
            .ipc_parent
            .open_directory(&name(&self.epoch.to_string())?)?;
        let current_meta = current.0.metadata()?;
        let held_meta = self.socket_leaf.0.metadata()?;
        trusted_group(&current_meta, Some(0o750), CLIENT_GID)?;
        if (current_meta.dev(), current_meta.ino()) != (held_meta.dev(), held_meta.ino()) {
            return Err(denied());
        }
        let socket_meta = fs::symlink_metadata(&self.socket)?;
        if !socket_meta.file_type().is_socket()
            || socket_meta.uid() != 0
            || socket_meta.gid() != CLIENT_GID
            || socket_meta.mode() & 0o777 != 0o660
            || socket_meta.nlink() != 1
            || (socket_meta.dev(), socket_meta.ino()) != self.socket_identity
        {
            return Err(denied());
        }
        Ok(())
    }

    /// Validate exactly one signal. All rejection paths close the connection.
    /// Even an authenticated signal only receives disabled/Unknown, no job ID,
    /// claim token, receipt or positive external-effect statement.
    pub fn accept_disabled_signal(&self) -> io::Result<()> {
        self.recheck()?;
        let (stream, _) = self.listener.accept()?;
        self.handle_disabled_signal(stream)
    }

    fn handle_disabled_signal(&self, stream: UnixStream) -> io::Result<()> {
        if wire::peer(&stream)? != (CLIENT_UID, CLIENT_GID) {
            return Err(denied());
        }
        let signal = Signal::parse(&wire::read_frame(&stream)?)?;
        self.recheck()?;
        let reply = DisabledReply {
            format_version: 2,
            epoch: self.epoch.to_string(),
            request_id: signal.request_id,
            disposition: DISABLED.into(),
        };
        wire::write_frame(&stream, &serde_json::to_vec(&reply).map_err(|_| invalid())?)
    }

    pub fn serve_disabled(&self) -> io::Result<()> {
        loop {
            // Bad client input does not kill a healthy Root custody process.
            // Custody replacement/permission changes do stop the service.
            self.recheck()?;
            let (stream, _) = self.listener.accept()?;
            let _ = self.handle_disabled_signal(stream);
        }
    }
}

/// A successful active exchange returns only a redacted scheduling state.
/// Transport timeout does not cancel the Root owner's operation or grant an
/// effect finding. The disabled response remains explicitly unavailable.
pub fn signal_next(config: &ClientConfig) -> io::Result<SignalDisposition> {
    config.validate()?;
    let leaf = config.socket.parent().ok_or_else(invalid)?;
    let held_leaf = Directory::open(leaf, 0o750, CLIENT_GID, true)?;
    let before = fs::symlink_metadata(&config.socket)?;
    if !before.file_type().is_socket()
        || before.uid() != 0
        || before.gid() != CLIENT_GID
        || before.mode() & 0o777 != 0o660
        || before.nlink() != 1
    {
        return Err(denied());
    }
    let stream = wire::connect(&config.socket)?;
    if wire::peer(&stream)? != (0, 0) {
        return Err(denied());
    }
    wire::write_frame(
        &stream,
        &serde_json::to_vec(&Signal::for_client(config)).map_err(|_| invalid())?,
    )?;
    // The original broker owner has a 60s publication deadline. Waiting for
    // its response uses one absolute bound; the server's request frame keeps
    // its original 2s bound, and no partial reply can reset this deadline.
    let wait = std::time::Duration::from_millis(
        config.lease_ms.saturating_add(5_000).clamp(65_000, 300_000),
    );
    let reply = DisabledReply::parse(&wire::read_reply(&stream, wait)?, config)?;
    let current_leaf = Directory::open(leaf, 0o750, CLIENT_GID, true)?;
    let held_parent = held_leaf.0.metadata()?;
    let current_parent = current_leaf.0.metadata()?;
    let after = fs::symlink_metadata(&config.socket)?;
    if (before.dev(), before.ino()) != (after.dev(), after.ino())
        || (held_parent.dev(), held_parent.ino()) != (current_parent.dev(), current_parent.ino())
    {
        return Err(invalid());
    }
    Ok(reply)
}

use crate::BackupError;
use crate::FileRecord;
use learning_assets::backup_fs::BackupDir;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
pub const MAX_DUMP: u64 = 1024 * 1024 * 1024;
pub const MAX_DECODE: u64 = 4 * 1024 * 1024 * 1024;
pub const MAX_ROW: u64 = 64 * 1024 * 1024;
pub const BUFFER: usize = 64 * 1024;
#[cfg(test)]
use super::dump_observer::{Guard, Observer, Step};
#[cfg(any(test, target_os = "linux"))]
const CLIENT_SHA: &str = "eba5f8a8361918f873e273b1107dd727c19fc4269e8f98854ed5589555d668c6";
#[cfg(any(test, target_os = "linux"))]
const VERSION: &[u8] = b"pg_restore (PostgreSQL) 18.6 (Debian 18.6-1.pgdg12+2)\n";
#[cfg(any(test, target_os = "linux"))]
fn decoder_bad() -> BackupError {
    BackupError::Invalid("fixed PG18 decoder")
}

#[cfg(any(test, target_os = "linux"))]
pub(super) fn require_decoder_version(
    raw: &[u8],
    #[cfg(test)] observer: Option<&Observer>,
) -> Result<(), BackupError> {
    if raw != VERSION {
        Err(dump_error!(
            observer,
            Step::VersionBytes,
            Guard::VersionBytes,
            decoder_bad()
        ))
    } else {
        Ok(())
    }
}

#[cfg(any(test, target_os = "linux"))]
pub(super) fn require_client_hash(
    held: &mut File,
    #[cfg(test)] observer: Option<&Observer>,
) -> Result<(), BackupError> {
    dump_observe!(
        observer,
        Step::DecoderClient,
        Guard::ClientHash,
        held.seek(SeekFrom::Start(0)).map_err(BackupError::from)
    )?;
    let mut writer = Hashing {
        output: &mut std::io::sink(),
        hash: Sha256::new(),
    };
    dump_observe!(
        observer,
        Step::DecoderClient,
        Guard::ClientHash,
        bounded_copy(held, &mut writer, 32 * 1024 * 1024)
    )?;
    if hex::encode(writer.hash.finalize()) != CLIENT_SHA {
        return Err(dump_error!(
            observer,
            Step::DecoderClient,
            Guard::ClientHash,
            decoder_bad()
        ));
    }
    dump_observe!(
        observer,
        Step::DecoderClient,
        Guard::ClientHash,
        held.seek(SeekFrom::Start(0)).map_err(BackupError::from)
    )?;
    Ok(())
}

#[cfg(any(test, target_os = "linux"))]
pub(super) fn require_run_ready(
    closed: bool,
    expired: bool,
    #[cfg(test)] operation: Operation,
    #[cfg(test)] observer: Option<&Observer>,
) -> Result<(), BackupError> {
    if closed {
        return Err(dump_error!(
            observer,
            operation.run_step(),
            Guard::ClosedBeforeRun,
            decoder_bad()
        ));
    }
    if expired {
        return Err(dump_error!(
            observer,
            operation.run_step(),
            Guard::DeadlineBeforeRun,
            decoder_bad()
        ));
    }
    Ok(())
}

#[cfg(any(test, target_os = "linux"))]
pub(super) fn native_predicate(
    status: std::process::ExitStatus,
    stderr_nonempty: bool,
    #[cfg(test)] operation: Operation,
    #[cfg(test)] observer: Option<&Observer>,
) -> Result<(), BackupError> {
    let result = if !status.success() || stderr_nonempty {
        Err(BackupError::Invalid("pg_restore decode exit"))
    } else {
        Ok(())
    };
    #[cfg(test)]
    if let Some(observer) = observer {
        observer.native_result(
            operation,
            Guard::NativePredicate,
            &result,
            Some(status),
            stderr_nonempty,
        );
    }
    result
}

#[cfg(test)]
pub(super) fn retain_native_result<T>(
    result: Result<T, BackupError>,
    operation: Operation,
    predicate: Option<std::process::ExitStatus>,
    stderr_nonempty: bool,
    observer: Option<&Observer>,
) -> Result<T, BackupError> {
    if let Some(observer) = observer {
        observer.native_result(
            operation,
            Guard::NativeRun,
            &result,
            predicate,
            stderr_nonempty,
        );
    }
    result
}

#[cfg(test)]
pub(super) fn require_native_status(
    status: Option<std::process::ExitStatus>,
    operation: Operation,
    stderr_nonempty: bool,
    observer: Option<&Observer>,
) -> Result<std::process::ExitStatus, BackupError> {
    let result = status.ok_or_else(decoder_bad);
    if let Some(observer) = observer {
        observer.native_result(
            operation,
            Guard::NativeHandleMissing,
            &result,
            None,
            stderr_nonempty,
        );
    }
    result
}

#[cfg(any(test, target_os = "linux"))]
pub(super) async fn decode_owner<O, F, Fut>(observer: O, call: F) -> Result<Decoded, BackupError>
where
    O: Send + 'static,
    F: FnOnce(tokio::sync::oneshot::Sender<Result<Decoded, BackupError>>, O) -> Fut
        + Send
        + 'static,
    Fut: std::future::Future<
            Output = (
                tokio::sync::oneshot::Sender<Result<Decoded, BackupError>>,
                Result<Decoded, BackupError>,
            ),
        > + Send
        + 'static,
{
    let (sender, receiver) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (sender, result) = call(sender, observer).await;
        let _ = sender.send(result);
    });
    receiver
        .await
        .map_err(|_| BackupError::Invalid("decoder supervisor lost"))?
}
#[cfg(test)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(super) mod native_observation {
    use super::*;
    use std::process::ExitStatus;
    use tokio::sync::{Mutex, MutexGuard, oneshot};

    pub(super) static NATIVE_LOCK: Mutex<()> = Mutex::const_new(());
    pub(in crate::full_restore) type NativeWait = (serde_json::Value, Vec<u8>);
    pub(super) struct WaitHook {
        pub first_version_only: bool,
        pub sender: oneshot::Sender<NativeWait>,
    }
    pub(super) static NATIVE_WAIT: std::sync::Mutex<Option<WaitHook>> = std::sync::Mutex::new(None);

    pub(super) fn take_wait(is_version: bool) -> Option<oneshot::Sender<NativeWait>> {
        let mut hook = NATIVE_WAIT.lock().unwrap();
        if hook
            .as_ref()
            .is_some_and(|h| !h.first_version_only || is_version)
        {
            hook.take().map(|h| h.sender)
        } else {
            None
        }
    }

    pub(in crate::full_restore) struct FirstVersion {
        receiver: oneshot::Receiver<NativeWait>,
        _lock: MutexGuard<'static, ()>,
    }
    impl FirstVersion {
        pub async fn arm() -> Self {
            let lock = NATIVE_LOCK.lock().await;
            let (sender, receiver) = oneshot::channel();
            *NATIVE_WAIT.lock().unwrap() = Some(WaitHook {
                first_version_only: true,
                sender,
            });
            Self {
                receiver,
                _lock: lock,
            }
        }
        // Never await a hook that a pre-spawn failure may not have emitted.
        // Consuming self clears the slot AND releases the lock before faults.
        pub fn drain(mut self) -> Option<NativeWait> {
            self.receiver.try_recv().ok()
        }
    }
    impl Drop for FirstVersion {
        fn drop(&mut self) {
            NATIVE_WAIT.lock().unwrap().take();
        }
    }

    fn status_value(status: ExitStatus) -> serde_json::Value {
        #[cfg(unix)]
        let signal = {
            use std::os::unix::process::ExitStatusExt;
            status.signal()
        };
        #[cfg(not(unix))]
        let signal: Option<i32> = None;
        serde_json::json!({"exit_code":status.code(),"signal":signal,"success":status.success()})
    }

    pub(in crate::full_restore) fn wait_receipt(
        pid: u32,
        predicate_status: Option<ExitStatus>,
        status: ExitStatus,
        errors: &[u8],
    ) -> serde_json::Value {
        serde_json::json!({"pid":pid,"wait_completed":true,"exit_code":status.code(),"success":status.success(),"predicate_status":predicate_status.map(status_value),"reap_status":status_value(status),"stderr_sha256":crate::digest(errors),"stderr_bytes":errors.len()})
    }

    fn encode_first(observed: Option<&NativeWait>) -> Result<Vec<u8>, BackupError> {
        if let Some((receipt, errors)) = observed {
            if errors.len() > 8192 {
                return Err(BackupError::Capacity("first Version stderr"));
            }
            if receipt["operation"] != "Version" || receipt["wait_completed"] != true {
                return Err(BackupError::Invalid("first Version observation"));
            }
        }
        let bytes = serde_json::to_vec(&serde_json::json!({
            "classification":"DIAGNOSTIC_NOT_AUTHORITY", "operation":"Version",
            "observation":if observed.is_some() { "OBSERVED" } else { "NOT_OBSERVED" },
            "native":observed.map(|(receipt, _)| receipt),
        }))?;
        if bytes.len() > 4096 {
            return Err(BackupError::Capacity("first Version receipt"));
        }
        Ok(bytes)
    }

    pub(in crate::full_restore) fn persist_first(
        root: &BackupDir,
        observed: Option<&NativeWait>,
    ) -> Result<(), BackupError> {
        let bytes = encode_first(observed)?;
        root.require_private_directory()?;
        if let Some((_, errors)) = observed {
            let mut file = root.create_file("first-version.stderr")?;
            file.write_all(errors)?;
            file.sync_all()?;
        }
        let mut file = root.create_file("first-version.json")?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        root.sync()?;
        if observed.is_none() {
            return Err(BackupError::Invalid("first Version not observed"));
        }
        Ok(())
    }

    pub(in crate::full_restore) fn retain_result<T>(
        baseline: Result<T, BackupError>,
        diagnostic: Result<(), BackupError>,
    ) -> Result<T, BackupError> {
        baseline.and_then(|value| diagnostic.map(|()| value))
    }

    fn exit(code: u32) -> ExitStatus {
        #[cfg(unix)]
        use std::os::unix::process::ExitStatusExt;
        #[cfg(windows)]
        use std::os::windows::process::ExitStatusExt;
        #[cfg(windows)]
        return ExitStatus::from_raw(code);
        #[cfg(unix)]
        return ExitStatus::from_raw((code as i32) << 8);
    }
    fn event(code: u32, errors: Vec<u8>) -> NativeWait {
        let mut receipt = wait_receipt(123, Some(exit(code)), exit(code), &errors);
        receipt["operation"] = "Version".into();
        (receipt, errors)
    }

    #[tokio::test]
    async fn first_version_observer_filters_drains_missing_and_releases_fault_hook() {
        let observer = FirstVersion::arm().await;
        assert!(take_wait(false).is_none());
        let sender = take_wait(true).unwrap();
        sender.send(event(0, b"abc".to_vec())).unwrap();
        assert!(take_wait(true).is_none());
        let observed = observer.drain().unwrap();
        assert_eq!(observed.1, b"abc");
        let value: serde_json::Value =
            serde_json::from_slice(&encode_first(Some(&observed)).unwrap()).unwrap();
        assert_eq!(value["observation"], "OBSERVED");
        assert_eq!(value["native"]["predicate_status"]["success"], true);
        assert_eq!(value["native"]["stderr_bytes"], 3);
        assert_eq!(
            value["native"]["stderr_sha256"],
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _lock = NATIVE_LOCK
            .try_lock()
            .expect("baseline released fault lock");
        assert!(NATIVE_WAIT.lock().unwrap().is_none());
        let (sender, mut receiver) = oneshot::channel();
        *NATIVE_WAIT.lock().unwrap() = Some(WaitHook {
            first_version_only: false,
            sender,
        });
        take_wait(false)
            .unwrap()
            .send(event(2, Vec::new()))
            .unwrap();
        let fault = receiver.try_recv().unwrap();
        assert_eq!(fault.0["predicate_status"]["exit_code"], 2);
        assert_eq!(fault.0["predicate_status"]["success"], false);
        assert_eq!(fault.0["stderr_bytes"], 0);
        drop(_lock);
        let observer = FirstVersion::arm().await;
        let observed = observer.drain();
        assert!(observed.is_none());
        let value: serde_json::Value =
            serde_json::from_slice(&encode_first(None).unwrap()).unwrap();
        assert_eq!(value["observation"], "NOT_OBSERVED");
        assert!(value["native"].is_null());
        assert!(NATIVE_WAIT.lock().unwrap().is_none());
        assert!(NATIVE_LOCK.try_lock().is_ok());
        let observer = FirstVersion::arm().await;
        drop(take_wait(true).unwrap());
        assert!(observer.drain().is_none());
        let observer = FirstVersion::arm().await;
        drop(observer);
        assert!(NATIVE_WAIT.lock().unwrap().is_none());
        assert!(NATIVE_LOCK.try_lock().is_ok());
    }

    #[test]
    fn first_version_diagnostics_reject_overflow_and_keep_original_error() {
        let observed = event(0, vec![0xff; 8192]);
        assert!(encode_first(Some(&observed)).unwrap().len() <= 4096);
        assert!(matches!(
            encode_first(Some(&event(0, vec![0; 8193]))),
            Err(BackupError::Capacity(_))
        ));
        let mut oversized = event(0, Vec::new());
        oversized.0["unexpected"] = "x".repeat(4096).into();
        assert!(matches!(
            encode_first(Some(&oversized)),
            Err(BackupError::Capacity(_))
        ));
        oversized.0["operation"] = "Toc".into();
        assert!(matches!(
            encode_first(Some(&oversized)),
            Err(BackupError::Invalid(_))
        ));
        for diagnostic in [
            Ok(()),
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied).into()),
        ] {
            let result = retain_result::<()>(
                Err(BackupError::Invalid("pg_restore decode exit")),
                diagnostic,
            );
            assert!(matches!(
                result,
                Err(BackupError::Invalid("pg_restore decode exit"))
            ));
        }
        assert!(
            retain_result(
                Ok(()),
                Err(BackupError::Invalid("first Version not observed"))
            )
            .is_err()
        );
        assert!(matches!(
            retain_result(
                Ok(()),
                Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied).into())
            ),
            Err(BackupError::Io(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn first_version_signal_and_missing_predicate_are_not_exit_zero() {
        use std::os::unix::process::ExitStatusExt;
        let receipt = wait_receipt(123, None, ExitStatus::from_raw(9), &[]);
        assert!(receipt["predicate_status"].is_null());
        assert!(receipt["reap_status"]["exit_code"].is_null());
        assert_eq!(receipt["reap_status"]["signal"], 9);
        assert_eq!(receipt["reap_status"]["success"], false);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn first_version_files_are_private_exact_and_exclusive() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let path = std::env::temp_dir().join(format!("kw-first-version-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let root = BackupDir::open_private_root(&path).unwrap();
        let observed = event(0, vec![0, 255, 10]);
        persist_first(&root, Some(&observed)).unwrap();
        let mut raw = Vec::new();
        let mut file = root.open_file("first-version.stderr").unwrap();
        file.read_to_end(&mut raw).unwrap();
        assert_eq!(raw, [0, 255, 10]);
        assert_eq!(file.metadata().unwrap().mode() & 0o777, 0o600);
        let mut json = Vec::new();
        root.open_file("first-version.json")
            .unwrap()
            .read_to_end(&mut json)
            .unwrap();
        assert_eq!(json, encode_first(Some(&observed)).unwrap());
        assert!(persist_first(&root, Some(&event(1, Vec::new()))).is_err());
        let missing = root.create_dir("missing").unwrap();
        assert!(persist_first(&missing, None).is_err());
        assert!(missing.open_file("first-version.stderr").is_err());
        let mut json = Vec::new();
        missing
            .open_file("first-version.json")
            .unwrap()
            .read_to_end(&mut json)
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&json).unwrap()["observation"],
            "NOT_OBSERVED"
        );
    }

    #[test]
    fn first_version_retains_predicate_status_separately_from_reap() {
        #[cfg(unix)]
        use std::os::unix::process::ExitStatusExt;
        #[cfg(windows)]
        use std::os::windows::process::ExitStatusExt;
        let predicate = ExitStatus::from_raw(0);
        let reap = ExitStatus::from_raw(9);
        let receipt = wait_receipt(123, Some(predicate), reap, b"abc");
        assert_eq!(receipt["predicate_status"]["exit_code"], 0);
        assert_eq!(receipt["predicate_status"]["success"], true);
        assert_eq!(receipt["success"], false);
        assert_eq!(receipt["stderr_bytes"], 3);
        assert_eq!(
            receipt["stderr_sha256"],
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
#[cfg(any(target_os = "linux", test))]
pub(super) async fn collect_pipe(
    input: &mut (impl tokio::io::AsyncRead + Unpin),
    output: &mut impl Write,
    cap: u64,
) -> Result<u64, BackupError> {
    use tokio::io::AsyncReadExt;
    // Keep the fixed chunk off nested supervisor futures' inline stack state.
    let mut buffer = vec![0u8; BUFFER];
    let mut total = 0;
    loop {
        let limit = (cap - total + 1).min(BUFFER as u64) as usize;
        let n = input.read(&mut buffer[..limit]).await?;
        if n == 0 {
            return Ok(total);
        }
        if n as u64 > cap - total {
            return Err(BackupError::Capacity("decoder stream"));
        }
        output.write_all(&buffer[..n])?;
        total += n as u64;
    }
}

pub(super) struct Decoded {
    pub sql: File,
    pub owned: File,
    pub toc: File,
}
#[cfg(all(test, target_os = "linux"))]
pub(super) async fn native_fault_cases(
    root: BackupDir,
) -> Result<Vec<serde_json::Value>, BackupError> {
    linux::native_cases(root).await
}

#[cfg(target_os = "linux")]
pub(super) async fn decode(
    dump: FrozenFullDump,
    root: BackupDir,
    #[cfg(test)] observer: Option<Observer>,
) -> Result<Decoded, BackupError> {
    // The owner remains alive when the caller cancels. It sees receiver closure,
    // kills and actually waits for its native child before dropping any handles.
    #[cfg(test)]
    let worker_observer = observer.clone();
    #[cfg(not(test))]
    let worker_observer = ();
    let result = decode_owner(worker_observer, |mut sender, _worker_observer| async move {
        let result = linux::decode_owned(
            dump,
            root,
            &mut sender,
            #[cfg(test)]
            _worker_observer.as_ref(),
        )
        .await;
        (sender, result)
    })
    .await;
    dump_observe!(
        observer.as_ref(),
        Step::DecoderOwner,
        Guard::OwnerLost,
        result
    )
}

#[cfg(not(target_os = "linux"))]
pub(super) async fn decode(
    _: FrozenFullDump,
    _: BackupDir,
    #[cfg(test)] _observer: Option<Observer>,
) -> Result<Decoded, BackupError> {
    Err(BackupError::Invalid(
        "full decoder requires trusted Linux PG18 client",
    ))
}

#[cfg(any(target_os = "linux", test))]
const CLIENT_PATH: &str = "/usr/lib/postgresql/18/bin/pg_restore";
#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Copy)]
pub(super) enum Operation {
    Version,
    Toc,
    OwnedSchema,
    Decode,
}
#[cfg(any(target_os = "linux", test))]
impl Operation {
    #[cfg(test)]
    pub(super) fn code(self) -> &'static str {
        match self {
            Self::Version => "VERSION",
            Self::Toc => "TOC",
            Self::OwnedSchema => "OWNED_SCHEMA",
            Self::Decode => "DECODE",
        }
    }
    #[cfg(test)]
    pub(super) fn run_step(self) -> Step {
        match self {
            Self::Version => Step::VersionRun,
            Self::Toc => Step::TocRun,
            Self::OwnedSchema => Step::OwnedSchemaRun,
            Self::Decode => Step::DecodeRun,
        }
    }
    #[cfg(all(test, target_os = "linux"))]
    fn seal_step(self) -> Step {
        match self {
            Self::Version => Step::VersionSeal,
            Self::Toc => Step::TocSeal,
            Self::OwnedSchema => Step::OwnedSchemaSeal,
            Self::Decode => Step::DecodeSeal,
        }
    }
    fn args(self) -> &'static [&'static str] {
        match self {
            Self::Version => &["--version"],
            Self::Toc => &["--list"],
            Self::OwnedSchema => &["--schema-only", "--file=-", "--exit-on-error"],
            Self::Decode => &["--file=-", "--no-owner", "--no-acl", "--exit-on-error"],
        }
    }
    fn prepare_argv(self) -> (Vec<std::ffi::CString>, Vec<usize>) {
        let strings: Vec<std::ffi::CString> = std::iter::once(CLIENT_PATH)
            .chain(self.args().iter().copied())
            .map(|s| std::ffi::CString::new(s).unwrap())
            .collect();
        let mut pointers: Vec<usize> = strings.iter().map(|s| s.as_ptr() as usize).collect();
        pointers.push(0);
        (strings, pointers)
    }
}

#[cfg(target_os = "linux")]
mod linux {
    #[cfg(test)]
    use super::native_observation::{NATIVE_LOCK, NATIVE_WAIT, WaitHook, take_wait, wait_receipt};
    use super::*;
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::fs::MetadataExt,
        },
        process::Stdio,
        time::Duration,
    };
    use tokio::{
        process::Command,
        time::{Instant, timeout_at},
    };
    fn bad() -> BackupError {
        decoder_bad()
    }

    fn client(#[cfg(test)] observer: Option<&Observer>) -> Result<File, BackupError> {
        let root = CString::new("/").unwrap();
        let fd = unsafe {
            libc::open(
                root.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(dump_error!(
                observer,
                Step::DecoderClient,
                Guard::ClientIo,
                std::io::Error::last_os_error().into()
            ));
        }
        let mut held = unsafe { File::from_raw_fd(fd) };
        let root_meta = dump_observe!(
            observer,
            Step::DecoderClient,
            Guard::RootMetadata,
            held.metadata().map_err(BackupError::from)
        )?;
        if root_meta.uid() != 0 || root_meta.mode() & 0o022 != 0 {
            return Err(dump_error!(
                observer,
                Step::DecoderClient,
                Guard::RootMetadata,
                bad()
            ));
        }
        for name in ["usr", "lib", "postgresql", "18", "bin", "pg_restore"] {
            let leaf = name == "pg_restore";
            let name = CString::new(name).unwrap();
            let flags = libc::O_RDONLY
                | libc::O_NOFOLLOW
                | libc::O_CLOEXEC
                | if leaf { 0 } else { libc::O_DIRECTORY };
            let fd = unsafe { libc::openat(held.as_raw_fd(), name.as_ptr(), flags) };
            if fd < 0 {
                return Err(dump_error!(
                    observer,
                    Step::DecoderClient,
                    Guard::ClientIo,
                    std::io::Error::last_os_error().into()
                ));
            }
            held = unsafe { File::from_raw_fd(fd) };
            let meta = dump_observe!(
                observer,
                Step::DecoderClient,
                Guard::ClientComponentMetadata,
                held.metadata().map_err(BackupError::from)
            )?;
            if meta.uid() != 0
                || meta.mode() & 0o022 != 0
                || if leaf {
                    !meta.is_file() || meta.nlink() != 1 || meta.mode() & 0o111 == 0
                } else {
                    !meta.is_dir()
                }
            {
                return Err(dump_error!(
                    observer,
                    Step::DecoderClient,
                    Guard::ClientComponentMetadata,
                    bad()
                ));
            }
        }
        require_client_hash(
            &mut held,
            #[cfg(test)]
            observer,
        )?;
        Ok(held)
    }

    /// fexecve executes the exact audited open inode. Prepared pointer arrays
    /// allocate in the parent; the post-fork closure only makes the syscall.
    fn fixed_child(
        executable: &File,
        operation: Operation,
        input: Stdio,
    ) -> Result<Command, BackupError> {
        let (strings, pointers) = operation.prepare_argv();
        let environment = CString::new("LC_ALL=C").unwrap();
        let env_pointers = [environment.as_ptr() as usize, 0];
        let fd = executable.as_raw_fd();
        let mut command = Command::new(CLIENT_PATH);
        command
            .env_clear()
            .stdin(input)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        unsafe {
            command.pre_exec(move || {
                // Keep the allocations that back the prepared pointers captured.
                let _keep = (&strings, &environment);
                libc::fexecve(fd, pointers.as_ptr().cast(), env_pointers.as_ptr().cast());
                Err(std::io::Error::last_os_error())
            });
        }
        Ok(command)
    }

    #[cfg(test)]
    struct RunControl<'a> {
        sender: &'a mut tokio::sync::oneshot::Sender<Result<Decoded, BackupError>>,
        observer: Option<&'a Observer>,
    }

    async fn run(
        executable: &File,
        operation: Operation,
        input: Stdio,
        output: &mut File,
        cap: u64,
        deadline: Instant,
        #[cfg(not(test))] sender: &mut tokio::sync::oneshot::Sender<Result<Decoded, BackupError>>,
        #[cfg(test)] control: RunControl<'_>,
    ) -> Result<(), BackupError> {
        #[cfg(test)]
        let RunControl { sender, observer } = control;
        let closed = sender.is_closed();
        require_run_ready(
            closed,
            !closed && Instant::now() >= deadline,
            #[cfg(test)]
            operation,
            #[cfg(test)]
            observer,
        )?;
        let mut child = dump_observe!(
            observer,
            operation.run_step(),
            Guard::NativeSpawn,
            fixed_child(executable, operation, input)
                .and_then(|mut command| command.spawn().map_err(BackupError::from))
        )?;
        #[cfg(test)]
        let native_pid = child.id().ok_or_else(|| {
            dump_error!(
                observer,
                operation.run_step(),
                Guard::NativeHandleMissing,
                bad()
            )
        })?;
        #[cfg(test)]
        if let Some(sender) = NATIVE_SPAWN.lock().unwrap().take() {
            let held = executable.metadata()?;
            let observed = std::fs::metadata(format!("/proc/{native_pid}/exe")).ok();
            let _ = sender.send((
                native_pid,
                observed.map(|m| (m.dev(), m.ino())) == Some((held.dev(), held.ino())),
            ));
        }
        let mut stdout = child.stdout.take().ok_or_else(|| {
            dump_error!(
                observer,
                operation.run_step(),
                Guard::NativeHandleMissing,
                bad()
            )
        })?;
        let mut stderr = child.stderr.take().ok_or_else(|| {
            dump_error!(
                observer,
                operation.run_step(),
                Guard::NativeHandleMissing,
                bad()
            )
        })?;
        let mut errors = Vec::new();
        #[cfg(test)]
        let mut native_status = None;
        #[cfg(test)]
        let mut predicate_status = None;
        let result = tokio::select! {
            biased;
            _=sender.closed()=>Err(BackupError::Invalid("decoder cancelled")),
            result=timeout_at(deadline,async{
                tokio::try_join!(collect_pipe(&mut stdout,output,cap),collect_pipe(&mut stderr,&mut errors,8192))?;
                let status=child.wait().await?;
                #[cfg(test)]{native_status=Some(status);predicate_status=Some(status);}
                native_predicate(status, !errors.is_empty(), #[cfg(test)] operation, #[cfg(test)] observer)
            })=>result.unwrap_or(Err(BackupError::Invalid("full decode deadline"))),
        };
        #[cfg(test)]
        let result = retain_native_result(
            result,
            operation,
            predicate_status,
            !errors.is_empty(),
            observer,
        );
        if result.is_err() {
            let _ = child.start_kill();
            // Do not return a capability, or release this owner, until wait has
            // actually reaped the child. kill_on_drop is only a panic fallback.
            let status = dump_observe!(
                observer,
                operation.run_step(),
                Guard::NativeRun,
                child.wait().await.map_err(BackupError::from)
            )?;
            #[cfg(test)]
            {
                native_status = Some(status);
            }
            #[cfg(not(test))]
            let _ = status;
        }
        #[cfg(test)]
        if let Some(sender) = take_wait(matches!(operation, Operation::Version)) {
            let status =
                require_native_status(native_status, operation, !errors.is_empty(), observer)?;
            let mut receipt = wait_receipt(native_pid, predicate_status, status, &errors);
            receipt["operation"] = match operation {
                Operation::Version => "Version",
                Operation::Toc => "Toc",
                Operation::OwnedSchema => "OwnedSchema",
                Operation::Decode => "Decode",
            }
            .into();
            let _ = sender.send((receipt, errors));
        }
        result
    }

    #[cfg(test)]
    static NATIVE_SPAWN: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<(u32, bool)>>> =
        std::sync::Mutex::new(None);

    /// Fixed test-only native fault cases. The waiting stdin pipe exists ONLY
    /// here; production callers can supply only the already frozen archive FD.
    #[cfg(test)]
    pub(super) async fn native_cases(
        root: BackupDir,
    ) -> Result<Vec<serde_json::Value>, BackupError> {
        let _lock = NATIVE_LOCK.lock().await;
        let mut records = Vec::new();
        for name in ["receiver_cancel", "deadline", "output_cap", "native_exit"] {
            let executable = client(None)?;
            let output = root.create_file(&format!("native-{name}.stdout"))?;
            let (spawn_tx, spawn_rx) = tokio::sync::oneshot::channel();
            let (wait_tx, wait_rx) = tokio::sync::oneshot::channel();
            *NATIVE_SPAWN.lock().unwrap() = Some(spawn_tx);
            *NATIVE_WAIT.lock().unwrap() = Some(WaitHook {
                first_version_only: false,
                sender: wait_tx,
            });
            let (mut sender, receiver) =
                tokio::sync::oneshot::channel::<Result<Decoded, BackupError>>();
            let (input, operation, cap, seconds) = match name {
                "receiver_cancel" => (Stdio::piped(), Operation::Toc, 8192, 10),
                "deadline" => (Stdio::piped(), Operation::Toc, 8192, 1),
                "output_cap" => (Stdio::null(), Operation::Version, 1, 10),
                "native_exit" => (Stdio::null(), Operation::Toc, 8192, 10),
                _ => unreachable!(),
            };
            let task = tokio::spawn(async move {
                let mut output = output;
                let result = run(
                    &executable,
                    operation,
                    input,
                    &mut output,
                    cap,
                    Instant::now() + Duration::from_secs(seconds),
                    RunControl {
                        sender: &mut sender,
                        observer: None,
                    },
                )
                .await;
                output.sync_all()?;
                Ok::<_, BackupError>(result)
            });
            let (pid, same_inode) = spawn_rx.await.map_err(|_| bad())?;
            if matches!(name, "receiver_cancel" | "deadline") && !same_inode {
                return Err(bad());
            }
            let mut receiver = Some(receiver);
            if name == "receiver_cancel" {
                drop(receiver.take());
            }
            let result = task.await.map_err(|_| bad())??;
            let expected = match name {
                "receiver_cancel" => {
                    matches!(result, Err(BackupError::Invalid("decoder cancelled")))
                }
                "deadline" => matches!(result, Err(BackupError::Invalid("full decode deadline"))),
                "output_cap" => matches!(result, Err(BackupError::Capacity("decoder stream"))),
                "native_exit" => {
                    matches!(result, Err(BackupError::Invalid("pg_restore decode exit")))
                }
                _ => false,
            };
            if !expected {
                return Err(bad());
            }
            let (wait, errors) = wait_rx.await.map_err(|_| bad())?;
            let mut stderr = root.create_file(&format!("native-{name}.stderr"))?;
            stderr.write_all(&errors)?;
            stderr.sync_all()?;
            if wait["pid"] != pid
                || wait["wait_completed"] != true
                || std::path::Path::new(&format!("/proc/{pid}")).exists()
            {
                return Err(bad());
            }
            let mut bytes = Vec::new();
            root.open_file(&format!("native-{name}.stdout"))?
                .read_to_end(&mut bytes)?;
            records.push(serde_json::json!({"case":name,"held_inode_observed":same_inode,"stdout_sha256":crate::digest(&bytes),"stdout_bytes":bytes.len(),"wait":wait}));
            drop(receiver);
        }
        let mut wrong = root.create_file("mismatched-client")?;
        wrong.write_all(b"not the pinned PG18 client")?;
        wrong.sync_all()?;
        drop(wrong);
        let mut wrong = root.open_file("mismatched-client")?;
        if require_client_hash(&mut wrong, None).is_ok() {
            return Err(bad());
        }
        records.push(serde_json::json!({"case":"held_client_hash_mismatch","child_started":false,"path_substitution_claim":false}));
        Ok(records)
    }

    pub(super) async fn decode_owned(
        mut dump: FrozenFullDump,
        root: BackupDir,
        sender: &mut tokio::sync::oneshot::Sender<Result<Decoded, BackupError>>,
        #[cfg(test)] observer: Option<&Observer>,
    ) -> Result<Decoded, BackupError> {
        let deadline = Instant::now() + Duration::from_secs(900);
        dump_observe!(
            observer,
            Step::DecoderClient,
            Guard::PrivateDirectory,
            root.require_private_directory().map_err(BackupError::from)
        )?;
        let executable = client(
            #[cfg(test)]
            observer,
        )?;
        if dump_observe!(
            observer,
            Step::FrozenMetadata,
            Guard::FrozenMetadata,
            dump.file.metadata().map_err(BackupError::from)
        )?
        .len()
            != dump.size
            || !crate::valid_digest(&dump.sha256)
        {
            return Err(dump_error!(
                observer,
                Step::FrozenMetadata,
                Guard::FrozenMetadata,
                bad()
            ));
        }
        let dir = dump_observe!(
            observer,
            Step::DecoderOwner,
            Guard::OutputCreate,
            root.create_dir(&format!("full-decoded-{}", uuid::Uuid::new_v4()))
                .map_err(BackupError::from)
        )?;
        let mut version = dump_observe!(
            observer,
            Step::VersionRun,
            Guard::OutputCreate,
            dir.create_file("version").map_err(BackupError::from)
        )?;
        run(
            &executable,
            Operation::Version,
            Stdio::null(),
            &mut version,
            8192,
            deadline,
            #[cfg(not(test))]
            sender,
            #[cfg(test)]
            RunControl { sender, observer },
        )
        .await?;
        let mut version = dump_observe!(
            observer,
            Step::VersionSeal,
            Guard::SealFile,
            finish_file(version, &dir, "version")
        )?;
        let mut bytes = Vec::new();
        dump_observe!(
            observer,
            Step::VersionBytes,
            Guard::InputRead,
            version.read_to_end(&mut bytes).map_err(BackupError::from)
        )?;
        require_decoder_version(
            &bytes,
            #[cfg(test)]
            observer,
        )?;
        let mut files = Vec::new();
        for (name, operation, cap) in [
            ("toc", Operation::Toc, 4 * 1024 * 1024),
            ("owned-schema", Operation::OwnedSchema, 4 * 1024 * 1024),
            ("decoded", Operation::Decode, MAX_DECODE),
        ] {
            dump_observe!(
                observer,
                operation.run_step(),
                Guard::InputSeek,
                dump.file
                    .seek(SeekFrom::Start(0))
                    .map_err(BackupError::from)
            )?;
            let input = Stdio::from(dump_observe!(
                observer,
                operation.run_step(),
                Guard::InputClone,
                dump.file.try_clone().map_err(BackupError::from)
            )?);
            let mut output = dump_observe!(
                observer,
                operation.run_step(),
                Guard::OutputCreate,
                dir.create_file(name).map_err(BackupError::from)
            )?;
            run(
                &executable,
                operation,
                input,
                &mut output,
                cap,
                deadline,
                #[cfg(not(test))]
                sender,
                #[cfg(test)]
                RunControl { sender, observer },
            )
            .await?;
            files.push(dump_observe!(
                observer,
                operation.seal_step(),
                Guard::SealFile,
                finish_file(output, &dir, name)
            )?);
        }
        dump_observe!(
            observer,
            Step::DecoderSeal,
            Guard::SealDirectory,
            dir.seal_dir().map_err(BackupError::from)
        )?;
        let sql = files
            .pop()
            .ok_or_else(|| dump_error!(observer, Step::DecoderSeal, Guard::OutputMissing, bad()))?;
        let owned = files
            .pop()
            .ok_or_else(|| dump_error!(observer, Step::DecoderSeal, Guard::OutputMissing, bad()))?;
        let toc = files
            .pop()
            .ok_or_else(|| dump_error!(observer, Step::DecoderSeal, Guard::OutputMissing, bad()))?;
        Ok(Decoded { sql, owned, toc })
    }
}

/// A snapshot of custom archive bytes, never evidence of input authority.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub struct FrozenFullDump {
    pub(super) file: File,
    pub(super) size: u64,
    pub(super) sha256: String,
}

pub fn freeze_full_dump(
    dump: &mut File,
    expected: &FileRecord,
    spool: &BackupDir,
) -> Result<FrozenFullDump, BackupError> {
    if expected.size > MAX_DUMP {
        return Err(BackupError::Capacity("full dump"));
    }
    spool.require_private_directory()?;
    if !dump.metadata()?.is_file() {
        return Err(BackupError::Invalid("dump file type"));
    }
    dump.seek(SeekFrom::Start(0))?;
    let dir = spool.create_dir(&format!("full-dump-{}", uuid::Uuid::new_v4()))?;
    let mut output = dir.create_file("dump")?;
    freeze_bytes(dump, &mut output, expected)?;
    let file = finish_file(output, &dir, "dump")?;
    dir.seal_dir()?;
    Ok(FrozenFullDump {
        file,
        size: expected.size,
        sha256: expected.sha256.clone(),
    })
}
fn freeze_bytes(
    input: &mut impl Read,
    output: &mut impl Write,
    expected: &FileRecord,
) -> Result<(), BackupError> {
    if expected.size > MAX_DUMP {
        return Err(BackupError::Capacity("full dump"));
    }
    if expected.path != "database.dump"
        || expected.size < 5
        || !crate::valid_digest(&expected.sha256)
    {
        return Err(BackupError::Invalid("dump manifest"));
    }
    let mut header = [0; 5];
    input.read_exact(&mut header)?;
    if &header != b"PGDMP" {
        return Err(BackupError::Invalid("custom dump header"));
    }
    let mut hash = Sha256::new();
    hash.update(header);
    output.write_all(&header)?;
    let mut writer = Hashing { output, hash };
    let length = bounded_copy(input, &mut writer, expected.size - 5)? + 5;
    if length != expected.size || hex::encode(writer.hash.finalize()) != expected.sha256 {
        return Err(BackupError::Invalid("dump length or SHA-256"));
    }
    Ok(())
}

struct Hashing<'a, W> {
    output: &'a mut W,
    hash: Sha256,
}
impl<W: Write> Write for Hashing<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let n = self.output.write(bytes)?;
        self.hash.update(&bytes[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.output.flush()
    }
}

/// Close the only writable handle before exposing an opaque readonly range.
/// Entries are fresh, private, no-follow and sealed; no path or file handle is
/// exposed by the public capability. Replacement of the source has no effect.
pub(super) fn finish_file(output: File, dir: &BackupDir, name: &str) -> Result<File, BackupError> {
    output.sync_all()?;
    let read = dir.open_file(name)?;
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt;
        let before = output.metadata()?;
        let after = read.metadata()?;
        if (before.dev(), before.ino(), before.len()) != (after.dev(), after.ino(), after.len())
            || after.nlink() != 1
            || after.uid() != 0
        {
            return Err(BackupError::Invalid("spool identity"));
        }
    }
    dir.seal_file(name)?;
    drop(output);
    dir.sync()?;
    Ok(read)
}

pub(super) fn bounded_copy(
    input: &mut impl Read,
    output: &mut impl Write,
    cap: u64,
) -> Result<u64, BackupError> {
    let mut total = 0;
    let mut buffer = [0u8; BUFFER];
    loop {
        let limit = (cap - total + 1).min(BUFFER as u64) as usize;
        let n = input.read(&mut buffer[..limit])?;
        if n == 0 {
            return Ok(total);
        }
        if n as u64 > cap - total {
            return Err(BackupError::Capacity("full stream bytes"));
        }
        output.write_all(&buffer[..n])?;
        total += n as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    #[test]
    fn fixed_native_argv_uses_absolute_self_path_and_fixed_operation_args() {
        let cases: [(Operation, &[&str]); 4] = [
            (Operation::Version, &["--version"]),
            (Operation::Toc, &["--list"]),
            (
                Operation::OwnedSchema,
                &["--schema-only", "--file=-", "--exit-on-error"],
            ),
            (
                Operation::Decode,
                &["--file=-", "--no-owner", "--no-acl", "--exit-on-error"],
            ),
        ];
        let mut self_paths = Vec::new();
        for (operation, expected_args) in cases {
            let (strings, pointers) = operation.prepare_argv();
            assert_eq!(pointers.len(), strings.len() + 1);
            assert_eq!(pointers.last(), Some(&0));
            for (string, pointer) in strings.iter().zip(&pointers) {
                assert_eq!(*pointer, string.as_ptr() as usize);
                // The returned backing allocations still own every C argument.
                assert_eq!(
                    unsafe { std::ffi::CStr::from_ptr(*pointer as _) },
                    string.as_c_str()
                );
            }
            let actual: Vec<_> = strings.iter().map(|s| s.to_str().unwrap()).collect();
            assert_eq!(&actual[1..], expected_args);
            self_paths.push(actual[0].to_owned());
        }
        assert_eq!(self_paths, ["/usr/lib/postgresql/18/bin/pg_restore"; 4]);
    }
    struct Synthetic(u64);
    impl Read for Synthetic {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let n = self.0.min(buffer.len() as u64) as usize;
            self.0 -= n as u64;
            // Sink never inspects values; no size-proportional allocation.
            Ok(n)
        }
    }
    #[test]
    fn full_dump_caps_reject_before_target_write() {
        for cap in [MAX_DUMP, MAX_DECODE, MAX_ROW] {
            assert_eq!(
                bounded_copy(&mut Synthetic(cap), &mut io::sink(), cap).unwrap(),
                cap
            );
            assert!(matches!(
                bounded_copy(&mut Synthetic(cap + 1), &mut io::sink(), cap),
                Err(BackupError::Capacity(_))
            ));
        }
    }
    #[test]
    fn freeze_rejects_changed_truncated_or_oversize_dump() {
        let expected = FileRecord {
            path: "database.dump".into(),
            size: 13,
            sha256: crate::digest(b"PGDMPoriginal"),
        };
        let mut output = Vec::new();
        freeze_bytes(&mut &b"PGDMPoriginal"[..], &mut output, &expected).unwrap();
        assert_eq!(output, b"PGDMPoriginal");
        for raw in [
            b"PGDMPmodified".as_slice(),
            b"PGDMPorigin",
            b"PGDMPoriginalextra",
            b"WRONGoriginal",
        ] {
            assert!(freeze_bytes(&mut &raw[..], &mut Vec::new(), &expected).is_err());
        }
        let mut oversized = expected;
        oversized.size = MAX_DUMP + 1;
        struct Unread;
        impl Read for Unread {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                panic!("oversize input read")
            }
        }
        assert!(matches!(
            freeze_bytes(&mut Unread, &mut Vec::new(), &oversized),
            Err(BackupError::Capacity(_))
        ));
    }
    #[tokio::test]
    async fn decoder_pipe_caps_before_writing_overflow() {
        let mut output = Vec::new();
        assert_eq!(
            collect_pipe(&mut &b"12345678"[..], &mut output, 8)
                .await
                .unwrap(),
            8
        );
        assert_eq!(output, b"12345678");
        let mut output = Vec::new();
        assert!(matches!(
            collect_pipe(&mut &b"123456789"[..], &mut output, 8).await,
            Err(BackupError::Capacity(_))
        ));
        assert!(output.len() <= 8);
    }
    #[test]
    fn decoder_pipe_future_has_bounded_stack_state() {
        let mut input = tokio::io::empty();
        let mut output = Vec::new();
        let future = collect_pipe(&mut input, &mut output, MAX_DECODE);
        let size = std::mem::size_of_val(&future);
        println!("actual collect_pipe future: {size} bytes");
        assert!(
            size <= 16 * 1024,
            "pipe future exceeds stack budget: {size}"
        );
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn full_dump_spool_immutable_after_source_rewrite() {
        use std::io::{Seek, SeekFrom};
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!("kw-full-spool-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let root = BackupDir::open_private_root(&path).unwrap();
        let mut source = root.create_file("source").unwrap();
        source.write_all(b"PGDMPoriginal").unwrap();
        source.sync_all().unwrap();
        let mut input = root.open_file("source").unwrap();
        let expected = FileRecord {
            path: "database.dump".into(),
            size: 13,
            sha256: crate::digest(b"PGDMPoriginal"),
        };
        let mut frozen = freeze_full_dump(&mut input, &expected, &root).unwrap();
        source.seek(SeekFrom::Start(0)).unwrap();
        source.write_all(b"PGDMPmodified").unwrap();
        source.sync_all().unwrap();
        let mut raw = Vec::new();
        frozen.file.read_to_end(&mut raw).unwrap();
        assert_eq!(raw, b"PGDMPoriginal");
        assert_eq!(frozen.size, 13);
        assert_eq!(frozen.sha256, expected.sha256);
        assert!(freeze_full_dump(&mut input, &expected, &root).is_err());
    }
}

//! Explicit Root fixtures. Run the root_ filter --ignored as uid/gid 0 with
//! CHOWN/SETUID/SETGID. A preopened public IPC leaf supplies cwd before exec for
//! test reachability; this is not an actual read-only container bind mount.
#![cfg(target_os = "linux")]
use learning_worker::snapshot_broker::DisabledBroker;
use std::{
    fs,
    io::{self, Read, Write},
    net::Shutdown,
    os::{
        fd::AsRawFd,
        unix::{
            fs::{MetadataExt, PermissionsExt},
            net::{UnixListener, UnixStream},
            process::CommandExt,
        },
    },
    path::{Path, PathBuf},
    process::{Command, Output},
};
use uuid::Uuid;

struct Fixture {
    base: PathBuf,
    custody: PathBuf,
    ipc: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        assert_eq!(
            unsafe { (libc::geteuid(), libc::getegid()) },
            (0, 0),
            "explicit Root fixture required"
        );
        let base = Path::new("/run").join(format!("kw-s1-{}", Uuid::new_v4()));
        fs::create_dir(&base).unwrap();
        fs::set_permissions(&base, fs::Permissions::from_mode(0o755)).unwrap();
        let custody = base.join("custody");
        let ipc = base.join("ipc");
        fs::create_dir(&custody).unwrap();
        fs::create_dir(&ipc).unwrap();
        fs::set_permissions(&custody, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&ipc, fs::Permissions::from_mode(0o700)).unwrap();
        Self { base, custody, ipc }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        assert_eq!(self.base.parent(), Some(Path::new("/run")));
        assert!(
            self.base
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("kw-s1-")
        );
        fs::remove_dir_all(&self.base).unwrap();
    }
}
fn client(socket: &Path, epoch: Uuid) -> Output {
    Command::new(env!("CARGO_BIN_EXE_learning-worker"))
        .args(["--snapshot-broker-client", "--socket"])
        .arg(socket)
        .args([
            "--epoch",
            &epoch.to_string(),
            "--request-id",
            &Uuid::new_v4().to_string(),
            "--lease-ms",
            "30000",
        ])
        .env_remove("DATABASE_URL")
        .env_remove("ASSET_ROOT")
        .env_remove("STAGING_ROOT")
        .env_remove("SNAPSHOT_ROOT")
        .uid(65532)
        .gid(65532)
        .output()
        .unwrap()
}
fn leaf_cwd_client(leaf: &Path, epoch: Uuid, uid: u32, gid: u32, accepted: bool) -> Output {
    let directory = fs::File::open(leaf).unwrap();
    let raw = directory.as_raw_fd();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "leaf_cwd_client_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env("BROKER_FIXTURE_EPOCH", epoch.to_string())
        .env("BROKER_FIXTURE_UID", uid.to_string())
        .env("BROKER_FIXTURE_GID", gid.to_string())
        .env(
            "BROKER_FIXTURE_ACCEPTED",
            if accepted { "yes" } else { "no" },
        );
    unsafe {
        command.pre_exec(move || {
            // The public 0750 IPC leaf only, never custody/journal/lock FDs.
            // Close it before exec. cwd is strictly a test bootstrap.
            if libc::fchdir(raw) != 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::setgroups(1, [65532 as libc::gid_t].as_ptr()) != 0
                || libc::setgid(gid) != 0
                || libc::setuid(uid) != 0
            {
                return Err(io::Error::last_os_error());
            }
            if libc::close(raw) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command.output().unwrap()
}
#[test]
#[ignore = "subprocess-only fixture; run root_ filter, never directly as a test group"]
fn leaf_cwd_client_fixture() {
    let epoch = std::env::var("BROKER_FIXTURE_EPOCH").expect("fixture parent required");
    let uid: u32 = std::env::var("BROKER_FIXTURE_UID")
        .unwrap()
        .parse()
        .unwrap();
    let gid: u32 = std::env::var("BROKER_FIXTURE_GID")
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(unsafe { (libc::geteuid(), libc::getegid()) }, (uid, gid));
    let accepted = std::env::var("BROKER_FIXTURE_ACCEPTED").unwrap() == "yes";
    let exchange = || -> io::Result<serde_json::Value> {
        let mut stream = UnixStream::connect("work.sock")?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(2)))?;
        stream.set_write_timeout(Some(std::time::Duration::from_secs(2)))?;
        let request = serde_json::to_vec(
            &serde_json::json!({"format_version":2,"command":"SIGNAL_NEXT","request_id":Uuid::new_v4(),"lease_ms":30000}),
        )?;
        stream.write_all(&(request.len() as u32).to_be_bytes())?;
        stream.write_all(&request)?;
        stream.shutdown(Shutdown::Write)?;
        let mut length = [0; 4];
        stream.read_exact(&mut length)?;
        let size = u32::from_be_bytes(length) as usize;
        if size > 4096 {
            return Err(io::Error::other("oversized reply"));
        }
        let mut body = vec![0; size];
        stream.read_exact(&mut body)?;
        Ok(serde_json::from_slice(&body)?)
    };
    if accepted {
        let reply = exchange().unwrap();
        assert_eq!(reply["epoch"], epoch);
        assert_eq!(
            reply["disposition"],
            "WRITER_DISABLED_EFFECT_UNKNOWN_NO_JOB_EXECUTED"
        );
        assert_eq!(reply.as_object().unwrap().len(), 4);
    } else {
        assert!(exchange().is_err());
    }
}
#[test]
#[ignore = "explicit Linux Root fixture with CHOWN/SETUID/SETGID; no silent skip"]
fn root_single_instance_lock_and_fresh_epoch_survive_drop_without_disk_revival() {
    let f = Fixture::new();
    let epoch = Uuid::new_v4();
    let broker = DisabledBroker::bind(&f.custody, &f.ipc, epoch).unwrap();
    let error = DisabledBroker::bind(&f.custody, &f.ipc, Uuid::new_v4())
        .err()
        .unwrap();
    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
    let lock = fs::metadata(f.custody.join("broker.lock")).unwrap();
    assert_eq!(
        (lock.uid(), lock.gid(), lock.mode() & 0o777, lock.nlink()),
        (0, 0, 0o600, 1)
    );
    drop(broker);
    assert!(DisabledBroker::bind(&f.custody, &f.ipc, epoch).is_err());
    let next = DisabledBroker::bind(&f.custody, &f.ipc, Uuid::new_v4()).unwrap();
    assert!(next.socket_path().exists());
}

#[test]
#[ignore = "explicit Linux Root fixture with CHOWN/SETUID/SETGID; no silent skip"]
fn root_nonroot_custody_and_symlink_or_writable_root_are_refused() {
    let f = Fixture::new();
    let output = Command::new(env!("CARGO_BIN_EXE_snapshot-effect-broker"))
        .args(["--serve-disabled", "--custody-root"])
        .arg(&f.custody)
        .arg("--ipc-parent")
        .arg(&f.ipc)
        .args(["--epoch", &Uuid::new_v4().to_string()])
        .uid(65532)
        .gid(65532)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(78));
    assert!(fs::read_dir(&f.custody).unwrap().next().is_none());
    let alias = f.base.join("custody-alias");
    std::os::unix::fs::symlink(&f.custody, &alias).unwrap();
    assert!(DisabledBroker::bind(&alias, &f.ipc, Uuid::new_v4()).is_err());
    fs::set_permissions(&f.custody, fs::Permissions::from_mode(0o770)).unwrap();
    assert!(DisabledBroker::bind(&f.custody, &f.ipc, Uuid::new_v4()).is_err());
    fs::set_permissions(&f.custody, fs::Permissions::from_mode(0o700)).unwrap();
}
#[test]
#[ignore = "explicit Linux Root fixture with CHOWN/SETUID/SETGID; no silent skip"]
fn root_actual_uid_gid_only_disabled_response_and_private_host_path_refusal() {
    let f = Fixture::new();
    let epoch = Uuid::new_v4();
    let broker = DisabledBroker::bind(&f.custody, &f.ipc, epoch).unwrap();
    // Correct UID cannot traverse the production private host parent. A real
    // RO leaf mount + ordinary worker CLI success is an additional gate.
    let output = client(broker.socket_path(), epoch);
    assert_eq!(output.status.code(), Some(78));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("transport unavailable; no local fallback"));
    assert!(!stderr.contains("DATABASE_URL"));
    assert!(output.stdout.is_empty());
    for (uid, gid, accepted) in [
        (65532, 65532, true),
        (0, 0, false),
        (65532, 65531, false),
        (65531, 65532, false),
    ] {
        std::thread::scope(|scope| {
            let handler = scope.spawn(|| broker.accept_disabled_signal());
            let output = leaf_cwd_client(
                broker.socket_path().parent().unwrap(),
                epoch,
                uid,
                gid,
                accepted,
            );
            let handled = handler.join().unwrap();
            if accepted {
                assert!(handled.is_ok());
            } else {
                assert_eq!(handled.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
            }
            assert!(output.status.success(), "{output:?}");
        });
    }
    assert!(
        fs::read_dir(
            f.custody
                .join("epochs")
                .join(epoch.to_string())
                .join("journal")
        )
        .unwrap()
        .next()
        .is_none()
    );
    assert_eq!(
        DisabledBroker::bind(&f.custody, &f.ipc, Uuid::new_v4())
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::WouldBlock
    );
}
#[test]
#[ignore = "explicit Linux Root fixture with CHOWN; no silent skip"]
fn root_socket_epoch_inode_and_parent_replacement_refuse_before_accept() {
    let f = Fixture::new();
    let epoch = Uuid::new_v4();
    let broker = DisabledBroker::bind(&f.custody, &f.ipc, epoch).unwrap();
    let original = f.ipc.join(epoch.to_string());
    fs::rename(&original, f.ipc.join("old-leaf")).unwrap();
    fs::create_dir(&original).unwrap();
    fs::set_permissions(&original, fs::Permissions::from_mode(0o750)).unwrap();
    std::os::unix::fs::chown(&original, Some(0), Some(65532)).unwrap();
    let _replacement = UnixListener::bind(original.join("work.sock")).unwrap();
    std::os::unix::fs::chown(original.join("work.sock"), Some(0), Some(65532)).unwrap();
    fs::set_permissions(
        original.join("work.sock"),
        fs::Permissions::from_mode(0o660),
    )
    .unwrap();
    assert_eq!(
        broker.accept_disabled_signal().unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
}

#[test]
#[ignore = "explicit Linux Root fixture with CHOWN; no silent skip"]
fn root_whole_custody_replacement_refuses_before_accept() {
    let f = Fixture::new();
    let broker = DisabledBroker::bind(&f.custody, &f.ipc, Uuid::new_v4()).unwrap();
    fs::rename(&f.custody, f.base.join("old-custody")).unwrap();
    fs::create_dir(&f.custody).unwrap();
    fs::set_permissions(&f.custody, fs::Permissions::from_mode(0o700)).unwrap();
    // A new path can have a different lock; the old held lock must not let
    // the displaced broker keep serving under the configured custody path.
    let _replacement = DisabledBroker::bind(&f.custody, &f.ipc, Uuid::new_v4()).unwrap();
    assert_eq!(
        broker.accept_disabled_signal().unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
}

#[test]
#[ignore = "explicit Linux Root fixture with CHOWN; no silent skip"]
fn root_whole_ipc_parent_replacement_refuses_before_accept() {
    let f = Fixture::new();
    let epoch = Uuid::new_v4();
    let broker = DisabledBroker::bind(&f.custody, &f.ipc, epoch).unwrap();
    fs::rename(&f.ipc, f.base.join("old-ipc")).unwrap();
    fs::create_dir(&f.ipc).unwrap();
    fs::set_permissions(&f.ipc, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        broker.accept_disabled_signal().unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
}

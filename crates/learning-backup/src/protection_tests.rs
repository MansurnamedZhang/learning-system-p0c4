use crate::{
    BackupError,
    registry::{ScanBudget, canonical_record, validate_generation_chain},
};
use serde_json::json;

#[test]
fn metadata_reader_charges_shared_rechecks_before_any_read() {
    use std::{
        io::Read,
        sync::{Arc, Mutex},
    };
    struct Observed {
        calls: usize,
        bytes: usize,
    }
    impl Read for Observed {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            self.calls += 1;
            self.bytes += output.len();
            output.fill(b'x');
            Ok(output.len())
        }
    }
    let budget = Arc::new(Mutex::new(ScanBudget::default()));
    budget.lock().unwrap().bytes = crate::registry::MAX_SCAN_BYTES - 8;
    let mut reader = Observed { calls: 0, bytes: 0 };
    assert_eq!(
        crate::registry::read_metadata(&mut reader, 4, &budget).unwrap(),
        b"xxxx"
    );
    assert_eq!(
        crate::registry::read_metadata(&mut reader, 4, &budget).unwrap(),
        b"xxxx"
    );
    assert_eq!((reader.calls, reader.bytes), (2, 8));
    assert!(matches!(
        crate::registry::read_metadata(&mut reader, 1, &budget),
        Err(BackupError::Capacity("registry scan bytes"))
    ));
    assert_eq!((reader.calls, reader.bytes), (2, 8));
}

fn source_group() -> crate::registry::SourceGroup {
    serde_json::from_value(json!({"format_version":1,"capability":"backup_source_group_v1","deployment_id":"11111111-1111-4111-8111-111111111111","group_id":"22222222-2222-4222-8222-222222222222","database":"learning_backup_c4_task3_33333333-3333-4333-8333-333333333333","database_oid":123,"system_identifier":"456","source_binding_sha256":"a".repeat(64),"application_commit":"b".repeat(40),"application_build_sha256":"c".repeat(64),"roots":{}})).unwrap()
}
#[test]
fn captured_generation_contains_group_and_all_four_original_root_hashes() {
    use crate::registry::{Generation, RootRecord, Roster, canonical, validate_capture_generation};
    use std::collections::BTreeMap;
    let mut group = source_group();
    let mut roots = BTreeMap::new();
    for (n, name) in ["assets", "control", "local_pins", "staging"]
        .iter()
        .enumerate()
    {
        let id = format!("{:08x}-1111-4111-8111-111111111111", n + 4);
        group.roots.insert((*name).into(), id.clone());
        let root:RootRecord=serde_json::from_value(json!({"format_version":1,"capability":"backup_root_v1","deployment_id":group.deployment_id,"enrollment_id":id,"group_id":group.group_id,"kind":"source_assets","path":format!("/var/lib/source/{name}"),"dev":7,"ino":n+9,"uid":0,"mode":448,"enrolled_generation":2})).unwrap();
        roots.insert((*name).into(), root);
    }
    let first = Generation {
        format_version: 1,
        capability: "backup_registry_generation_v1".into(),
        deployment_id: group.deployment_id.clone(),
        generation: 1,
        previous_generation_sha256: None,
        roots: vec![],
        groups: vec![],
    };
    let mut second = first.clone();
    second.generation = 2;
    second.previous_generation_sha256 = Some(crate::digest(&canonical(&first).unwrap()));
    second.groups = vec![Roster {
        id: group.group_id.clone(),
        sha256: crate::digest(&canonical(&group).unwrap()),
    }];
    second.roots = roots
        .values()
        .map(|r| Roster {
            id: r.enrollment_id.clone(),
            sha256: crate::digest(&canonical(r).unwrap()),
        })
        .collect();
    second.roots.sort_by(|a, b| a.id.cmp(&b.id));
    let hash1 = crate::digest(&canonical(&first).unwrap());
    let hash2 = crate::digest(&canonical(&second).unwrap());
    let generations = [first, second.clone()];
    validate_capture_generation(&generations, &group, &roots, 2, &hash2, None).unwrap();
    assert!(
        validate_capture_generation(&generations, &group, &roots, 1, &hash1, None).is_err(),
        "generation 1 never enrolled this group"
    );
    let mut changed = generations.clone();
    changed[1].roots[0].sha256 = "f".repeat(64);
    let changed_hash = crate::digest(&canonical(&changed[1]).unwrap());
    assert!(
        validate_capture_generation(&changed, &group, &roots, 2, &changed_hash, None).is_err(),
        "all four immutable hashes required"
    );
    // Both generations contain the same enrollment: a later stage still cannot
    // substitute a different valid generation pair for the pending pair.
    let mut old = second.clone();
    old.generation = 1;
    old.previous_generation_sha256 = None;
    let old_hash = crate::digest(&canonical(&old).unwrap());
    second.previous_generation_sha256 = Some(old_hash.clone());
    let new_hash = crate::digest(&canonical(&second).unwrap());
    assert!(
        validate_capture_generation(
            &[old, second],
            &group,
            &roots,
            2,
            &new_hash,
            Some((1, &old_hash))
        )
        .is_err()
    );
}
#[test]
fn existing_source_lookup_is_unique_and_bound_to_group() {
    let first = source_group();
    let mut second = first.clone();
    second.group_id = "33333333-3333-4333-8333-333333333333".into();
    assert!(crate::registry::unique_source(std::iter::empty()).is_err());
    assert_eq!(
        crate::registry::unique_source(std::iter::once(&first))
            .unwrap()
            .group_id,
        first.group_id
    );
    assert!(crate::registry::unique_source([&first, &second].into_iter()).is_err());
    assert!(
        crate::registry::validate_build_binding(
            &first,
            Some(&"f".repeat(64)),
            Some(&first.application_commit),
            Some(&first.application_build_sha256)
        )
        .is_err()
    );
}
#[test]
fn current_source_fixtures_require_independent_registry() {
    let group = source_group();
    for (binding, commit, build) in [
        (
            None,
            Some(group.application_commit.as_str()),
            Some(group.application_build_sha256.as_str()),
        ),
        (
            Some(group.source_binding_sha256.as_str()),
            None,
            Some(group.application_build_sha256.as_str()),
        ),
        (
            Some(group.source_binding_sha256.as_str()),
            Some(group.application_commit.as_str()),
            None,
        ),
    ] {
        assert!(crate::registry::validate_build_binding(&group, binding, commit, build).is_err());
    }
    crate::registry::validate_build_binding(
        &group,
        Some(&group.source_binding_sha256),
        Some(&group.application_commit),
        Some(&group.application_build_sha256),
    )
    .unwrap();
}
#[test]
fn incomplete_catalog_protection_blocks_reconciliation() {
    assert!(matches!(
        crate::protection::require_finite_protection(true),
        Err(BackupError::Invalid(
            "registry protection incomplete catalog"
        ))
    ));
    crate::protection::require_finite_protection(false).unwrap();
}
#[test]
fn retained_backup_protects_blob_after_live_reference_removed() {
    let digest = "a".repeat(64);
    let plan = crate::BackupPlan::from_rows(vec![crate::AssetRow {
        space_id: uuid::Uuid::new_v4(),
        id: uuid::Uuid::new_v4(),
        sha256: digest.clone(),
        byte_size: 42,
        storage_key: crate::asset_key(&digest),
    }])
    .unwrap();
    let mut budget = ScanBudget::default();
    let mut counts = std::collections::BTreeMap::new();
    crate::protection::record_backup_keys(&mut budget, &mut counts, &plan).unwrap();
    crate::protection::record_backup_keys(&mut budget, &mut counts, &plan).unwrap();
    let empty_live = crate::BackupPlan::from_rows(vec![]).unwrap();
    crate::protection::record_backup_keys(&mut budget, &mut counts, &empty_live).unwrap();
    assert_eq!(counts[&digest], 2);
}

#[test]
fn registry_reopens_original_roots_and_rejects_same_bytes_new_inode() {
    let bytes=br#"{"capability":"backup_root_v1","deployment_id":"11111111-1111-4111-8111-111111111111","dev":1,"enrolled_generation":1,"enrollment_id":"22222222-2222-4222-8222-222222222222","format_version":1,"group_id":"33333333-3333-4333-8333-333333333333","ino":2,"kind":"source_assets","mode":448,"path":"/var/lib/source/assets","uid":0}"#;
    let root: crate::registry::RootRecord = canonical_record(bytes, 16384).unwrap();
    let path = std::path::Path::new("/var/lib/source/assets");
    crate::registry::validate_root_identity(&root, path, (1, 2)).unwrap();
    assert!(crate::registry::validate_root_identity(&root, path, (1, 3)).is_err());
    assert!(
        crate::registry::validate_root_identity(
            &root,
            std::path::Path::new("/var/lib/copied/assets"),
            (1, 2)
        )
        .is_err()
    );
}

#[test]
fn registry_same_byte_replacement_rejects_original_dev_inode() {
    let mut root:crate::registry::RootRecord=serde_json::from_value(json!({"format_version":1,"capability":"backup_root_v1","deployment_id":"11111111-1111-4111-8111-111111111111","enrollment_id":"22222222-2222-4222-8222-222222222222","group_id":"33333333-3333-4333-8333-333333333333","kind":"local_pins","path":"/var/lib/pins","dev":7,"ino":9,"uid":0,"mode":448,"enrolled_generation":1})).unwrap();
    let bytes = crate::registry::canonical(&root).unwrap();
    assert!(
        crate::registry::validate_root_identity(
            &root,
            std::path::Path::new("/var/lib/pins"),
            (7, 10)
        )
        .is_err()
    );
    root = canonical_record(&bytes, 16384).unwrap();
    assert!(
        crate::registry::validate_root_identity(
            &root,
            std::path::Path::new("/var/lib/pins"),
            (8, 9)
        )
        .is_err()
    );
}

#[test]
fn registry_generation_requires_complete_monotonic_unique_roster() {
    use crate::registry::{Generation, Roster};
    let r = Roster {
        id: "22222222-2222-4222-8222-222222222222".into(),
        sha256: "a".repeat(64),
    };
    let first = Generation {
        format_version: 1,
        capability: "backup_registry_generation_v1".into(),
        deployment_id: "11111111-1111-4111-8111-111111111111".into(),
        generation: 1,
        previous_generation_sha256: None,
        roots: vec![r.clone()],
        groups: vec![],
    };
    validate_generation_chain(std::slice::from_ref(&first), std::slice::from_ref(&r), &[]).unwrap();
    assert!(validate_generation_chain(std::slice::from_ref(&first), &[], &[]).is_err());
    let mut next = first.clone();
    next.generation = 2;
    next.previous_generation_sha256 =
        Some(crate::digest(&crate::registry::canonical(&first).unwrap()));
    validate_generation_chain(
        &[first.clone(), next.clone()],
        std::slice::from_ref(&r),
        &[],
    )
    .unwrap();
    next.roots.clear();
    assert!(validate_generation_chain(&[first.clone(), next], &[], &[]).is_err());
    let mut duplicate = first.clone();
    duplicate.roots.push(r.clone());
    assert!(validate_generation_chain(&[duplicate], &[r.clone(), r], &[]).is_err());
}

#[cfg(target_os = "linux")]
mod live_helpers {
    use crate::BackupError;
    use crate::{ManagementRegistry, source::lifecycle_tests::live::Fixture};
    use learning_assets::backup_fs::BackupDir;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    pub struct LockHolder {
        pid: libc::pid_t,
        release: std::fs::File,
        proof: BackupDir,
        ready_sha: String,
    }
    impl Drop for LockHolder {
        fn drop(&mut self) {
            use std::io::Write;
            let _ = self.release.write_all(b"x");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut status = 0;
            loop {
                let outcome = unsafe { libc::waitpid(self.pid, &mut status, libc::WNOHANG) };
                if outcome == self.pid {
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    unsafe {
                        libc::kill(self.pid, libc::SIGKILL);
                        libc::waitpid(self.pid, &mut status, 0);
                    }
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            if !std::thread::panicking() {
                assert!(
                    libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0,
                    "lock holder exit"
                );
            }
            if !std::thread::panicking() {
                let raw=crate::registry::canonical(&serde_json::json!({"format_version":1,"capability":"registry_lock_holder_reaped_v1","pid":self.pid,"ready_sha256":self.ready_sha,"exit_code":libc::WEXITSTATUS(status),"state":"REAPED"})).unwrap();
                let mut file = self
                    .proof
                    .create_file("registry-lock-holder-reaped.json")
                    .unwrap();
                file.write_all(&raw).unwrap();
                file.sync_all().unwrap();
                self.proof.sync().unwrap();
            }
        }
    }
    pub fn hold_registry(
        registry: &crate::ManagementRegistry,
        proof_path: &std::path::Path,
    ) -> LockHolder {
        use std::{
            io::Read,
            os::unix::{fs::MetadataExt, io::FromRawFd},
        };
        let path =
            std::ffi::CString::new(format!("{}/registry.lock", crate::registry::REGISTRY_PATH))
                .unwrap();
        let lock = registry.root.open_file("registry.lock").unwrap();
        let meta = lock.metadata().unwrap();
        let expected = (meta.dev(), meta.ino());
        drop(lock);
        let mut ready = [0; 2];
        let mut release = [0; 2];
        assert_eq!(
            unsafe { libc::pipe2(ready.as_mut_ptr(), libc::O_CLOEXEC) },
            0
        );
        assert_eq!(
            unsafe { libc::pipe2(release.as_mut_ptr(), libc::O_CLOEXEC) },
            0
        );
        let low = ready[1].min(release[0]) as u32;
        let high = ready[1].max(release[0]) as u32;
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0);
        if pid == 0 {
            // Only syscalls after fork. Close all inherited SQL/socket/secret
            // descriptors; preserve just the two handshake pipe endpoints.
            // The child's flock uses a NEW open file description.
            unsafe {
                if libc::syscall(libc::SYS_close_range, 0_u32, low - 1, 0_u32) != 0 {
                    libc::_exit(4);
                }
                if high > low + 1
                    && libc::syscall(libc::SYS_close_range, low + 1, high - 1, 0_u32) != 0
                {
                    libc::_exit(4);
                }
                if libc::syscall(libc::SYS_close_range, high + 1, u32::MAX, 0_u32) != 0 {
                    libc::_exit(4);
                }
                let fd = libc::open(
                    path.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                );
                if fd < 0 {
                    libc::_exit(2);
                }
                let mut m = std::mem::MaybeUninit::<libc::stat>::uninit();
                if libc::fstat(fd, m.as_mut_ptr()) != 0 {
                    libc::_exit(2);
                }
                let m = m.assume_init();
                if (m.st_dev, m.st_ino) != expected {
                    libc::_exit(2);
                }
                if libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) != 0 {
                    libc::_exit(2);
                }
                let byte = b'x';
                if libc::write(ready[1], (&byte as *const u8).cast(), 1) != 1 {
                    libc::_exit(3);
                }
                let mut signal = 0_u8;
                libc::read(release[0], (&mut signal as *mut u8).cast(), 1);
                libc::flock(fd, libc::LOCK_UN);
                libc::close(fd);
                libc::_exit(0);
            }
        }
        unsafe {
            libc::close(ready[1]);
            libc::close(release[0]);
        }
        let mut holder = LockHolder {
            pid,
            release: unsafe { std::fs::File::from_raw_fd(release[1]) },
            proof: BackupDir::open_trusted_private_root(proof_path).unwrap(),
            ready_sha: String::new(),
        };
        let mut poll = libc::pollfd {
            fd: ready[0],
            events: libc::POLLIN,
            revents: 0,
        };
        let result = unsafe { libc::poll(&mut poll, 1, 5000) };
        let mut read = unsafe { std::fs::File::from_raw_fd(ready[0]) };
        assert_eq!(result, 1, "bounded lock-holder handshake");
        let mut byte = [0];
        read.read_exact(&mut byte).unwrap();
        assert_eq!(byte, [b'x']);
        let mut stat = String::new();
        std::fs::File::open(format!("/proc/{pid}/stat"))
            .unwrap()
            .take(4097)
            .read_to_string(&mut stat)
            .unwrap();
        assert!(stat.len() <= 4096);
        let end = stat.rfind(')').unwrap();
        let starttime = stat[end + 1..]
            .split_whitespace()
            .nth(19)
            .unwrap()
            .parse::<u64>()
            .unwrap();
        assert!(starttime > 0);
        let raw=crate::registry::canonical(&serde_json::json!({"format_version":1,"capability":"registry_lock_holder_v1","pid":pid,"parent_pid":unsafe{libc::getpid()},"starttime":starttime,"lock_dev":expected.0,"lock_ino":expected.1,"state":"HELD_NEW_OFD"})).unwrap();
        holder.ready_sha = crate::digest(&raw);
        use std::io::Write;
        let mut file = holder
            .proof
            .create_file("registry-lock-holder.json")
            .unwrap();
        file.write_all(&raw).unwrap();
        file.sync_all().unwrap();
        holder.proof.sync().unwrap();
        holder
    }
    pub async fn captured() -> (Fixture, crate::SourceLocalPin) {
        let f = Fixture::new().await;
        let pin = crate::prepare_source_backup(&f.pool, &f.assets, &f.config)
            .await
            .unwrap();
        f.retained(&pin).await;
        assert!(f.acl().await);
        assert_eq!(f.journal().record().phase(), crate::GatePhase::Released);
        (f, pin)
    }
    pub fn source(f: &Fixture, lease: &crate::RegistryLease) -> crate::EnrolledSource {
        lease.admit_source(&f.root(), &f.assets, &f.config).unwrap()
    }
    pub fn replace_root(f: &Fixture, name: &str) {
        let path = if name == "registry" {
            std::path::PathBuf::from(crate::registry::REGISTRY_PATH)
        } else if name == "assets" {
            f.config.control_root.parent().unwrap().join("assets")
        } else {
            f.config.local_pin_root.clone()
        };
        if name == "registry" {
            fn copy(from: &std::path::Path, to: &std::path::Path) {
                for entry in std::fs::read_dir(from).unwrap() {
                    let e = entry.unwrap();
                    let dest = to.join(e.file_name());
                    if e.file_type().unwrap().is_dir() {
                        std::fs::DirBuilder::new()
                            .mode(0o700)
                            .create(&dest)
                            .unwrap();
                        copy(&e.path(), &dest);
                    } else {
                        std::fs::copy(e.path(), &dest).unwrap();
                        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o600))
                            .unwrap();
                    }
                }
            }
            // A mounted registry root cannot be renamed (EBUSY). A retained
            // test-only alternate namespace opens the copied original bytes.
            let substitute = f
                .config
                .control_root
                .parent()
                .unwrap()
                .join("regressions")
                .join(format!("registry-copy-{}", uuid::Uuid::new_v4()));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&substitute)
                .unwrap();
            copy(&path, &substitute);
            assert!(matches!(
                ManagementRegistry::open_test(&substitute),
                Err(BackupError::Invalid("registry original dev/inode"))
            ));
            return;
        }
        let retained = path.with_file_name(format!("retained-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::rename(&path, &retained).unwrap();
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        let lease = ManagementRegistry::open_installed()
            .unwrap()
            .try_lock()
            .unwrap();
        assert!(lease.admit_source(&f.root(), &f.assets, &f.config).is_err());
        drop(lease);
        // Preserve substitute evidence outside the strict tree; restore only
        // this test's held original. Never remove either populated directory.
        let failed = path.with_file_name(format!("substitute-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::rename(&path, &failed).unwrap();
        std::fs::rename(&retained, &path).unwrap();
        assert!(BackupDir::open_trusted_private_root(&path).is_ok());
    }

    pub fn attempt(f: &Fixture) -> std::path::PathBuf {
        let lease = ManagementRegistry::open_installed()
            .unwrap()
            .try_lock()
            .unwrap();
        let source = source(f, &lease);
        std::path::Path::new(crate::registry::REGISTRY_PATH)
            .join("protection")
            .join(source.group_id())
            .join(f.config.backup_id.to_string())
    }
    pub fn registry_inventory() -> Vec<(String, Option<Vec<u8>>)> {
        fn collect(dir: &BackupDir, prefix: &str, out: &mut Vec<(String, Option<Vec<u8>>)>) {
            use learning_assets::backup_fs::BackupEntryKind;
            for name in dir.list_bounded(1000).unwrap() {
                let path = format!("{prefix}{name}");
                match dir.kind(&name).unwrap() {
                    BackupEntryKind::Directory => {
                        out.push((path.clone(), None));
                        collect(&dir.open_dir(&name).unwrap(), &(path + "/"), out);
                    }
                    BackupEntryKind::File => {
                        use std::io::Read;
                        let file = dir.open_file(&name).unwrap();
                        assert!(file.metadata().unwrap().len() <= 524288);
                        let mut bytes = Vec::new();
                        file.take(524289).read_to_end(&mut bytes).unwrap();
                        out.push((path, Some(bytes)));
                    }
                    _ => panic!("unknown own registry fixture entry"),
                }
            }
        }
        let mut out = vec![];
        collect(
            &BackupDir::open_trusted_private_root(std::path::Path::new(
                crate::registry::REGISTRY_PATH,
            ))
            .unwrap(),
            "",
            &mut out,
        );
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }
    pub fn originals(f: &Fixture, pin: &crate::SourceLocalPin, old_phases: &[(String, Vec<u8>)]) {
        for (path, bytes) in &f.originals {
            assert_eq!(std::fs::read(path).unwrap(), *bytes);
        }
        assert_eq!(
            std::fs::read(f.config.control_root.join("source-binding.json")).unwrap(),
            f.binding
        );
        let old_id = pin.sealed().backup_id();
        for (name, bytes) in old_phases {
            assert_eq!(
                std::fs::read(
                    f.config
                        .control_root
                        .join(format!("{old_id}.control"))
                        .join(name)
                )
                .unwrap(),
                *bytes
            );
        }
        assert_eq!(
            crate::verify_sealed(&f.config.local_pin_root, old_id)
                .unwrap()
                .manifest_sha256(),
            pin.sealed().manifest_sha256()
        );
    }
    pub fn failed(value: Result<crate::SourceLocalPin, BackupError>) {
        assert!(matches!(
            value,
            Err(BackupError::Invalid("controlled lifecycle test fault"))
        ));
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "fresh independently provisioned PG18 root completion case"]
async fn real_registry_pending_staged_fault() {
    let (mut f, pin) = live_helpers::captured().await;
    let old = f.legacy_bytes();
    f.next().await;
    let controls = f.root().list().unwrap();
    let attempt = live_helpers::attempt(&f);
    crate::source::lifecycle_tests::fault("protection_pending_before_publish", 0);
    live_helpers::failed(crate::prepare_source_backup(&f.pool, &f.assets, &f.config).await);
    assert_eq!(f.root().list().unwrap(), controls);
    assert!(f.acl().await);
    assert!(!attempt.exists());
    let staging =
        crate::ManagementRegistry::open_test(std::path::Path::new(crate::registry::REGISTRY_PATH));
    assert!(matches!(
        staging,
        Err(BackupError::Invalid("registry publication incomplete"))
    ));
    let registry = learning_assets::backup_fs::BackupDir::open_trusted_private_root(
        std::path::Path::new(crate::registry::REGISTRY_PATH),
    )
    .unwrap();
    let staged = registry.open_dir("staging").unwrap();
    let names = staged.list_bounded(1).unwrap();
    assert_eq!(names.len(), 1);
    assert_eq!(
        staged.open_dir(&names[0]).unwrap().list_bounded(1).unwrap(),
        ["00-pending.json"]
    );
    let before = live_helpers::registry_inventory();
    assert!(crate::ManagementRegistry::open_installed().is_err());
    assert_eq!(live_helpers::registry_inventory(), before);
    live_helpers::originals(&f, &pin, &old);
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "fresh independently provisioned PG18 root completion case"]
async fn real_registry_pending_visible_fault() {
    let (mut f, pin) = live_helpers::captured().await;
    let old = f.legacy_bytes();
    f.next().await;
    let controls = f.root().list().unwrap();
    let attempt = live_helpers::attempt(&f);
    crate::source::lifecycle_tests::fault("protection_pending_after_publish", 0);
    live_helpers::failed(crate::prepare_source_backup(&f.pool, &f.assets, &f.config).await);
    assert_eq!(f.root().list().unwrap(), controls);
    assert!(f.acl().await);
    let pending = std::fs::read(attempt.join("00-pending.json")).unwrap();
    let before = live_helpers::registry_inventory();
    let registry = crate::ManagementRegistry::open_installed().unwrap();
    let lease = registry.try_lock().unwrap();
    let source = live_helpers::source(&f, &lease);
    // The retained attempt plus the visible pending are validated before
    // each resync. Every held publication-parent failure withholds authority.
    for skip in 0..6 {
        crate::source::lifecycle_tests::fault("protection_reopen_before_sync", skip);
        assert!(matches!(
            crate::CaptureProtection::reopen(&lease, &source, f.config.backup_id),
            Err(BackupError::Invalid("controlled lifecycle test fault"))
        ));
        assert_eq!(live_helpers::registry_inventory(), before);
    }
    assert!(
        crate::discover_backup_protection(&lease, &source).is_err(),
        "visible pending lacks a catalog even after successful resync"
    );
    assert_eq!(
        std::fs::read(attempt.join("00-pending.json")).unwrap(),
        pending
    );
    assert_eq!(live_helpers::registry_inventory(), before);
    assert!(f.acl().await);
    live_helpers::originals(&f, &pin, &old);
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "fresh independently provisioned PG18 root completion case"]
async fn real_registry_catalog_fault() {
    let (mut f, pin) = live_helpers::captured().await;
    let old = f.legacy_bytes();
    f.next().await;
    let attempt = live_helpers::attempt(&f);
    crate::source::lifecycle_tests::fault("protection_index_before_catalog", 0);
    live_helpers::failed(crate::prepare_source_backup(&f.pool, &f.assets, &f.config).await);
    assert_eq!(f.journal().record().phase(), crate::GatePhase::Drained);
    assert!(!f.acl().await);
    assert!(attempt.join("asset-index.json").is_file());
    assert!(!attempt.join("10-catalog.json").exists());
    let registry = learning_assets::backup_fs::BackupDir::open_trusted_private_root(
        std::path::Path::new(crate::registry::REGISTRY_PATH),
    )
    .unwrap();
    let staging = registry.open_dir("staging").unwrap();
    let names = staging.list_bounded(1).unwrap();
    assert_eq!(names.len(), 1);
    assert_eq!(
        staging
            .open_dir(&names[0])
            .unwrap()
            .list_bounded(1)
            .unwrap(),
        ["10-catalog.json"]
    );
    let before = live_helpers::registry_inventory();
    assert!(crate::ManagementRegistry::open_installed().is_err());
    assert_eq!(live_helpers::registry_inventory(), before);
    live_helpers::originals(&f, &pin, &old);
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "fresh independently provisioned PG18 root completion case"]
async fn real_registry_retained_release_fault() {
    let (mut f, pin) = live_helpers::captured().await;
    let old = f.legacy_bytes();
    f.next().await;
    let attempt = live_helpers::attempt(&f);
    crate::source::lifecycle_tests::fault("protection_retained_before_release", 0);
    live_helpers::failed(crate::prepare_source_backup(&f.pool, &f.assets, &f.config).await);
    assert_eq!(
        f.journal().record().phase(),
        crate::GatePhase::DumpAndIndexDurable
    );
    assert!(!f.acl().await);
    let retained: serde_json::Value =
        serde_json::from_slice(&std::fs::read(attempt.join("20-retained.json")).unwrap()).unwrap();
    let current = crate::verify_sealed(&f.config.local_pin_root, f.config.backup_id).unwrap();
    assert_eq!(
        retained["manifest_sha256"],
        json!(current.manifest_sha256())
    );
    assert!(
        !f.config
            .control_root
            .join(format!("{}.control/pins-durable.json", f.config.backup_id))
            .exists()
    );
    let before = live_helpers::registry_inventory();
    let lease = crate::ManagementRegistry::open_installed()
        .unwrap()
        .try_lock()
        .unwrap();
    let source = live_helpers::source(&f, &lease);
    crate::discover_backup_protection(&lease, &source)
        .unwrap()
        .recheck(&lease)
        .unwrap();
    assert_eq!(live_helpers::registry_inventory(), before);
    assert!(!f.acl().await);
    live_helpers::originals(&f, &pin, &old);
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "fresh independently provisioned PG18 root completion case"]
async fn real_registry_abandon_ready_fault() {
    let (mut f, pin) = live_helpers::captured().await;
    let old = f.legacy_bytes();
    f.next().await;
    f.early(crate::GatePhase::Drained).await;
    let journal = f.legacy_bytes();
    let attempt = live_helpers::attempt(&f);
    crate::source::lifecycle_tests::fault("protection_abandon_ready_before_release", 0);
    assert!(matches!(
        crate::abandon_source_backup(&f.pool, &f.config).await,
        Err(BackupError::Invalid("controlled lifecycle test fault"))
    ));
    assert_eq!(f.legacy_bytes(), journal);
    assert!(!f.acl().await);
    assert!(attempt.join("30-abandon-ready.json").is_file());
    assert!(!attempt.join("40-abandoned.json").exists());
    let side = f
        .config
        .control_root
        .join(format!("{}.abandonment", f.config.backup_id));
    assert!(side.join("ready.json").is_file());
    assert!(!side.join("abandoned.json").exists());
    let record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(attempt.join("30-abandon-ready.json")).unwrap())
            .unwrap();
    assert_eq!(
        record["abandonment_ready_sha256"],
        json!(crate::digest(
            &std::fs::read(side.join("ready.json")).unwrap()
        ))
    );
    assert_eq!(
        record["source_journal_sha256"],
        json!(crate::digest(
            &serde_json::to_vec(f.journal().record()).unwrap()
        ))
    );
    live_helpers::originals(&f, &pin, &old);
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "fresh independently provisioned PG18 root completion case"]
async fn real_registry_abandon_terminal_fault() {
    let (mut f, pin) = live_helpers::captured().await;
    let old = f.legacy_bytes();
    f.next().await;
    f.early(crate::GatePhase::Drained).await;
    let journal = f.legacy_bytes();
    let attempt = live_helpers::attempt(&f);
    crate::source::lifecycle_tests::fault("protection_abandon_terminal_before_registry", 0);
    assert!(matches!(
        crate::abandon_source_backup(&f.pool, &f.config).await,
        Err(BackupError::Invalid("controlled lifecycle test fault"))
    ));
    assert_eq!(f.legacy_bytes(), journal);
    assert!(!f.acl().await);
    assert!(attempt.join("30-abandon-ready.json").is_file());
    assert!(!attempt.join("40-abandoned.json").exists());
    let side = f
        .config
        .control_root
        .join(format!("{}.abandonment", f.config.backup_id));
    let ready = std::fs::read(side.join("ready.json")).unwrap();
    let terminal = std::fs::read(side.join("abandoned.json")).unwrap();
    crate::source::lifecycle::parse_terminal(
        &terminal,
        f.config.backup_id,
        &crate::digest(&f.binding),
        &ready,
    )
    .unwrap();
    let before = live_helpers::registry_inventory();
    f.settle().await;
    f.refresh().await;
    assert!(matches!(
        crate::abandon_source_backup(&f.pool, &f.config).await,
        Err(BackupError::Invalid(
            "terminal abandonment ACL ambiguous; manual recovery required"
        ))
    ));
    assert_eq!(live_helpers::registry_inventory(), before);
    assert_eq!(
        std::fs::read(side.join("abandoned.json")).unwrap(),
        terminal
    );
    assert!(!f.acl().await);
    live_helpers::originals(&f, &pin, &old);
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "fresh independently provisioned PG18 root completion case"]
async fn real_registry_reopen() {
    let (mut f, pin) = live_helpers::captured().await;
    let lease = crate::ManagementRegistry::open_installed()
        .unwrap()
        .try_lock()
        .unwrap();
    let source = live_helpers::source(&f, &lease);
    let snapshot = crate::discover_backup_protection(&lease, &source).unwrap();
    assert_eq!(snapshot.backup_reference_counts().len(), 3);
    snapshot.recheck(&lease).unwrap();
    drop(lease);
    f.refresh().await;
    let original = f.legacy_bytes();
    crate::finish_source_backup(&f.pool, &f.config)
        .await
        .unwrap();
    assert_eq!(original, f.legacy_bytes());
    f.retained(&pin).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "fresh independently provisioned PG18 root completion case"]
async fn real_registry_root_replacement() {
    let (f, pin) = live_helpers::captured().await;
    let before = f.legacy_bytes();
    for name in ["registry", "assets", "pins"] {
        live_helpers::replace_root(&f, name);
    }
    assert_eq!(before, f.legacy_bytes());
    assert!(f.acl().await);
    f.retained(&pin).await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "fresh independently provisioned PG18 root completion case"]
async fn real_registry_corrupt_stale() {
    use std::io::Write;
    let (f, pin) = live_helpers::captured().await;
    let before = f.legacy_bytes();
    let path = std::path::Path::new(crate::registry::REGISTRY_PATH)
        .join("generations/00000000000000000001.json");
    let original = std::fs::read(&path).unwrap();
    // Overwrite only this independently allocated test registry, restoring the
    // exact original inode before the final retained-volume audit.
    {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        file.write_all(b"{\"generation\":2}").unwrap();
        file.sync_all().unwrap();
    }
    assert!(
        crate::reconcile_registered_assets(&f.pool, &f.assets, std::time::SystemTime::now())
            .await
            .is_err()
    );
    {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        file.write_all(&original).unwrap();
        file.sync_all().unwrap();
    }
    let lease = crate::ManagementRegistry::open_installed()
        .unwrap()
        .try_lock()
        .unwrap();
    let source = live_helpers::source(&f, &lease);
    crate::discover_backup_protection(&lease, &source)
        .unwrap()
        .recheck(&lease)
        .unwrap();
    assert_eq!(before, f.legacy_bytes());
    assert!(f.acl().await);
    drop(lease);
    f.retained(&pin).await;
}

#[cfg(target_os = "linux")]
async fn case_registry_concurrent_capture() {
    let (mut f, pin) = live_helpers::captured().await;
    let before = f.legacy_bytes();
    let original_id = f.config.backup_id;
    let names = f.root().list().unwrap();
    f.next().await;
    let registry = crate::ManagementRegistry::open_installed().unwrap();
    let holder = live_helpers::hold_registry(&registry, &f.proof_root);
    let start = std::time::Instant::now();
    assert!(matches!(
        registry.try_lock(),
        Err(BackupError::Invalid("registry busy"))
    ));
    assert!(start.elapsed() < std::time::Duration::from_secs(1));
    assert!(matches!(
        crate::prepare_source_backup(&f.pool, &f.assets, &f.config).await,
        Err(BackupError::Invalid("registry busy"))
    ));
    let heartbeat = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        tokio::time::sleep(std::time::Duration::from_millis(1)),
    )
    .await;
    assert!(heartbeat.is_ok());
    assert_eq!(names, f.root().list().unwrap());
    assert!(
        f.root()
            .open_dir(&format!("{}.control", f.config.backup_id))
            .is_err()
    );
    f.config.backup_id = original_id;
    assert_eq!(before, f.legacy_bytes());
    drop(holder);
    assert!(f.acl().await);
    f.retained(&pin).await;
}

#[cfg(target_os = "linux")]
async fn case_registry_protection_release() {
    let (mut f, pin) = live_helpers::captured().await;
    let lease = crate::ManagementRegistry::open_installed()
        .unwrap()
        .try_lock()
        .unwrap();
    let source = live_helpers::source(&f, &lease);
    let first = crate::discover_backup_protection(&lease, &source)
        .unwrap()
        .backup_reference_counts();
    drop(lease);
    f.next().await;
    crate::prepare_source_backup(&f.pool, &f.assets, &f.config)
        .await
        .unwrap();
    let lease = crate::ManagementRegistry::open_installed()
        .unwrap()
        .try_lock()
        .unwrap();
    let source = live_helpers::source(&f, &lease);
    let second = crate::discover_backup_protection(&lease, &source)
        .unwrap()
        .backup_reference_counts();
    for key in first.keys() {
        assert_eq!(second[key], 2);
    }
    drop(lease);
    // Ready original independent of any logical use stays protected. The
    // fixture's linked row has FKs; remove only one unlinked test-owned row.
    let unlinked:Option<(uuid::Uuid,uuid::Uuid,String)>=sqlx::query_as("SELECT a.space_id,a.id,a.sha256 FROM public.asset a WHERE NOT EXISTS(SELECT 1 FROM public.resource_version r WHERE r.space_id=a.space_id AND r.asset_id=a.id) ORDER BY a.space_id,a.id LIMIT 1").fetch_optional(&f.pool).await.unwrap();
    let (space, id, digest) = unlinked.expect("unlinked ready fixture");
    sqlx::query("DELETE FROM public.asset WHERE space_id=$1 AND id=$2")
        .bind(space)
        .bind(id)
        .execute(&f.pool)
        .await
        .unwrap();
    let report = crate::reconcile_registered_assets(
        &f.pool,
        &f.assets,
        std::time::SystemTime::now() + std::time::Duration::from_secs(1),
    )
    .await
    .unwrap();
    assert!(
        !report
            .candidates
            .iter()
            .any(|c| c.path == crate::asset_key(&digest))
    );
    crate::verify_sealed(&f.config.local_pin_root, pin.sealed().backup_id()).unwrap();
    f.next().await;
    f.early(crate::GatePhase::Intent).await;
    f.abandon().await;
    let attempt = live_helpers::attempt(&f);
    let ready_raw = std::fs::read(attempt.join("30-abandon-ready.json")).unwrap();
    let terminal_path = attempt.join("40-abandoned.json");
    let terminal_raw = std::fs::read(&terminal_path).unwrap();
    let ready: serde_json::Value = canonical_record(&ready_raw, 16384).unwrap();
    let mut terminal: serde_json::Value = canonical_record(&terminal_raw, 16384).unwrap();
    let side = f
        .config
        .control_root
        .join(format!("{}.abandonment", f.config.backup_id));
    assert_eq!(
        ready["abandonment_ready_sha256"],
        json!(crate::digest(
            &std::fs::read(side.join("ready.json")).unwrap()
        ))
    );
    assert_eq!(
        terminal["previous_sha256"],
        json!(crate::digest(&ready_raw))
    );
    assert_eq!(
        terminal["abandonment_terminal_sha256"],
        json!(crate::digest(
            &std::fs::read(side.join("abandoned.json")).unwrap()
        ))
    );
    assert!(
        crate::reconcile_registered_assets(&f.pool, &f.assets, std::time::SystemTime::now())
            .await
            .is_err()
    );
    for (path, bytes) in &f.originals {
        assert_eq!(std::fs::read(path).unwrap(), *bytes);
    }
    let lease = crate::ManagementRegistry::open_installed()
        .unwrap()
        .try_lock()
        .unwrap();
    let source = live_helpers::source(&f, &lease);
    // A canonical but wrong immutable terminal link must block all discovery.
    // Preserve the bad bytes and all earlier records; no repair or cleanup.
    terminal["abandonment_terminal_sha256"] = json!("f".repeat(64));
    let changed = crate::registry::canonical(&terminal).unwrap();
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&terminal_path)
        .unwrap();
    file.write_all(&changed).unwrap();
    file.sync_all().unwrap();
    assert!(matches!(
        crate::discover_backup_protection(&lease, &source),
        Err(BackupError::Invalid(
            "registry abandonment terminal linkage"
        ))
    ));
    assert_eq!(std::fs::read(terminal_path).unwrap(), changed);
    assert_eq!(
        std::fs::read(attempt.join("30-abandon-ready.json")).unwrap(),
        ready_raw
    );
}

#[test]
fn registry_scan_512mib_and_unique_digest_caps_fail_closed() {
    let mut budget = ScanBudget::default();
    budget.charge(536870912).unwrap();
    assert!(matches!(
        budget.charge(1),
        Err(BackupError::Capacity("registry scan bytes"))
    ));
    let mut keys = ScanBudget::default();
    for n in 0..100000 {
        keys.protect(&format!("{n:064x}")).unwrap();
    }
    keys.protect(&format!("{:064x}", 0)).unwrap();
    assert!(matches!(
        keys.protect(&format!("{:064x}", 100000)),
        Err(BackupError::Capacity("registry protected digests"))
    ));
}

#[test]
fn registry_missing_corrupt_stale_or_unknown_withholds_all_candidates() {
    assert!(validate_generation_chain(&[], &[], &[]).is_err());
    assert!(canonical_record::<serde_json::Value>(b"{\"a\":1,\"a\":1}", 4096).is_err());
    assert!(canonical_record::<serde_json::Value>(b"{\"a\":1}\n", 4096).is_err());
    assert!(canonical_record::<serde_json::Value>(b"{\"a\":1.0}", 4096).is_err());
}

#[test]
fn registry_canonical_cross_language_golden() {
    let bytes = b"{\"capability\":\"backup_registry_generation_v1\",\"deployment_id\":\"11111111-1111-4111-8111-111111111111\",\"format_version\":1,\"generation\":1,\"groups\":[],\"previous_generation_sha256\":null,\"roots\":[]}";
    let value: serde_json::Value = canonical_record(bytes, 524288).unwrap();
    assert_eq!(serde_json::to_vec(&value).unwrap(), bytes);
    assert_eq!(value["generation"], json!(1));
}

#[test]
fn journal_v1_bytes_unchanged_by_registry() {
    let id = uuid::Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap();
    assert_eq!(serde_json::to_vec(&crate::SourceGateRecord::new(id)).unwrap(), b"{\"backup_id\":\"11111111-1111-4111-8111-111111111111\",\"phase\":\"intent\",\"dump_and_index_sha256\":null,\"pins_sha256\":null}");
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "fresh independently provisioned PG18 root completion case"]
async fn real_registry_concurrent_capture() {
    case_registry_concurrent_capture().await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "fresh independently provisioned PG18 root completion case"]
async fn real_registry_protection_release() {
    case_registry_protection_release().await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "fresh independently provisioned PG18 root completion case; same body as concurrent case"]
async fn registry_busy_is_immediate_and_mutation_free() {
    case_registry_concurrent_capture().await;
}
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "fresh independently provisioned PG18 root completion case; same body as protection-release case"]
async fn prepare_finish_abandon_share_enrolled_roots_and_keep_all_protection() {
    case_registry_protection_release().await;
}

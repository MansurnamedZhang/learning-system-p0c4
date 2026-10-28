use super::{
    Error,
    job::{self, ControlData, ControlState, Direction, ProcessRole},
    process,
};
use learning_assets::root_journal::SharedJournalBudget;
use std::{ffi::CString, os::fd::AsRawFd};

const JOB: &str = "11111111-1111-4111-8111-111111111111";
const BACKUP: &str = "22222222-2222-4222-8222-222222222222";
const REQUEST: &str = "33333333-3333-4333-8333-333333333333";
const DB: &str = "learning_backup_c4_task3_22222222-2222-4222-8222-222222222222";
fn value(kind: &str, sequence: u64) -> serde_json::Value {
    let mut v = serde_json::json!({"schema_version":1,"job_id":JOB,"root_cookie":"a".repeat(64),"sequence":sequence,"kind":kind});
    let object = v.as_object_mut().unwrap();
    for (k, s) in match kind {
        "SourceBootstrap" | "CaptureBootstrap" => vec![("source_start_sha256", "b".repeat(64))],
        "SpawnCapture" => vec![],
        "CaptureInit" => vec![
            ("backup_id", BACKUP.into()),
            ("capture_nonce", "c".repeat(64)),
            ("request_id", REQUEST.into()),
        ],
        "CaptureStartBound" => vec![
            ("backup_id", BACKUP.into()),
            ("capture_start_sha256", "d".repeat(64)),
        ],
        "RequestInstallProof" | "RequestCaptureProof" => {
            vec![("backup_id", BACKUP.into()), ("request_id", REQUEST.into())]
        }
        "BrokerInstalled" | "BrokerInstallObserved" => {
            let mut fields = vec![
                ("backup_id", BACKUP.into()),
                ("request_id", REQUEST.into()),
                ("binding_id", "44444444-4444-4444-8444-444444444444".into()),
                ("epoch_id", "55555555-5555-4555-8555-555555555555".into()),
                ("descriptor_sha256", "6".repeat(64)),
            ];
            if kind == "BrokerInstallObserved" {
                fields.push(("observation_sha256", "7".repeat(64)));
            }
            fields
        }
        "InstallProofBound" | "CaptureProofBound" => vec![
            ("backup_id", BACKUP.into()),
            ("request_id", REQUEST.into()),
            ("generation_binding_sha256", "e".repeat(64)),
        ],
        "CapturedDataReady" | "SettleCapture" => vec![
            ("backup_id", BACKUP.into()),
            ("captured_data_sha256", "f".repeat(64)),
        ],
        "CaptureSettled" => vec![
            ("backup_id", BACKUP.into()),
            ("capture_settlement_sha256", "0".repeat(64)),
        ],
        "Abort" => vec![("reason_code", "RootOwnerLost".into())],
        _ => panic!("test only fixed message"),
    } {
        object.insert(k.into(), s.into());
    }
    v
}
fn data(kind: &str, sequence: u64) -> ControlData {
    job::parse(&job::canonical(&value(kind, sequence)).unwrap(), 4096).unwrap()
}
fn observe(state: &mut ControlState, kind: &str, sequence: u64, direction: Direction) {
    state.observe(&data(kind, sequence), direction).unwrap();
}

#[test]
fn typed_all_fifteen_message_shapes_round_trip_without_qualification() {
    for kind in [
        "SourceBootstrap",
        "SpawnCapture",
        "CaptureBootstrap",
        "CaptureInit",
        "CaptureStartBound",
        "RequestInstallProof",
        "InstallProofBound",
        "BrokerInstalled",
        "BrokerInstallObserved",
        "RequestCaptureProof",
        "CaptureProofBound",
        "CapturedDataReady",
        "SettleCapture",
        "CaptureSettled",
        "Abort",
    ] {
        let raw = job::canonical(&value(kind, 0)).unwrap();
        let parsed: ControlData = job::parse(&raw, 4096).unwrap();
        assert_eq!(parsed.kind(), kind);
        assert_eq!(job::canonical(&parsed).unwrap(), raw);
    }
}
#[test]
fn typed_control_rejects_duplicates_unknown_fields_and_unknown_kind() {
    let raw = job::canonical(&value("SpawnCapture", 0)).unwrap();
    let text = String::from_utf8(raw).unwrap();
    let duplicate = text.replacen(
        "\"schema_version\":1",
        "\"schema_version\":1,\"schema_version\":1",
        1,
    );
    assert!(job::parse::<ControlData>(duplicate.as_bytes(), 4096).is_err());
    let mut v = value("SpawnCapture", 0);
    v["unapproved"] = true.into();
    assert!(job::parse::<ControlData>(&job::canonical(&v).unwrap(), 4096).is_err());
    v.as_object_mut().unwrap().remove("unapproved");
    v["kind"] = "Heartbeat".into();
    assert!(job::parse::<ControlData>(&job::canonical(&v).unwrap(), 4096).is_err());
}
#[test]
fn typed_control_rejects_noncanonical_layout_numbers_and_extra_bytes() {
    let raw = job::canonical(&value("SpawnCapture", 0)).unwrap();
    let mut newline = raw.clone();
    newline.push(b'\n');
    assert!(job::parse::<ControlData>(&newline, 4096).is_err());
    let mut leading = vec![b' '];
    leading.extend(&raw);
    assert!(job::parse::<ControlData>(&leading, 4096).is_err());
    let float = String::from_utf8(raw)
        .unwrap()
        .replace("\"sequence\":0", "\"sequence\":0.0");
    assert!(job::parse::<ControlData>(float.as_bytes(), 4096).is_err());
}
#[test]
fn typed_control_rejects_wrong_ranges_schema_uuid_and_digest_case() {
    for (field, bad) in [
        ("schema_version", serde_json::json!(2)),
        ("sequence", serde_json::json!(-1)),
        (
            "job_id",
            serde_json::json!("11111111-1111-1111-8111-111111111111"),
        ),
        ("root_cookie", serde_json::json!("A".repeat(64))),
    ] {
        let mut v = value("SpawnCapture", 0);
        v[field] = bad;
        assert!(job::parse::<ControlData>(&job::canonical(&v).unwrap(), 4096).is_err());
    }
}
#[test]
fn typed_nested_record_rejects_duplicate_unknown_and_noncanonical_digest() {
    let good =
        r#"{"data_dev":1,"data_ino":2,"postmaster_start_ticks":3,"socket_dev":4,"socket_ino":5}"#;
    assert!(job::parse::<job::Native>(good.as_bytes(), 4096).is_ok());
    let duplicate = good.replace("\"data_dev\":1", "\"data_dev\":1,\"data_dev\":1");
    assert!(job::parse::<job::Native>(duplicate.as_bytes(), 4096).is_err());
    let extra = good.replace("\"socket_ino\":5", "\"socket_ino\":5,\"unchecked\":0");
    assert!(job::parse::<job::Native>(extra.as_bytes(), 4096).is_err());
    assert!(
        job::parse::<job::Native>(
            good.replace("\"data_dev\":1", "\"data_dev\":0").as_bytes(),
            4096
        )
        .is_err()
    );
}
#[test]
fn typed_paths_uuid_and_system_identifier_are_exact_data_predicates() {
    assert!(job::path("/opt/knowweave-c4/bin/exact"));
    for bad in [
        "relative",
        "//root",
        "/root/../secret",
        "/root/./file",
        "/root//file",
        "/root/",
        "/root/\0file",
    ] {
        assert!(!job::path(bad));
    }
    assert!(job::uuid(JOB, true));
    assert!(!job::uuid("11111111-1111-4111-A111-111111111111", true));
    assert!(job::database(DB));
    assert!(!job::database(
        "learning_backup_c4_task3_11111111-1111-1111-8111-111111111111"
    ));
}
#[test]
fn source_data_phase_allows_only_original_order_and_contiguous_directions() {
    let mut s = ControlState::new(ProcessRole::Source);
    for (k, n, d) in [
        ("SourceBootstrap", 0, Direction::Receive),
        ("SpawnCapture", 0, Direction::Send),
        ("CapturedDataReady", 1, Direction::Receive),
        ("SettleCapture", 1, Direction::Send),
        ("CaptureSettled", 2, Direction::Receive),
    ] {
        observe(&mut s, k, n, d);
    }
    assert!(!s.is_failed());
    assert_eq!(
        s.observe(&data("SpawnCapture", 2), Direction::Send),
        Err(Error::Phase)
    );
    assert!(s.is_failed());
}
#[test]
fn capture_data_phase_requires_approved_fifteen_contract_ack() {
    let mut s = ControlState::new(ProcessRole::Capture);
    for (k, n, d) in [
        ("CaptureBootstrap", 0, Direction::Receive),
        ("CaptureInit", 0, Direction::Send),
        ("CaptureStartBound", 1, Direction::Receive),
        ("RequestInstallProof", 1, Direction::Send),
        ("InstallProofBound", 2, Direction::Receive),
        ("BrokerInstalled", 2, Direction::Send),
        ("BrokerInstallObserved", 3, Direction::Receive),
        ("RequestCaptureProof", 3, Direction::Send),
        ("CaptureProofBound", 4, Direction::Receive),
        ("CapturedDataReady", 4, Direction::Send),
        ("SettleCapture", 5, Direction::Receive),
    ] {
        observe(&mut s, k, n, d);
    }
    assert!(!s.is_failed());
    assert_eq!(
        s.observe(&data("CaptureSettled", 5), Direction::Send),
        Err(Error::Phase)
    );
}
#[test]
fn wrong_direction_sequence_and_phase_make_data_model_sticky() {
    for (kind, sequence, direction) in [
        ("SourceBootstrap", 0, Direction::Send),
        ("SourceBootstrap", 1, Direction::Receive),
        ("CapturedDataReady", 0, Direction::Receive),
    ] {
        let mut s = ControlState::new(ProcessRole::Source);
        assert!(s.observe(&data(kind, sequence), direction).is_err());
        assert_eq!(
            s.observe(&data("SourceBootstrap", 0), Direction::Receive),
            Err(Error::Poisoned)
        );
    }
}
#[test]
fn changed_job_and_cookie_are_not_adopted_after_bootstrap() {
    for key in ["job_id", "root_cookie"] {
        let mut s = ControlState::new(ProcessRole::Source);
        observe(&mut s, "SourceBootstrap", 0, Direction::Receive);
        let mut v = value("SpawnCapture", 0);
        v[key] = if key == "job_id" {
            BACKUP.into()
        } else {
            "0".repeat(64).into()
        };
        let d: ControlData = job::parse(&job::canonical(&v).unwrap(), 4096).unwrap();
        assert_eq!(s.observe(&d, Direction::Send), Err(Error::Identity));
        assert!(s.is_failed());
    }
}
#[test]
fn changed_backup_request_or_digest_is_rejected_without_replay() {
    for key in ["backup_id", "request_id"] {
        let mut s = ControlState::new(ProcessRole::Capture);
        for (k, n, d) in [
            ("CaptureBootstrap", 0, Direction::Receive),
            ("CaptureInit", 0, Direction::Send),
            ("CaptureStartBound", 1, Direction::Receive),
            ("RequestInstallProof", 1, Direction::Send),
        ] {
            observe(&mut s, k, n, d);
        }
        let mut v = value("InstallProofBound", 2);
        v[key] = JOB.into();
        let d: ControlData = job::parse(&job::canonical(&v).unwrap(), 4096).unwrap();
        assert_eq!(s.observe(&d, Direction::Receive), Err(Error::Identity));
        assert!(s.is_failed());
    }
    let mut s = ControlState::new(ProcessRole::Source);
    for (k, n, d) in [
        ("SourceBootstrap", 0, Direction::Receive),
        ("SpawnCapture", 0, Direction::Send),
        ("CapturedDataReady", 1, Direction::Receive),
    ] {
        observe(&mut s, k, n, d);
    }
    let mut v = value("SettleCapture", 1);
    v["captured_data_sha256"] = "0".repeat(64).into();
    let d: ControlData = job::parse(&job::canonical(&v).unwrap(), 4096).unwrap();
    assert_eq!(s.observe(&d, Direction::Send), Err(Error::Identity));
}
#[test]
fn abort_is_terminal_even_when_shape_cookie_and_sequence_match() {
    let mut s = ControlState::new(ProcessRole::Source);
    observe(&mut s, "SourceBootstrap", 0, Direction::Receive);
    assert_eq!(
        s.observe(&data("Abort", 1), Direction::Receive),
        Err(Error::RootLost)
    );
    assert!(s.is_failed());
    assert_eq!(
        s.observe(&data("SpawnCapture", 0), Direction::Send),
        Err(Error::Poisoned)
    );
}
#[test]
fn control_reserves_exact_prefix_inclusive_sixty_four_frame_budget() {
    let scan = SharedJournalBudget::default();
    let mut s = ControlState::new(ProcessRole::Source);
    for _ in 0..64 {
        s.reserve(4096, &scan).unwrap();
    }
    let usage = scan.usage().unwrap();
    assert_eq!(usage.charged_bytes, 262400);
    assert_eq!(usage.charged_entries, 64);
    assert_eq!(s.reserve(1, &scan), Err(Error::Capacity));
    assert_eq!(scan.usage().unwrap(), usage);
    assert_eq!(s.reserve(1, &scan), Err(Error::Poisoned));
}
#[test]
fn control_length_rejection_precedes_allocation_and_reservation_is_sticky() {
    for length in [0, 4097, usize::MAX] {
        let scan = SharedJournalBudget::default();
        let mut s = ControlState::new(ProcessRole::Source);
        assert_eq!(s.reserve(length, &scan), Err(Error::Capacity));
        assert_eq!(scan.usage().unwrap().charged_bytes, 0);
        assert_eq!(s.reserve(1, &scan), Err(Error::Poisoned));
    }
}
#[test]
fn control_scan_exhaustion_cannot_be_replaced_with_a_new_scope() {
    let scan = SharedJournalBudget::default();
    let alias = scan.clone();
    scan.charge(512 * 1024 * 1024, 0).unwrap();
    let mut s = ControlState::new(ProcessRole::Source);
    assert_eq!(s.reserve(1, &scan), Err(Error::Capacity));
    assert!(alias.usage().unwrap().exhausted);
    assert_eq!(
        s.reserve(1, &SharedJournalBudget::default()),
        Err(Error::Poisoned)
    );
}
#[test]
fn actual_ordinary_uid_rejects_factory_attempt_before_other_observation() {
    assert_ne!(
        unsafe { libc::geteuid() },
        0,
        "this exact negative belongs to ordinary UID Linux checks"
    );
    assert_eq!(process::burn_process_attempt(), Err(Error::Identity));
    assert_eq!(process::burn_process_attempt(), Err(Error::Poisoned));
}
#[test]
fn actual_socket_and_fd_primitives_do_not_qualify_ordinary_root_peer() {
    assert_ne!(unsafe { libc::geteuid() }, 0);
    let (a, _b) = std::os::unix::net::UnixStream::pair().unwrap();
    process::mark_cloexec_nonblocking(a.as_raw_fd()).unwrap();
    process::require_cloexec(a.as_raw_fd()).unwrap();
    assert_eq!(
        process::source_socket_facts(a.as_raw_fd()),
        Err(Error::Identity)
    );
}
#[test]
fn actual_poll_deadline_and_closed_peer_are_not_success_or_settlement() {
    let (a, b) = std::os::unix::net::UnixStream::pair().unwrap();
    assert_eq!(
        process::poll_fd(a.as_raw_fd(), libc::POLLIN, std::time::Instant::now()),
        Err(Error::Deadline)
    );
    drop(b);
    let result = process::poll_fd(
        a.as_raw_fd(),
        libc::POLLOUT,
        std::time::Instant::now() + std::time::Duration::from_secs(1),
    );
    // POLLOUT can accompany HUP: readiness is never natural-settlement proof.
    assert!(matches!(result, Ok(()) | Err(Error::Eof)));
}
#[test]
fn proc_stat_parser_handles_parentheses_and_rejects_pid_or_tick_changes() {
    let tail = format!("S {} 900", vec!["0"; 18].join(" "));
    let stat = format!("42 (name ) ( nested)) {tail}");
    assert_eq!(process::parse_start(stat.as_bytes(), 42), Ok(900));
    assert_eq!(
        process::parse_start(stat.as_bytes(), 41),
        Err(Error::Identity)
    );
    assert!(process::parse_start(stat.replace("900", "0900").as_bytes(), 42).is_err());
    assert!(process::parse_start(b"42 (truncated)", 42).is_err());
}
#[test]
fn actual_pidfd_keeps_exact_start_and_never_opens_an_arbitrary_parent_ctx() {
    let scan = SharedJournalBudget::default();
    let pid = std::process::id();
    let start = process::actual_start(pid, &scan).unwrap();
    let held = process::ParentPid::open(pid, start, &scan).unwrap();
    held.revalidate(&scan).unwrap();
    assert!(process::ParentPid::open(pid, start.checked_add(1).unwrap(), &scan).is_err());
    assert!(process::ParentPid::open(0, start, &scan).is_err());
}
fn env(entries: &[&str]) -> Vec<CString> {
    entries.iter().map(|s| CString::new(*s).unwrap()).collect()
}
fn scan_env(values: &[CString]) -> super::Result<()> {
    let mut ptrs: Vec<_> = values.iter().map(|s| s.as_ptr()).collect();
    ptrs.push(std::ptr::null());
    unsafe {
        process::scan_environment(
            ptrs.as_ptr(),
            "/var/run/postgresql",
            DB,
            &SharedJournalBudget::default(),
        )
    }
}
fn fixed_env() -> Vec<CString> {
    env(&[
        "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
        "HOME=/root",
        "LANG=C",
        "PGHOST=/var/run/postgresql",
        "PGPORT=5432",
        "PGUSER=learning_admin",
        "PGDATABASE=learning_backup_c4_task3_22222222-2222-4222-8222-222222222222",
    ])
}
#[test]
fn environment_exact_seven_keys_values_accept_data_only_and_reject_all_extras() {
    let good = fixed_env();
    assert_eq!(scan_env(&good), Ok(()));
    for key in [
        "PGPASSWORD",
        "PGPASSFILE",
        "DATABASE_URL",
        "LD_PRELOAD",
        "TEST_DATABASE_URL",
        "PGHOSTADDR",
        "OTHER",
    ] {
        let mut altered = fixed_env();
        altered[0] = CString::new(format!("{key}=not-read")).unwrap();
        assert_eq!(scan_env(&altered), Err(Error::Identity));
    }
    let mut extra = fixed_env();
    extra.push(CString::new("PATH=x").unwrap());
    assert!(scan_env(&extra).is_err());
}
#[test]
fn environment_missing_duplicate_and_wrong_values_never_repair_global_env() {
    let mut missing = fixed_env();
    missing.pop();
    assert_eq!(scan_env(&missing), Err(Error::Identity));
    let mut duplicate = fixed_env();
    duplicate[1] =
        CString::new("PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin").unwrap();
    assert_eq!(scan_env(&duplicate), Err(Error::Identity));
    for (index, bad) in [
        (0, "PATH=/bin"),
        (1, "HOME=/tmp"),
        (2, "LANG=C.UTF-8"),
        (3, "PGHOST=localhost"),
        (4, "PGPORT=05432"),
        (5, "PGUSER=postgres"),
        (6, "PGDATABASE=other"),
    ] {
        let mut wrong = fixed_env();
        wrong[index] = CString::new(bad).unwrap();
        assert_eq!(scan_env(&wrong), Err(Error::Identity));
    }
}
#[test]
fn forbidden_environment_value_guard_page_is_never_dereferenced() {
    let page = usize::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).unwrap();
    assert!(page >= 4096);
    let raw = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            page * 2,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    assert_ne!(raw, libc::MAP_FAILED);
    let key = b"PGPASSWORD=";
    let start = unsafe { raw.cast::<u8>().add(page - key.len()) };
    unsafe {
        std::ptr::copy_nonoverlapping(key.as_ptr(), start, key.len());
    }
    assert_eq!(
        unsafe { libc::mprotect(raw.cast::<u8>().add(page).cast(), page, libc::PROT_NONE) },
        0
    );
    let ptrs = [
        start.cast::<libc::c_char>() as *const libc::c_char,
        std::ptr::null(),
    ];
    let result = unsafe {
        process::scan_environment(
            ptrs.as_ptr(),
            "/var/run/postgresql",
            DB,
            &SharedJournalBudget::default(),
        )
    };
    assert_eq!(unsafe { libc::munmap(raw, page * 2) }, 0);
    assert_eq!(result, Err(Error::Identity));
}
#[test]
fn nonregular_named_file_is_not_code_and_no_test_creates_a_qualified_ctx() {
    let raw = std::fs::OpenOptions::new()
        .read(true)
        .open("/dev/null")
        .unwrap();
    let scan = SharedJournalBudget::default();
    let stamp = process::Stamp::observe(&raw, &scan).unwrap();
    assert_eq!(stamp.mode & libc::S_IFMT, libc::S_IFCHR);
    // DATA-only actual metadata; no Root or qualified process is manufactured.
    // Exercise the production rejection entry instead of crediting stat alone.
    let declared = job::CodeFile {
        path: "/dev/null".into(),
        stamp: job::FileStamp {
            dev: stamp.dev,
            ino: stamp.ino,
            uid: u64::from(stamp.uid),
            gid: u64::from(stamp.gid),
            mode: u64::from(stamp.mode),
            nlink: stamp.links,
            bytes: stamp.bytes,
            mtime_ns: stamp.mtime_ns,
            ctime_ns: stamp.ctime_ns,
        },
        sha256: "0".repeat(64),
    };
    assert!(matches!(
        process::HeldCode::open(&declared, false, &scan),
        Err(Error::Identity)
    ));
    assert!(job::parse::<job::RecordRef>(br#"{"bytes":1,"record":"arbitrary.json","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,4096).is_err());
}

#[test]
fn mapping_parser_rejects_unknown_anonymous_writable_alias_and_deleted_code() {
    let stamp = process::Stamp {
        dev: libc::makedev(8, 1),
        ino: 42,
        uid: 0,
        gid: 0,
        mode: libc::S_IFREG | 0o500,
        links: 1,
        bytes: 4096,
        mtime_ns: 1,
        ctime_ns: 1,
    };
    let roster = [("/opt/knowweave-c4/bin/knowweave-c4-b1-source", stamp)];
    let good=b"1000-2000 r-xp 00000000 08:01 42 /opt/knowweave-c4/bin/knowweave-c4-b1-source\n2000-3000 r-xp 00000000 00:00 0 [vdso]\n";
    assert!(process::parse_executable_mappings(good, &roster).is_ok());
    for bad in [
        "1000-2000 rwxp 0 08:01 42 /opt/knowweave-c4/bin/knowweave-c4-b1-source\n",
        "1000-2000 r-xp 0 08:01 42 /different-alias\n",
        "1000-2000 r-xp 0 08:01 42 /opt/knowweave-c4/bin/knowweave-c4-b1-source (deleted)\n",
        "1000-2000 r-xp 0 00:00 0\n",
        "1000-2000 r-xp 0 08:01 41 /opt/knowweave-c4/bin/knowweave-c4-b1-source\n",
    ] {
        assert!(process::parse_executable_mappings(bad.as_bytes(), &roster).is_err());
    }
}
#[test]
fn launch_manifest_retains_old_network_source_shape_without_process_authority() {
    let mut v = serde_json::json!({"format_version":1,"capability":"task9_b1_launch_manifest_v1","profile":"task9-b1-limited-v1","launch_id":JOB,"role":"source","code_commit":"a".repeat(40),"build_input_sha256":"b".repeat(64),"binary_sha256":"c".repeat(64),"launcher_sha256":"d".repeat(64),"policy_sha256":"e".repeat(64),"anchor_sha256":"f".repeat(64),"source_auth_public_key_sha256":"0".repeat(64),"destination_verifier_public_key_sha256":"1".repeat(64),"transport_public_key_sha256":"2".repeat(64)});
    assert!(job::parse::<job::LaunchManifest>(&job::canonical(&v).unwrap(), 4096).is_ok());
    for bad in ["capture", "root-launcher", "destination"] {
        v["role"] = bad.into();
        assert!(job::parse::<job::LaunchManifest>(&job::canonical(&v).unwrap(), 4096).is_err());
    }
    v["role"] = "source".into();
    v["process_role"] = "root-launcher".into();
    assert!(job::parse::<job::LaunchManifest>(&job::canonical(&v).unwrap(), 4096).is_err());
}
#[test]
fn actual_zero_argument_factory_denies_ordinary_process_in_fresh_child() {
    assert_ne!(unsafe { libc::geteuid() }, 0);
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "snapshot_broker::b1_peer::tests::ordinary_factory_fresh_child_entry",
            "--ignored",
            "--test-threads=1",
        ])
        .env_clear()
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(
        std::str::from_utf8(&result.stdout)
            .unwrap()
            .contains("1 passed; 0 failed")
    );
}
#[test]
#[ignore = "ordinary UID only; invoked once by the exact subprocess negative"]
fn ordinary_factory_fresh_child_entry() {
    assert_ne!(unsafe { libc::geteuid() }, 0);
    assert!(matches!(
        super::B1InstalledSourceProcess::admit_installed(),
        Err(super::super::ObservationError::Invalid(
            "B1 installed source process rejected"
        ))
    ));
    assert!(matches!(
        super::B1InstalledSourceProcess::admit_installed(),
        Err(super::super::ObservationError::Invalid(
            "B1 source process permanently held"
        ))
    ));
}

#[test]
fn identity_mapping_parser_rejects_remap_multisegment_overflow_and_extra_text() {
    assert!(process::supported_identity_mapping(b"         0          0 4294967295\n").is_ok());
    for bad in [
        b"0 1000 1\n".as_slice(),
        b"0 0 4294967295\n1 1 1\n",
        b"00 0 4294967295\n",
        b"0 0 4294967296\n",
        b"0 0 -1\n",
        b"0 0 4294967295 extra\n",
        b"0 0 4294967295\r\n",
        b"\n",
        b"0 0 4294967295\n\n",
    ] {
        assert_eq!(
            process::supported_identity_mapping(bad),
            Err(Error::Identity)
        );
    }
    assert_eq!(
        process::supported_identity_mapping(&[b' '; 257]),
        Err(Error::Identity)
    );
}
#[test]
fn actual_namespace_kind_guard_rejects_non_namespace_device_without_qualification() {
    assert_ne!(unsafe { libc::geteuid() }, 0);
    let file = std::fs::OpenOptions::new()
        .read(true)
        .open("/dev/null")
        .unwrap();
    assert_eq!(
        process::require_user_namespace_fd(&file, &SharedJournalBudget::default()),
        Err(Error::Identity)
    );
}
#[test]
fn actual_ordinary_proc_view_holds_real_self_userns_objects_without_initial_claim() {
    assert_ne!(unsafe { libc::geteuid() }, 0);
    let scan = SharedJournalBudget::default();
    let view = process::ProcView::open(&scan).unwrap();
    let pid = std::process::id();
    let first = view.user_namespace(pid, &scan).unwrap();
    let second = view.user_namespace(pid, &scan).unwrap();
    assert!(first.same_object(&second));
    let raw = view
        .read_relative(&format!("{pid}/uid_map"), 256, &scan)
        .unwrap();
    assert!(!raw.is_empty());
    assert!(
        view.read_relative(&format!("{pid}/../1/status"), 65536, &scan)
            .is_err()
    );
    assert!(
        view.read_relative(&format!("{pid}/ns/user"), 256, &scan)
            .is_err()
    );
    // No qualified process, Root peer, initial-host identity or C fact is made.
}
#[test]
fn proc_root_descriptor_rejects_subtree_bind_wrong_mount_and_nonproc_data() {
    let dev = libc::makedev(0, 22);
    let good = b"42 1 0:22 / /proc rw,nosuid - proc proc rw\n";
    assert!(process::proc_root_mount_descriptor(good, 42, dev).is_ok());
    for bad in [
        "42 1 0:22 /different /proc rw - proc proc rw\n",
        "42 1 0:22 / /proc rw - tmpfs tmpfs rw\n",
        "42 1 0:22 / /alias rw - proc proc rw\n",
        "41 1 0:22 / /proc rw - proc proc rw\n",
        "42 1 0:23 / /proc rw - proc proc rw\n",
        "42 1 0:22 / /proc rw - proc proc rw\n42 1 0:22 / /proc rw - proc proc rw\n",
    ] {
        assert!(process::proc_root_mount_descriptor(bad.as_bytes(), 42, dev).is_err());
    }
}
#[test]
fn actual_original_pid_guard_rejects_fork_inheritor_in_fresh_ordinary_process() {
    assert_ne!(unsafe { libc::geteuid() }, 0);
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "snapshot_broker::b1_peer::tests::ordinary_pid_fork_child_entry",
            "--ignored",
            "--test-threads=1",
        ])
        .env_clear()
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(
        std::str::from_utf8(&result.stdout)
            .unwrap()
            .contains("1 passed; 0 failed")
    );
}
#[test]
#[ignore = "ordinary UID only; exact primitive wrapper, no qualified context"]
fn ordinary_pid_fork_child_entry() {
    assert_ne!(unsafe { libc::geteuid() }, 0);
    let scan = SharedJournalBudget::default();
    let original = std::process::id();
    assert_eq!(process::require_same_current_pid(original, &scan), Ok(()));
    let child = unsafe { libc::fork() };
    assert!(child >= 0);
    if child == 0 {
        // This child inherits only a DATA PID and an unqualified accounting
        // primitive. No factory/opaque process is constructed or transplanted.
        let rejected = process::require_same_current_pid(original, &scan) == Err(Error::Identity);
        unsafe { libc::_exit(if rejected { 0 } else { 1 }) };
    }
    let mut status = 0;
    loop {
        let result = unsafe { libc::waitpid(child, &mut status, 0) };
        if result == child {
            break;
        }
        assert!(
            result < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
        );
    }
    assert!(libc::WIFEXITED(status));
    assert_eq!(libc::WEXITSTATUS(status), 0);
    assert_eq!(process::require_same_current_pid(original, &scan), Ok(()));
}

fn proc_sys_mounts(extra: &str) -> Vec<u8> {
    format!("42 1 0:22 / /proc rw,nosuid,nodev,noexec - proc proc rw\n{extra}").into_bytes()
}
#[test]
fn proc_sys_shape_data_selects_direct_or_exact_readonly_native_subtree_only() {
    let dev = libc::makedev(0, 22);
    let direct = process::parse_proc_sys_shape(&proc_sys_mounts(""), 42, dev).unwrap();
    assert!(direct.direct_data());
    assert_eq!(direct.native_ro_mount_data(), None);
    let row = "43 42 0:22 /sys /proc/sys ro,nosuid,nodev,noexec,relatime - proc proc rw\n";
    let native = process::parse_proc_sys_shape(&proc_sys_mounts(row), 42, dev).unwrap();
    assert!(!native.direct_data());
    assert_eq!(native.native_ro_mount_data(), Some(43));
    let sibling = "44 42 0:22 /irq /proc/irq ro,nosuid,nodev,noexec - proc proc rw\n";
    assert!(
        process::parse_proc_sys_shape(&proc_sys_mounts(sibling), 42, dev)
            .unwrap()
            .direct_data()
    );
}
#[test]
fn proc_sys_shape_data_rejects_wrong_root_parent_device_flags_stack_and_children() {
    let dev = libc::makedev(0, 22);
    let row = "43 42 0:22 /sys /proc/sys ro,nosuid,nodev,noexec - proc proc rw\n";
    for bad in [
        row.replace("/sys /proc", "/other /proc"),
        row.replace("43 42", "43 41"),
        row.replace("0:22 /sys", "0:23 /sys"),
        row.replace("43 42", "42 42"),
        row.replace("ro,nosuid", "rw,nosuid"),
        row.replace("ro,nosuid", "ro,rw,nosuid"),
        row.replace("ro,nosuid", "ro,ro,nosuid"),
        row.replace(",noexec", ""),
        row.replace(" - proc proc ", " - tmpfs tmpfs "),
        row.replace("/proc/sys ", "/proc/sys\\040alias "),
        format!("{row}{row}"),
        format!(
            "{row}44 43 0:22 /sys/kernel /proc/sys/kernel ro,nosuid,nodev,noexec - proc proc rw\n"
        ),
        format!(
            "{row}44 43 0:22 /sys/kernel/random/boot_id /proc/sys/kernel/random/boot_id ro,nosuid,nodev,noexec - proc proc rw\n"
        ),
    ] {
        assert!(process::parse_proc_sys_shape(&proc_sys_mounts(&bad), 42, dev).is_err());
    }
    assert!(process::parse_proc_sys_shape(&proc_sys_mounts(row)[..20], 42, dev).is_err());
    assert!(process::parse_proc_sys_shape(&[b' '; 1024 * 1024 + 1], 42, dev).is_err());
}
#[test]
fn capture_self_native_component_reads_boot_id_without_pg_namespace_or_qualification() {
    assert_ne!(unsafe { libc::geteuid() }, 0);
    let scan = SharedJournalBudget::default();
    // The real ordinary process owns only this component's actual self userns
    // and kernel boot view. It is not a qualified capture/factory/PG context.
    // There is no installed PG or M4 producer in this ordinary component test.
    let native = process::SupportedNativeView::observe_capture(&scan).unwrap();
    let first = native.boot_id(&scan).unwrap();
    assert!(job::uuid(&first, false));
    native.revalidate(&scan).unwrap();
    assert_eq!(native.boot_id(&scan).unwrap(), first);
    assert!(scan.usage().unwrap().charged_bytes > 0);
}

// New C11 cases exercise closed DATA and phase correlation only. No fixture
// constructs a process/session/installed binding or asserts Root observation.
fn c11_waiting_for_ack() -> ControlState {
    let mut state = ControlState::new(ProcessRole::Capture);
    for (kind, sequence, direction) in [
        ("CaptureBootstrap", 0, Direction::Receive),
        ("CaptureInit", 0, Direction::Send),
        ("CaptureStartBound", 1, Direction::Receive),
        ("RequestInstallProof", 1, Direction::Send),
        ("InstallProofBound", 2, Direction::Receive),
        ("BrokerInstalled", 2, Direction::Send),
    ] {
        observe(&mut state, kind, sequence, direction);
    }
    state
}
#[test]
fn c11_installed_and_observed_require_exact_new_data_fields() {
    for kind in ["BrokerInstalled", "BrokerInstallObserved"] {
        for field in ["binding_id", "epoch_id", "descriptor_sha256"] {
            let mut v = value(kind, 0);
            v.as_object_mut().unwrap().remove(field);
            assert!(job::parse::<ControlData>(&job::canonical(&v).unwrap(), 4096).is_err());
        }
        for (field, bad) in [
            (
                "binding_id",
                "44444444-4444-1444-8444-444444444444".to_owned(),
            ),
            (
                "epoch_id",
                "55555555-5555-4555-A555-555555555555".to_owned(),
            ),
            ("descriptor_sha256", "A".repeat(64)),
        ] {
            let mut v = value(kind, 0);
            v[field] = bad.into();
            assert!(job::parse::<ControlData>(&job::canonical(&v).unwrap(), 4096).is_err());
        }
    }
    let mut v = value("BrokerInstallObserved", 0);
    v.as_object_mut().unwrap().remove("observation_sha256");
    assert!(job::parse::<ControlData>(&job::canonical(&v).unwrap(), 4096).is_err());
}
#[test]
fn c11_ack_is_required_correlated_once_and_failure_stays_poisoned() {
    let mut missing = c11_waiting_for_ack();
    assert_eq!(
        missing.observe(&data("RequestCaptureProof", 3), Direction::Send),
        Err(Error::Phase)
    );
    for field in [
        "job_id",
        "root_cookie",
        "backup_id",
        "request_id",
        "binding_id",
        "epoch_id",
        "descriptor_sha256",
    ] {
        let mut state = c11_waiting_for_ack();
        let mut v = value("BrokerInstallObserved", 3);
        v[field] = if field.ends_with("sha256") || field == "root_cookie" {
            "8".repeat(64).into()
        } else {
            "88888888-8888-4888-8888-888888888888".into()
        };
        let changed: ControlData = job::parse(&job::canonical(&v).unwrap(), 4096).unwrap();
        assert_eq!(
            state.observe(&changed, Direction::Receive),
            Err(Error::Identity)
        );
        assert_eq!(
            state.observe(&data("BrokerInstallObserved", 3), Direction::Receive),
            Err(Error::Poisoned)
        );
    }
    let mut replay = c11_waiting_for_ack();
    observe(&mut replay, "BrokerInstallObserved", 3, Direction::Receive);
    assert!(
        replay
            .observe(&data("BrokerInstallObserved", 4), Direction::Receive)
            .is_err()
    );
    assert!(replay.is_failed());
}
fn c11_observation() -> serde_json::Value {
    serde_json::json!({
        "schema_version":1,"capability":"b1_install_cutover_observation_v1",
        "job_id":JOB,"backup_id":BACKUP,"phase":"Install","request_id":REQUEST,
        "binding_id":"44444444-4444-4444-8444-444444444444",
        "epoch_id":"55555555-5555-4555-8555-555555555555",
        "generation_binding_sha256":"1".repeat(64),"descriptor_sha256":"2".repeat(64),
        "epoch_header_sha256":"3".repeat(64),"genesis_frame_sha256":"4".repeat(64),
        "observed_unix_ms":1,
        "host_cutover":{
            "container_id":"5".repeat(64),"image_sha256":format!("sha256:{}","6".repeat(64)),
            "export_mount":"/host/export","journal_mount":"/host/journal",
            "assets_mount":{"Type":"bind","Source":"/host/assets","Destination":"/assets","RW":false},
            "legacy_producers":[],
            "complete_container_projection":[{
                "container_id":"5".repeat(64),"image_sha256":format!("sha256:{}","6".repeat(64)),
                "running":true,"process_id":1,"started_at":"actual-data-only","restart_count":0,
                "project":null,"service":null,
                "mounts":[{"destination":"/assets","source":"/host/assets","kind":"bind","rw":false,"name":null}]
            }],
            "daemon_id":"daemon-data-only","mountinfo_sha256":"7".repeat(64),"docker_cli_sha256":"8".repeat(64)
        }
    })
}
#[test]
fn c11_assets_round_trip_preserves_missing_null_strings_and_rejects_unknown() {
    for optional in [
        None,
        Some(serde_json::Value::Null),
        Some(serde_json::json!("")),
        Some(serde_json::json!("rprivate")),
    ] {
        let mut v = c11_observation();
        if let Some(optional) = optional {
            v["host_cutover"]["assets_mount"]["Propagation"] = optional;
        }
        let raw = job::canonical(&v).unwrap();
        let parsed: job::InstallCutoverObservation = job::parse(&raw, 1024 * 1024).unwrap();
        assert_eq!(job::canonical(&parsed).unwrap(), raw);
    }
    for path in [
        vec!["host_cutover", "unchecked"],
        vec!["host_cutover", "assets_mount", "UncheckedDockerField"],
    ] {
        let mut v = c11_observation();
        let mut nested = &mut v;
        for key in &path[..path.len() - 1] {
            nested = &mut nested[*key];
        }
        nested[path[path.len() - 1]] = true.into();
        assert!(
            job::parse::<job::InstallCutoverObservation>(&job::canonical(&v).unwrap(), 1024 * 1024)
                .is_err()
        );
    }
}
#[test]
fn c11_record_rejects_missing_nullable_noncanonical_duplicate_and_bounds() {
    for field in ["project", "service"] {
        let mut v = c11_observation();
        v["host_cutover"]["complete_container_projection"][0]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(
            job::parse::<job::InstallCutoverObservation>(&job::canonical(&v).unwrap(), 1024 * 1024)
                .is_err()
        );
    }
    let mut v = c11_observation();
    v["host_cutover"]["complete_container_projection"][0]["mounts"][0]
        .as_object_mut()
        .unwrap()
        .remove("name");
    assert!(
        job::parse::<job::InstallCutoverObservation>(&job::canonical(&v).unwrap(), 1024 * 1024)
            .is_err()
    );
    let raw = job::canonical(&c11_observation()).unwrap();
    let text = String::from_utf8(raw.clone()).unwrap();
    let duplicate = text.replacen(
        "\"observed_unix_ms\":1",
        "\"observed_unix_ms\":1,\"observed_unix_ms\":1",
        1,
    );
    assert!(
        job::parse::<job::InstallCutoverObservation>(duplicate.as_bytes(), 1024 * 1024).is_err()
    );
    let mut newline = raw;
    newline.push(b'\n');
    assert!(job::parse::<job::InstallCutoverObservation>(&newline, 1024 * 1024).is_err());
    let mut v = c11_observation();
    v["phase"] = "Capture".into();
    assert!(
        job::parse::<job::InstallCutoverObservation>(&job::canonical(&v).unwrap(), 1024 * 1024)
            .is_err()
    );
    let mut v = c11_observation();
    v["host_cutover"]["daemon_id"] = "x".repeat(129).into();
    assert!(
        job::parse::<job::InstallCutoverObservation>(&job::canonical(&v).unwrap(), 1024 * 1024)
            .is_err()
    );
}
#[test]
fn c11_genesis_data_hashes_original_payload_header_and_complete_frame_separately() {
    use sha2::{Digest, Sha256};
    let binding = uuid::Uuid::parse_str("44444444-4444-4444-8444-444444444444").unwrap();
    let epoch = uuid::Uuid::parse_str("55555555-5555-4555-8555-555555555555").unwrap();
    let descriptor = br#"{"descriptor":"exact original bytes"}"#;
    let mut header = b"KWSJEPO2".to_vec();
    header.extend_from_slice(epoch.as_bytes());
    header.extend_from_slice(binding.as_bytes());
    let hd: [u8; 32] = Sha256::digest(&header).into();
    let dd: [u8; 32] = Sha256::digest(descriptor).into();
    let mut frame = b"KWSJBYT2".to_vec();
    frame.extend_from_slice(&1u64.to_be_bytes());
    frame.extend_from_slice(epoch.as_bytes());
    frame.extend_from_slice(binding.as_bytes());
    frame.extend_from_slice(&hd);
    frame.extend_from_slice(&dd);
    frame.extend_from_slice(&(descriptor.len() as u64).to_be_bytes());
    frame.extend_from_slice(descriptor);
    let fd: [u8; 32] = Sha256::digest(&frame).into();
    assert_eq!(
        process::install_genesis_data(&header, &frame, descriptor, (binding, epoch)).unwrap(),
        (dd, hd, fd)
    );
    assert_ne!(dd, hd);
    assert_ne!(dd, fd);
    for offset in [0usize, 8, 16, 32, 48, 80, 112, 120] {
        let mut changed = frame.clone();
        changed[offset] ^= 1;
        assert!(
            process::install_genesis_data(&header, &changed, descriptor, (binding, epoch)).is_err()
        );
    }
    assert!(
        process::install_genesis_data(&header, &frame, b"reencoded", (binding, epoch)).is_err()
    );
    assert!(
        process::install_genesis_data(&header, &frame[..119], descriptor, (binding, epoch))
            .is_err()
    );
}

//! Endpoint tests distinguish structural unit coverage from real PG evidence.
mod startup_failure {
    use super::diagnostic::Fault;
    use crate::restore_preflight::target_binding::child_attestation::ChildFailure;
    use std::{io, process::ExitStatus, sync::Mutex};

    #[derive(Clone, Copy, serde::Serialize)]
    pub(super) enum Phase {
        #[serde(rename = "executable_mismatch")]
        Executable,
        #[serde(rename = "arguments_environment_mismatch")]
        ArgumentsEnvironment,
    }

    #[derive(Default, serde::Serialize)]
    #[serde(rename_all = "snake_case")]
    enum Status {
        #[default]
        NotReached,
        NotReturned,
        Completed,
        NoStatusReturned,
        Error,
    }

    #[derive(Default)]
    pub(super) struct Evidence {
        attempted: bool,
        emitted: bool,
        phase: Option<Phase>,
        status: Status,
        status_present: bool,
        exit_code: Option<i32>,
        signal: Option<i32>,
    }

    impl Evidence {
        pub(super) fn after_strict_result(
            state: &Mutex<Self>,
            active: bool,
            fault: Fault,
            phase: Phase,
            result: Result<(), ChildFailure>,
            observe: impl FnOnce() -> io::Result<Option<ExitStatus>>,
        ) -> Result<(), ChildFailure> {
            if !active || fault != Fault::None || result != Err(ChildFailure::Identity) {
                return result;
            }
            {
                let mut evidence = state.lock().unwrap();
                if evidence.attempted {
                    return result;
                }
                // Consume BEFORE the native call, including None/error/unwind.
                evidence.attempted = true;
                evidence.phase = Some(phase);
                evidence.status = Status::NotReturned;
            }
            // This is a potential owned-child REAP, not a read-only getter.
            // Strict rejection is already decided; no outcome can authorize it.
            let observed = observe();
            let mut evidence = state.lock().unwrap();
            match observed {
                Ok(Some(status)) => {
                    evidence.status = Status::Completed;
                    evidence.status_present = true;
                    evidence.exit_code = status.code();
                    #[cfg(unix)]
                    {
                        use std::os::unix::process::ExitStatusExt;
                        evidence.signal = status.signal();
                    }
                }
                Ok(None) => evidence.status = Status::NoStatusReturned,
                Err(_) => evidence.status = Status::Error,
            }
            result
        }

        pub(super) fn failed_body<T>(
            state: &Mutex<Self>,
            result: &Result<T, crate::BackupError>,
            emit: impl FnOnce(&str),
        ) {
            if result.is_ok() {
                return;
            }
            let line = {
                let mut evidence = state.lock().unwrap();
                if evidence.emitted {
                    return;
                }
                evidence.emitted = true;
                let raw = serde_json::json!({
                    "format_version": 1, "state": "DIAGNOSTIC_NOT_AUTHORITY",
                    "attempted": evidence.attempted, "phase": evidence.phase,
                    "status": evidence.status, "status_present": evidence.status_present,
                    "exit_code": evidence.exit_code, "signal": evidence.signal,
                });
                // Only finite labels/bools and optional i32s enter this line.
                format!("SOURCE_STARTUP_PRE_CLEANUP_DIAGNOSTIC_V1={raw}")
            };
            emit(&line);
        }
    }

    mod tests {
        use super::*;
        use std::cell::{Cell, RefCell};
        const PREFIX: &str = "SOURCE_STARTUP_PRE_CLEANUP_DIAGNOSTIC_V1=";

        fn status(code: i32) -> ExitStatus {
            #[cfg(windows)]
            {
                use std::os::windows::process::ExitStatusExt;
                ExitStatus::from_raw(code as u32)
            }
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                ExitStatus::from_raw(code << 8)
            }
        }

        fn marker(state: &Mutex<Evidence>) -> serde_json::Value {
            let failed: Result<(), _> = Err(crate::BackupError::Invalid("RAW_SECRET_DSN"));
            let mut lines = Vec::new();
            Evidence::failed_body(state, &failed, |line| lines.push(line.to_owned()));
            Evidence::failed_body(state, &failed, |_| panic!("duplicate marker"));
            assert_eq!(lines.len(), 1);
            let line = &lines[0];
            assert!(line.len() < 512);
            assert!(!line.contains("RAW_SECRET_DSN"));
            assert!(!line.contains('\n'));
            let value: serde_json::Value =
                serde_json::from_str(line.strip_prefix(PREFIX).unwrap()).unwrap();
            assert_eq!(value["state"], "DIAGNOSTIC_NOT_AUTHORITY");
            assert_eq!(value["format_version"], 1);
            assert_eq!(
                value
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                [
                    "attempted",
                    "exit_code",
                    "format_version",
                    "phase",
                    "signal",
                    "state",
                    "status",
                    "status_present"
                ]
            );
            value
        }

        #[test]
        fn strict_rejection_precedes_once_observation_without_holding_mutex() {
            let state = Mutex::new(Evidence::default());
            let events = RefCell::new(Vec::new());
            events.borrow_mut().push("strict_identity");
            let result = Evidence::after_strict_result(
                &state,
                true,
                Fault::None,
                Phase::ArgumentsEnvironment,
                Err(ChildFailure::Identity),
                || {
                    assert!(state.try_lock().is_ok(), "native call held Trace mutex");
                    events.borrow_mut().push("try_wait");
                    assert_eq!(
                        Evidence::after_strict_result(
                            &state,
                            true,
                            Fault::None,
                            Phase::Executable,
                            Err(ChildFailure::Identity),
                            || panic!("reentered native observation")
                        ),
                        Err(ChildFailure::Identity)
                    );
                    Ok(None)
                },
            );
            events.borrow_mut().push("cleanup");
            assert_eq!(result, Err(ChildFailure::Identity));
            assert_eq!(*events.borrow(), ["strict_identity", "try_wait", "cleanup"]);
            let value = marker(&state);
            assert_eq!(value["status"], "no_status_returned");
            assert_eq!(value["phase"], "arguments_environment_mismatch");
            assert_eq!(value["attempted"], true);
            assert_eq!(value["status_present"], false);
        }

        #[test]
        fn every_native_result_keeps_identity_and_consumes_the_attempt() {
            for (native, label, code) in [
                (Ok(Some(status(0))), "completed", Some(0)),
                (Ok(Some(status(2))), "completed", Some(2)),
                (Ok(None), "no_status_returned", None),
                (Err(io::Error::other("RAW_SECRET_DSN")), "error", None),
            ] {
                let state = Mutex::new(Evidence::default());
                let calls = Cell::new(0);
                assert_eq!(
                    Evidence::after_strict_result(
                        &state,
                        true,
                        Fault::None,
                        Phase::Executable,
                        Err(ChildFailure::Identity),
                        || {
                            calls.set(calls.get() + 1);
                            native
                        }
                    ),
                    Err(ChildFailure::Identity)
                );
                assert_eq!(
                    Evidence::after_strict_result(
                        &state,
                        true,
                        Fault::None,
                        Phase::Executable,
                        Err(ChildFailure::Identity),
                        || panic!("second native observation")
                    ),
                    Err(ChildFailure::Identity)
                );
                assert_eq!(calls.get(), 1);
                let value = marker(&state);
                assert_eq!(value["status"], label);
                assert_eq!(value["exit_code"], serde_json::json!(code));
                assert_eq!(value["status_present"], code.is_some());
                assert!(value["signal"].is_null());
            }
        }

        #[test]
        fn success_inactive_io_and_other_faults_never_observe() {
            for (active, fault, result) in [
                (true, Fault::None, Ok(())),
                (false, Fault::None, Err(ChildFailure::Identity)),
                (true, Fault::None, Err(ChildFailure::Io)),
                (true, Fault::Timeout, Err(ChildFailure::Identity)),
                (true, Fault::Cancel, Err(ChildFailure::Identity)),
                (true, Fault::Overflow, Err(ChildFailure::Identity)),
                (true, Fault::Exit, Err(ChildFailure::Identity)),
                (true, Fault::LockLoss, Err(ChildFailure::Identity)),
            ] {
                let state = Mutex::new(Evidence::default());
                assert_eq!(
                    Evidence::after_strict_result(
                        &state,
                        active,
                        fault,
                        Phase::Executable,
                        result,
                        || panic!("excluded path observed")
                    ),
                    result
                );
                let value = marker(&state);
                assert_eq!(value["attempted"], false);
                assert_eq!(value["status"], "not_reached");
                assert!(value["phase"].is_null());
            }
        }

        #[test]
        fn successful_body_never_emits_and_failure_emits_once_before_following_work() {
            let state = Mutex::new(Evidence::default());
            Evidence::failed_body(&state, &Ok::<_, crate::BackupError>(()), |_| {
                panic!("success marker")
            });
            let events = RefCell::new(vec!["prepare_err"]);
            let result: Result<(), _> = Err(crate::BackupError::Invalid("RAW_SECRET_DSN"));
            Evidence::failed_body(&state, &result, |_| events.borrow_mut().push("marker"));
            events.borrow_mut().push("v2_publication");
            events.borrow_mut().push("original_unwrap");
            assert_eq!(
                *events.borrow(),
                ["prepare_err", "marker", "v2_publication", "original_unwrap"]
            );
            assert!(result.is_err());
        }

        #[cfg(unix)]
        #[test]
        fn signaled_projection_has_no_exit_code_or_cause_inference() {
            use std::os::unix::process::ExitStatusExt;
            let state = Mutex::new(Evidence::default());
            assert_eq!(
                Evidence::after_strict_result(
                    &state,
                    true,
                    Fault::None,
                    Phase::Executable,
                    Err(ChildFailure::Identity),
                    || Ok(Some(ExitStatus::from_raw(9)))
                ),
                Err(ChildFailure::Identity)
            );
            let value = marker(&state);
            assert_eq!(value["status"], "completed");
            assert_eq!(value["signal"], 9);
            assert_eq!(value["status_present"], true);
            assert!(value["exit_code"].is_null());
        }

        #[test]
        fn full_i32_projection_is_bounded_and_unwind_cannot_retry() {
            let state = Mutex::new(Evidence::default());
            let unwound = std::panic::catch_unwind(|| {
                Evidence::after_strict_result(
                    &state,
                    true,
                    Fault::None,
                    Phase::ArgumentsEnvironment,
                    Err(ChildFailure::Identity),
                    || panic!("native seam unwind"),
                )
            });
            assert!(unwound.is_err());
            assert_eq!(
                Evidence::after_strict_result(
                    &state,
                    true,
                    Fault::None,
                    Phase::Executable,
                    Err(ChildFailure::Identity),
                    || panic!("retry after unwind")
                ),
                Err(ChildFailure::Identity)
            );
            assert_eq!(marker(&state)["status"], "not_returned");
            // Projection-only boundary fixtures; these are not native outcomes.
            for code in [i32::MIN, i32::MAX] {
                let state = Mutex::new(Evidence {
                    attempted: true,
                    phase: Some(Phase::ArgumentsEnvironment),
                    status: Status::Completed,
                    status_present: true,
                    exit_code: Some(code),
                    signal: Some(code),
                    ..Evidence::default()
                });
                let value = marker(&state);
                assert_eq!(value["exit_code"], code);
                assert_eq!(value["signal"], code);
            }
        }

        #[cfg(target_os = "linux")]
        #[tokio::test]
        async fn cached_reap_still_allows_original_cleanup_kill_and_wait() {
            use std::process::Stdio;
            let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "restore_preflight::target_binding::child_attestation::bounded_process::tests::process_fixture", "--nocapture"])
                .env_clear().env("C4_CHILD_UNIT_FIXTURE", "secret")
                .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
                .kill_on_drop(true).spawn().unwrap();
            // Deterministic fixture completion populates Tokio's cached status.
            // This characterizes cached reap compatibility, not live proc timing.
            let original = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(original.code(), Some(2));
            let state = Mutex::new(Evidence::default());
            assert_eq!(
                Evidence::after_strict_result(
                    &state,
                    true,
                    Fault::None,
                    Phase::Executable,
                    Err(ChildFailure::Identity),
                    || child.try_wait()
                ),
                Err(ChildFailure::Identity)
            );
            let _ = child.start_kill();
            assert_eq!(child.wait().await.unwrap(), original);
            let value = marker(&state);
            assert_eq!(value["status"], "completed");
            assert_eq!(value["exit_code"], 2);
        }
    }
}

pub(in crate::source) mod diagnostic {
    use crate::BackupError;
    use crate::restore_preflight::target_binding::child_attestation::ChildFailure;
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(super) enum Fault {
        None,
        Timeout,
        Cancel,
        Overflow,
        Exit,
        LockLoss,
    }
    impl Fault {
        pub(super) fn pauses_postmaster(self) -> bool {
            matches!(
                self,
                Self::Timeout | Self::Cancel | Self::Overflow | Self::Exit | Self::LockLoss
            )
        }
    }
    #[test]
    fn finite_pause_projection_includes_overflow_and_excludes_normal_capture() {
        for fault in [
            Fault::Timeout,
            Fault::Cancel,
            Fault::Overflow,
            Fault::Exit,
            Fault::LockLoss,
        ] {
            assert!(fault.pauses_postmaster());
        }
        assert!(!Fault::None.pauses_postmaster());
    }
    pub(super) fn early_return<T>(
        snapshot: &Snapshot,
        fault: Fault,
        backup: uuid::Uuid,
        result: &Result<T, BackupError>,
    ) -> ! {
        // This Result and Snapshot are already owned by the failing branch.
        // Neither the returned value nor an error payload enters the projection.
        let projection = (|| {
            let inner: serde_json::Value =
                serde_json::from_slice(&snapshot.bytes(backup, result.is_ok(), None)?)?;
            let fault = match fault {
                Fault::None => "none",
                Fault::Timeout => "timeout",
                Fault::Cancel => "cancel",
                Fault::Overflow => "overflow",
                Fault::Exit => "exit",
                Fault::LockLoss => "lock_loss",
            };
            let prepare_error = match result {
                Ok(_) => "unexpected_ok",
                Err(BackupError::Invalid(message)) => match *message {
                    "source dump failed; gate remains closed" => "dump_failed",
                    "source dump version failed" => "dump_version_failed",
                    "source dump version differs" => "dump_version_differs",
                    "source fault proof missing" => "fault_proof_missing",
                    "source fault nonce" => "fault_nonce",
                    "source fault ack size" => "fault_ack_size",
                    "source fault exact ack" => "fault_exact_ack",
                    "source fault ack scope" => "fault_ack_scope",
                    "source fault postmaster" => "fault_postmaster",
                    "source fault actual state unconfirmed" => "fault_actual_state_unconfirmed",
                    "source fault ack deadline" => "fault_ack_deadline",
                    "source fault pause missing" => "fault_pause_missing",
                    _ => "other_invalid",
                },
                Err(BackupError::Overflow) => "overflow",
                Err(BackupError::Capacity(_)) => "capacity",
                Err(BackupError::Database(_)) => "database",
                Err(BackupError::Json(_)) => "json",
                Err(BackupError::Io(_)) => "io",
                Err(BackupError::Asset(_)) => "asset",
            };
            Ok(serde_json::to_vec(&serde_json::json!({
                "format_version":1,"capability":"source_fault_early_return_diagnostic_v1",
                "state":"DIAGNOSTIC_NOT_AUTHORITY","fault":fault,
                "prepare_error":prepare_error,"snapshot":inner
            }))?)
        })();
        fail_early_return(projection)
    }
    fn fail_early_return(projection: Result<Vec<u8>, BackupError>) -> ! {
        let text = projection
            .and_then(bounded)
            .ok()
            .and_then(|raw| String::from_utf8(raw).ok());
        // Projection failure must neither expose its error nor suppress failure.
        panic!(
            "source fault did not reach the real fixed pg_dump SOURCE_FAULT_EARLY_RETURN_DIAGNOSTIC={}",
            text.as_deref().unwrap_or("unavailable")
        )
    }
    #[derive(Clone, Copy)]
    pub(super) enum ProcRead {
        ChildId,
        Executable,
        CommandLine,
        Environment,
        StartTicks,
    }
    #[derive(Clone, serde::Serialize)]
    #[serde(rename_all = "snake_case")]
    enum ShapeClass {
        Exact,
        Empty,
        MissingFinalNulOnly,
        ExtraFinalNulsOnly,
        EntryOrderOnly,
        OtherMismatch,
        OverObservationBound,
    }
    #[derive(Clone, serde::Serialize)]
    struct BufferShape {
        bytes: usize,
        nul_count: Option<usize>,
        trailing_nul_count: Option<usize>,
        expected_presence_mask: Option<u8>,
        classification: ShapeClass,
    }
    impl BufferShape {
        fn observe<const N: usize>(actual: &[u8], expected: &[u8]) -> Self {
            // This bound limits diagnostics only, never native read/capture decisions.
            if actual.len() > 4096 {
                return Self {
                    bytes: 4097,
                    nul_count: None,
                    trailing_nul_count: None,
                    expected_presence_mask: None,
                    classification: ShapeClass::OverObservationBound,
                };
            }
            let nuls = actual.iter().filter(|byte| **byte == 0).count();
            let trailing = actual.iter().rev().take_while(|byte| **byte == 0).count();
            // Borrow only the known eight argv or two environment fields; no
            // external strings, sorted tokens or raw values survive this call.
            let mut parts = expected.split(|byte| *byte == 0);
            let expected_fields: [&[u8]; N] =
                std::array::from_fn(|_| parts.next().unwrap_or_default());
            let expected_well_formed = expected_fields.iter().all(|field| !field.is_empty())
                && parts.next() == Some(&b""[..])
                && parts.next().is_none();
            let mut presence = 0_u8;
            let mut matched = [false; N];
            let mut entry_count = 0;
            let mut empty_entry = false;
            for field in actual
                .strip_suffix(b"\0")
                .unwrap_or(actual)
                .split(|byte| *byte == 0)
            {
                entry_count += 1;
                empty_entry |= field.is_empty();
                for (index, expected_field) in expected_fields.iter().enumerate() {
                    if !field.is_empty() && field == *expected_field {
                        presence |= 1 << index;
                    }
                }
                if let Some(index) = expected_fields
                    .iter()
                    .enumerate()
                    .position(|(index, expected_field)| !matched[index] && field == *expected_field)
                {
                    matched[index] = true;
                }
            }
            let expected_unterminated = expected.strip_suffix(b"\0");
            let classification = if actual == expected {
                ShapeClass::Exact
            } else if actual.is_empty() {
                ShapeClass::Empty
            } else if expected_unterminated == Some(actual) {
                ShapeClass::MissingFinalNulOnly
            } else if trailing > 1
                && expected_unterminated == Some(&actual[..actual.len() - trailing])
            {
                ShapeClass::ExtraFinalNulsOnly
            } else if expected_well_formed
                && trailing == 1
                && !empty_entry
                && entry_count == N
                && matched.iter().all(|value| *value)
            {
                ShapeClass::EntryOrderOnly
            } else {
                ShapeClass::OtherMismatch
            };
            Self {
                bytes: actual.len(),
                nul_count: Some(nuls),
                trailing_nul_count: Some(trailing),
                expected_presence_mask: Some(presence),
                classification,
            }
        }
    }
    #[derive(Clone, Default)]
    pub(in crate::source) struct Snapshot {
        pub(super) callback_entered: bool,
        pub(super) callback_completed: bool,
        pub(super) executable_matches: Option<bool>,
        pub(super) argv_matches: Option<bool>,
        pub(super) environment_matches: Option<bool>,
        argv_shape: Option<BufferShape>,
        environment_shape: Option<BufferShape>,
        pub(super) failed_proc_read: Option<ProcRead>,
        pub(super) pid: Option<u32>,
        pub(super) start_ticks: Option<u64>,
        pub(super) backend: Option<i32>,
        pub(super) oid: Option<u64>,
        pub(super) epoch: Option<uuid::Uuid>,
        first_cleanup: Option<ChildFailure>,
        last_cleanup: Option<ChildFailure>,
        final_stream: Option<ChildFailure>,
        stream_result: Option<bool>,
        pub(super) dropped: Option<bool>,
        pub(super) pid_start_unobserved: Option<bool>,
    }
    impl Snapshot {
        pub(super) fn cleanup(&mut self, cause: ChildFailure) {
            self.first_cleanup.get_or_insert(cause);
            self.last_cleanup = Some(cause);
        }
        pub(in crate::source) fn stream_result(&mut self, result: Result<(), ChildFailure>) {
            self.stream_result = Some(result.is_ok());
            self.final_stream = result.err();
        }
        pub(super) fn comparisons(&mut self, argv: &[u8], expected: &[u8], environment: &[u8]) {
            self.argv_matches = Some(argv == expected);
            self.environment_matches = Some(environment == b"LC_ALL=C\0PGCONNECT_TIMEOUT=5\0");
            self.argv_shape = Some(BufferShape::observe::<8>(argv, expected));
            self.environment_shape = Some(BufferShape::observe::<2>(
                environment,
                b"LC_ALL=C\0PGCONNECT_TIMEOUT=5\0",
            ));
        }
        pub(super) fn bytes(
            &self,
            backup: uuid::Uuid,
            prepare_ok: bool,
            dump_size: Option<u64>,
        ) -> Result<Vec<u8>, BackupError> {
            // Only typed observations and fixed labels enter this projection.
            // Native wait/exit were never stored by the original Trace.
            bounded(serde_json::to_vec(&serde_json::json!({
                "format_version":2,"capability":"source_bound_dump_diagnostic_v2","state":"DIAGNOSTIC_NOT_AUTHORITY",
                "backup_id":backup,"source_epoch":self.epoch,"prepare_result":if prepare_ok{"ok"}else{"err"},"returned_pin":prepare_ok,"dump_metadata_size":dump_size,
                "spawn_callback_entered":self.callback_entered,"spawn_callback_completed":self.callback_completed,
                "exact_executable_matches":self.executable_matches,"exact_argv_matches":self.argv_matches,"exact_environment_matches":self.environment_matches,
                "argv_shape":self.argv_shape,"environment_shape":self.environment_shape,
                "failed_proc_read":self.failed_proc_read.map(proc_name),"observed_child_pid":self.pid,"observed_child_start_ticks":self.start_ticks,
                "admitted_backend":self.backend,"database_oid":self.oid,"first_cleanup_error":self.first_cleanup.map(error_name),"last_cleanup_error":self.last_cleanup.map(error_name),"final_stream_error":self.final_stream.map(error_name),
                "stream_result":self.stream_result.map(|ok|if ok{"ok"}else{"err"}),"test_control_dropped":self.dropped,"pid_start_no_longer_observed_after_drop":self.pid_start_unobserved,
                "native_wait":"not_observed","native_exit_code":Option::<i32>::None,"native_signal":Option::<i32>::None,"guard_stage":"not_observed"
            }))?)
        }
    }
    fn error_name(value: ChildFailure) -> &'static str {
        match value {
            ChildFailure::Session => "Session",
            ChildFailure::Identity => "Identity",
            ChildFailure::Protocol => "Protocol",
            ChildFailure::Version => "Version",
            ChildFailure::Deadline => "Deadline",
            ChildFailure::StdoutLimit => "StdoutLimit",
            ChildFailure::StderrLimit => "StderrLimit",
            ChildFailure::Exit => "Exit",
            ChildFailure::Stderr => "Stderr",
            ChildFailure::Io => "Io",
            ChildFailure::Unusable => "Unusable",
        }
    }
    fn proc_name(value: ProcRead) -> &'static str {
        match value {
            ProcRead::ChildId => "child_id",
            ProcRead::Executable => "executable",
            ProcRead::CommandLine => "command_line",
            ProcRead::Environment => "environment",
            ProcRead::StartTicks => "start_ticks",
        }
    }
    fn bounded(raw: Vec<u8>) -> Result<Vec<u8>, BackupError> {
        if raw.len() > 4096 {
            return Err(BackupError::Invalid("source diagnostic size"));
        }
        Ok(raw)
    }
    mod early_return_tests {
        use super::*;
        const MARKER: &str = " SOURCE_FAULT_EARLY_RETURN_DIAGNOSTIC=";
        const SENTINEL: &str = "SENTINEL_PASSWORD_DSN_RAW_ERROR_OR_PIN";

        fn panic_text(action: impl FnOnce()) -> String {
            let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(action))
                .err()
                .expect("early return must still fail");
            payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| {
                    payload
                        .downcast_ref::<&str>()
                        .map(|text| (*text).to_owned())
                })
                .expect("fixed panic text")
        }
        fn observation(text: &str) -> serde_json::Value {
            let (old, raw) = text.split_once(MARKER).expect("bounded diagnostic missing");
            assert_eq!(old, "source fault did not reach the real fixed pg_dump");
            assert_eq!(text.matches(MARKER).count(), 1);
            assert!(raw.len() <= 4096);
            assert!(!text.contains(SENTINEL), "raw payload was exposed");
            let value: serde_json::Value = serde_json::from_str(raw).unwrap();
            assert_eq!(serde_json::to_string(&value).unwrap(), raw);
            assert_eq!(
                value
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                [
                    "capability",
                    "fault",
                    "format_version",
                    "prepare_error",
                    "snapshot",
                    "state"
                ]
            );
            assert_eq!(value["format_version"], 1);
            assert_eq!(
                value["capability"],
                "source_fault_early_return_diagnostic_v1"
            );
            assert_eq!(value["state"], "DIAGNOSTIC_NOT_AUTHORITY");
            assert_eq!(value["snapshot"].as_object().unwrap().len(), 30);
            assert_eq!(value["snapshot"]["format_version"], 2);
            assert_eq!(
                value["snapshot"]["capability"],
                "source_bound_dump_diagnostic_v2"
            );
            assert!(value["snapshot"]["dump_metadata_size"].is_null());
            value
        }
        #[tokio::test]
        async fn early_return_completed_error_before_callback_is_identified_without_inference() {
            let result: Result<(), BackupError> =
                std::future::ready(Err(BackupError::Invalid("source fault ack deadline"))).await;
            let backup = uuid::Uuid::new_v4();
            let text =
                panic_text(|| early_return(&Snapshot::default(), Fault::Cancel, backup, &result));
            let value = observation(&text);
            assert_eq!(value["fault"], "cancel");
            assert_eq!(value["prepare_error"], "fault_ack_deadline");
            assert_eq!(value["snapshot"]["backup_id"], backup.to_string());
            assert_eq!(value["snapshot"]["prepare_result"], "err");
            assert_eq!(value["snapshot"]["spawn_callback_entered"], false);
            assert!(value["snapshot"]["stream_result"].is_null());
            assert!(value["snapshot"]["first_cleanup_error"].is_null());
        }
        #[test]
        fn early_return_callback_failure_preserves_first_final_and_unknown_wait() {
            let mut snapshot = Snapshot::default();
            snapshot.callback_entered = true;
            snapshot.comparisons(b"", b"a\0b\0c\0d\0e\0f\0g\0h\0", b"");
            snapshot.cleanup(ChildFailure::Identity);
            snapshot.cleanup(ChildFailure::Io);
            snapshot.stream_result(Err(ChildFailure::Session));
            snapshot.dropped = Some(true);
            snapshot.pid_start_unobserved = Some(true);
            let result: Result<(), _> = Err(BackupError::Invalid(
                "source dump failed; gate remains closed",
            ));
            let value = observation(&panic_text(|| {
                early_return(&snapshot, Fault::Overflow, uuid::Uuid::new_v4(), &result)
            }));
            assert_eq!(value["prepare_error"], "dump_failed");
            assert_eq!(value["fault"], "overflow");
            let inner = &value["snapshot"];
            assert_eq!(inner["first_cleanup_error"], "Identity");
            assert_eq!(inner["last_cleanup_error"], "Io");
            assert_eq!(inner["final_stream_error"], "Session");
            assert_eq!(inner["argv_shape"]["classification"], "empty");
            assert_eq!(inner["environment_shape"]["classification"], "empty");
            assert_eq!(inner["native_wait"], "not_observed");
            assert!(inner["native_exit_code"].is_null());
            assert!(inner["native_signal"].is_null());
            assert_eq!(inner["guard_stage"], "not_observed");
            assert_eq!(inner["pid_start_no_longer_observed_after_drop"], true);
        }
        #[test]
        fn early_return_all_fault_labels_and_unexpected_ok_still_fail_without_pin_payload() {
            for (fault, label) in [
                (Fault::None, "none"),
                (Fault::Timeout, "timeout"),
                (Fault::Cancel, "cancel"),
                (Fault::Overflow, "overflow"),
                (Fault::Exit, "exit"),
                (Fault::LockLoss, "lock_loss"),
            ] {
                // Deliberately no Debug or Serialize implementation on a returned value.
                struct PinLike {
                    _secret: &'static str,
                }
                let result = Ok(PinLike { _secret: SENTINEL });
                let value = observation(&panic_text(|| {
                    early_return(&Snapshot::default(), fault, uuid::Uuid::new_v4(), &result)
                }));
                assert_eq!(value["fault"], label);
                assert_eq!(value["prepare_error"], "unexpected_ok");
                assert_eq!(value["snapshot"]["prepare_result"], "ok");
            }
        }
        #[test]
        fn early_return_error_classification_never_formats_payloads() {
            let mut rows = vec![
                (BackupError::Invalid(SENTINEL), "other_invalid"),
                (BackupError::Capacity(SENTINEL), "capacity"),
                (BackupError::Overflow, "overflow"),
                (
                    BackupError::Database(sqlx::Error::Protocol(SENTINEL.to_owned())),
                    "database",
                ),
                (
                    BackupError::Json(
                        serde_json::from_str::<serde_json::Value>(SENTINEL).unwrap_err(),
                    ),
                    "json",
                ),
                (BackupError::Io(std::io::Error::other(SENTINEL)), "io"),
                (
                    BackupError::Asset(learning_assets::AssetIoError::Missing(SENTINEL.to_owned())),
                    "asset",
                ),
            ];
            for (message, label) in [
                ("source dump failed; gate remains closed", "dump_failed"),
                ("source dump version failed", "dump_version_failed"),
                ("source dump version differs", "dump_version_differs"),
                ("source fault proof missing", "fault_proof_missing"),
                ("source fault nonce", "fault_nonce"),
                ("source fault ack size", "fault_ack_size"),
                ("source fault exact ack", "fault_exact_ack"),
                ("source fault ack scope", "fault_ack_scope"),
                ("source fault postmaster", "fault_postmaster"),
                (
                    "source fault actual state unconfirmed",
                    "fault_actual_state_unconfirmed",
                ),
                ("source fault ack deadline", "fault_ack_deadline"),
                ("source fault pause missing", "fault_pause_missing"),
            ] {
                rows.push((BackupError::Invalid(message), label));
            }
            for (error, label) in rows {
                let result: Result<(), _> = Err(error);
                let value = observation(&panic_text(|| {
                    early_return(
                        &Snapshot::default(),
                        Fault::Timeout,
                        uuid::Uuid::new_v4(),
                        &result,
                    )
                }));
                assert_eq!(value["prepare_error"], label);
            }
        }
        #[test]
        fn early_return_projection_failure_or_oversize_remains_redacted_failure() {
            for projection in [Err(BackupError::Invalid(SENTINEL)), Ok(vec![b'x'; 4097])] {
                let text = panic_text(|| fail_early_return(projection));
                assert_eq!(
                    text,
                    "source fault did not reach the real fixed pg_dump SOURCE_FAULT_EARLY_RETURN_DIAGNOSTIC=unavailable"
                );
                assert!(!text.contains(SENTINEL));
            }
            let text = panic_text(|| fail_early_return(Ok(vec![b'x'; 4096])));
            let (_, raw) = text
                .split_once(MARKER)
                .expect("cap boundary must not truncate");
            assert_eq!(raw.len(), 4096);
        }
    }
    #[test]
    fn diagnostic_preserves_first_cleanup_separately_from_final_stream_failure() {
        let mut state = Snapshot::default();
        state.cleanup(ChildFailure::Identity);
        state.cleanup(ChildFailure::Io);
        state.stream_result(Err(ChildFailure::Session));
        assert_eq!(state.first_cleanup, Some(ChildFailure::Identity));
        assert_eq!(state.last_cleanup, Some(ChildFailure::Io));
        assert_eq!(state.final_stream, Some(ChildFailure::Session));
    }
    #[test]
    fn diagnostic_unknown_wait_and_exit_are_not_promoted_by_pid_disappearance() {
        let mut state = Snapshot::default();
        state.pid = Some(42);
        state.pid_start_unobserved = Some(true);
        state.dropped = Some(true);
        let value: serde_json::Value =
            serde_json::from_slice(&state.bytes(uuid::Uuid::new_v4(), false, Some(0)).unwrap())
                .unwrap();
        assert_eq!(value["native_wait"], "not_observed");
        assert!(value["native_exit_code"].is_null());
        assert!(value["native_signal"].is_null());
        assert_eq!(value["pid_start_no_longer_observed_after_drop"], true);
    }
    #[test]
    fn diagnostic_schema_is_exact_canonical_and_4096_byte_bounded() {
        let raw = Snapshot::default()
            .bytes(uuid::Uuid::new_v4(), false, None)
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        let keys = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            keys,
            std::collections::BTreeSet::from([
                "format_version",
                "capability",
                "state",
                "backup_id",
                "source_epoch",
                "prepare_result",
                "returned_pin",
                "dump_metadata_size",
                "spawn_callback_entered",
                "spawn_callback_completed",
                "exact_executable_matches",
                "exact_argv_matches",
                "exact_environment_matches",
                "argv_shape",
                "environment_shape",
                "failed_proc_read",
                "observed_child_pid",
                "observed_child_start_ticks",
                "admitted_backend",
                "database_oid",
                "first_cleanup_error",
                "last_cleanup_error",
                "final_stream_error",
                "stream_result",
                "test_control_dropped",
                "pid_start_no_longer_observed_after_drop",
                "native_wait",
                "native_exit_code",
                "native_signal",
                "guard_stage"
            ])
        );
        assert_eq!(value["format_version"], 2);
        assert_eq!(value["capability"], "source_bound_dump_diagnostic_v2");
        assert_eq!(value["state"], "DIAGNOSTIC_NOT_AUTHORITY");
        assert!(value["argv_shape"].is_null());
        assert!(value["environment_shape"].is_null());
        assert_eq!(serde_json::to_vec(&value).unwrap(), raw);
        assert!(raw.len() <= 4096);
        assert!(bounded(vec![b'x'; 4096]).is_ok());
        assert!(bounded(vec![b'x'; 4097]).is_err());
    }
    #[test]
    fn diagnostic_argv_shape_distinguishes_structure_without_accepting_nonexact_bytes() {
        let expected = b"a\0b\0c\0d\0e\0f\0g\0h\0";
        let rows: &[(&[u8], usize, usize, usize, u8, &str)] = &[
            (expected, 16, 8, 1, 255, "exact"),
            (b"", 0, 0, 0, 0, "empty"),
            (
                b"a\0b\0c\0d\0e\0f\0g\0h",
                15,
                7,
                0,
                255,
                "missing_final_nul_only",
            ),
            (
                b"a\0b\0c\0d\0e\0f\0g\0h\0\0\0",
                18,
                10,
                3,
                255,
                "extra_final_nuls_only",
            ),
            (
                b"h\0g\0f\0e\0d\0c\0b\0a\0",
                16,
                8,
                1,
                255,
                "entry_order_only",
            ),
            (b"a\0a\0c\0d\0e\0f\0g\0h\0", 16, 8, 1, 253, "other_mismatch"),
            (
                b"a\0b\0c\0d\0e\0f\0g\0h\0a\0",
                18,
                9,
                1,
                255,
                "other_mismatch",
            ),
            (b"a\0b\0c\0d\0e\0f\0g\0", 14, 7, 1, 127, "other_mismatch"),
            (
                b"a\0b\0c\0d\0e\0f\0g\0h\0x\0",
                18,
                9,
                1,
                255,
                "other_mismatch",
            ),
            (b"a\0b\0c\0d\0e\0f\0g\0x\0", 16, 8, 1, 127, "other_mismatch"),
            (b"h\0g\0f\0e\0d\0c\0b\0a", 15, 7, 0, 255, "other_mismatch"),
            (
                b"h\0g\0f\0e\0d\0c\0b\0a\0\0",
                17,
                9,
                2,
                255,
                "other_mismatch",
            ),
            (
                b"a\0b\0c\0\0d\0e\0f\0g\0h\0",
                17,
                9,
                1,
                255,
                "other_mismatch",
            ),
            (b"\0", 1, 1, 1, 0, "other_mismatch"),
        ];
        for (actual, bytes, nuls, trailing, presence, class) in rows {
            let mut state = Snapshot::default();
            state.comparisons(actual, expected, b"LC_ALL=C\0PGCONNECT_TIMEOUT=5\0");
            let raw = state.bytes(uuid::Uuid::new_v4(), false, None).unwrap();
            let value: serde_json::Value = serde_json::from_slice(&raw).unwrap();
            assert_eq!(
                value["argv_shape"],
                serde_json::json!({
                    "bytes":bytes,"nul_count":nuls,"trailing_nul_count":trailing,
                    "expected_presence_mask":presence,"classification":class
                })
            );
            assert_eq!(value["exact_argv_matches"], *class == "exact");
            assert_eq!(value["exact_environment_matches"], true);
            assert_eq!(serde_json::to_vec(&value).unwrap(), raw);
        }
    }
    #[test]
    fn diagnostic_environment_shape_requires_whole_fields_and_exact_multiplicity() {
        let rows: &[(&[u8], usize, usize, usize, u8, &str)] = &[
            (b"LC_ALL=C\0PGCONNECT_TIMEOUT=5\0", 29, 2, 1, 3, "exact"),
            (b"", 0, 0, 0, 0, "empty"),
            (
                b"LC_ALL=C\0PGCONNECT_TIMEOUT=5",
                28,
                1,
                0,
                3,
                "missing_final_nul_only",
            ),
            (
                b"LC_ALL=C\0PGCONNECT_TIMEOUT=5\0\0",
                30,
                3,
                2,
                3,
                "extra_final_nuls_only",
            ),
            (
                b"PGCONNECT_TIMEOUT=5\0LC_ALL=C\0",
                29,
                2,
                1,
                3,
                "entry_order_only",
            ),
            (b"LC_ALL=C\0LC_ALL=C\0", 18, 2, 1, 1, "other_mismatch"),
            (
                b"LC_ALL=C\0PGCONNECT_TIMEOUT=5\0LC_ALL=C\0",
                38,
                3,
                1,
                3,
                "other_mismatch",
            ),
            (b"LC_ALL=C\0", 9, 1, 1, 1, "other_mismatch"),
            (
                b"LC_ALL=C\0PGCONNECT_TIMEOUT=5\0X=1\0",
                33,
                3,
                1,
                3,
                "other_mismatch",
            ),
            (
                b"LC_ALL=C\0PGCONNECT_TIMEOUT=6\0",
                29,
                2,
                1,
                1,
                "other_mismatch",
            ),
            (
                b"XLC_ALL=C\0PGCONNECT_TIMEOUT=5\0",
                30,
                2,
                1,
                2,
                "other_mismatch",
            ),
            (
                b"PGCONNECT_TIMEOUT=5\0LC_ALL=C",
                28,
                1,
                0,
                3,
                "other_mismatch",
            ),
            (
                b"LC_ALL=C\0\0PGCONNECT_TIMEOUT=5\0",
                30,
                3,
                1,
                3,
                "other_mismatch",
            ),
        ];
        for (actual, bytes, nuls, trailing, presence, class) in rows {
            let mut state = Snapshot::default();
            let expected = b"a\0b\0c\0d\0e\0f\0g\0h\0";
            state.comparisons(expected, expected, actual);
            let value: serde_json::Value =
                serde_json::from_slice(&state.bytes(uuid::Uuid::new_v4(), false, None).unwrap())
                    .unwrap();
            assert_eq!(
                value["environment_shape"],
                serde_json::json!({
                    "bytes":bytes,"nul_count":nuls,"trailing_nul_count":trailing,
                    "expected_presence_mask":presence,"classification":class
                })
            );
            assert_eq!(value["exact_environment_matches"], *class == "exact");
            assert_eq!(value["exact_argv_matches"], true);
        }
    }
    #[test]
    fn diagnostic_shape_observation_bound_saturates_without_native_limit_changes() {
        for size in [4096, 4097, 8192] {
            let actual = vec![0; size];
            let mut state = Snapshot::default();
            state.comparisons(&actual, b"a\0b\0c\0d\0e\0f\0g\0h\0", &actual);
            let raw = state.bytes(uuid::Uuid::new_v4(), false, None).unwrap();
            assert!(raw.len() <= 4096);
            let value: serde_json::Value = serde_json::from_slice(&raw).unwrap();
            for key in ["argv_shape", "environment_shape"] {
                let shape = &value[key];
                assert_eq!(shape["bytes"], size.min(4097));
                if size == 4096 {
                    assert_eq!(shape["nul_count"], 4096);
                    assert_eq!(shape["trailing_nul_count"], 4096);
                    assert_eq!(shape["expected_presence_mask"], 0);
                    assert_eq!(shape["classification"], "other_mismatch");
                } else {
                    assert!(shape["nul_count"].is_null());
                    assert!(shape["trailing_nul_count"].is_null());
                    assert!(shape["expected_presence_mask"].is_null());
                    assert_eq!(shape["classification"], "over_observation_bound");
                }
            }
            assert_eq!(value["exact_argv_matches"], false);
            assert_eq!(value["exact_environment_matches"], false);
        }
    }
    #[test]
    fn diagnostic_comparisons_never_serialize_raw_sentinel_values() {
        let sentinel = b"SENTINEL_RAW_ENV_ARGV_SQL_DSN_password";
        let mut state = Snapshot::default();
        state.comparisons(sentinel, b"fixed", sentinel);
        let raw = state.bytes(uuid::Uuid::new_v4(), false, None).unwrap();
        assert!(!raw.windows(sentinel.len()).any(|part| part == sentinel));
        let value: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(value["exact_argv_matches"], false);
        assert_eq!(value["exact_environment_matches"], false);
    }
    #[test]
    fn diagnostic_partial_proc_read_stays_unknown_and_uses_only_fixed_stage_labels() {
        for stage in [
            ProcRead::ChildId,
            ProcRead::Executable,
            ProcRead::CommandLine,
            ProcRead::Environment,
            ProcRead::StartTicks,
        ] {
            let mut state = Snapshot::default();
            state.failed_proc_read = Some(stage);
            state.callback_entered = true;
            let value: serde_json::Value =
                serde_json::from_slice(&state.bytes(uuid::Uuid::new_v4(), false, None).unwrap())
                    .unwrap();
            assert_eq!(value["failed_proc_read"], proc_name(stage));
            assert_eq!(value["spawn_callback_completed"], false);
            assert!(value["exact_environment_matches"].is_null());
            assert!(value["argv_shape"].is_null());
            assert!(value["environment_shape"].is_null());
            assert!(value["final_stream_error"].is_null());
        }
    }
}

#[test]
fn source_dump_command_has_only_fixed_local_socket_and_no_caller_endpoint() {
    let database = "learning_backup_c4_task3_11111111-1111-4111-8111-111111111111";
    assert_eq!(
        super::endpoint::dump_args(database).unwrap(),
        vec![
            "--format=custom",
            "--no-password",
            "--lock-wait-timeout=5000",
            "--host=/var/run/postgresql",
            "--port=5432",
            "--username=learning_admin",
            "--dbname=learning_backup_c4_task3_11111111-1111-4111-8111-111111111111"
        ]
    );
    for bad in [
        "postgres",
        "--dbname=other",
        "learning_backup_c4_task3_foo;SELECT 1",
    ] {
        assert!(super::endpoint::dump_args(bad).is_err());
    }
}

#[test]
fn same_ids_without_original_challenge_cannot_admit_source_observation() {
    let database = "learning_backup_c4_task3_11111111-1111-4111-8111-111111111111";
    let identity = format!("learning_admin|{database}|16385|123456\n");
    let keys = crate::restore_preflight::target_binding::ChallengeKeys::for_test(7, 4294967298);
    assert!(
        super::endpoint::validate_socket_observation(
            database, 16385, "123456", 123, keys, &identity
        )
        .is_err()
    );
    let original =
        format!("{identity}123|16385|0|7|1|ExclusiveLock|t\n123|16385|1|2|1|ExclusiveLock|t\n");
    assert!(
        super::endpoint::validate_socket_observation(
            database, 16385, "123456", 123, keys, &original
        )
        .is_ok()
    );
    for bad in [
        original.replace("123|16385", "124|16385"),
        original.replace("ExclusiveLock|t", "ExclusiveLock|f"),
        original.replace("16385", "16386"),
    ] {
        assert!(
            super::endpoint::validate_socket_observation(
                database, 16385, "123456", 123, keys, &bad
            )
            .is_err()
        );
    }
}

#[test]
fn source_native_observation_rejects_namespace_restart_socket_and_data_changes() {
    let good = super::endpoint::NativeIdentity {
        pid_namespace: "pid:[123]".into(),
        mount_namespace: "mnt:[124]".into(),
        postmaster_start_ticks: 55,
        data_dev: 1,
        data_ino: 2,
        socket_dev: 3,
        socket_ino: 4,
    };
    assert!(good.require_same(&good).is_ok());
    for which in 0..7 {
        let mut changed = good.clone();
        match which {
            0 => changed.pid_namespace = "pid:[223]".into(),
            1 => changed.mount_namespace = "mnt:[224]".into(),
            2 => changed.postmaster_start_ticks += 1,
            3 => changed.data_dev += 1,
            4 => changed.data_ino += 1,
            5 => changed.socket_dev += 1,
            _ => changed.socket_ino += 1,
        }
        assert!(good.require_same(&changed).is_err());
    }
}
#[cfg(target_os = "linux")]
use super::*;

#[cfg(target_os = "linux")]
fn registry_prewrite_inventory() -> Vec<(String, Vec<u8>)> {
    fn walk(root: &BackupDir, prefix: &str, rows: &mut Vec<(String, Vec<u8>)>) {
        use learning_assets::backup_fs::BackupEntryKind;
        for name in root.list_bounded(1000).unwrap() {
            assert!(rows.len() < 1000);
            let path = format!("{prefix}{name}");
            match root.kind(&name).unwrap() {
                BackupEntryKind::Directory => {
                    rows.push((path.clone(), vec![]));
                    walk(&root.open_dir(&name).unwrap(), &(path + "/"), rows);
                }
                BackupEntryKind::File => {
                    let mut bytes = Vec::new();
                    root.open_file(&name)
                        .unwrap()
                        .take(1048577)
                        .read_to_end(&mut bytes)
                        .unwrap();
                    assert!(bytes.len() <= 1048576);
                    rows.push((path, bytes));
                }
                _ => panic!("source clone registry inventory has unknown entry"),
            }
        }
    }
    let mut rows = Vec::new();
    walk(
        &BackupDir::open_trusted_private_root(Path::new(crate::registry::REGISTRY_PATH)).unwrap(),
        "",
        &mut rows,
    );
    rows.sort();
    rows
}

#[cfg(target_os = "linux")]
thread_local! {
    static CLONE_OBSERVER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static CLONE_OBSERVED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Observation-only RED hook: the actual existing SourceAdmission owns these
/// locks. No alternative source flow, pool connection or caller SQL is used.
#[cfg(target_os = "linux")]
pub(super) async fn observe_admitted_source(
    admission: &mut SourceAdmission,
    config: &SourceBackupConfig,
) -> Result<(), BackupError> {
    if !CLONE_OBSERVER.with(|state| state.get()) {
        return Ok(());
    }
    CLONE_OBSERVER.with(|state| state.set(false));
    let keys = crate::restore_preflight::target_binding::ChallengeKeys::random()?;
    for key in keys.values() {
        let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1::bigint)")
            .bind(key)
            .fetch_one(admission.connection())
            .await?;
        if !locked {
            return Err(BackupError::Invalid("clone observer challenge busy"));
        }
    }
    let (pid,oid,system): (i32,i64,String)=sqlx::query_as("SELECT pg_backend_pid(),d.oid::bigint,pcs.system_identifier::text FROM pg_database d CROSS JOIN pg_control_system() pcs WHERE d.datname=current_database()")
        .fetch_one(admission.connection()).await?;
    let nonce = Uuid::new_v4();
    let root =
        BackupDir::open_trusted_private_root(config.isolation_attestation.parent().unwrap())?;
    let request = serde_json::to_vec(
        &serde_json::json!({"format_version":1,"backup_id":config.backup_id,"nonce":nonce,"backend_pid":pid,"database_oid":oid,"system_identifier":system,"challenge_keys":keys.values()}),
    )?;
    assert!(request.len() <= 4096);
    let mut file = root.create_file(&format!("source-endpoint-observe-{nonce}.json"))?;
    file.write_all(&request)?;
    file.sync_all()?;
    root.sync()?;
    drop(file);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match root.open_file(&format!("source-endpoint-observed-{nonce}.json")) {
            Ok(file) => {
                let mut raw = Vec::new();
                file.take(16385).read_to_end(&mut raw)?;
                if raw.len() > 16384 {
                    return Err(BackupError::Invalid("clone observer response size"));
                }
                let response: serde_json::Value = serde_json::from_slice(&raw)?;
                let identity = format!(
                    "learning_admin|{}|{oid}|{system}\n",
                    config.expected_database
                );
                if response["nonce"] != nonce.to_string()
                    || response["backup_id"] != config.backup_id.to_string()
                    || response["source_backend_pid"] != pid
                    || response["source_database_oid"] != oid
                    || response["source_system_identifier"] != system
                    || response["clone_socket_output"] != identity
                    || response["clone_project"] != config.expected_compose_project
                    || response["original_container_id"] == response["clone_container_id"]
                    || response["physical_backup_verified"] != true
                {
                    return Err(BackupError::Invalid("clone observer identity/challenge"));
                }
                let now_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
                    .fetch_one(admission.connection())
                    .await?;
                if now_pid != pid {
                    return Err(BackupError::Invalid("clone observer lost admitted backend"));
                }
                CLONE_OBSERVED.with(|state| state.set(true));
                return Ok(());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
        if Instant::now() >= deadline {
            return Err(BackupError::Invalid("clone observer deadline"));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Root's fixed host observer targets the fresh physical clone; manager and
/// admitted session remain in the original, with no cross-bridge attachment.
/// Equal physical IDs and missing actual admitted-session locks are observed
/// independently. Rejection must precede any source mutation, even if the old
/// weaker route later fails for some unrelated reason.
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "requires fresh independently verified physical source clone and fixed root observer"]
async fn source_same_id_physical_clone_rejected_before_journal_or_acl() {
    let mut f = lifecycle_tests::live::Fixture::new().await;
    let proof = BackupDir::open_trusted_private_root(&f.proof_root).unwrap();
    let mut setup = Vec::new();
    proof
        .open_file("source-clone-setup.json")
        .unwrap()
        .take(16385)
        .read_to_end(&mut setup)
        .unwrap();
    assert!(setup.len() <= 16384);
    let setup: serde_json::Value = serde_json::from_slice(&setup).unwrap();
    assert_eq!(setup["physical_backup_verified"], true);
    assert_ne!(setup["original_container_id"], setup["clone_container_id"]);
    assert_eq!(setup["database"], f.config.expected_database);
    f.config.expected_compose_project = setup["clone_project"].as_str().unwrap().to_owned();
    f.next().await;
    let before = binding::fixture_inventory(&f.config.control_root).unwrap();
    let pins = BackupDir::open_trusted_private_root(&f.config.local_pin_root).unwrap();
    let pin_before = pins.list().unwrap();
    let registry_before = registry_prewrite_inventory();
    assert!(f.acl().await);
    CLONE_OBSERVED.with(|state| state.set(false));
    CLONE_OBSERVER.with(|state| state.set(true));
    let result = prepare_source_backup(&f.pool, &f.assets, &f.config).await;
    assert!(
        CLONE_OBSERVED.with(|state| state.get()),
        "setup refusal is not clone RED/GREEN evidence"
    );
    let acl_after = f.acl().await;
    let controls_unchanged = binding::fixture_inventory(&f.config.control_root).unwrap() == before;
    let pins_unchanged = pins.list().unwrap() == pin_before;
    let registry_unchanged = registry_prewrite_inventory() == registry_before;
    let exact_namespace_refusal = matches!(
        &result,
        Err(BackupError::Invalid(
            "source executor namespace/postmaster/socket changed"
        ))
    );
    let observation=serde_json::to_vec(&serde_json::json!({"format_version":1,"backup_id":f.config.backup_id,"exact_namespace_refusal":exact_namespace_refusal,"actual_admitted_clone_observation":true,"capture_returned_pin":result.is_ok(),"runtime_connect_unchanged":acl_after,"control_journal_dump_unchanged":controls_unchanged,"pins_unchanged":pins_unchanged,"registry_unchanged":registry_unchanged})).unwrap();
    let mut file = proof
        .create_file("source-clone-prewrite-result.json")
        .unwrap();
    file.write_all(&observation).unwrap();
    file.sync_all().unwrap();
    proof.sync().unwrap();
    drop(file);
    assert!(
        controls_unchanged,
        "clone rejection precedes journal/dump changes"
    );
    assert!(
        registry_unchanged,
        "clone rejection precedes protection changes"
    );
    assert!(acl_after, "clone rejection precedes CONNECT changes");
    assert!(
        pins_unchanged,
        "clone rejection publishes no sealed/complete pin"
    );
    assert!(
        result.is_err(),
        "wrong endpoint must never produce a source pin"
    );
    assert!(
        exact_namespace_refusal,
        "wrong endpoint must fail the actual namespace check, not a missing proof or incidental fixture condition"
    );
    f.pool.close().await;
}

// The overflow controller cannot resume the paused server until the exact
// native spawned checks have acknowledged startup. This is test-only state.
#[derive(Default)]
struct OverflowResume {
    started: bool,
    request: Option<serde_json::Value>,
    inflight: Option<serde_json::Value>,
    resumed: bool,
}
impl OverflowResume {
    fn begin(&mut self) -> Result<Option<serde_json::Value>, crate::BackupError> {
        if self.resumed {
            return Ok(None);
        }
        if !self.started || self.inflight.is_some() {
            return Err(crate::BackupError::Invalid(
                "overflow strict start or resume ownership",
            ));
        }
        let mut request = self
            .request
            .clone()
            .ok_or(crate::BackupError::Invalid("overflow pause missing"))?;
        request["phase"] = serde_json::json!("resume");
        request["nonce"] = serde_json::json!(uuid::Uuid::new_v4());
        self.inflight = Some(request.clone());
        Ok(Some(request))
    }
    fn acknowledge(&mut self, request: &serde_json::Value) -> Result<(), crate::BackupError> {
        if self.resumed || self.inflight.as_ref() != Some(request) {
            return Err(crate::BackupError::Invalid(
                "overflow exact resume acknowledgement",
            ));
        }
        self.resumed = true;
        self.request = None;
        self.inflight = None;
        Ok(())
    }
}
#[test]
fn overflow_resume_requires_strict_start_and_single_exact_ack() {
    use uuid::Uuid;
    let pause = serde_json::json!({"backup_id":Uuid::new_v4(),"epoch":Uuid::new_v4(),"nonce":Uuid::new_v4(),"phase":"pause","backend_pid":123,"database_oid":456,"challenge_keys":[1,2],"postmaster_start_ticks":9});
    let mut state = OverflowResume {
        request: Some(pause.clone()),
        ..Default::default()
    };
    assert!(state.begin().is_err());
    state.started = true;
    let resume = state.begin().unwrap().unwrap();
    assert_eq!(resume["phase"], "resume");
    assert_ne!(resume["nonce"], pause["nonce"]);
    for field in [
        "backup_id",
        "epoch",
        "backend_pid",
        "database_oid",
        "challenge_keys",
        "postmaster_start_ticks",
    ] {
        assert_eq!(resume[field], pause[field]);
    }
    assert!(state.begin().is_err());
    let mut wrong = resume.clone();
    wrong["epoch"] = serde_json::json!(Uuid::new_v4());
    assert!(state.acknowledge(&wrong).is_err());
    assert!(!state.resumed);
    assert!(state.request.is_some());
    state.acknowledge(&resume).unwrap();
    assert!(state.begin().unwrap().is_none());
    assert!(state.acknowledge(&resume).is_err());
}

#[cfg(target_os = "linux")]
pub(super) mod faults {
    use super::diagnostic::Fault;
    use super::*;
    use crate::restore_preflight::target_binding::child_attestation::ChildFailure;
    use sqlx::{Connection, postgres::PgConnectOptions};
    use std::sync::{Arc, Mutex};
    #[derive(Clone, Default)]
    struct Observation {
        pid: u32,
        start: u64,
        backend: i32,
        oid: u64,
        keys: [i64; 2],
        exe: String,
        cause: Option<ChildFailure>,
        dropped: bool,
        reaped: bool,
    }
    struct Trace {
        fault: Fault,
        observation: Mutex<Observation>,
        diagnostic: Arc<Mutex<super::diagnostic::Snapshot>>,
        startup_failure: Mutex<super::startup_failure::Evidence>,
        started: tokio::sync::Notify,
        cleanup: tokio::sync::Notify,
        release: tokio::sync::Notify,
        overflow: Mutex<super::OverflowResume>,
        overflow_proof: Mutex<Option<BackupDir>>,
    }
    impl Trace {
        async fn resume_overflow(&self) -> Result<(), BackupError> {
            let request = self.overflow.lock().unwrap().begin()?;
            if let Some(request) = request {
                let proof = self
                    .overflow_proof
                    .lock()
                    .unwrap()
                    .as_ref()
                    .ok_or(BackupError::Invalid("overflow proof missing"))?
                    .try_clone()?;
                // No state reset on failure/cancellation: the external finally
                // guard remains armed, and no second resume is issued locally.
                TestControl::exchange_with(&proof, &request).await?;
                self.overflow.lock().unwrap().acknowledge(&request)?;
            }
            Ok(())
        }
    }
    thread_local! {static TRACE:std::cell::RefCell<Option<Arc<Trace>>>=const{std::cell::RefCell::new(None)};}
    struct Scope(Arc<Trace>);
    impl Scope {
        fn new(fault: Fault) -> Self {
            let trace = Arc::new(Trace {
                fault,
                observation: Mutex::new(Observation::default()),
                diagnostic: Arc::new(Mutex::new(super::diagnostic::Snapshot::default())),
                startup_failure: Mutex::new(super::startup_failure::Evidence::default()),
                started: tokio::sync::Notify::new(),
                cleanup: tokio::sync::Notify::new(),
                release: tokio::sync::Notify::new(),
                overflow: Mutex::new(super::OverflowResume::default()),
                overflow_proof: Mutex::new(None),
            });
            TRACE.with(|state| *state.borrow_mut() = Some(trace.clone()));
            Self(trace)
        }
        fn publish_diagnostic(
            &self,
            proof: &BackupDir,
            backup: Uuid,
            prepare_ok: bool,
            dump_size: Option<u64>,
        ) -> Result<(), BackupError> {
            use std::os::unix::fs::MetadataExt;
            let snapshot = self.0.diagnostic.lock().unwrap().clone();
            let raw = snapshot.bytes(backup, prepare_ok, dump_size)?;
            proof.require_private_directory()?;
            let name = "source-bound-dump-diagnostic.json";
            let mut file = proof.create_file(name)?;
            let metadata = file.metadata()?;
            if !metadata.is_file()
                || metadata.uid() != 0
                || metadata.mode() & 0o7777 != 0o600
                || metadata.nlink() != 1
            {
                return Err(BackupError::Invalid("source diagnostic private file"));
            }
            file.write_all(&raw)?;
            file.sync_all()?;
            proof.sync()?;
            drop(file);
            let mut readback = Vec::new();
            proof
                .open_file(name)?
                .take(4097)
                .read_to_end(&mut readback)?;
            if readback != raw {
                return Err(BackupError::Invalid("source diagnostic readback"));
            }
            Ok(())
        }
    }
    impl Drop for Scope {
        fn drop(&mut self) {
            TRACE.with(|state| *state.borrow_mut() = None);
            self.0.release.notify_one();
        }
    }
    pub(in crate::source) struct TestControl {
        trace: Arc<Trace>,
        active: bool,
        paused: bool,
        lost_lock: bool,
        proof: Option<BackupDir>,
        pause_request: Option<serde_json::Value>,
    }
    pub(in crate::source) fn current_control() -> Option<TestControl> {
        TRACE.with(|state| {
            state.borrow().as_ref().map(|trace| TestControl {
                trace: trace.clone(),
                active: false,
                paused: false,
                lost_lock: false,
                proof: None,
                pause_request: None,
            })
        })
    }
    fn process_start(pid: u32) -> Option<u64> {
        let raw = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        raw.rsplit_once(") ")?
            .1
            .split_whitespace()
            .nth(19)?
            .parse()
            .ok()
    }
    impl TestControl {
        pub(in crate::source) fn diagnostic_handle(
            &self,
        ) -> Arc<Mutex<super::diagnostic::Snapshot>> {
            // The clone contains only typed observations, never held authority.
            Arc::clone(&self.trace.diagnostic)
        }
        pub(in crate::source) async fn before_dump(
            &mut self,
            proof: &BackupDir,
            backup_id: Uuid,
            epoch: Uuid,
            start: u64,
            backend: i32,
            oid: u64,
            keys: [i64; 2],
        ) -> Result<(Duration, u64), BackupError> {
            self.active = true;
            {
                let mut state = self.trace.diagnostic.lock().unwrap();
                state.backend = Some(backend);
                state.oid = Some(oid);
                state.epoch = Some(epoch);
            }
            if self.trace.fault.pauses_postmaster() {
                self.proof = Some(proof.try_clone()?);
                self.pause_request = Some(
                    serde_json::json!({"format_version":1,"backup_id":backup_id,"epoch":epoch,"nonce":Uuid::new_v4(),"phase":"pause","backend_pid":backend,"database_oid":oid,"challenge_keys":keys,"postmaster_start_ticks":start}),
                );
                self.paused = true; // External cleanup guard armed before send.
                self.exchange(self.pause_request.as_ref().unwrap()).await?;
                if self.trace.fault == Fault::Overflow {
                    self.trace.overflow.lock().unwrap().request = self.pause_request.clone();
                    *self.trace.overflow_proof.lock().unwrap() = Some(proof.try_clone()?);
                }
            }
            Ok((
                if self.trace.fault == Fault::Timeout {
                    Duration::from_millis(800)
                } else {
                    Duration::from_secs(900)
                },
                if self.trace.fault == Fault::Overflow {
                    64
                } else {
                    1024 * 1024 * 1024
                },
            ))
        }
        async fn exchange(&self, request: &serde_json::Value) -> Result<(), BackupError> {
            let proof = self
                .proof
                .as_ref()
                .ok_or(BackupError::Invalid("source fault proof missing"))?;
            Self::exchange_with(proof, request).await
        }
        async fn exchange_with(
            proof: &BackupDir,
            request: &serde_json::Value,
        ) -> Result<(), BackupError> {
            let nonce = request["nonce"]
                .as_str()
                .ok_or(BackupError::Invalid("source fault nonce"))?;
            let raw = serde_json::to_vec(request)?;
            let mut file = proof.create_file(&format!("source-fault-request-{nonce}.json"))?;
            file.write_all(&raw)?;
            file.sync_all()?;
            proof.sync()?;
            drop(file);
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                match proof.open_file(&format!("source-fault-ready-{nonce}.json")) {
                    Ok(file) => {
                        let mut raw = Vec::new();
                        file.take(4097).read_to_end(&mut raw)?;
                        if raw.len() > 4096 {
                            return Err(BackupError::Invalid("source fault ack size"));
                        }
                        let ack: serde_json::Value = serde_json::from_slice(&raw)?;
                        if ack.as_object().is_none_or(|row| row.len() != 6)
                            || serde_json::to_vec(&ack)? != raw
                        {
                            return Err(BackupError::Invalid("source fault exact ack"));
                        }
                        for key in [
                            "backup_id",
                            "epoch",
                            "nonce",
                            "phase",
                            "postmaster_start_ticks",
                        ] {
                            if ack[key] != request[key] {
                                return Err(BackupError::Invalid("source fault ack scope"));
                            }
                        }
                        let state = std::fs::read_to_string("/proc/1/stat")?;
                        let tail = state
                            .strip_prefix("1 (postgres) ")
                            .ok_or(BackupError::Invalid("source fault postmaster"))?
                            .split_whitespace()
                            .collect::<Vec<_>>();
                        if tail.len() < 20
                            || tail[19].parse::<u64>().ok()
                                != request["postmaster_start_ticks"].as_u64()
                            || (if request["phase"] == "pause" {
                                tail[0] != "T" || ack["observed_state"] != "T"
                            } else {
                                !matches!(tail[0], "R" | "S" | "D" | "I")
                                    || !matches!(
                                        ack["observed_state"].as_str(),
                                        Some("R" | "S" | "D" | "I")
                                    )
                            })
                        {
                            return Err(BackupError::Invalid(
                                "source fault actual state unconfirmed",
                            ));
                        }
                        return Ok(());
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                    Err(error) => return Err(error.into()),
                }
                if Instant::now() >= deadline {
                    return Err(BackupError::Invalid("source fault ack deadline"));
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        pub(in crate::source) async fn resume(&mut self) -> Result<(), BackupError> {
            if self.paused {
                if self.trace.fault == Fault::Overflow {
                    self.trace.resume_overflow().await?;
                    self.paused = false;
                    return Ok(());
                }
                let mut request = self
                    .pause_request
                    .as_ref()
                    .ok_or(BackupError::Invalid("source fault pause missing"))?
                    .clone();
                request["phase"] = serde_json::json!("resume");
                request["nonce"] = serde_json::json!(Uuid::new_v4());
                self.exchange(&request).await?;
                self.paused = false;
            }
            Ok(())
        }
        pub(in crate::source) fn after_dump(&mut self) {
            self.active = false;
        }
        pub(in crate::source) fn spawned(
            &mut self,
            child: &mut tokio::process::Child,
            backend: i32,
            oid: u64,
            keys: [i64; 2],
            database: &str,
        ) -> Result<(), ChildFailure> {
            if !self.active {
                return Ok(());
            }
            self.trace.diagnostic.lock().unwrap().callback_entered = true;
            let pid = child.id().ok_or_else(|| {
                self.trace.diagnostic.lock().unwrap().failed_proc_read =
                    Some(super::diagnostic::ProcRead::ChildId);
                ChildFailure::Io
            })?;
            self.trace.diagnostic.lock().unwrap().pid = Some(pid);
            let exe = std::fs::read_link(format!("/proc/{pid}/exe")).map_err(|_| {
                self.trace.diagnostic.lock().unwrap().failed_proc_read =
                    Some(super::diagnostic::ProcRead::Executable);
                ChildFailure::Io
            })?;
            self.trace.diagnostic.lock().unwrap().executable_matches =
                Some(exe == Path::new("/usr/lib/postgresql/18/bin/pg_dump"));
            if exe != Path::new("/usr/lib/postgresql/18/bin/pg_dump") {
                return super::startup_failure::Evidence::after_strict_result(
                    &self.trace.startup_failure,
                    self.active,
                    self.trace.fault,
                    super::startup_failure::Phase::Executable,
                    Err(ChildFailure::Identity),
                    || child.try_wait(),
                );
            }
            let cmd = std::fs::read(format!("/proc/{pid}/cmdline")).map_err(|_| {
                self.trace.diagnostic.lock().unwrap().failed_proc_read =
                    Some(super::diagnostic::ProcRead::CommandLine);
                ChildFailure::Io
            })?;
            let expected = [
                "/usr/lib/postgresql/18/bin/pg_dump".into(),
                "--format=custom".into(),
                "--no-password".into(),
                "--lock-wait-timeout=5000".into(),
                "--host=/var/run/postgresql".into(),
                "--port=5432".into(),
                "--username=learning_admin".into(),
                format!("--dbname={database}"),
            ]
            .join("\0")
                + "\0";
            let env = std::fs::read(format!("/proc/{pid}/environ")).map_err(|_| {
                self.trace.diagnostic.lock().unwrap().failed_proc_read =
                    Some(super::diagnostic::ProcRead::Environment);
                ChildFailure::Io
            })?;
            self.trace
                .diagnostic
                .lock()
                .unwrap()
                .comparisons(&cmd, expected.as_bytes(), &env);
            if cmd != expected.as_bytes() || env != b"LC_ALL=C\0PGCONNECT_TIMEOUT=5\0" {
                return super::startup_failure::Evidence::after_strict_result(
                    &self.trace.startup_failure,
                    self.active,
                    self.trace.fault,
                    super::startup_failure::Phase::ArgumentsEnvironment,
                    Err(ChildFailure::Identity),
                    || child.try_wait(),
                );
            }
            let start = process_start(pid).ok_or_else(|| {
                self.trace.diagnostic.lock().unwrap().failed_proc_read =
                    Some(super::diagnostic::ProcRead::StartTicks);
                ChildFailure::Io
            })?;
            self.trace.diagnostic.lock().unwrap().start_ticks = Some(start);
            *self.trace.observation.lock().unwrap() = Observation {
                pid,
                start,
                backend,
                oid,
                keys,
                exe: exe.to_string_lossy().into(),
                ..Observation::default()
            };
            if self.trace.fault == Fault::Overflow {
                self.trace.overflow.lock().unwrap().started = true;
            }
            self.trace.started.notify_one();
            if self.trace.fault == Fault::Exit {
                child.start_kill().map_err(|_| ChildFailure::Io)?;
            }
            self.trace.diagnostic.lock().unwrap().callback_completed = true;
            Ok(())
        }
        pub(in crate::source) fn lose_lock(&mut self) -> bool {
            if self.active && self.trace.fault == Fault::LockLoss && !self.lost_lock {
                self.lost_lock = true;
                true
            } else {
                false
            }
        }
        pub(in crate::source) async fn cleanup(&mut self, reason: ChildFailure) {
            if !self.active {
                return;
            }
            self.trace.observation.lock().unwrap().cause = Some(reason);
            self.trace.diagnostic.lock().unwrap().cleanup(reason);
            self.trace.cleanup.notify_one();
            if self.trace.fault == Fault::Cancel {
                let _ = tokio::time::timeout(Duration::from_secs(5), self.trace.release.notified())
                    .await;
            }
        }
    }
    impl Drop for TestControl {
        fn drop(&mut self) {
            if self.active {
                let mut observed = self.trace.observation.lock().unwrap();
                observed.reaped =
                    observed.pid > 0 && process_start(observed.pid) != Some(observed.start);
                observed.dropped = true;
                let mut state = self.trace.diagnostic.lock().unwrap();
                state.dropped = Some(true);
                state.pid_start_unobserved = (observed.pid > 0).then_some(observed.reaped);
            }
            // Controller owns an external resume guard from before STOP and
            // attempts it in finally before its exact PG stop/readback, even
            // if this native future was cancelled before request acknowledgement.
        }
    }
    async fn observer() -> PgConnection {
        PgConnection::connect_with(
            &PgConnectOptions::new()
                .host("/var/run/postgresql")
                .port(5432)
                .username("learning_admin")
                .database("postgres"),
        )
        .await
        .unwrap()
    }
    async fn one_fault(
        f: &mut lifecycle_tests::live::Fixture,
        fault: Fault,
        observer: &mut PgConnection,
    ) {
        let scope = Scope::new(fault);
        let mut capture = Box::pin(prepare_source_backup(&f.pool, &f.assets, &f.config));
        tokio::select! {
            biased;
            _=scope.0.started.notified()=>(),
            result=&mut capture=>{
                let snapshot = scope.0.diagnostic.lock().unwrap().clone();
                super::diagnostic::early_return(&snapshot, fault, f.config.backup_id, &result)
            },
            _=tokio::time::sleep(Duration::from_secs(45))=>panic!("source dump spawn deadline"),
        }
        let initial = scope.0.observation.lock().unwrap().clone();
        assert_eq!(initial.exe, "/usr/lib/postgresql/18/bin/pg_dump");
        if fault == Fault::Overflow {
            scope.0.resume_overflow().await.unwrap();
        }
        if fault == Fault::Cancel {
            drop(capture);
            tokio::time::timeout(Duration::from_secs(5), scope.0.cleanup.notified())
                .await
                .unwrap();
            assert_eq!(
                process_start(initial.pid),
                Some(initial.start),
                "test cleanup barrier must hold exact native child alive"
            );
            let count:i64=sqlx::query_scalar("SELECT count(*) FROM pg_locks WHERE locktype='advisory' AND pid=$1 AND database::bigint=$2 AND objsubid=1 AND mode='ExclusiveLock' AND granted AND ((classid::bigint,objid::bigint)=(($3::bigint >> 32) & 4294967295,$3::bigint & 4294967295) OR (classid::bigint,objid::bigint)=(($4::bigint >> 32) & 4294967295,$4::bigint & 4294967295))")
                .bind(initial.backend).bind(initial.oid as i64).bind(initial.keys[0]).bind(initial.keys[1]).fetch_one(&mut *observer).await.unwrap();
            assert_eq!(
                count, 2,
                "original admission locks remain while cancelled child's supervisor owns cleanup"
            );
            assert!(matches!(
                crate::ManagementRegistry::open_installed()
                    .unwrap()
                    .try_lock(),
                Err(BackupError::Invalid("registry busy"))
            ));
            scope.0.release.notify_one();
        } else {
            assert!(capture.await.is_err());
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if scope.0.observation.lock().unwrap().dropped {
                break;
            }
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let actual = scope.0.observation.lock().unwrap().clone();
        assert!(
            actual.reaped,
            "guard must not release authority before exact native child wait/reap"
        );
        let expected = match fault {
            Fault::Timeout => ChildFailure::Deadline,
            Fault::Cancel => ChildFailure::Unusable,
            Fault::Overflow => ChildFailure::StdoutLimit,
            Fault::Exit => ChildFailure::Exit,
            Fault::LockLoss => ChildFailure::Session,
            _ => unreachable!(),
        };
        assert_eq!(actual.cause, Some(expected));
        loop {
            let count: i64 =
                sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE pid=$1")
                    .bind(initial.backend)
                    .fetch_one(&mut *observer)
                    .await
                    .unwrap();
            if count == 0 {
                break;
            }
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!f.acl().await);
        assert_eq!(f.journal().record().phase(), crate::GatePhase::Drained);
        assert!(
            BackupDir::open_trusted_private_root(&f.config.local_pin_root)
                .unwrap()
                .list()
                .unwrap()
                .is_empty()
        );
        let registry = crate::ManagementRegistry::open_installed().unwrap();
        let lease = registry.try_lock().unwrap();
        let enrolled = lease.admit_source(&f.root(), &f.assets, &f.config).unwrap();
        let protection =
            crate::CaptureProtection::reopen(&lease, &enrolled, f.config.backup_id).unwrap();
        let before = registry_prewrite_inventory();
        protection
            .recheck_capture(enrolled.group_id(), f.config.backup_id)
            .unwrap();
        assert!(
            protection.recheck_before_release().is_err(),
            "catalog validation cannot grant release authority"
        );
        assert_eq!(
            registry_prewrite_inventory(),
            before,
            "capture recheck is read-only"
        );
        drop(protection);
        drop(lease);
        let root = BackupDir::open_trusted_private_root(&f.proof_root).unwrap();
        let mut out = root
            .create_file(&format!("source-fault-{}.json", f.config.backup_id))
            .unwrap();
        out.write_all(&serde_json::to_vec(&serde_json::json!({"backup_id":f.config.backup_id,"child_pid":initial.pid,"source_backend":initial.backend,"source_oid":initial.oid,"fixed_executable":initial.exe,"failure":format!("{expected:?}"),"child_reaped_before_authority_drop":actual.reaped,"source_backend_disappeared":true,"runtime_connect_closed":true,"registry_protection_retained":true,"actual_postmaster_restart":false,"test_postmaster_pause":fault.pauses_postmaster(),"test_dump_cap":if fault==Fault::Overflow{64_u64}else{1024*1024*1024},"test_dump_timeout_ms":if fault==Fault::Timeout{800}else{900000}})).unwrap()).unwrap();
        out.sync_all().unwrap();
        root.sync().unwrap();
    }
    pub(super) async fn cancellations() {
        let mut f = lifecycle_tests::live::Fixture::new().await;
        let mut observer = observer().await;
        for (index, fault) in [Fault::Timeout, Fault::Cancel, Fault::Overflow]
            .into_iter()
            .enumerate()
        {
            if index > 0 {
                f.next().await;
            }
            one_fault(&mut f, fault, &mut observer).await;
            f.abandon().await;
        }
        observer.close().await.unwrap();
        f.pool.close().await;
    }
    pub(super) async fn failure() {
        let mut f = lifecycle_tests::live::Fixture::new().await;
        let mut observer = observer().await;
        one_fault(&mut f, Fault::Exit, &mut observer).await;
        observer.close().await.unwrap();
        f.pool.close().await;
    }
    pub(super) async fn lock_loss() {
        let mut f = lifecycle_tests::live::Fixture::new().await;
        let mut observer = observer().await;
        one_fault(&mut f, Fault::LockLoss, &mut observer).await;
        observer.close().await.unwrap();
        f.pool.close().await;
    }
    pub(super) async fn bound_dump() {
        let f = lifecycle_tests::live::Fixture::new().await;
        let scope = Scope::new(Fault::None);
        let proof = BackupDir::open_trusted_private_root(&f.proof_root).unwrap();
        let prepared = prepare_source_backup(&f.pool, &f.assets, &f.config).await;
        super::startup_failure::Evidence::failed_body(
            &scope.0.startup_failure,
            &prepared,
            |line| {
                println!("{line}");
            },
        );
        let dump_size = BackupDir::open_trusted_private_root(&f.config.control_root)
            .and_then(|root| root.open_dir(&format!("{}.source", f.config.backup_id)))
            .and_then(|source| source.open_file("database.dump"))
            .and_then(|file| file.metadata())
            .ok()
            .map(|metadata| metadata.len());
        scope
            .publish_diagnostic(&proof, f.config.backup_id, prepared.is_ok(), dump_size)
            .unwrap();
        let pin = prepared.unwrap();
        let observed = scope.0.observation.lock().unwrap().clone();
        assert!(observed.pid > 0 && observed.backend > 0);
        assert_eq!(observed.exe, "/usr/lib/postgresql/18/bin/pg_dump");
        assert!(f.acl().await);
        let mut receipt = proof.create_file("source-bound-dump.json").unwrap();
        receipt.write_all(&serde_json::to_vec(&serde_json::json!({"capability":"source_native_dump_observation_v1","backup_id":f.config.backup_id,"child_pid":observed.pid,"source_backend":observed.backend,"source_oid":observed.oid,"fixed_executable":observed.exe,"fixed_argv_observed":true,"cleared_environment_observed":true,"manifest_sha256":pin.sealed().manifest_sha256(),"child_wait_success":true,"state":"LOCAL_SEALED_NOT_COMPLETE"})).unwrap()).unwrap();
        receipt.sync_all().unwrap();
        proof.sync().unwrap();
        f.retained(&pin).await;
        f.pool.close().await;
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "requires fresh enrolled PG18.6 same-namespace source and fixed root driver"]
async fn source_dump_uses_exact_container_socket_and_admitted_backend() {
    faults::bound_dump().await;
}
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "requires fresh enrolled source; actual lock loss, no container restart credit"]
async fn source_restart_or_lock_loss_blocks_release() {
    faults::lock_loss().await;
}
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "requires actual fixed native pg_dump and private test fault supervisor"]
async fn source_dump_timeout_cancel_and_overflow_reap_child_keep_gate_closed() {
    faults::cancellations().await;
}
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "requires actual fixed native pg_dump and fresh registry protection"]
async fn source_dump_failure_keeps_registry_protection() {
    faults::failure().await;
}

//! Case-owned test evidence, never an import capability or public diagnostic.
use super::{CandidateImportReport, ImportFailure, SupervisorBoundary};
use serde::Serialize;
use std::{
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

pub(super) const CAP: usize = 8192;
#[cfg(target_os = "linux")]
pub(super) const FILES: [&str; 3] = [
    "full-supervisor-observation.stdout.raw",
    "full-supervisor-observation.stderr.raw",
    "full-supervisor-observation.meta.json",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) enum Point {
    Prepare,
    BeforeWriterReview,
    Prelude,
    Start,
    FirstLineRead,
    FirstBlankCheck,
    ReadyRead,
    ReadyParse,
    ReadyIdentity,
    BeforeAttemptReview,
    Attempt,
    BeforePayloadReview,
    PayloadRead,
    PayloadSend,
    PostcheckSend,
    PrecommitRead,
    PrecommitParse,
    PrecommitIdentity,
    BeforeIntentReview,
    Intent,
    FinalReview,
    CommitSend,
    ConfirmationSend,
    ConfirmationRead,
    ConfirmationParse,
    Finish,
    Readback,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(super) enum WaitSource {
    Natural,
    BeforeCleanup,
    Cleanup,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct NativeStatus {
    pub success: bool,
    pub code: Option<i32>,
    pub signal: Option<i32>,
}
impl NativeStatus {
    fn actual(status: std::process::ExitStatus) -> Self {
        #[cfg(unix)]
        let signal = {
            use std::os::unix::process::ExitStatusExt;
            status.signal()
        };
        #[cfg(not(unix))]
        let signal = None;
        Self {
            success: status.success(),
            code: status.code(),
            signal,
        }
    }
}
#[derive(Clone, Default, Debug, Serialize)]
pub(super) struct Pipe {
    #[serde(skip)]
    pub bytes: Vec<u8>,
    pub truncated: bool,
    pub eof: bool,
    pub settled: bool,
}
#[derive(Clone, Debug, Default, Serialize)]
pub(super) struct Snapshot {
    pub created_unix_micros: Option<u128>,
    pub point: Option<Point>,
    pub point_elapsed_micros: Option<u128>,
    pub operation_failure: Option<&'static str>,
    pub cleanup_failure: Option<&'static str>,
    pub quarantine_failure: Option<&'static str>,
    pub commit_attempted: Option<bool>,
    pub report_failure: Option<&'static str>,
    pub report_phase: Option<&'static str>,
    pub stop_confirmed: Option<bool>,
    pub content_verified: Option<bool>,
    pub stdout: Pipe,
    pub stderr: Pipe,
    pub wait_status: Option<NativeStatus>,
    pub wait_source: Option<WaitSource>,
    pub exited_before_cleanup: Option<bool>,
    pub cleanup_requested: bool,
}
#[derive(Clone)]
pub(super) struct Observation {
    state: Arc<Mutex<Snapshot>>,
    start: Instant,
}
impl Observation {
    pub(super) fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(Snapshot {
                created_unix_micros: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .ok()
                    .map(|v| v.as_micros()),
                ..Snapshot::default()
            })),
            start: Instant::now(),
        }
    }
    pub(super) fn snapshot(&self) -> Snapshot {
        self.state.lock().unwrap().clone()
    }
    pub(super) fn point(&self, point: Point) {
        let mut state = self.state.lock().unwrap();
        if state.operation_failure.is_some() {
            return;
        }
        state.point = Some(point);
        state.point_elapsed_micros = Some(self.start.elapsed().as_micros());
    }
    pub(super) fn failure(&self, boundary: SupervisorBoundary, failure: ImportFailure) {
        let mut state = self.state.lock().unwrap();
        let field = match boundary {
            SupervisorBoundary::Operation => &mut state.operation_failure,
            SupervisorBoundary::Cleanup => &mut state.cleanup_failure,
            SupervisorBoundary::Quarantine => &mut state.quarantine_failure,
        };
        field.get_or_insert(failure.diagnostic_code());
    }
    pub(super) fn report(&self, report: &CandidateImportReport) {
        use super::state::ImportPhase as P;
        let mut state = self.state.lock().unwrap();
        state.commit_attempted = Some(report.commit_attempted);
        state.report_failure = report.failure.map(ImportFailure::diagnostic_code);
        state.report_phase = Some(match report.phase {
            P::InputFrozen => "INPUT_FROZEN",
            P::SqlVerified => "SQL_VERIFIED",
            P::WriterReady => "WRITER_READY",
            P::AttemptDurable => "ATTEMPT_DURABLE",
            P::PayloadSent => "PAYLOAD_SENT",
            P::PrecommitVerified => "PRECOMMIT_VERIFIED",
            P::CommitAttempted => "COMMIT_ATTEMPTED",
            P::ImportObserved => "IMPORT_OBSERVED",
            P::Quarantined => "QUARANTINED",
            P::Cancelled => "CANCELLED",
            P::CommitUnknown => "COMMIT_UNKNOWN",
        });
        state.stop_confirmed = Some(report.stop_confirmed);
        state.content_verified = Some(report.content_verified);
    }
    pub(super) fn bytes(&self, stdout: bool, bytes: &[u8]) {
        let mut state = self.state.lock().unwrap();
        let pipe = if stdout {
            &mut state.stdout
        } else {
            &mut state.stderr
        };
        let kept = bytes.len().min(CAP.saturating_sub(pipe.bytes.len()));
        pipe.bytes.extend_from_slice(&bytes[..kept]);
        pipe.truncated |= kept != bytes.len();
    }
    pub(super) fn eof(&self, stdout: bool) {
        let mut state = self.state.lock().unwrap();
        if stdout {
            state.stdout.eof = true;
        } else {
            state.stderr.eof = true;
        }
    }
    pub(super) fn settled(&self, stdout: bool) {
        let mut state = self.state.lock().unwrap();
        if stdout {
            state.stdout.settled = true;
        } else {
            state.stderr.settled = true;
        }
    }
    pub(super) fn before_cleanup(
        &self,
        status: &Result<Option<std::process::ExitStatus>, std::io::Error>,
    ) {
        let mut state = self.state.lock().unwrap();
        state.cleanup_requested = true;
        state.exited_before_cleanup = match status {
            Ok(Some(_)) => Some(true),
            Ok(None) => Some(false),
            Err(_) => None,
        };
        if let Ok(Some(status)) = status {
            state.wait_status = Some(NativeStatus::actual(*status));
            state.wait_source = Some(WaitSource::BeforeCleanup);
        }
    }
    pub(super) fn waited(&self, status: std::process::ExitStatus, source: WaitSource) {
        let mut state = self.state.lock().unwrap();
        if state.wait_source != Some(WaitSource::BeforeCleanup) {
            state.wait_status = Some(NativeStatus::actual(status));
            state.wait_source = Some(source);
        }
    }
    #[cfg(target_os = "linux")]
    pub(super) fn persist_secondary(
        &self,
        diagnostic: &crate::full_restore::rehearsal_diagnostic::Diagnostic,
        root: Result<&learning_assets::backup_fs::BackupDir, &crate::BackupError>,
    ) {
        use crate::{BackupError, full_restore::rehearsal_diagnostic::Stage};
        let _ = diagnostic.settle(Stage::Observation, || {
            let root =
                root.map_err(|_| BackupError::Invalid("supervisor observation root unavailable"))?;
            self.persist(root)
                .map_err(|_| BackupError::Invalid("supervisor observation persistence failed"))
        });
    }
    #[cfg(target_os = "linux")]
    pub(super) fn persist(
        &self,
        root: &learning_assets::backup_fs::BackupDir,
    ) -> Result<(), crate::BackupError> {
        use sha2::{Digest, Sha256};
        use std::io::Write;
        let snapshot = self.snapshot();
        let first = snapshot
            .stdout
            .bytes
            .iter()
            .position(|b| *b == b'\n')
            .map(|n| &snapshot.stdout.bytes[..=n]);
        let metadata = serde_json::to_vec(&serde_json::json!({
            "format_version":1,"scope":"FULL_SUPERVISOR_TEST_OBSERVATION_NOT_AUTHORITY",
            "observation":snapshot,
            "stdout_size":snapshot.stdout.bytes.len(),"stdout_sha256":hex::encode(Sha256::digest(&snapshot.stdout.bytes)),
            "stderr_size":snapshot.stderr.bytes.len(),"stderr_sha256":hex::encode(Sha256::digest(&snapshot.stderr.bytes)),
            "first_line_size":first.map(<[u8]>::len),"first_line_sha256":first.map(|v|hex::encode(Sha256::digest(v)))
        }))?;
        if metadata.len() > 4096 {
            return Err(crate::BackupError::Capacity(
                "supervisor observation metadata",
            ));
        }
        for (name, bytes) in FILES.into_iter().zip([
            snapshot.stdout.bytes.as_slice(),
            snapshot.stderr.bytes.as_slice(),
            metadata.as_slice(),
        ]) {
            let mut file = root.create_file(name)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            let reopened = root.open_file(name)?;
            use std::os::unix::fs::MetadataExt;
            let m = reopened.metadata()?;
            if m.uid() != 0
                || m.mode() & 0o7777 != 0o600
                || m.nlink() != 1
                || m.len() != bytes.len() as u64
            {
                return Err(crate::BackupError::Invalid("supervisor observation file"));
            }
        }
        root.sync()?;
        Ok(())
    }
}

pub(super) struct Reader<R> {
    inner: R,
    observation: Option<Observation>,
    stdout: bool,
}
impl<R> Reader<R> {
    pub(super) fn new(inner: R, observation: Option<Observation>, stdout: bool) -> Self {
        Self {
            inner,
            observation,
            stdout,
        }
    }
}
impl<R: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for Reader<R> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let before = buffer.filled().len();
        let result = std::pin::Pin::new(&mut self.inner).poll_read(cx, buffer);
        if let std::task::Poll::Ready(Ok(())) = &result
            && let Some(observation) = &self.observation
        {
            let bytes = &buffer.filled()[before..];
            if bytes.is_empty() {
                observation.eof(self.stdout);
            } else {
                observation.bytes(self.stdout, bytes);
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::super::stream::{StreamBudget, spawn_stream};
    use super::super::{integration_tests::Model, supervise_held};
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;
    use tokio::sync::watch;
    #[tokio::test]
    async fn c3_supervisor_distinguishes_actual_protocol_predicates() {
        for (slot, point) in [
            (0, Point::FirstBlankCheck),
            (1, Point::ReadyParse),
            (2, Point::PrecommitParse),
        ] {
            let observation = Observation::new();
            let mut model = Model::new("");
            let nonce = model.expected.parts().5.hex();
            let mut lines = vec![
                b"\n".to_vec(),
                format!("KW_C4|{nonce}|READY|42|1234567|99\n").into_bytes(),
                format!("KW_C4|{nonce}|PRECOMMIT|42|1234567|99\n").into_bytes(),
            ];
            lines[slot] = b"WRONG_MARKER\n".to_vec();
            model.c3_lines = Some(lines);
            model.c3_observation = Some(observation.clone());
            let (_sender, rx) = watch::channel(false);
            let report = supervise_held(&mut model, rx, Arc::new(AtomicBool::new(false))).await;
            assert_eq!(report.failure, Some(ImportFailure::Protocol));
            let snapshot = observation.snapshot();
            assert_eq!(snapshot.point, Some(point));
            assert_eq!(snapshot.operation_failure, Some("PROTOCOL"));
            assert_eq!(snapshot.commit_attempted, Some(false));
        }
    }
    #[tokio::test]
    async fn c3_supervisor_keeps_preupgrade_protocol_and_actual_commit_attempt() {
        let observation = Observation::new();
        let mut model = Model::new("");
        let nonce = model.expected.parts().5.hex();
        model.c3_lines = Some(vec![
            b"\n".to_vec(),
            format!("KW_C4|{nonce}|READY|42|1234567|99\n").into_bytes(),
            format!("KW_C4|{nonce}|PRECOMMIT|42|1234567|99\n").into_bytes(),
            b"WRONG\n".to_vec(),
        ]);
        model.c3_observation = Some(observation.clone());
        let (_sender, rx) = watch::channel(false);
        let report = supervise_held(&mut model, rx, Arc::new(AtomicBool::new(false))).await;
        assert_eq!(report.failure, Some(ImportFailure::CommitUnknown));
        assert!(report.commit_attempted);
        let snapshot = observation.snapshot();
        assert_eq!(snapshot.point, Some(Point::ConfirmationParse));
        assert_eq!(snapshot.operation_failure, Some("PROTOCOL"));
        assert_eq!(snapshot.report_failure, Some("COMMIT_UNKNOWN"));
        assert_eq!(snapshot.commit_attempted, Some(true));
    }
    #[tokio::test]
    async fn c3_supervisor_eof_and_none_keep_original_failure_and_order() {
        let observation = Observation::new();
        let mut observed = Model::new("");
        observed.c3_lines = Some(vec![b"\n".to_vec()]);
        observed.c3_observation = Some(observation.clone());
        let events = observed.events.clone();
        let (_sender, rx) = watch::channel(false);
        let report = supervise_held(&mut observed, rx, Arc::new(AtomicBool::new(false))).await;
        assert_eq!(report.failure, Some(ImportFailure::Protocol));
        assert_eq!(observation.snapshot().point, Some(Point::ReadyRead));
        let before = events.lock().unwrap().clone();
        let mut plain = Model::new("");
        plain.c3_lines = Some(vec![b"\n".to_vec()]);
        let (_sender, rx) = watch::channel(false);
        let other = supervise_held(&mut plain, rx, Arc::new(AtomicBool::new(false))).await;
        assert_eq!(other.failure, report.failure);
        assert_eq!(plain.events.lock().unwrap().as_slice(), before.as_slice());
    }

    fn child(mode: &str, observation: Option<Observation>) -> super::super::stream::StreamOwner {
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        command.args(["--exact","restore_preflight::target_binding::controlled_import::stream::tests::process_fixture","--nocapture"]).env_clear().env("C4_STREAM_FIXTURE",mode);
        let budget = StreamBudget::full_writer(Instant::now() + Duration::from_secs(5));
        let budget = observation.map_or_else(
            || StreamBudget::full_writer(Instant::now() + Duration::from_secs(5)),
            |value| budget.observed(value),
        );
        spawn_stream(command, budget).unwrap()
    }
    #[tokio::test]
    async fn c3_actual_pipe_and_natural_wait_are_preserved() {
        let observation = Observation::new();
        let mut owner = child("nonzero", Some(observation.clone()));
        assert_eq!(owner.finish().await, Err(ImportFailure::Exit));
        let snapshot = observation.snapshot();
        assert!(snapshot.stdout.bytes.windows(9).any(|v| v == b"TERMINAL\n"));
        assert!(
            snapshot.stdout.eof
                && snapshot.stderr.eof
                && snapshot.stdout.settled
                && snapshot.stderr.settled
        );
        assert_eq!(snapshot.wait_source, Some(WaitSource::Natural));
        assert_eq!(snapshot.wait_status.unwrap().code, Some(7));
    }
    #[tokio::test]
    async fn c3_actual_pipe_caps_still_refuse_and_capture_is_truncated() {
        for (mode, failure, stdout) in [
            ("stdout", ImportFailure::StdoutLimit, true),
            ("stderr", ImportFailure::StderrLimit, false),
        ] {
            let observation = Observation::new();
            let mut owner = child(mode, Some(observation.clone()));
            assert_eq!(owner.finish().await, Err(failure));
            let snapshot = observation.snapshot();
            let pipe = if stdout {
                snapshot.stdout
            } else {
                snapshot.stderr
            };
            assert_eq!(pipe.bytes.len(), CAP);
            assert!(pipe.truncated && pipe.settled);
        }
    }
    #[tokio::test]
    async fn c3_cleanup_origin_and_none_path_remain_distinct() {
        let observation = Observation::new();
        let mut owner = child("stdout_block", Some(observation.clone()));
        let deadline = Instant::now() + Duration::from_secs(2);
        while observation.snapshot().stdout.bytes.len() < 4096 && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        owner.kill_and_wait().await.unwrap();
        let snapshot = observation.snapshot();
        assert_eq!(snapshot.exited_before_cleanup, Some(false));
        assert_eq!(snapshot.wait_source, Some(WaitSource::Cleanup));
        assert!(snapshot.stdout.settled && snapshot.stderr.settled);
        let mut plain = child("nonzero", None);
        assert_eq!(plain.finish().await, Err(ImportFailure::Exit));
    }
    #[cfg(target_os = "linux")]
    fn private_case() -> (std::path::PathBuf, learning_assets::backup_fs::BackupDir) {
        use std::os::unix::fs::PermissionsExt;
        let parent = std::path::PathBuf::from(
            std::env::var_os("TEST_C4_C3_OBSERVATION_ROOT")
                .expect("fresh explicitly supplied private root"),
        );
        let path = parent.join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let root = learning_assets::backup_fs::BackupDir::open_trusted_private_root(&path).unwrap();
        (path, root)
    }
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "fresh Root-owned Linux observation fixture required"]
    fn c3_current_case_create_new_and_nofollow_refuse() {
        use std::{
            io::Read,
            os::unix::fs::{MetadataExt, symlink},
        };
        let (_, root) = private_case();
        let observation = Observation::new();
        observation.bytes(true, &vec![b'O'; CAP + 1]);
        observation.bytes(false, &vec![b'E'; CAP + 1]);
        observation.persist(&root).unwrap();
        assert!(observation.persist(&root).is_err());
        for (name, limit) in FILES.into_iter().zip([CAP, CAP, 4096]) {
            let file = root.open_file(name).unwrap();
            let metadata = file.metadata().unwrap();
            assert!(metadata.len() <= limit as u64);
            assert_eq!(metadata.uid(), 0);
            assert_eq!(metadata.mode() & 0o7777, 0o600);
            assert_eq!(metadata.nlink(), 1);
        }
        let (path, other) = private_case();
        let mut sentinel = other.create_file("sentinel").unwrap();
        use std::io::Write;
        sentinel.write_all(b"KEEP").unwrap();
        symlink("sentinel", path.join(FILES[0])).unwrap();
        assert!(observation.persist(&other).is_err());
        let mut bytes = Vec::new();
        other
            .open_file("sentinel")
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(bytes, b"KEEP");
    }
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "fresh Root-owned Linux observation fixture required"]
    fn c3_storage_failure_cannot_change_original_report() {
        use crate::full_restore::rehearsal_diagnostic::{Diagnostic, Stage};
        let (_, root) = private_case();
        let observation = Observation::new();
        let mut occupied = root.create_file(FILES[0]).unwrap();
        use std::io::Write;
        occupied.write_all(b"KEEP").unwrap();
        let report = CandidateImportReport {
            phase: super::super::state::ImportPhase::Quarantined,
            failure: Some(ImportFailure::CommitUnknown),
            stop_confirmed: true,
            content_verified: false,
            commit_attempted: true,
        };
        observation.point(Point::ConfirmationParse);
        observation.failure(SupervisorBoundary::Operation, ImportFailure::Protocol);
        observation.report(&report);
        let diagnostic = Diagnostic::new();
        diagnostic.supervisor_failure(Stage::Supervisor, "PROTOCOL", false);
        observation.persist_secondary(&diagnostic, Ok(&root));
        assert!(diagnostic.require_settlement().is_err());
        let mut output = Vec::new();
        assert!(
            diagnostic
                .public_result_to(
                    Err::<(), _>(crate::BackupError::Invalid(
                        "restore commit unknown; unusable; no replay"
                    )),
                    &mut output
                )
                .is_err()
        );
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "FULL_REHEARSAL_FAILURE kind=PRIMARY stage=SUPERVISOR class=IMPORT_FAILURE code=PROTOCOL\nFULL_REHEARSAL_FAILURE kind=SECONDARY stage=OBSERVATION class=INVALID code=UNKNOWN\n"
        );
        let snapshot = observation.snapshot();
        assert_eq!(snapshot.operation_failure, Some("PROTOCOL"));
        assert_eq!(snapshot.report_failure, Some("COMMIT_UNKNOWN"));
        assert_eq!(snapshot.commit_attempted, Some(true));
        assert_eq!(report.failure, Some(ImportFailure::CommitUnknown));
    }
}

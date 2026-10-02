//! Private controlled-fixture composition. No product restore authority.
mod candidate_attempt;
mod commands;
#[cfg(test)]
mod diagnostics;
mod fixture_sql;
#[cfg(all(test, target_os = "linux"))]
mod linux;
#[cfg(all(test, target_os = "linux"))]
mod live_tests;
mod protocol;
mod state;
mod stream;
mod writer_sql;

use candidate_attempt::{DurableAttempt, DurableCommitIntent};
use fixture_sql::VerifiedFixtureSql;
use protocol::{WriterExpected, WriterIdentity};
use state::ImportPhase;
use std::future::Future;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::{sync::watch, task::JoinHandle};

#[derive(Debug)]
struct CandidateImportReport {
    phase: ImportPhase,
    failure: Option<ImportFailure>,
    stop_confirmed: bool,
    content_verified: bool,
    commit_attempted: bool,
}

struct CandidateImportHandle {
    cancel: watch::Sender<bool>,
    task: Option<JoinHandle<CandidateImportReport>>,
    commit_started: Arc<AtomicBool>,
}
impl CandidateImportHandle {
    fn cancel(&self) {
        self.cancel.send_replace(true);
    }
    async fn wait(mut self) -> CandidateImportReport {
        self.task
            .take()
            .unwrap()
            .await
            .unwrap_or_else(|_| CandidateImportReport {
                phase: if self.commit_started.load(Ordering::Acquire) {
                    ImportPhase::CommitUnknown
                } else {
                    ImportPhase::Cancelled
                },
                failure: Some(ImportFailure::UnconfirmedIsolation),
                stop_confirmed: false,
                content_verified: false,
                commit_attempted: self.commit_started.load(Ordering::Acquire),
            })
    }
}
impl Drop for CandidateImportHandle {
    fn drop(&mut self) {
        self.cancel();
    }
}

trait AdmissionQuarantine: Send + 'static {
    fn quarantine(&mut self) -> impl Future<Output = ()> + Send;
}

struct AdmissionHandoff<T: AdmissionQuarantine> {
    admission: Option<T>,
    runtime: tokio::runtime::Handle,
}
impl<T: AdmissionQuarantine> AdmissionHandoff<T> {
    fn take(mut self) -> T {
        self.admission.take().unwrap()
    }
}
impl<T: AdmissionQuarantine> Drop for AdmissionHandoff<T> {
    fn drop(&mut self) {
        if let Some(mut admission) = self.admission.take() {
            // The envelope owns authority even after send succeeds. A queued
            // value's destruction transfers it to exactly one cleanup owner.
            // Use the originating runtime even outside an entered context.
            // Actual runtime shutdown cannot promise isolation or a receipt.
            self.runtime.spawn(async move {
                admission.quarantine().await;
            });
        }
    }
}

fn handoff_admission<T: AdmissionQuarantine>(
    sender: tokio::sync::oneshot::Sender<Result<AdmissionHandoff<T>, ImportFailure>>,
    result: Result<T, ImportFailure>,
) -> bool {
    sender
        .send(result.map(|admission| AdmissionHandoff {
            admission: Some(admission),
            runtime: tokio::runtime::Handle::current(),
        }))
        .is_ok()
}

// The adapter owns all authority and the sole StreamOwner. External effects are
// injected below the ordering algorithm; opaque journal permits stay concrete.
trait CandidateIo: Send {
    fn expected(&self) -> &WriterExpected;
    fn prepare(&mut self)
    -> impl Future<Output = Result<VerifiedFixtureSql, ImportFailure>> + Send;
    fn start(&mut self, header: Vec<u8>) -> impl Future<Output = Result<(), ImportFailure>> + Send;
    fn line(&mut self) -> impl Future<Output = Result<Vec<u8>, ImportFailure>> + Send;
    fn recheck(&mut self) -> impl Future<Output = Result<(), ImportFailure>> + Send;
    fn attempt(
        &mut self,
        sql: &VerifiedFixtureSql,
        writer: WriterIdentity,
    ) -> impl Future<Output = Result<DurableAttempt, ImportFailure>> + Send;
    fn send(&mut self, bytes: &[u8]) -> impl Future<Output = Result<(), ImportFailure>> + Send;
    fn intent(
        &mut self,
        attempt: DurableAttempt,
        writer: WriterIdentity,
    ) -> impl Future<Output = Result<DurableCommitIntent, ImportFailure>> + Send;
    fn finish(&mut self) -> impl Future<Output = Result<(), ImportFailure>> + Send;
    fn cleanup(&mut self) -> impl Future<Output = Result<(), ImportFailure>> + Send;
    fn readback(
        &mut self,
        writer: WriterIdentity,
        committed: bool,
    ) -> impl Future<Output = bool> + Send;
    fn quarantine(&mut self) -> impl Future<Output = bool> + Send;
}

fn spawn_candidate(io: impl CandidateIo + 'static) -> CandidateImportHandle {
    let (cancel, rx) = watch::channel(false);
    let commit_started = Arc::new(AtomicBool::new(false));
    let task = tokio::spawn(supervise(io, rx, commit_started.clone()));
    CandidateImportHandle {
        cancel,
        task: Some(task),
        commit_started,
    }
}

fn gate(cancel: &watch::Receiver<bool>, deadline: std::time::Instant) -> Result<(), ImportFailure> {
    if *cancel.borrow() {
        Err(ImportFailure::Cancelled)
    } else if std::time::Instant::now() >= deadline {
        Err(ImportFailure::Deadline)
    } else {
        Ok(())
    }
}

async fn step<T>(
    cancel: &mut watch::Receiver<bool>,
    deadline: std::time::Instant,
    future: impl Future<Output = Result<T, ImportFailure>>,
) -> Result<T, ImportFailure> {
    gate(cancel, deadline)?;
    tokio::select! {
        biased;
        _ = cancel.changed() => Err(ImportFailure::Cancelled),
        _ = tokio::time::sleep_until(deadline.into()) => Err(ImportFailure::Deadline),
        result = future => { gate(cancel, deadline)?; result }
    }
}

async fn send_commit(
    cancel: &mut watch::Receiver<bool>,
    deadline: std::time::Instant,
    machine: &mut state::ImportMachine,
    attempted: &mut bool,
    diagnostic: &AtomicBool,
    send: impl Future<Output = Result<(), ImportFailure>>,
) -> Result<(), ImportFailure> {
    step(cancel, deadline, async {
        // This body is polled only after step's rejection gates. The send is
        // polled immediately after the monotone boundary in this same poll.
        machine.accept(state::ImportEvent::CommitPermitRequested)?;
        *attempted = true;
        diagnostic.store(true, Ordering::Release);
        send.await
    })
    .await
}

async fn settle_finish(
    cancel: &mut watch::Receiver<bool>,
    deadline: std::time::Instant,
    finish: impl Future<Output = Result<(), ImportFailure>>,
) -> Result<(), ImportFailure> {
    // StreamOwner owns the fixed absolute deadline. Once finish is polled it
    // must settle its child/readers; external rejection is latched, not used
    // to drop the future that owns their JoinHandles.
    let result = finish.await;
    gate(cancel, deadline)?;
    result
}

async fn supervise(
    mut io: impl CandidateIo,
    mut cancel: watch::Receiver<bool>,
    commit_started: Arc<AtomicBool>,
) -> CandidateImportReport {
    use protocol::{WriterEvent, parse_writer_line};
    use state::{ImportEvent as E, ImportMachine};
    use std::time::{Duration, Instant};
    let mut machine = ImportMachine::new();
    let mut writer = None;
    let mut commit_attempted = false;
    let result = async {
        let sql = step(
            &mut cancel,
            Instant::now() + Duration::from_secs(15),
            io.prepare(),
        )
        .await?;
        machine.accept(E::SqlVerified)?;
        let deadline = Instant::now() + Duration::from_secs(45);
        step(&mut cancel, deadline, io.recheck()).await?;
        let header = writer_sql::prelude(io.expected(), &sql)?;
        step(&mut cancel, deadline, io.start(header)).await?;
        let ready_deadline = deadline.min(Instant::now() + Duration::from_secs(10));
        if step(&mut cancel, ready_deadline, io.line()).await? != b"\n" {
            return Err(ImportFailure::Protocol);
        }
        machine.accept(E::HeaderBlank)?;
        let nonce = io.expected().parts().5;
        let line = step(&mut cancel, ready_deadline, io.line()).await?;
        let WriterEvent::Ready(id) = parse_writer_line(&line, &nonce)? else {
            return Err(ImportFailure::Protocol);
        };
        if id.parts().0 == io.expected().parts().3 {
            return Err(ImportFailure::Identity);
        }
        writer = Some(id);
        machine.accept(E::Ready(id))?;
        step(&mut cancel, deadline, io.recheck()).await?;
        // A durable operation is never abandoned on cancellation/deadline. Its
        // late result is settled here while this task still owns both guards.
        let attempt = io.attempt(&sql, id).await;
        gate(&cancel, deadline)?;
        let attempt = attempt?;
        machine.accept(E::AttemptSynced)?;
        step(&mut cancel, deadline, io.recheck()).await?;
        step(&mut cancel, deadline, io.send(sql.payload())).await?;
        machine.accept(E::PayloadSent)?;
        let post = writer_sql::postcheck(io.expected(), &id);
        step(&mut cancel, deadline, io.send(&post)).await?;
        let line = step(
            &mut cancel,
            deadline.min(Instant::now() + Duration::from_secs(10)),
            io.line(),
        )
        .await?;
        let WriterEvent::Precommit(same) = parse_writer_line(&line, &nonce)? else {
            return Err(ImportFailure::Protocol);
        };
        machine.accept(E::Precommit(same))?;
        step(&mut cancel, deadline, io.recheck()).await?;
        let intent = io.intent(attempt, id).await;
        gate(&cancel, deadline)?;
        let _intent = intent?;
        machine.accept(E::CommitIntentSynced)?;
        step(&mut cancel, deadline, io.recheck()).await?;
        gate(&cancel, deadline)?;
        machine.accept(E::FinalReviewPassed)?;
        send_commit(
            &mut cancel,
            deadline,
            &mut machine,
            &mut commit_attempted,
            &commit_started,
            io.send(b"COMMIT;\n"),
        )
        .await?;
        let confirmation = writer_sql::commit_confirmation(&nonce);
        step(&mut cancel, deadline, io.send(&confirmation)).await?;
        let line = step(&mut cancel, deadline, io.line()).await?;
        if parse_writer_line(&line, &nonce)? != WriterEvent::Committed {
            return Err(ImportFailure::Protocol);
        }
        machine.accept(E::Committed)?;
        settle_finish(&mut cancel, deadline, io.finish()).await?;
        Ok(())
    }
    .await;
    let mut failure = result.err();
    if failure.is_some() && commit_attempted {
        failure = Some(ImportFailure::CommitUnknown);
        let _ = machine.accept(E::CommitSendFailed);
    }
    if failure == Some(ImportFailure::Cancelled) {
        let _ = machine.accept(E::Cancelled);
    }
    // No blind ROLLBACK: a cancelled send may have left COPY/SQL incomplete.
    if io.cleanup().await.is_err() && failure.is_none() {
        failure = Some(ImportFailure::Io);
    }
    let content_verified = if let Some(writer) = writer {
        if !commit_attempted || failure.is_none() {
            io.readback(writer, commit_attempted).await
        } else {
            false
        }
    } else {
        false
    };
    if failure.is_none() && !content_verified {
        failure = Some(ImportFailure::Fixture);
    }
    if failure.is_none() {
        let _ = machine.accept(E::ImportObserved);
    }
    let stop_confirmed = io.quarantine().await;
    if stop_confirmed {
        let _ = machine.accept(E::Quarantined);
    } else {
        failure = Some(ImportFailure::UnconfirmedIsolation);
    }
    CandidateImportReport {
        phase: machine.phase(),
        failure,
        stop_confirmed,
        content_verified,
        commit_attempted,
    }
}

#[allow(dead_code)] // Shared failure vocabulary for the subsequent private tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportFailure {
    Identity,
    Session,
    Version,
    Protocol,
    Fixture,
    InputLimit,
    Deadline,
    StdoutLimit,
    StderrLimit,
    Stderr,
    Exit,
    Io,
    Journal,
    Cancelled,
    CommitUnknown,
    UnconfirmedIsolation,
}

#[cfg(test)]
mod sibling_contract_usage {
    use super::commands::FixedImportCommand;
    use super::protocol::{Nonce, WriterEvent, WriterExpected, WriterIdentity, parse_writer_line};

    #[test]
    fn enclosing_private_module_can_consume_command_and_receipt_contracts() {
        let id = "a".repeat(64);
        let db = "learning_restore_c4_2b8a1252-54d5-48aa-b176-a9586a86bea3";
        assert!(!FixedImportCommand::decoder(&id).unwrap().argv().is_empty());
        assert!(
            !FixedImportCommand::writer(&id, db)
                .unwrap()
                .argv()
                .is_empty()
        );
        let nonce = Nonce::random().unwrap();
        let receipt = format!("KW_C4|{}|READY|42|1234567|99\n", nonce.hex());
        let ready = parse_writer_line(receipt.as_bytes(), &nonce).unwrap();
        let WriterEvent::Ready(identity): WriterEvent = ready else {
            panic!("READY required")
        };
        let identity_as_contract = |value: WriterIdentity| value;
        let precommit = format!("KW_C4|{}|PRECOMMIT|42|1234567|99\n", nonce.hex());
        assert_eq!(
            parse_writer_line(precommit.as_bytes(), &nonce),
            Ok(WriterEvent::Precommit(identity_as_contract(identity)))
        );
        let _: Option<WriterExpected> = None; // Type reachability, without exposing fields.
    }
}

#[cfg(test)]
mod integration_tests {
    use super::super::ChallengeKeys;
    use super::*;
    use candidate_attempt::{
        CandidateAttemptContext, JournalIo, persist_attempt_with_io, persist_commit_intent_with_io,
    };
    use learning_assets::backup_fs::BackupEntryKind;
    use std::{
        collections::BTreeMap,
        io::{self, Write},
        sync::{Arc, Mutex},
    };
    use tokio::sync::Notify;
    const DB: &str = "learning_restore_c4_2b8a1252-54d5-48aa-b176-a9586a86bea3";
    type Entries = Arc<Mutex<BTreeMap<String, Arc<Mutex<Vec<u8>>>>>>;
    #[derive(Default)]
    struct ModelDir(Entries);
    struct ModelFile(Arc<Mutex<Vec<u8>>>);
    impl Write for ModelFile {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl JournalIo for ModelDir {
        type File = ModelFile;
        type Reader = io::Cursor<Vec<u8>>;
        fn identity(&self) -> io::Result<(u64, u64)> {
            Ok((1, 2))
        }
        fn kind(&self, name: &str) -> io::Result<BackupEntryKind> {
            if self.0.lock().unwrap().contains_key(name) {
                Ok(BackupEntryKind::File)
            } else {
                Err(io::ErrorKind::NotFound.into())
            }
        }
        fn create_file(&self, name: &str) -> io::Result<Self::File> {
            let mut entries = self.0.lock().unwrap();
            if entries.contains_key(name) {
                return Err(io::ErrorKind::AlreadyExists.into());
            }
            let bytes = Arc::new(Mutex::new(Vec::new()));
            entries.insert(name.into(), bytes.clone());
            Ok(ModelFile(bytes))
        }
        fn open_file(&self, name: &str) -> io::Result<Self::Reader> {
            let entries = self.0.lock().unwrap();
            let bytes = entries.get(name).ok_or(io::ErrorKind::NotFound)?;
            Ok(io::Cursor::new(bytes.lock().unwrap().clone()))
        }
        fn sync_file(&self, _: &Self::File) -> io::Result<()> {
            Ok(())
        }
        fn sync_dir(&self) -> io::Result<()> {
            Ok(())
        }
    }
    struct Model {
        expected: WriterExpected,
        events: Arc<Mutex<Vec<&'static str>>>,
        fail: &'static str,
        dir: ModelDir,
        lines: usize,
        reviews: usize,
        attempt_entered: Arc<Notify>,
        attempt_release: Option<Arc<Notify>>,
        stop_entered: Arc<Notify>,
        stop_release: Option<Arc<Notify>>,
    }
    impl Model {
        fn new(fail: &'static str) -> Self {
            Self {
                expected: WriterExpected::new(
                    DB.into(),
                    16384,
                    "1234",
                    10,
                    ChallengeKeys::for_test(1, -1),
                    protocol::Nonce::random().unwrap(),
                )
                .unwrap(),
                events: Arc::default(),
                fail,
                dir: ModelDir::default(),
                lines: 0,
                reviews: 0,
                attempt_entered: Arc::default(),
                attempt_release: None,
                stop_entered: Arc::default(),
                stop_release: None,
            }
        }
        fn step(&self, name: &'static str) -> Result<(), ImportFailure> {
            self.events.lock().unwrap().push(name);
            if self.fail == name {
                Err(if name == "attempt" || name == "intent" {
                    ImportFailure::Journal
                } else {
                    ImportFailure::Identity
                })
            } else {
                Ok(())
            }
        }
    }
    impl Drop for Model {
        fn drop(&mut self) {
            self.events.lock().unwrap().push("guards_drop");
        }
    }
    impl CandidateIo for Model {
        fn expected(&self) -> &WriterExpected {
            &self.expected
        }
        async fn prepare(&mut self) -> Result<VerifiedFixtureSql, ImportFailure> {
            self.step("prepare")?;
            let sql =
                include_str!("../../../tests/fixtures/c4-controlled-import/pg18-fixture.sql.in")
                    .replace("{{RESTRICT_KEY}}", "ModelKey77");
            fixture_sql::verify_fixture_sql(sql.as_bytes())
        }
        async fn start(&mut self, _: Vec<u8>) -> Result<(), ImportFailure> {
            self.step("start")
        }
        async fn line(&mut self) -> Result<Vec<u8>, ImportFailure> {
            let stage = ["blank", "READY", "PRECOMMIT", "COMMITTED"][self.lines];
            self.lines += 1;
            self.step(stage)?;
            Ok(match stage {
                "blank" => b"\n".to_vec(),
                "COMMITTED" => {
                    format!("KW_C4|{}|COMMITTED\n", self.expected.parts().5.hex()).into_bytes()
                }
                _ => format!(
                    "KW_C4|{}|{stage}|42|1234567|99\n",
                    self.expected.parts().5.hex()
                )
                .into_bytes(),
            })
        }
        async fn recheck(&mut self) -> Result<(), ImportFailure> {
            self.reviews += 1;
            self.step(match self.reviews {
                1 => "before_writer",
                2 => "before_attempt",
                3 => "before_payload",
                4 => "before_intent",
                _ => "final_review",
            })
        }
        async fn attempt(
            &mut self,
            sql: &VerifiedFixtureSql,
            writer: WriterIdentity,
        ) -> Result<DurableAttempt, ImportFailure> {
            self.attempt_entered.notify_one();
            if let Some(release) = &self.attempt_release {
                release.notified().await;
            }
            self.step("attempt")?;
            let context = CandidateAttemptContext::new(
                uuid::Uuid::new_v4(),
                DB.into(),
                [1; 32],
                [2; 32],
                [3; 32],
                sql.raw_sha256(),
                sql.transformed_sha256(),
                writer,
            )?;
            persist_attempt_with_io(&self.dir, &context)
        }
        async fn send(&mut self, bytes: &[u8]) -> Result<(), ImportFailure> {
            let stage = if bytes == b"COMMIT;\n" {
                "commit"
            } else if bytes.starts_with(b"SELECT 'KW_C4|") {
                "confirmation"
            } else if bytes.starts_with(b"DO $kw$") {
                "postcheck"
            } else {
                "payload"
            };
            assert!(
                !(self.fail == "panic_after_commit" && stage == "confirmation"),
                "injected supervisor panic"
            );
            self.step(stage)
        }
        async fn intent(
            &mut self,
            attempt: DurableAttempt,
            writer: WriterIdentity,
        ) -> Result<DurableCommitIntent, ImportFailure> {
            self.step("intent")?;
            persist_commit_intent_with_io(&self.dir, &attempt, &writer)
        }
        async fn finish(&mut self) -> Result<(), ImportFailure> {
            self.step("finish")
        }
        async fn cleanup(&mut self) -> Result<(), ImportFailure> {
            self.step("cleanup")
        }
        async fn readback(&mut self, _: WriterIdentity, committed: bool) -> bool {
            self.step(if committed {
                "read_committed"
            } else {
                "read_zero"
            })
            .is_ok()
        }
        async fn quarantine(&mut self) -> bool {
            self.stop_entered.notify_one();
            if let Some(release) = &self.stop_release {
                release.notified().await;
            }
            self.step("quarantine").is_ok()
        }
    }
    #[tokio::test]
    async fn no_payload_before_attempt_sync() {
        let model = Model::new("attempt");
        let events = model.events.clone();
        let report = spawn_candidate(model).wait().await;
        assert_eq!(report.failure, Some(ImportFailure::Journal));
        let events = events.lock().unwrap();
        assert!(events.contains(&"attempt"));
        assert!(!events.contains(&"payload"));
        assert!(!events.contains(&"commit"));
        assert_eq!(&events[events.len() - 2..], &["quarantine", "guards_drop"]);
    }
    #[tokio::test]
    async fn no_commit_before_intent_and_final_recheck() {
        for fail in ["intent", "final_review"] {
            let model = Model::new(fail);
            let events = model.events.clone();
            let report = spawn_candidate(model).wait().await;
            assert_eq!(
                report.failure,
                Some(if fail == "intent" {
                    ImportFailure::Journal
                } else {
                    ImportFailure::Identity
                })
            );
            assert!(!events.lock().unwrap().contains(&"commit"));
            assert!(!report.commit_attempted);
        }
        let model = Model::new("");
        let events = model.events.clone();
        let report = spawn_candidate(model).wait().await;
        assert!(report.content_verified && report.commit_attempted && report.stop_confirmed);
        assert_eq!(report.failure, None);
        assert_eq!(report.phase, ImportPhase::Quarantined);
        assert_eq!(
            *events.lock().unwrap(),
            vec![
                "prepare",
                "before_writer",
                "start",
                "blank",
                "READY",
                "before_attempt",
                "attempt",
                "before_payload",
                "payload",
                "postcheck",
                "PRECOMMIT",
                "before_intent",
                "intent",
                "final_review",
                "commit",
                "confirmation",
                "COMMITTED",
                "finish",
                "cleanup",
                "read_committed",
                "quarantine",
                "guards_drop"
            ]
        );
    }
    #[tokio::test]
    async fn cancel_keeps_guards_until_quarantine() {
        let mut model = Model::new("");
        let events = model.events.clone();
        let attempt = model.attempt_entered.clone();
        let release = Arc::new(Notify::new());
        model.attempt_release = Some(release.clone());
        let stop = model.stop_entered.clone();
        let stop_release = Arc::new(Notify::new());
        model.stop_release = Some(stop_release.clone());
        let handle = spawn_candidate(model);
        tokio::time::timeout(std::time::Duration::from_secs(2), attempt.notified())
            .await
            .unwrap();
        handle.cancel();
        release.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(2), stop.notified())
            .await
            .unwrap();
        assert!(!events.lock().unwrap().contains(&"guards_drop"));
        assert!(!events.lock().unwrap().contains(&"payload"));
        stop_release.notify_one();
        let report = handle.wait().await;
        assert_eq!(report.failure, Some(ImportFailure::Cancelled));
        assert_eq!(events.lock().unwrap().last(), Some(&"guards_drop"));
    }

    #[tokio::test]
    async fn cancelled_durable_failure_cannot_replace_accepted_cancellation() {
        let mut model = Model::new("attempt");
        let entered = model.attempt_entered.clone();
        let release = Arc::new(Notify::new());
        model.attempt_release = Some(release.clone());
        let handle = spawn_candidate(model);
        entered.notified().await;
        handle.cancel();
        release.notify_one();
        assert_eq!(handle.wait().await.failure, Some(ImportFailure::Cancelled));
    }

    #[tokio::test]
    async fn dropped_handle_cancels_but_does_not_drop_supervisor_authority() {
        let mut model = Model::new("");
        let events = model.events.clone();
        let entered = model.attempt_entered.clone();
        let release = Arc::new(Notify::new());
        model.attempt_release = Some(release.clone());
        let stop = model.stop_entered.clone();
        let stop_release = Arc::new(Notify::new());
        model.stop_release = Some(stop_release.clone());
        let handle = spawn_candidate(model);
        entered.notified().await;
        drop(handle);
        release.notify_one();
        stop.notified().await;
        assert!(!events.lock().unwrap().contains(&"guards_drop"));
        assert!(!events.lock().unwrap().contains(&"payload"));
        stop_release.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while !events.lock().unwrap().contains(&"guards_drop") {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn missing_commit_proof_is_unknown_and_quarantine_failure_dominates() {
        for fail in ["commit", "confirmation", "COMMITTED", "finish"] {
            let model = Model::new(fail);
            let events = model.events.clone();
            let report = spawn_candidate(model).wait().await;
            assert_eq!(report.failure, Some(ImportFailure::CommitUnknown));
            assert!(report.commit_attempted && report.stop_confirmed);
            assert!(!report.content_verified);
            assert!(!events.lock().unwrap().contains(&"read_zero"));
            assert!(!events.lock().unwrap().contains(&"read_committed"));
        }
        let report = spawn_candidate(Model::new("quarantine")).wait().await;
        assert_eq!(report.failure, Some(ImportFailure::UnconfirmedIsolation));
        assert!(!report.stop_confirmed);
    }

    #[tokio::test]
    async fn cleanup_failure_cannot_report_import_success() {
        let report = spawn_candidate(Model::new("cleanup")).wait().await;
        assert_eq!(report.failure, Some(ImportFailure::Io));
        assert!(!report.content_verified);
    }

    #[tokio::test]
    async fn supervisor_panic_cannot_erase_commit_attempt() {
        let report = spawn_candidate(Model::new("panic_after_commit"))
            .wait()
            .await;
        assert_eq!(report.failure, Some(ImportFailure::UnconfirmedIsolation));
        assert!(report.commit_attempted);
        assert!(!report.stop_confirmed);
    }
}

#[cfg(test)]
mod fix1_boundary_tests {
    use super::*;
    use std::time::{Duration, Instant};
    fn final_reviewed() -> state::ImportMachine {
        use state::ImportEvent as E;
        let nonce = protocol::Nonce::random().unwrap();
        let protocol::WriterEvent::Ready(id) = protocol::parse_writer_line(
            format!("KW_C4|{}|READY|42|1234567|99\n", nonce.hex()).as_bytes(),
            &nonce,
        )
        .unwrap() else {
            panic!("ready")
        };
        let mut machine = state::ImportMachine::new();
        for event in [
            E::SqlVerified,
            E::HeaderBlank,
            E::Ready(id),
            E::AttemptSynced,
            E::PayloadSent,
            E::Precommit(id),
            E::CommitIntentSynced,
            E::FinalReviewPassed,
        ] {
            machine.accept(event).unwrap();
        }
        machine
    }
    pub(super) async fn assert_unpolled_commit_is_not_attempted(cancelled: bool) {
        {
            let (_tx, mut rx) = watch::channel(cancelled);
            let deadline = if cancelled {
                Instant::now() + Duration::from_secs(1)
            } else {
                Instant::now() - Duration::from_millis(1)
            };
            let mut machine = final_reviewed();
            let mut attempted = false;
            let diagnostic = AtomicBool::new(false);
            let polled = AtomicBool::new(false);
            let result = send_commit(
                &mut rx,
                deadline,
                &mut machine,
                &mut attempted,
                &diagnostic,
                async {
                    polled.store(true, Ordering::Release);
                    Ok(())
                },
            )
            .await;
            assert_eq!(
                result,
                Err(if cancelled {
                    ImportFailure::Cancelled
                } else {
                    ImportFailure::Deadline
                })
            );
            assert!(!polled.load(Ordering::Acquire));
            assert!(
                !attempted && !diagnostic.load(Ordering::Acquire),
                "unpolled send crossed commit boundary"
            );
            assert_eq!(machine.phase(), ImportPhase::PrecommitVerified);
        }
    }
    #[tokio::test]
    async fn first_send_poll_marks_attempt_even_when_partial_write_fails() {
        let (_tx, mut rx) = watch::channel(false);
        let mut machine = final_reviewed();
        let mut attempted = false;
        let diagnostic = AtomicBool::new(false);
        let result = send_commit(
            &mut rx,
            Instant::now() + Duration::from_secs(1),
            &mut machine,
            &mut attempted,
            &diagnostic,
            async {
                assert!(diagnostic.load(Ordering::Acquire));
                Err(ImportFailure::Io)
            },
        )
        .await;
        assert_eq!(result, Err(ImportFailure::Io));
        assert!(attempted && diagnostic.load(Ordering::Acquire));
        assert_eq!(machine.phase(), ImportPhase::CommitAttempted);
    }
}

#[cfg(test)]
mod fix1_unpolled_tests {
    #[tokio::test]
    async fn cancelled_commit_with_no_send_poll_is_not_attempted() {
        super::fix1_boundary_tests::assert_unpolled_commit_is_not_attempted(true).await;
    }
    #[tokio::test]
    async fn expired_commit_with_no_send_poll_is_not_attempted() {
        super::fix1_boundary_tests::assert_unpolled_commit_is_not_attempted(false).await;
    }
}

#[cfg(test)]
mod fix1_admission_tests {
    use super::*;
    use std::{sync::Mutex, time::Duration};
    use tokio::sync::{Notify, oneshot};
    struct Admission {
        events: Arc<Mutex<Vec<&'static str>>>,
        entered: Arc<Notify>,
        release: Arc<Notify>,
        dropped: Arc<Notify>,
    }
    impl AdmissionQuarantine for Admission {
        async fn quarantine(&mut self) {
            self.events.lock().unwrap().push("quarantine_started");
            self.entered.notify_one();
            self.release.notified().await;
            self.events.lock().unwrap().push("quarantine_settled");
        }
    }
    impl Drop for Admission {
        fn drop(&mut self) {
            self.events.lock().unwrap().push("guards_drop");
            self.dropped.notify_one();
        }
    }
    fn admission() -> Admission {
        Admission {
            events: Arc::default(),
            entered: Arc::default(),
            release: Arc::default(),
            dropped: Arc::default(),
        }
    }
    async fn abandoned_receiver(queued: bool) {
        let admission = admission();
        let events = admission.events.clone();
        let entered = admission.entered.clone();
        let release = admission.release.clone();
        let dropped = admission.dropped.clone();
        let (tx, rx) = oneshot::channel();
        if queued {
            assert!(handoff_admission(tx, Ok(admission)));
            drop(rx);
        } else {
            drop(rx);
            assert!(!handoff_admission(tx, Ok(admission)));
        }
        tokio::time::timeout(Duration::from_millis(250), entered.notified())
            .await
            .expect("abandoned admission never reached quarantine owner");
        assert_eq!(*events.lock().unwrap(), vec!["quarantine_started"]);
        release.notify_one();
        tokio::time::timeout(Duration::from_secs(1), dropped.notified())
            .await
            .unwrap();
        assert_eq!(
            *events.lock().unwrap(),
            vec!["quarantine_started", "quarantine_settled", "guards_drop"]
        );
    }
    #[tokio::test]
    async fn queued_admission_abandonment_retains_guards_through_quarantine() {
        abandoned_receiver(true).await;
    }
    #[tokio::test]
    async fn undelivered_admission_retains_guards_through_quarantine() {
        abandoned_receiver(false).await;
    }
    #[test]
    fn queued_admission_dropped_outside_runtime_retains_origin_cleanup_owner() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let admission = admission();
        let events = admission.events.clone();
        let entered = admission.entered.clone();
        let release = admission.release.clone();
        let dropped = admission.dropped.clone();
        let (sender, receiver) = oneshot::channel();
        runtime.block_on(async {
            assert!(handoff_admission(sender, Ok(admission)));
        });
        assert!(tokio::runtime::Handle::try_current().is_err());
        drop(receiver);
        assert!(
            events.lock().unwrap().is_empty(),
            "authority dropped before origin runtime could settle quarantine"
        );
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(1), entered.notified())
                .await
                .expect("origin runtime did not receive abandoned admission cleanup");
            assert_eq!(*events.lock().unwrap(), vec!["quarantine_started"]);
            release.notify_one();
            tokio::time::timeout(Duration::from_secs(1), dropped.notified())
                .await
                .unwrap();
        });
        assert_eq!(
            *events.lock().unwrap(),
            vec!["quarantine_started", "quarantine_settled", "guards_drop"]
        );
    }
    #[tokio::test]
    async fn successful_admission_transfer_has_one_owner_and_no_early_cleanup() {
        let admission = admission();
        let events = admission.events.clone();
        let (tx, rx) = oneshot::channel();
        assert!(handoff_admission(tx, Ok(admission)));
        let envelope = rx.await.unwrap().unwrap();
        assert!(events.lock().unwrap().is_empty());
        let mut owner = envelope.take();
        tokio::task::yield_now().await;
        assert!(
            events.lock().unwrap().is_empty(),
            "handoff cleaned a transferred owner"
        );
        owner.release.notify_one();
        owner.quarantine().await;
        drop(owner);
        assert_eq!(
            *events.lock().unwrap(),
            vec!["quarantine_started", "quarantine_settled", "guards_drop"]
        );
    }
}

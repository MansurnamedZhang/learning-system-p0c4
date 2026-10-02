//! Private test-only sanitized rejection context. No resource actions.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]
use super::{CandidateImportReport, ImportFailure, ImportPhase};
use tokio::task::JoinError;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Case {
    Success,
    ReadyEof,
    PrecommitEof,
    Cancel,
    Restart,
    SqlError,
    CopyTruncated,
    AttemptSync,
    IntentSync,
    CommitUnknown,
    WrongEndpoint,
}
impl Case {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::ReadyEof => "ready-eof",
            Self::PrecommitEof => "precommit-eof",
            Self::Cancel => "precommit-cancel",
            Self::Restart => "ready-restart",
            Self::SqlError => "sql-error",
            Self::CopyTruncated => "copy-truncated",
            Self::AttemptSync => "attempt-sync-failure",
            Self::IntentSync => "commit-intent-sync-failure",
            Self::CommitUnknown => "commit-unknown",
            Self::WrongEndpoint => "wrong-endpoint",
        }
    }
}

#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum Phase {
    AuthorityCaseEnv,
    SourceConfig,
    TargetConfig,
    ArtifactRootEnv,
    FreshFixtureCapture,
    AcquireTargetGuard,
    TargetControlConnection,
    CandidateAdmission,
    LiveToc,
    LiveClone,
    CandidateReport,
    EvidenceUnwrap,
    HarnessJoin,
}
#[derive(Clone, Copy)]
enum Reason {
    Import(ImportFailure),
    JoinPanic,
    JoinCancelled,
    JoinUnknown,
    EvidenceShared,
    EvidencePoisoned,
}
#[derive(Clone, Copy)]
struct CandidateSnapshot {
    phase: ImportPhase,
    failure: Option<ImportFailure>,
    stop_confirmed: bool,
    content_verified: bool,
    commit_attempted: bool,
}
pub(super) struct Failure {
    phase: Phase,
    reason: Reason,
    candidate: Option<CandidateSnapshot>,
}
impl Failure {
    pub(super) fn at(phase: Phase) -> impl FnOnce(ImportFailure) -> Self {
        move |error| Self {
            phase,
            reason: Reason::Import(error),
            candidate: None,
        }
    }
    pub(super) fn report(report: &CandidateImportReport) -> Self {
        Self {
            phase: Phase::CandidateReport,
            reason: Reason::Import(ImportFailure::Protocol),
            candidate: Some(CandidateSnapshot {
                phase: report.phase,
                failure: report.failure,
                stop_confirmed: report.stop_confirmed,
                content_verified: report.content_verified,
                commit_attempted: report.commit_attempted,
            }),
        }
    }
}

impl Failure {
    pub(super) fn evidence_shared(report: &CandidateImportReport) -> Self {
        Self {
            phase: Phase::EvidenceUnwrap,
            reason: Reason::EvidenceShared,
            ..Self::report(report)
        }
    }
    pub(super) fn evidence_poisoned(report: &CandidateImportReport) -> Self {
        Self {
            phase: Phase::EvidenceUnwrap,
            reason: Reason::EvidencePoisoned,
            ..Self::report(report)
        }
    }
    fn event(&self, case: Case) -> String {
        #[derive(serde::Serialize)]
        struct Candidate {
            phase: &'static str,
            failure: Option<&'static str>,
            stop_confirmed: bool,
            content_verified: bool,
            commit_attempted: bool,
        }
        #[derive(serde::Serialize)]
        struct Event {
            schema: u32,
            case: &'static str,
            phase: Phase,
            reason: &'static str,
            candidate: Option<Candidate>,
        }
        let reason = match self.reason {
            Reason::Import(error) => failure_name(error),
            Reason::JoinPanic => "JoinPanic",
            Reason::JoinCancelled => "JoinCancelled",
            Reason::JoinUnknown => "JoinUnknown",
            Reason::EvidenceShared => "EvidenceShared",
            Reason::EvidencePoisoned => "EvidencePoisoned",
        };
        let candidate = self.candidate.map(|snapshot| Candidate {
            phase: match snapshot.phase {
                ImportPhase::InputFrozen => "InputFrozen",
                ImportPhase::SqlVerified => "SqlVerified",
                ImportPhase::WriterReady => "WriterReady",
                ImportPhase::AttemptDurable => "AttemptDurable",
                ImportPhase::PayloadSent => "PayloadSent",
                ImportPhase::PrecommitVerified => "PrecommitVerified",
                ImportPhase::CommitAttempted => "CommitAttempted",
                ImportPhase::ImportObserved => "ImportObserved",
                ImportPhase::Quarantined => "Quarantined",
                ImportPhase::Cancelled => "Cancelled",
                ImportPhase::CommitUnknown => "CommitUnknown",
            },
            failure: snapshot.failure.map(failure_name),
            stop_confirmed: snapshot.stop_confirmed,
            content_verified: snapshot.content_verified,
            commit_attempted: snapshot.commit_attempted,
        });
        // Every string is a compile-time whitelist value and the shape is fixed.
        // Current maximum is < 512 ASCII bytes, including the tag.
        format!(
            "KW_C4_IMPORT_DIAGNOSTIC|{}",
            serde_json::to_string(&Event {
                schema: 1,
                case: case.name(),
                phase: self.phase,
                reason,
                candidate,
            })
            .expect("fixed diagnostic serialization")
        )
    }
}
fn failure_name(error: ImportFailure) -> &'static str {
    match error {
        ImportFailure::Identity => "Identity",
        ImportFailure::Session => "Session",
        ImportFailure::Version => "Version",
        ImportFailure::Protocol => "Protocol",
        ImportFailure::Fixture => "Fixture",
        ImportFailure::InputLimit => "InputLimit",
        ImportFailure::Deadline => "Deadline",
        ImportFailure::StdoutLimit => "StdoutLimit",
        ImportFailure::StderrLimit => "StderrLimit",
        ImportFailure::Stderr => "Stderr",
        ImportFailure::Exit => "Exit",
        ImportFailure::Io => "Io",
        ImportFailure::Journal => "Journal",
        ImportFailure::Cancelled => "Cancelled",
        ImportFailure::CommitUnknown => "CommitUnknown",
        ImportFailure::UnconfirmedIsolation => "UnconfirmedIsolation",
    }
}

pub(super) fn settled<T>(
    case: Case,
    result: Result<Result<T, Failure>, JoinError>,
) -> Result<T, String> {
    let failure = match result {
        Ok(Ok(value)) => return Ok(value),
        Ok(Err(failure)) => failure,
        Err(error) => Failure {
            phase: Phase::HarnessJoin,
            reason: if error.is_panic() {
                Reason::JoinPanic
            } else if error.is_cancelled() {
                Reason::JoinCancelled
            } else {
                Reason::JoinUnknown
            },
            candidate: None,
        },
    };
    Err(failure.event(case))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn decode(event: String) -> serde_json::Value {
        assert!(
            event.starts_with("KW_C4_IMPORT_DIAGNOSTIC|"),
            "typed diagnostic lost: {event}"
        );
        assert!(event.is_ascii());
        assert!(event.len() <= 1024);
        serde_json::from_str(event.strip_prefix("KW_C4_IMPORT_DIAGNOSTIC|").unwrap()).unwrap()
    }
    #[test]
    fn settled_typed_failure_preserves_boundary_and_original_reason() {
        let event = settled::<()>(
            Case::Success,
            Ok(Err(Failure::at(Phase::FreshFixtureCapture)(
                ImportFailure::Session,
            ))),
        )
        .unwrap_err();
        assert_eq!(
            decode(event),
            json!({"schema":1,"case":"success","phase":"fresh-fixture-capture","reason":"Session","candidate":null})
        );
    }
    #[test]
    fn settled_report_keeps_typed_candidate_and_unknown_commit_booleans() {
        let report = CandidateImportReport {
            phase: ImportPhase::CommitUnknown,
            failure: Some(ImportFailure::CommitUnknown),
            stop_confirmed: false,
            content_verified: false,
            commit_attempted: true,
        };
        let event =
            settled::<()>(Case::CommitUnknown, Ok(Err(Failure::report(&report)))).unwrap_err();
        assert_eq!(
            decode(event),
            json!({"schema":1,"case":"commit-unknown","phase":"candidate-report","reason":"Protocol",
            "candidate":{"phase":"CommitUnknown","failure":"CommitUnknown","stop_confirmed":false,"content_verified":false,"commit_attempted":true}})
        );
    }
    #[tokio::test]
    async fn settled_join_keeps_panic_category_without_private_payload() {
        let task = tokio::spawn(async {
            panic!("password=private SELECT secret;");
        });
        let error = task.await.unwrap_err();
        let event = settled::<()>(Case::Success, Err(error)).unwrap_err();
        assert!(!event.contains("private"));
        assert_eq!(
            decode(event),
            json!({"schema":1,"case":"success","phase":"harness-join","reason":"JoinPanic","candidate":null})
        );
    }
    #[tokio::test]
    async fn settled_join_cancellation_is_distinct() {
        let task = tokio::spawn(std::future::pending::<()>());
        task.abort();
        let event = settled::<()>(Case::Success, Err(task.await.unwrap_err())).unwrap_err();
        assert_eq!(
            decode(event),
            json!({"schema":1,"case":"success","phase":"harness-join","reason":"JoinCancelled","candidate":null})
        );
    }
    #[test]
    fn evidence_rejection_keeps_safe_report_without_claiming_recovery() {
        let report = CandidateImportReport {
            phase: ImportPhase::Quarantined,
            failure: None,
            stop_confirmed: true,
            content_verified: true,
            commit_attempted: true,
        };
        for (failure, reason) in [
            (Failure::evidence_shared(&report), "EvidenceShared"),
            (Failure::evidence_poisoned(&report), "EvidencePoisoned"),
        ] {
            let record = decode(settled::<()>(Case::Success, Ok(Err(failure))).unwrap_err());
            assert_eq!(record["phase"], "evidence-unwrap");
            assert_eq!(record["reason"], reason);
            assert_eq!(
                record["candidate"],
                json!({"phase":"Quarantined","failure":null,
                "stop_confirmed":true,"content_verified":true,"commit_attempted":true})
            );
        }
    }
    #[test]
    fn every_unit_error_is_bounded_ascii_for_each_case_and_boundary() {
        let cases = [
            Case::Success,
            Case::ReadyEof,
            Case::PrecommitEof,
            Case::Cancel,
            Case::Restart,
            Case::SqlError,
            Case::CopyTruncated,
            Case::AttemptSync,
            Case::IntentSync,
            Case::CommitUnknown,
            Case::WrongEndpoint,
        ];
        let phases = [
            Phase::AuthorityCaseEnv,
            Phase::SourceConfig,
            Phase::TargetConfig,
            Phase::ArtifactRootEnv,
            Phase::FreshFixtureCapture,
            Phase::AcquireTargetGuard,
            Phase::TargetControlConnection,
            Phase::CandidateAdmission,
            Phase::LiveToc,
            Phase::LiveClone,
            Phase::CandidateReport,
            Phase::EvidenceUnwrap,
            Phase::HarnessJoin,
        ];
        let errors = [
            (ImportFailure::Identity, "Identity"),
            (ImportFailure::Session, "Session"),
            (ImportFailure::Version, "Version"),
            (ImportFailure::Protocol, "Protocol"),
            (ImportFailure::Fixture, "Fixture"),
            (ImportFailure::InputLimit, "InputLimit"),
            (ImportFailure::Deadline, "Deadline"),
            (ImportFailure::StdoutLimit, "StdoutLimit"),
            (ImportFailure::StderrLimit, "StderrLimit"),
            (ImportFailure::Stderr, "Stderr"),
            (ImportFailure::Exit, "Exit"),
            (ImportFailure::Io, "Io"),
            (ImportFailure::Journal, "Journal"),
            (ImportFailure::Cancelled, "Cancelled"),
            (ImportFailure::CommitUnknown, "CommitUnknown"),
            (ImportFailure::UnconfirmedIsolation, "UnconfirmedIsolation"),
        ];
        for case in cases {
            for phase in phases {
                for (error, name) in errors {
                    let record = decode(
                        settled::<()>(case, Ok(Err(Failure::at(phase)(error)))).unwrap_err(),
                    );
                    assert_eq!(record["reason"], name);
                    assert!(record["candidate"].is_null());
                }
            }
        }
    }
    #[test]
    fn successful_settlement_has_no_diagnostic() {
        assert_eq!(settled(Case::Success, Ok(Ok(42))).unwrap(), 42);
    }
}

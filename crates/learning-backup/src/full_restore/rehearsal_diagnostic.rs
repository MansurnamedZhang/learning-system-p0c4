//! Test-only failure decisions shared by the actual Linux rehearsal and unit tests.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]
use crate::BackupError;
use std::sync::{Arc, Mutex};

pub(crate) fn result_and_observation<T, O>(
    result: Result<T, BackupError>,
    observation: Option<O>,
    required: bool,
) -> Result<(Result<T, BackupError>, Option<O>), BackupError> {
    if required && observation.is_none() {
        if let Err(error) = result {
            return Err(error);
        }
        return Err(BackupError::Invalid("full writer observation missing"));
    }
    Ok((result, observation))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    Fixture,
    Capture,
    TargetConfiguration,
    Package,
    TargetAdmission,
    AdmissionGuard,
    Profile,
    TargetConnection,
    CandidateAdmission,
    Preflight,
    OwnedAdmission,
    Spool,
    DumpOpen,
    DumpFault,
    DumpFreeze,
    DumpValidation,
    Payload,
    Supervisor,
    SupervisorCleanup,
    SupervisorStop,
    SupervisorReport,
    Originals,
    Recheck,
    NegativeTargetAudit,
    Stop,
    NegativeAudit,
    Observation,
    Report,
    DurableInventory,
    StopInspection,
    Proof,
}
impl Stage {
    fn code(self) -> &'static str {
        match self {
            Self::Fixture => "FIXTURE",
            Self::Capture => "CAPTURE",
            Self::TargetConfiguration => "TARGET_CONFIGURATION",
            Self::Package => "PACKAGE",
            Self::TargetAdmission => "TARGET_ADMISSION",
            Self::AdmissionGuard => "ADMISSION_GUARD",
            Self::Profile => "PROFILE",
            Self::TargetConnection => "TARGET_CONNECTION",
            Self::CandidateAdmission => "CANDIDATE_ADMISSION",
            Self::Preflight => "PREFLIGHT",
            Self::OwnedAdmission => "OWNED_ADMISSION",
            Self::Spool => "SPOOL",
            Self::DumpOpen => "DUMP_OPEN",
            Self::DumpFault => "DUMP_FAULT",
            Self::DumpFreeze => "DUMP_FREEZE",
            Self::DumpValidation => "DUMP_VALIDATION",
            Self::Payload => "PAYLOAD",
            Self::Supervisor => "SUPERVISOR",
            Self::SupervisorCleanup => "SUPERVISOR_CLEANUP",
            Self::SupervisorStop => "SUPERVISOR_STOP",
            Self::SupervisorReport => "SUPERVISOR_REPORT",
            Self::Originals => "ORIGINALS",
            Self::Recheck => "RECHECK",
            Self::NegativeTargetAudit => "NEGATIVE_TARGET_AUDIT",
            Self::Stop => "STOP",
            Self::NegativeAudit => "NEGATIVE_AUDIT",
            Self::Observation => "OBSERVATION",
            Self::Report => "REPORT",
            Self::DurableInventory => "DURABLE_INVENTORY",
            Self::StopInspection => "STOP_INSPECTION",
            Self::Proof => "PROOF",
        }
    }
}

#[derive(Clone)]
struct Failure {
    stage: Stage,
    class: &'static str,
    code: String,
}
impl Failure {
    fn from_error(stage: Stage, error: &BackupError) -> Self {
        let (class, code) = match error {
            BackupError::Invalid(reason) | BackupError::Capacity(reason) => {
                let code = match *reason {
                    "fresh manifest differs" => "FRESH_MANIFEST_DIFFERS",
                    "fresh index differs" => "FRESH_INDEX_DIFFERS",
                    "fresh dump" => "FRESH_DUMP_MISSING",
                    "custom dump header" => "CUSTOM_DUMP_HEADER",
                    "full payload preparation lost" => "PAYLOAD_JOIN_LOST",
                    "guarded full restore rejected; unusable" => "GUARDED_IMPORT_REJECTED",
                    "restore commit unknown; unusable; no replay" => "COMMIT_UNKNOWN",
                    "restore cancelled; unusable; no replay" => "CANCELLED",
                    "full target isolation unconfirmed" => "ISOLATION_UNCONFIRMED",
                    "full writer observation missing" => "OBSERVATION_MISSING",
                    "full rehearsal report" => "REPORT_INVALID",
                    "RECOVERY_SOURCE_LEASES_SURVIVED_IMPORT" => "RECOVERY_SOURCE_LEASES_SURVIVED_IMPORT",
                    "full body target stop unconfirmed" => "STOP_INSPECTION_UNCONFIRMED",
                    "full schema contract" => "SCHEMA_CONTRACT_REJECTED",
                    "fixed PG18 decoder" => "FIXED_DECODER_REJECTED",
                    "decoder supervisor lost" => "DECODER_OWNER_LOST",
                    "decoder cancelled" => "DECODER_CANCELLED",
                    "pg_restore decode exit" => "DECODER_NATIVE_REJECTED",
                    "full decode deadline" => "DECODE_DEADLINE",
                    "spool identity" => "SPOOL_IDENTITY_REJECTED",
                    "full validator task lost" => "VALIDATOR_TASK_LOST",
                    "COPY text frame" => "COPY_FRAME_REJECTED",
                    "COPY UTF-8" => "COPY_UTF8_REJECTED",
                    "migration row count" => "MIGRATION_ROW_COUNT_REJECTED",
                    "checked range coverage" => "RANGE_COVERAGE_REJECTED",
                    _ => "UNKNOWN",
                };
                (
                    if matches!(error, BackupError::Invalid(_)) {
                        "INVALID"
                    } else {
                        "CAPACITY"
                    },
                    code.to_owned(),
                )
            }
            BackupError::Overflow => ("OVERFLOW", "UNKNOWN".to_owned()),
            BackupError::Io(error) => ("IO", errno(error)),
            BackupError::Json(_) => ("JSON", "UNKNOWN".to_owned()),
            BackupError::Asset(_) => ("ASSET", "UNKNOWN".to_owned()),
            BackupError::Database(error) => (
                "DATABASE",
                match error {
                    sqlx::Error::PoolTimedOut => "POOL_TIMED_OUT".to_owned(),
                    sqlx::Error::PoolClosed => "POOL_CLOSED".to_owned(),
                    sqlx::Error::Io(error) => errno(error),
                    sqlx::Error::Database(error) => error
                        .code()
                        .map_or("UNKNOWN", |code| sqlstate(&code))
                        .to_owned(),
                    _ => "UNKNOWN".to_owned(),
                },
            ),
        };
        Self { stage, class, code }
    }
    fn line(&self, kind: &'static str) -> String {
        format!(
            "FULL_REHEARSAL_FAILURE kind={kind} stage={} class={} code={}",
            self.stage.code(),
            self.class,
            self.code
        )
    }
}
fn errno(error: &std::io::Error) -> String {
    error
        .raw_os_error()
        .map_or_else(|| "UNKNOWN".to_owned(), |n| format!("ERRNO_{n}"))
}
fn sqlstate(code: &str) -> &'static str {
    match code {
        "08000" => "SQLSTATE_08000",
        "08003" => "SQLSTATE_08003",
        "08006" => "SQLSTATE_08006",
        "28000" => "SQLSTATE_28000",
        "28P01" => "SQLSTATE_28P01",
        "3D000" => "SQLSTATE_3D000",
        "42501" => "SQLSTATE_42501",
        "42P01" => "SQLSTATE_42P01",
        "23505" => "SQLSTATE_23505",
        "55P03" => "SQLSTATE_55P03",
        "57014" => "SQLSTATE_57014",
        "XX000" => "SQLSTATE_XX000",
        _ => "UNKNOWN",
    }
}

struct State {
    stage: Stage,
    first: Option<Failure>,
    secondary: Vec<Failure>,
    settlement_failed: bool,
}
#[derive(Clone)]
pub(crate) struct Diagnostic(Arc<Mutex<State>>);
impl Diagnostic {
    pub(super) fn dump_failure(
        &self,
        step: super::dump_observer::Step,
        guard: super::dump_observer::Guard,
        error: &BackupError,
        native: Option<super::dump_observer::Native>,
    ) {
        let mut failure = Failure::from_error(Stage::DumpValidation, error);
        // Unlisted free-text reasons still produce exactly UNKNOWN.
        if !matches!(error, BackupError::Invalid(_) | BackupError::Capacity(_))
            || failure.code != "UNKNOWN"
        {
            use std::fmt::Write;
            let _ = write!(failure.code, "__{}__{}", step.code(), guard.code());
            if let Some(native) = native {
                native.append_to(&mut failure.code);
            }
        }
        self.record_failure(failure, false);
    }
    pub(crate) fn new() -> Self {
        Self(Arc::new(Mutex::new(State {
            stage: Stage::Fixture,
            first: None,
            secondary: Vec::with_capacity(4),
            settlement_failed: false,
        })))
    }
    pub(crate) fn enter(&self, stage: Stage) {
        self.0.lock().unwrap().stage = stage;
    }
    pub(crate) fn returned<T>(&self, result: &Result<T, BackupError>) {
        if let Err(error) = result {
            self.record(error, false);
        }
    }
    fn record(&self, error: &BackupError, settlement: bool) {
        let stage = self.0.lock().unwrap().stage;
        self.record_failure(Failure::from_error(stage, error), settlement);
    }
    fn record_failure(&self, failure: Failure, settlement: bool) {
        let mut state = self.0.lock().unwrap();
        state.settlement_failed |= settlement;
        if state.first.is_none() {
            state.first = Some(failure);
        } else if settlement
            && state.secondary.len() < 4
            && !state.first.as_ref().is_some_and(|first| {
                first.stage == failure.stage
                    && first.class == failure.class
                    && first.code == failure.code
            })
            && !state.secondary.iter().any(|old| old.stage == failure.stage)
        {
            state.secondary.push(failure);
        }
    }
    pub(crate) fn import_failure(&self, original_class: &str) {
        let stage = self.0.lock().unwrap().stage;
        self.supervisor_failure(stage, original_class, false);
    }
    pub(crate) fn supervisor_failure(&self, stage: Stage, original_class: &str, settlement: bool) {
        let code = match original_class {
            "IDENTITY" => "IDENTITY",
            "SESSION" => "SESSION",
            "VERSION" => "VERSION",
            "PROTOCOL" => "PROTOCOL",
            "FIXTURE" => "FIXTURE",
            "INPUT_LIMIT" => "INPUT_LIMIT",
            "DEADLINE" => "DEADLINE",
            "STDOUT_LIMIT" => "STDOUT_LIMIT",
            "STDERR_LIMIT" => "STDERR_LIMIT",
            "STDERR" => "STDERR",
            "EXIT" => "EXIT",
            "IO" => "IO",
            "JOURNAL" => "JOURNAL",
            "CANCELLED" => "CANCELLED",
            "COMMIT_UNKNOWN" => "COMMIT_UNKNOWN",
            "UNCONFIRMED_ISOLATION" => "UNCONFIRMED_ISOLATION",
            _ => "UNKNOWN",
        };
        self.record_failure(
            Failure {
                stage,
                class: "IMPORT_FAILURE",
                code: code.to_owned(),
            },
            settlement,
        );
    }
    pub(crate) fn observe<T, O>(
        &self,
        result: Result<T, BackupError>,
        observation: Option<O>,
        required: bool,
    ) -> Result<(Result<T, BackupError>, Option<O>), BackupError> {
        self.returned(&result);
        self.enter(Stage::Observation);
        if required && observation.is_none() {
            self.record(
                &BackupError::Invalid("full writer observation missing"),
                true,
            );
        }
        result_and_observation(result, observation, required)
    }
    pub(crate) fn run<T>(
        &self,
        stage: Stage,
        call: impl FnOnce() -> Result<T, BackupError>,
    ) -> Result<T, BackupError> {
        self.enter(stage);
        let result = call();
        self.returned(&result);
        result
    }
    pub(crate) async fn run_async<T>(
        &self,
        stage: Stage,
        call: impl std::future::Future<Output = Result<T, BackupError>>,
    ) -> Result<T, BackupError> {
        self.enter(stage);
        let result = call.await;
        self.returned(&result);
        result
    }
    pub(crate) fn settle<T>(
        &self,
        stage: Stage,
        call: impl FnOnce() -> Result<T, BackupError>,
    ) -> Result<T, BackupError> {
        self.enter(stage);
        let result = call();
        if let Err(error) = &result {
            self.record(error, true);
        }
        result
    }
    pub(crate) async fn settle_async<T>(
        &self,
        stage: Stage,
        call: impl std::future::Future<Output = Result<T, BackupError>>,
    ) -> Result<T, BackupError> {
        self.enter(stage);
        let result = call.await;
        if let Err(error) = &result {
            self.record(error, true);
        }
        result
    }
    pub(crate) fn require_settlement(&self) -> Result<(), BackupError> {
        if self.0.lock().unwrap().settlement_failed {
            Err(BackupError::Invalid("full rehearsal settlement failed"))
        } else {
            Ok(())
        }
    }
    pub(crate) fn public_result<T>(
        &self,
        result: Result<T, BackupError>,
    ) -> Result<T, BackupError> {
        self.public_result_to(result, &mut std::io::stderr().lock())
    }
    pub(crate) fn public_result_to<T>(
        &self,
        result: Result<T, BackupError>,
        output: &mut impl std::io::Write,
    ) -> Result<T, BackupError> {
        if let Err(error) = &result {
            let stage = self.0.lock().unwrap().stage;
            if matches!(
                stage,
                Stage::Report | Stage::DurableInventory | Stage::StopInspection | Stage::Proof
            ) {
                self.record(error, true);
            } else {
                self.returned(&result);
            }
            let lines = {
                let state = self.0.lock().unwrap();
                state
                    .first
                    .iter()
                    .map(|failure| failure.line("PRIMARY"))
                    .chain(
                        state
                            .secondary
                            .iter()
                            .map(|failure| failure.line("SECONDARY")),
                    )
                    .collect::<Vec<_>>()
            };
            // Only final rejection emits. Correct expected negatives are quiet
            // for the unchanged actual parse_test consumer. At most five lines.
            for line in lines {
                let _ = writeln!(output, "{line}");
            }
        }
        result.map_err(|_| BackupError::Invalid("full rehearsal failed; see bounded diagnostic"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_operation_error_is_not_overwritten_by_missing_observation() {
        let error = BackupError::Io(std::io::Error::from_raw_os_error(13));
        let decision = result_and_observation::<(), ()>(Err(error), None, true);
        assert!(matches!(decision, Err(BackupError::Io(ref e)) if e.raw_os_error() == Some(13)));
    }

    #[test]
    fn success_without_required_observation_still_rejects() {
        assert!(matches!(
            result_and_observation::<(), ()>(Ok(()), None, true),
            Err(BackupError::Invalid("full writer observation missing"))
        ));
    }

    #[test]
    fn original_error_with_observation_remains_pending_for_mandatory_audit() {
        let (result, observed) = result_and_observation(
            Err::<(), _>(BackupError::Invalid("custom dump header")),
            Some(7),
            true,
        )
        .unwrap();
        assert!(result.is_err());
        assert_eq!(observed, Some(7));
        let diagnostic = Diagnostic::new();
        let mut audited = false;
        let audit = diagnostic.settle(Stage::NegativeAudit, || {
            audited = true;
            Err::<(), _>(BackupError::Invalid("negative boundary not proved"))
        });
        assert!(audited && audit.is_err() && diagnostic.require_settlement().is_err());
    }

    #[test]
    fn successful_report_and_observation_are_both_required() {
        let (result, observed) = result_and_observation(Ok(9), Some(7), true).unwrap();
        assert_eq!(result.unwrap(), 9);
        assert_eq!(observed, Some(7));
        let diagnostic = Diagnostic::new();
        let report = diagnostic.run(Stage::Report, || {
            Err::<(), _>(BackupError::Invalid("full rehearsal report"))
        });
        assert!(diagnostic.public_result(report).is_err());
    }

    #[tokio::test]
    async fn actual_flow_boundaries_keep_first_operation_separate_from_stop_failure() {
        let diagnostic = Diagnostic::new();
        let operation = diagnostic
            .run_async(Stage::Payload, async {
                Err::<(), _>(BackupError::Io(std::io::Error::from_raw_os_error(13)))
            })
            .await;
        let stop = diagnostic
            .settle_async(Stage::Stop, async {
                Err::<(), _>(BackupError::Invalid("full target isolation unconfirmed"))
            })
            .await;
        assert!(operation.is_err() && stop.is_err() && diagnostic.require_settlement().is_err());
        let state = diagnostic.0.lock().unwrap();
        assert_eq!(
            state.first.as_ref().unwrap().line("PRIMARY"),
            "FULL_REHEARSAL_FAILURE kind=PRIMARY stage=PAYLOAD class=IO code=ERRNO_13"
        );
        assert_eq!(
            state.secondary[0].line("SECONDARY"),
            "FULL_REHEARSAL_FAILURE kind=SECONDARY stage=STOP class=INVALID code=ISOLATION_UNCONFIRMED"
        );
    }

    #[tokio::test]
    async fn nested_actual_entry_is_not_attributed_to_elapsed_time_or_outer_call() {
        let diagnostic = Diagnostic::new();
        let result = diagnostic
            .run_async(Stage::TargetAdmission, async {
                diagnostic.enter(Stage::TargetConnection);
                Err::<(), _>(BackupError::Database(sqlx::Error::PoolTimedOut))
            })
            .await;
        assert!(result.is_err());
        assert_eq!(
            diagnostic
                .0
                .lock()
                .unwrap()
                .first
                .as_ref()
                .unwrap()
                .line("PRIMARY"),
            "FULL_REHEARSAL_FAILURE kind=PRIMARY stage=TARGET_CONNECTION class=DATABASE code=POOL_TIMED_OUT"
        );
    }

    #[test]
    fn synthetic_secrets_never_reach_diagnostics_or_outer_unwrap_error() {
        for error in [
            BackupError::Invalid("postgres://user:SECRET@host/private SQL password=SECRET"),
            BackupError::Capacity("SECRET path dump bytes"),
            BackupError::Io(std::io::Error::other("DSN SECRET")),
            BackupError::Database(sqlx::Error::Protocol("SQL SECRET".into())),
            BackupError::Json(serde_json::from_str::<serde_json::Value>("SECRET").unwrap_err()),
            BackupError::Asset(learning_assets::AssetIoError::Missing("SECRET".into())),
        ] {
            let diagnostic = Diagnostic::new();
            let result = diagnostic.run(Stage::DumpValidation, || Err::<(), _>(error));
            let line = diagnostic
                .0
                .lock()
                .unwrap()
                .first
                .as_ref()
                .unwrap()
                .line("PRIMARY");
            assert!(!line.contains("SECRET") && !line.contains("postgres") && line.len() < 160);
            assert!(line.ends_with("code=UNKNOWN"));
            assert!(matches!(
                diagnostic.public_result(result),
                Err(BackupError::Invalid(
                    "full rehearsal failed; see bounded diagnostic"
                ))
            ));
        }
    }

    #[test]
    fn later_failures_cannot_replace_first_or_expand_bounded_diagnostics() {
        let diagnostic = Diagnostic::new();
        let _ = diagnostic.run(Stage::Package, || {
            Err::<(), _>(BackupError::Invalid("fresh index differs"))
        });
        for _ in 0..100 {
            let _ = diagnostic.settle(Stage::NegativeAudit, || {
                Err::<(), _>(BackupError::Invalid("SECRET"))
            });
        }
        let state = diagnostic.0.lock().unwrap();
        assert_eq!(state.first.as_ref().unwrap().code, "FRESH_INDEX_DIFFERS");
        assert_eq!(state.secondary[0].code, "UNKNOWN");
        assert_eq!(state.secondary.len(), 1);
        assert!(state.settlement_failed);
    }

    #[test]
    fn sqlstate_is_a_fixed_allowlist_never_database_error_text() {
        assert_eq!(sqlstate("42501"), "SQLSTATE_42501");
        for value in [
            "SECRET",
            "TOKEN",
            "secret",
            "42501 SECRET",
            "postgres://SECRET",
            "ZZ999",
        ] {
            assert_eq!(sqlstate(value), "UNKNOWN");
        }
    }

    #[test]
    fn expected_refusal_with_valid_real_audit_cannot_accept_stop_failure() {
        use crate::full_restore::negative::{Boundary, NegativeEvidence};
        let diagnostic = Diagnostic::new();
        let result = diagnostic.run(Stage::DumpFreeze, || {
            Err::<(), _>(BackupError::Invalid("custom dump header"))
        });
        let _ = diagnostic.settle(Stage::Stop, || {
            Err::<(), _>(BackupError::Invalid("full target isolation unconfirmed"))
        });
        let mut audit = NegativeEvidence::default();
        let inventory = serde_json::json!({"catalog":{},"assets":{},"attempts":{}});
        audit.injected(Boundary::DumpFreeze);
        audit.refused(Boundary::DumpFreeze);
        audit.before(inventory.clone());
        audit.after(inventory);
        diagnostic
            .settle(Stage::NegativeAudit, || {
                audit.validate(Boundary::DumpFreeze, result.is_err())
            })
            .unwrap();
        assert!(diagnostic.require_settlement().is_err());
        let (result, observed) = result_and_observation::<(), ()>(result, None, false).unwrap();
        assert!(result.is_err() && observed.is_none());
    }

    #[test]
    fn missing_observation_is_secondary_to_original_operation_and_never_replaces_it() {
        let diagnostic = Diagnostic::new();
        let result = diagnostic.run(Stage::Package, || {
            Err::<(), _>(BackupError::Io(std::io::Error::from_raw_os_error(13)))
        });
        assert!(matches!(
            diagnostic.observe::<(), ()>(result, None, true),
            Err(BackupError::Io(_))
        ));
        let state = diagnostic.0.lock().unwrap();
        assert_eq!(
            state.first.as_ref().unwrap().line("PRIMARY"),
            "FULL_REHEARSAL_FAILURE kind=PRIMARY stage=PACKAGE class=IO code=ERRNO_13"
        );
        assert_eq!(
            state.secondary[0].line("SECONDARY"),
            "FULL_REHEARSAL_FAILURE kind=SECONDARY stage=OBSERVATION class=INVALID code=OBSERVATION_MISSING"
        );
    }

    #[test]
    fn original_fixed_import_class_survives_lossy_backup_error_mapping() {
        for (original, want) in [
            ("IDENTITY", "IDENTITY"),
            ("SESSION", "SESSION"),
            ("PROTOCOL", "PROTOCOL"),
            ("SECRET DSN", "UNKNOWN"),
        ] {
            let diagnostic = Diagnostic::new();
            diagnostic.enter(Stage::CandidateAdmission);
            diagnostic.import_failure(original);
            diagnostic.returned(&Err::<(), _>(BackupError::Invalid(
                "guarded full restore rejected; unusable",
            )));
            let state = diagnostic.0.lock().unwrap();
            assert_eq!(state.first.as_ref().unwrap().class, "IMPORT_FAILURE");
            assert_eq!(state.first.as_ref().unwrap().code, want);
            assert!(
                !state
                    .first
                    .as_ref()
                    .unwrap()
                    .line("PRIMARY")
                    .contains("SECRET")
            );
        }
    }

    #[test]
    fn stop_audit_and_observation_failures_each_remain_secondary() {
        let diagnostic = Diagnostic::new();
        let result = diagnostic.run(Stage::Package, || {
            Err::<(), _>(BackupError::Invalid("fresh index differs"))
        });
        for stage in [
            Stage::Stop,
            Stage::NegativeTargetAudit,
            Stage::NegativeAudit,
        ] {
            let _ = diagnostic.settle(stage, || Err::<(), _>(BackupError::Invalid("SECRET")));
        }
        let _ = diagnostic.observe::<(), ()>(result, None, true);
        let state = diagnostic.0.lock().unwrap();
        assert_eq!(state.first.as_ref().unwrap().code, "FRESH_INDEX_DIFFERS");
        assert_eq!(
            state
                .secondary
                .iter()
                .map(|failure| failure.stage.code())
                .collect::<Vec<_>>(),
            [
                "STOP",
                "NEGATIVE_TARGET_AUDIT",
                "NEGATIVE_AUDIT",
                "OBSERVATION"
            ]
        );
        assert!(
            state
                .secondary
                .iter()
                .all(|failure| failure.line("SECONDARY").len() < 160)
        );
    }

    #[test]
    fn final_stop_inspection_failure_cannot_disappear_behind_expected_refusal() {
        let diagnostic = Diagnostic::new();
        let _ = diagnostic.run(Stage::Package, || {
            Err::<(), _>(BackupError::Invalid("fresh index differs"))
        });
        diagnostic.enter(Stage::StopInspection);
        assert!(
            diagnostic
                .public_result(Err::<(), _>(BackupError::Invalid(
                    "full body target stop unconfirmed"
                )))
                .is_err()
        );
        let state = diagnostic.0.lock().unwrap();
        assert_eq!(state.first.as_ref().unwrap().code, "FRESH_INDEX_DIFFERS");
        assert!(
            !state.secondary.is_empty(),
            "final stop failure secondary missing"
        );
        assert_eq!(
            state.secondary[0].line("SECONDARY"),
            "FULL_REHEARSAL_FAILURE kind=SECONDARY stage=STOP_INSPECTION class=INVALID code=STOP_INSPECTION_UNCONFIRMED"
        );
    }

    #[test]
    fn accepted_expected_refusal_with_complete_real_audit_and_observation_emits_nothing() {
        use crate::full_restore::negative::{Boundary, NegativeEvidence};
        let diagnostic = Diagnostic::new();
        let result = diagnostic.run(Stage::DumpFreeze, || {
            Err::<(), _>(BackupError::Invalid("custom dump header"))
        });
        diagnostic.settle(Stage::Stop, || Ok(())).unwrap();
        let mut audit = NegativeEvidence::default();
        let inventory = serde_json::json!({"catalog":{},"assets":{},"attempts":{}});
        audit.injected(Boundary::DumpFreeze);
        audit.refused(Boundary::DumpFreeze);
        audit.before(inventory.clone());
        audit.after(inventory);
        diagnostic
            .settle(Stage::NegativeAudit, || {
                audit.validate(Boundary::DumpFreeze, result.is_err())
            })
            .unwrap();
        diagnostic.require_settlement().unwrap();
        let (result, observed) = diagnostic.observe(result, Some(7), true).unwrap();
        assert!(result.is_err());
        assert_eq!(observed, Some(7));
        let mut output = Vec::new();
        assert!(diagnostic.public_result_to(Ok(()), &mut output).is_ok());
        assert!(diagnostic.public_result(Ok(())).is_ok());
        assert!(
            output.is_empty(),
            "accepted negative added forbidden output"
        );
    }

    #[test]
    fn rejected_rehearsal_emits_original_and_secondary_only_at_real_safe_exit() {
        let diagnostic = Diagnostic::new();
        let result = diagnostic.run(Stage::Package, || {
            Err::<(), _>(BackupError::Io(std::io::Error::from_raw_os_error(13)))
        });
        let decision = diagnostic.observe::<(), ()>(result, None, true);
        let mut output = Vec::new();
        assert!(matches!(
            diagnostic.public_result_to(decision, &mut output),
            Err(BackupError::Invalid(
                "full rehearsal failed; see bounded diagnostic"
            ))
        ));
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "FULL_REHEARSAL_FAILURE kind=PRIMARY stage=PACKAGE class=IO code=ERRNO_13\nFULL_REHEARSAL_FAILURE kind=SECONDARY stage=OBSERVATION class=INVALID code=OBSERVATION_MISSING\n"
        );
    }
}

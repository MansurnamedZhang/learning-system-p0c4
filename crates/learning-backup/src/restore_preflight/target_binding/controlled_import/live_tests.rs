//! Opt-in root-only fresh-fixture cases. Every entry needs a separately sealed
//! source/target birth pair and fresh resources; this is not product restore.
use super::super as binding;
use super::super::super as preflight;
use super::*;
use serde::Serialize;
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions, PgSslMode},
};
use std::{collections::BTreeSet, fs::File, sync::Mutex, time::Duration};

use diagnostics::{Case, Failure, Phase};

#[derive(Default, Serialize)]
pub(super) struct Observation {
    pub(super) schema: u32,
    pub(super) case: &'static str,
    pub(super) failure: Option<&'static str>,
    pub(super) checkpoints: BTreeSet<&'static str>,
    pub(super) commit_attempted: bool,
    pub(super) rollback_verified: bool,
    pub(super) retry_allowed: bool,
    pub(super) dump_sha256: String,
    pub(super) toc_sha256: String,
    pub(super) raw_sql_sha256: String,
    pub(super) transformed_sql_sha256: String,
    pub(super) content_sha256: Option<String>,
}
pub(super) type Evidence = Arc<Mutex<Observation>>;

pub(super) fn required(name: &str) -> Result<String, ImportFailure> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or(ImportFailure::Identity)
}

fn config(role: &str) -> Result<preflight::RestorePreflightConfig, ImportFailure> {
    let root = std::path::PathBuf::from(required(&format!("KNOWWEAVE_C4_IMPORT_{role}_ROOT"))?);
    let batch = root
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(ImportFailure::Identity)?;
    let uuid = uuid::Uuid::parse_str(batch).map_err(|_| ImportFailure::Identity)?;
    if uuid.to_string() != batch || uuid.get_version_num() != 4 {
        return Err(ImportFailure::Identity);
    }
    let config = preflight::RestorePreflightConfig {
        control_root: root.join("control"),
        destination_root: root.join("destination"),
        asset_root: root.join("assets"),
        expected_database: format!("learning_restore_c4_{batch}"),
        trust_path: std::env::temp_dir(),
    };
    config.validate().map_err(|_| ImportFailure::Identity)?;
    Ok(config)
}

pub(super) fn control_pool(
    config: &preflight::RestorePreflightConfig,
    guard: &binding::BoundTargetGuard<File, Option<File>>,
) -> Result<impl Future<Output = Result<PgPool, ImportFailure>> + Send + use<>, ImportFailure> {
    let ip = guard.target_ip().map_err(|_| ImportFailure::Identity)?;
    let password = binding::linux::read_admin_password(
        config
            .control_root
            .parent()
            .ok_or(ImportFailure::Identity)?,
    )
    .map_err(|_| ImportFailure::Identity)?;
    let options = PgConnectOptions::new()
        .host(&ip.to_string())
        .port(5432)
        .username("learning_admin")
        .password(&password)
        .database(&config.expected_database)
        .ssl_mode(PgSslMode::Disable);
    drop(password);
    Ok(async move {
        tokio::time::timeout(
            Duration::from_secs(10),
            PgPoolOptions::new()
                .max_connections(2)
                .acquire_timeout(Duration::from_secs(10))
                .connect_with(options),
        )
        .await
        .map_err(|_| ImportFailure::Deadline)?
        .map_err(|_| ImportFailure::Session)
    })
}

async fn execute(case: Case) -> Result<Observation, Failure> {
    if unsafe { libc::geteuid() } != 0
        || required("KNOWWEAVE_C4_IMPORT_CASE").map_err(Failure::at(Phase::AuthorityCaseEnv))?
            != case.name()
    {
        return Err(Failure::at(Phase::AuthorityCaseEnv)(
            ImportFailure::Identity,
        ));
    }
    let source = config("SOURCE").map_err(Failure::at(Phase::SourceConfig))?;
    let target = config("TARGET").map_err(Failure::at(Phase::TargetConfig))?;
    let dump = fixture_sql::capture_fresh_fixture(
        source,
        target.expected_database.clone(),
        case.name(),
        required("KNOWWEAVE_C4_IMPORT_ARTIFACT_ROOT")
            .map_err(Failure::at(Phase::ArtifactRootEnv))?
            .into(),
    )
    .await
    .map_err(Failure::at(Phase::FreshFixtureCapture))?;
    let guard = binding::acquire_for_restore(&target)
        .map_err(|_| Failure::at(Phase::AcquireTargetGuard)(ImportFailure::Identity))?;
    let pool = control_pool(&target, &guard)
        .map_err(Failure::at(Phase::TargetControlConnection))?
        .await
        .map_err(Failure::at(Phase::TargetControlConnection))?;
    drop(guard);
    let admission = linux::admit_candidate_target(target, pool)
        .await
        .map_err(Failure::at(Phase::CandidateAdmission))?;
    linux::run_live_case(admission, dump, case).await
}

async fn run(case: Case) {
    // Ownership work is detached from this awaiting harness and settles its
    // guards/processes/quarantine before an expected result is asserted.
    let result = tokio::spawn(execute(case)).await;
    let record = match diagnostics::settled(case, result) {
        Ok(record) => record,
        Err(event) => {
            eprintln!("{event}");
            panic!("CONTROLLED_IMPORT_CASE_REJECTED");
        }
    };
    println!(
        "KW_C4_IMPORT|{}",
        serde_json::to_string(&record).expect("fixed receipt serialization")
    );
}

macro_rules! live_case {
    ($name:ident, $case:ident) => {
        #[tokio::test]
        #[ignore = "requires separately authorized fresh root PG18 source/target and both compile-time birth pins"]
        async fn $name() { run(Case::$case).await; }
    };
}
live_case!(live_controlled_import_commits_fixture, Success);
live_case!(live_controlled_import_ready_eof, ReadyEof);
live_case!(live_controlled_import_precommit_eof, PrecommitEof);
live_case!(live_controlled_import_cancel_before_commit, Cancel);
live_case!(live_controlled_import_restart_before_ddl, Restart);
live_case!(live_controlled_import_sql_error, SqlError);
live_case!(live_controlled_import_copy_truncated, CopyTruncated);
live_case!(live_controlled_import_attempt_sync_failure, AttemptSync);
live_case!(
    live_controlled_import_commit_intent_sync_failure,
    IntentSync
);
live_case!(
    live_controlled_import_commit_confirmation_lost,
    CommitUnknown
);
live_case!(live_controlled_import_same_id_wrong_endpoint, WrongEndpoint);

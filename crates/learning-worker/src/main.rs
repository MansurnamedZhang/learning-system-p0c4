//! Restricted asset-integrity Worker. It dispatches durable events, claims
//! jobs through the database-clock lease boundary, and never writes a result
//! without the Task 4 processor's current-authorization/fencing checks.
use learning_assets::FsAssetStore;
use learning_core::JobInput;
use learning_db::{
    AssetIntegrityProcessor, AssetProcessOutcome, AssetStore, JobLease, JobStore, SnapshotStore,
};
use learning_worker::ensure_restricted_runtime;
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process,
    str::FromStr,
    time::Duration,
};
use tokio::sync::oneshot;
use uuid::Uuid;

const MAX_LEASE_MS: u64 = 300_000;
const MIN_RUN_LEASE_MS: u64 = 30_000;
const DEFAULT_RUN_LEASE_MS: u64 = 30_000;
const DEFAULT_POLL_MS: u64 = 500;

enum Mode {
    Once { job_id: Uuid, lease_ms: u64 },
    Run { poll_ms: u64, lease_ms: u64 },
}

#[tokio::main]
async fn main() {
    if let Err(message) = run().await {
        eprintln!("worker error: {message}");
        process::exit(78);
    }
}

async fn run() -> Result<(), String> {
    let mode = parse_args(env::args().skip(1).collect())?;
    let url = env::var("DATABASE_URL").map_err(|_| "DATABASE_URL is required")?;
    let asset_root = absolute_path("ASSET_ROOT")?;
    let staging_root = absolute_path("STAGING_ROOT")?;
    let options = PgConnectOptions::from_str(&url).map_err(|_| "invalid DATABASE_URL")?;
    if options.get_username() != "learning_runtime" {
        return Err("worker requires the restricted learning_runtime login".into());
    }
    let pool = PgPoolOptions::new()
        .max_connections(6)
        .connect_with(options)
        .await
        .map_err(|_| "runtime database connection failed")?;
    ensure_restricted_runtime(&pool).await?;
    ensure_asset_root(&pool, &asset_root).await?;
    let files = FsAssetStore::new(asset_root, staging_root)
        .map_err(|_| "asset store path is unavailable")?;
    let jobs = JobStore::new(pool.clone());
    let snapshot_root = env::var_os("SNAPSHOT_ROOT").map(PathBuf::from);
    if snapshot_root
        .as_ref()
        .is_some_and(|root| !root.is_absolute())
    {
        return Err("SNAPSHOT_ROOT must be absolute".into());
    }
    let mut snapshots = SnapshotStore::new(pool.clone());
    if let Some(root) = snapshot_root {
        snapshots = snapshots.with_export_storage(root, files.clone());
    }
    let processor = AssetIntegrityProcessor::new(AssetStore::new(pool, files), jobs.clone());
    match mode {
        Mode::Once { job_id, lease_ms } => {
            jobs.dispatch_pending(256)
                .await
                .map_err(|_| "event dispatch failed")?;
            if let Some(lease) =
                claim_available(&jobs, &snapshots, job_id, Duration::from_millis(lease_ms))
                    .await
                    .map_err(|_| "job claim failed")?
            {
                process_claim(&jobs, &processor, &snapshots, lease, lease_ms).await?;
            }
        }
        Mode::Run { poll_ms, lease_ms } => {
            run_loop(&jobs, &processor, &snapshots, poll_ms, lease_ms).await?;
        }
    }
    Ok(())
}

fn parse_args(args: Vec<String>) -> Result<Mode, String> {
    match args.as_slice() {
        [once, job_flag, job, lease_flag, lease]
            if once == "--once" && job_flag == "--job-id" && lease_flag == "--lease-ms" =>
        {
            let job_id = job.parse().map_err(|_| "invalid job id")?;
            let lease_ms = parse_range(lease, 1, MAX_LEASE_MS, "lease duration")?;
            Ok(Mode::Once { job_id, lease_ms })
        }
        [run] if run == "--run" => Ok(Mode::Run {
            poll_ms: DEFAULT_POLL_MS,
            lease_ms: DEFAULT_RUN_LEASE_MS,
        }),
        [run, poll_flag, poll, lease_flag, lease]
            if run == "--run" && poll_flag == "--poll-ms" && lease_flag == "--lease-ms" =>
        {
            let poll_ms = parse_range(poll, 50, 60_000, "poll interval")?;
            let lease_ms =
                parse_range(lease, MIN_RUN_LEASE_MS, MAX_LEASE_MS, "run lease duration")?;
            Ok(Mode::Run { poll_ms, lease_ms })
        }
        _ => Err(
            "expected --once --job-id UUID --lease-ms N or --run [--poll-ms N --lease-ms N]".into(),
        ),
    }
}

fn parse_range(value: &str, min: u64, max: u64, label: &str) -> Result<u64, String> {
    let value: u64 = value.parse().map_err(|_| format!("invalid {label}"))?;
    if !(min..=max).contains(&value) {
        return Err(format!("invalid {label}"));
    }
    Ok(value)
}

fn absolute_path(name: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(env::var_os(name).ok_or_else(|| format!("{name} is required"))?);
    if !path.is_absolute() {
        return Err(format!("{name} must be absolute"));
    }
    Ok(path)
}

async fn ensure_asset_root(pool: &PgPool, root: &Path) -> Result<(), String> {
    let has_ready_assets: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.asset WHERE status='ready')")
            .fetch_one(pool)
            .await
            .map_err(|_| "cannot inspect ready assets")?;
    validate_asset_root(root, has_ready_assets)
}

fn validate_asset_root(root: &Path, has_ready_assets: bool) -> Result<(), String> {
    let metadata = fs::symlink_metadata(root).map_err(|_| "asset root is unavailable")?;
    if !metadata.file_type().is_dir() {
        return Err("asset root must be a real directory".into());
    }
    if has_ready_assets && !contains_canonical_asset(root)? {
        return Err("asset root has no canonical digest files for ready assets".into());
    }
    Ok(())
}

fn contains_canonical_asset(root: &Path) -> Result<bool, String> {
    let digest_root = root.join("sha256");
    let prefixes = match fs::symlink_metadata(&digest_root) {
        Ok(metadata) if metadata.file_type().is_dir() => {
            fs::read_dir(&digest_root).map_err(|_| "cannot inspect asset digest directory")?
        }
        Ok(_) => return Err("asset digest root must be a real directory".into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err("cannot inspect asset digest directory".into()),
    };
    for prefix in prefixes {
        let prefix = prefix.map_err(|_| "cannot inspect asset digest directory")?;
        let name = prefix.file_name();
        let Some(prefix_name) = name.to_str() else {
            continue;
        };
        if prefix_name.len() != 2
            || !prefix_name
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            || !prefix
                .file_type()
                .map_err(|_| "cannot inspect asset digest directory")?
                .is_dir()
        {
            continue;
        }
        let entries =
            fs::read_dir(prefix.path()).map_err(|_| "cannot inspect asset digest directory")?;
        for entry in entries {
            let entry = entry.map_err(|_| "cannot inspect asset digest directory")?;
            let name = entry.file_name();
            let Some(digest) = name.to_str() else {
                continue;
            };
            if digest.len() == 64
                && digest.starts_with(prefix_name)
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
                && entry
                    .file_type()
                    .map_err(|_| "cannot inspect asset digest directory")?
                    .is_file()
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

async fn claim_available(
    jobs: &JobStore,
    snapshots: &SnapshotStore,
    id: Uuid,
    duration: Duration,
) -> Result<Option<JobLease>, learning_core::ContentError> {
    match jobs.claim(id, duration).await? {
        Some(lease) => Ok(Some(lease)),
        None => snapshots.claim_export(id, duration).await,
    }
}

async fn run_loop(
    jobs: &JobStore,
    processor: &AssetIntegrityProcessor,
    snapshots: &SnapshotStore,
    poll_ms: u64,
    lease_ms: u64,
) -> Result<(), String> {
    // SIGTERM/SIGINT stops new claims. An in-flight processor is allowed to
    // finish under its renewed lease; SIGKILL still relies on lease recovery.
    let mut stop = Box::pin(shutdown_signal());
    eprintln!("worker ready mode=run");
    loop {
        jobs.dispatch_pending(256)
            .await
            .map_err(|_| "event dispatch failed")?;
        let candidates = jobs.runnable_ids(64).await.map_err(|_| "job scan failed")?;
        for job_id in candidates {
            let claim = tokio::select! {
                () = &mut stop => return Ok(()),
                claim = claim_available(jobs, snapshots, job_id, Duration::from_millis(lease_ms)) => {
                    claim.map_err(|_| "job claim failed")?
                }
            };
            if let Some(lease) = claim {
                process_claim(jobs, processor, snapshots, lease, lease_ms).await?;
            }
        }
        tokio::select! {
            () = &mut stop => return Ok(()),
            () = tokio::time::sleep(Duration::from_millis(poll_ms)) => {}
        }
    }
}

async fn process_claim(
    jobs: &JobStore,
    processor: &AssetIntegrityProcessor,
    snapshots: &SnapshotStore,
    lease: JobLease,
    lease_ms: u64,
) -> Result<(), String> {
    debug_gate(&lease).await?;
    let (stop_sender, mut stop_receiver) = oneshot::channel::<()>();
    let heartbeat_jobs = jobs.clone();
    let heartbeat_job_id = lease.job_id;
    let heartbeat_token = lease.token;
    let interval = Duration::from_millis((lease_ms / 3).max(250));
    let heartbeat = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut stop_receiver => return Ok::<(), String>(()),
                _ = tokio::time::sleep(interval) => {
                    match heartbeat_jobs.renew(heartbeat_job_id, heartbeat_token, Duration::from_millis(lease_ms)).await {
                        Ok(true) => {},
                        Ok(false) => return Ok(()),
                        Err(_) => return Err("lease renewal failed".into()),
                    }
                }
            }
        }
    });
    let outcome = match jobs.get(lease.job_id).await {
        Ok(Some(job)) => match job.input {
            JobInput::AssetIntegrity { .. } => processor.process(&lease).await,
            JobInput::SnapshotExport { .. } => snapshots.process_snapshot_export(&lease).await,
        }
        .map_err(|_| "job processor failed"),
        _ => Err("claimed job unavailable"),
    };
    let _ = stop_sender.send(());
    heartbeat
        .await
        .map_err(|_| "lease renewal task stopped unexpectedly")??;
    let outcome = outcome?;
    let state = match outcome {
        AssetProcessOutcome::Succeeded => "succeeded",
        AssetProcessOutcome::RetryWait => "retry_wait",
        AssetProcessOutcome::Failed => "failed",
        AssetProcessOutcome::LeaseLost => "lease_lost",
    };
    eprintln!(
        "worker processed job={} attempt={} outcome={state}",
        lease.job_id, lease.attempt_count
    );
    Ok(())
}

#[cfg(debug_assertions)]
async fn debug_gate(lease: &JobLease) -> Result<(), String> {
    if let Some(path) = env::var_os("KNOWWEAVE_WORKER_TEST_GATE_DIR") {
        let gate = PathBuf::from(path);
        std::fs::write(
            gate.join("claimed"),
            format!("pid={} attempt={}\n", process::id(), lease.attempt_count),
        )
        .map_err(|_| "cannot mark debug gate")?;
        while !gate.join("go").exists() {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    Ok(())
}

#[cfg(not(debug_assertions))]
async fn debug_gate(_: &JobLease) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn shutdown_signal() -> impl std::future::Future<Output = ()> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
    let mut interrupt = signal(SignalKind::interrupt()).expect("SIGINT handler");
    async move {
        tokio::select! {
            _ = term.recv() => {},
            _ = interrupt.recv() => {},
        }
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_empty_asset_root_is_valid_but_hidden_junk_is_not_evidence_for_ready_assets() {
        let root = env::temp_dir().join(format!("knowweave-worker-root-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        assert!(validate_asset_root(&root, false).is_ok());
        fs::write(root.join(".keep"), []).unwrap();
        assert!(validate_asset_root(&root, true).is_err());
        fs::remove_file(root.join(".keep")).unwrap();
        fs::remove_dir(root).unwrap();
    }
}

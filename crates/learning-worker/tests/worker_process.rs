#![cfg(unix)]

use learning_assets::{FsAssetStore, UploadDeclaration};
use learning_core::{
    AssetRef, BlockRef, BodyV3, ContentDraft, ContentV3, CreateContent, Intent, JobInput, Principal,
};
use learning_db::{
    AssetMedia, AssetStore, JobFailureClass, JobStore, MIGRATOR, VersionedContentStore,
};
use learning_worker::ensure_restricted_runtime;
use serde_json::json;
use sqlx::{
    ConnectOptions, PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{
    fs::{self, File},
    ops::{Deref, DerefMut},
    os::unix::process::ExitStatusExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    str::FromStr,
    time::{Duration, Instant},
};
use uuid::Uuid;

const PNG: &[u8] = &[137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 0, 73, 69, 78, 68];
const LEASE_MS: &str = "1800";

struct Rig {
    admin: PgPool,
    runtime: PgPool,
    admin_url: String,
    runtime_url: String,
}

impl Rig {
    async fn new() -> Self {
        let admin_url = std::env::var("TEST_ADMIN_DATABASE_URL").expect("isolated admin DSN");
        let runtime_url = std::env::var("TEST_DATABASE_URL").expect("non-owner runtime DSN");
        let admin = PgPoolOptions::new()
            .max_connections(4)
            .connect(&admin_url)
            .await
            .unwrap();
        MIGRATOR.run(&admin).await.unwrap();
        let runtime = PgPoolOptions::new()
            .max_connections(4)
            .connect(&runtime_url)
            .await
            .unwrap();
        Self {
            admin,
            runtime,
            admin_url,
            runtime_url,
        }
    }

    async fn actor_space(&self) -> (Principal, Uuid) {
        let actor = Principal {
            actor_id: Uuid::new_v4(),
        };
        let space = Uuid::new_v4();
        sqlx::query("INSERT INTO public.app_user VALUES ($1)")
            .bind(actor.actor_id)
            .execute(&self.admin)
            .await
            .unwrap();
        sqlx::query("INSERT INTO public.space VALUES ($1,$2)")
            .bind(space)
            .bind(actor.actor_id)
            .execute(&self.admin)
            .await
            .unwrap();
        sqlx::query("INSERT INTO public.space_grant VALUES ($1,$2,true)")
            .bind(actor.actor_id)
            .bind(space)
            .execute(&self.admin)
            .await
            .unwrap();
        (actor, space)
    }
}

struct Files {
    root: PathBuf,
    assets: FsAssetStore,
    preserve: bool,
}

struct WorkerChild(Child);

fn worker_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_learning-worker"));
    command.env_clear();
    command
}

#[cfg(target_os = "linux")]
fn assert_restricted_child_environment(child: &Child) {
    let contents = fs::read(format!("/proc/{}/environ", child.id()))
        .expect("the running Linux Worker process must expose its environment");
    let mut runtime_url_present = false;
    let mut inherited_test_url = false;
    for item in contents.split(|byte| *byte == 0) {
        let name = item.split(|byte| *byte == b'=').next().unwrap_or(&[]);
        runtime_url_present |= name == b"DATABASE_URL";
        inherited_test_url |= name.starts_with(b"TEST_") && name.ends_with(b"_DATABASE_URL");
    }
    assert!(runtime_url_present, "Worker must receive its runtime DSN");
    assert!(
        !inherited_test_url,
        "Worker must not inherit any test database DSN"
    );
    eprintln!(
        "worker_environment pid={} runtime_dsn_present=true test_database_dsns_absent=true",
        child.id()
    );
}

#[cfg(not(target_os = "linux"))]
fn assert_restricted_child_environment(_: &Child) {}

impl Deref for WorkerChild {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.0
    }
}

impl DerefMut for WorkerChild {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.0
    }
}

impl Drop for WorkerChild {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

impl Files {
    fn new() -> Self {
        let artifact_dir = std::env::var_os("WORKER_TEST_ARTIFACT_DIR");
        let preserve = artifact_dir.is_some();
        let base = artifact_dir
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let root = base.join(format!("p0c2-worker-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let assets = FsAssetStore::new(root.join("assets"), root.join("staging")).unwrap();
        eprintln!("WORKER_TEST_ARTIFACT_DIR={}", root.display());
        Self {
            root,
            assets,
            preserve,
        }
    }

    fn gate(&self, label: &str) -> PathBuf {
        let gate = self.root.join(format!("gate-{label}"));
        fs::create_dir(&gate).unwrap();
        gate
    }

    async fn pending(&self, rig: &Rig, actor: Principal, space: Uuid) -> String {
        let source = self.root.join(format!("source-{}", Uuid::new_v4()));
        fs::write(&source, PNG).unwrap();
        let blob = self
            .assets
            .put_from_file(
                Uuid::new_v4(),
                &source,
                UploadDeclaration {
                    expected_size_bytes: PNG.len() as u64,
                    max_size_bytes: 1_000_000,
                },
            )
            .unwrap();
        let asset = AssetStore::new(rig.runtime.clone(), self.assets.clone())
            .register_verified(
                actor,
                space,
                Uuid::new_v4(),
                blob,
                AssetMedia {
                    media_type: "image/png".into(),
                    original_file_name: "source.png".into(),
                },
            )
            .await
            .unwrap();
        let draft = image_draft(asset.reference);
        let saved = VersionedContentStore::new(rig.runtime.clone())
            .create(
                actor,
                space,
                CreateContent {
                    request_id: Uuid::new_v4(),
                    draft,
                    reason: "worker process test".into(),
                },
            )
            .await
            .unwrap();
        let input = JobInput::asset_integrity(
            actor.actor_id,
            space,
            BlockRef {
                block_id: saved.block_id,
                revision_id: saved.revision_id,
            },
        )
        .unwrap();
        input.business_key()
    }

    async fn job(&self, rig: &Rig, actor: Principal, space: Uuid) -> Uuid {
        let key = self.pending(rig, actor, space).await;
        let jobs = JobStore::new(rig.runtime.clone());
        for _ in 0..32 {
            jobs.dispatch_pending(256).await.unwrap();
            let found: Option<Uuid> =
                sqlx::query_scalar("SELECT id FROM public.job WHERE idempotency_key=$1")
                    .bind(&key)
                    .fetch_optional(&rig.admin)
                    .await
                    .unwrap();
            if let Some(job) = found {
                return job;
            }
        }
        panic!("fixture event was not dispatched within 32 bounded batches")
    }

    fn spawn(&self, rig: &Rig, job: Uuid, label: &str, gate: Option<&Path>) -> WorkerChild {
        let stdout = File::create(self.root.join(format!("{label}.stdout.log"))).unwrap();
        let stderr = File::create(self.root.join(format!("{label}.stderr.log"))).unwrap();
        let mut cmd = worker_command();
        cmd.args([
            "--once",
            "--job-id",
            &job.to_string(),
            "--lease-ms",
            LEASE_MS,
        ])
        .env("DATABASE_URL", &rig.runtime_url)
        .env("ASSET_ROOT", self.root.join("assets"))
        .env("STAGING_ROOT", self.root.join("staging"))
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
        if let Some(gate) = gate {
            cmd.env("KNOWWEAVE_WORKER_TEST_GATE_DIR", gate);
        }
        let child = cmd.spawn().unwrap();
        eprintln!("worker_spawn label={label} pid={} job={job}", child.id());
        WorkerChild(child)
    }

    fn spawn_daemon(&self, rig: &Rig, label: &str) -> WorkerChild {
        let stdout = File::create(self.root.join(format!("{label}.stdout.log"))).unwrap();
        let stderr = File::create(self.root.join(format!("{label}.stderr.log"))).unwrap();
        let child = worker_command()
            .args(["--run", "--poll-ms", "50", "--lease-ms", "30000"])
            .env("DATABASE_URL", &rig.runtime_url)
            .env("ASSET_ROOT", self.root.join("assets"))
            .env("STAGING_ROOT", self.root.join("staging"))
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .unwrap();
        eprintln!("worker_spawn label={label} pid={} mode=daemon", child.id());
        WorkerChild(child)
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        if let Ok(entries) = fs::read_dir(&self.root) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with("gate-") {
                    let _ = fs::remove_dir_all(entry.path());
                }
            }
        }
        if !self.preserve {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

fn image_draft(asset: AssetRef) -> ContentDraft {
    ContentDraft::V3(ContentV3 {
        intent: Intent::Note,
        language: "en".into(),
        title: "Worker process fixture".into(),
        body: BodyV3::Figure {
            asset,
            usage: "lecture_diagram".into(),
            caption: "Diagram".into(),
            alt: "Diagram".into(),
            decorative: false,
        },
        basis_refs: vec![],
        requires_context: vec![],
        source_run: None,
    })
}

async fn claimed(rig: &Rig, child: &mut Child, gate: &Path, job: Uuid, attempt: i32) -> Uuid {
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        if gate.join("claimed").is_file() {
            assert_restricted_child_environment(child);
            let row: (String, i32, Option<Uuid>, bool, bool) = sqlx::query_as(
                "SELECT status,attempt_count,lease_token,lease_expires_at>clock_timestamp(),output_digest IS NULL FROM public.job WHERE id=$1",
            ).bind(job).fetch_one(&rig.admin).await.unwrap();
            assert_eq!(row.0, "running");
            assert_eq!(row.1, attempt);
            assert!(row.3, "lease must still be current when the gate opens");
            assert!(row.4, "no output may exist before processing");
            let token = row.2.expect("running job has lease token");
            eprintln!(
                "job_claimed job={job} attempt={attempt} status=running token_present=true output_present=false"
            );
            return token;
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!(
                "worker exited before claim: pid={} exit={status}",
                child.id()
            );
        }
        assert!(
            Instant::now() < deadline,
            "worker did not claim job before deadline"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn db_clock_expired(rig: &Rig, job: Uuid) {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let row: (String, bool, Option<String>) = sqlx::query_as(
            "SELECT status,lease_expires_at<=clock_timestamp(),output_digest FROM public.job WHERE id=$1",
        ).bind(job).fetch_one(&rig.admin).await.unwrap();
        if row.1 {
            assert_eq!(row.0, "running");
            assert!(row.2.is_none());
            eprintln!("job_expired_by_db_clock job={job} status=running output_present=false");
            return;
        }
        assert!(
            Instant::now() < deadline,
            "database clock did not reach lease deadline"
        );
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

async fn wait_exit(child: &mut Child) -> std::process::ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "worker pid={} did not exit",
            child.id()
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn no_result(rig: &Rig, job: Uuid) {
    let row: (Option<String>, i64) = sqlx::query_as(
        "SELECT j.output_digest,(SELECT count(*) FROM public.asset_integrity_result WHERE job_id=j.id) FROM public.job j WHERE j.id=$1",
    ).bind(job).fetch_one(&rig.admin).await.unwrap();
    assert_eq!(row, (None, 0));
}

#[tokio::test]
async fn owner_database_identity_is_rejected_before_any_claim() {
    let rig = Rig::new().await;
    let files = Files::new();
    let (actor, space) = rig.actor_space().await;
    let job = files.job(&rig, actor, space).await;
    let stdout = File::create(files.root.join("owner.stdout.log")).unwrap();
    let stderr = File::create(files.root.join("owner.stderr.log")).unwrap();
    let child = worker_command()
        .args([
            "--once",
            "--job-id",
            &job.to_string(),
            "--lease-ms",
            LEASE_MS,
        ])
        .env("DATABASE_URL", &rig.admin_url)
        .env("ASSET_ROOT", files.root.join("assets"))
        .env("STAGING_ROOT", files.root.join("staging"))
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .unwrap();
    let mut child = WorkerChild(child);
    let status = wait_exit(&mut child).await;
    assert_eq!(status.code(), Some(78));
    let row: (String, i32) =
        sqlx::query_as("SELECT status,attempt_count FROM public.job WHERE id=$1")
            .bind(job)
            .fetch_one(&rig.admin)
            .await
            .unwrap();
    assert_eq!(row, ("queued".into(), 0));
    no_result(&rig, job).await;
}

#[tokio::test]
async fn privileged_login_with_runtime_current_role_is_rejected_before_claim() {
    let rig = Rig::new().await;
    let files = Files::new();
    let (actor, space) = rig.actor_space().await;
    let job = files.job(&rig, actor, space).await;
    let super_url = std::env::var("TEST_SUPERUSER_DATABASE_URL")
        .expect("isolated test superuser DSN is required for session identity RED");
    let probe = PgPoolOptions::new()
        .max_connections(1)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE learning_runtime")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&super_url)
        .await
        .unwrap_or_else(|_| panic!("isolated superuser must be able to SET ROLE"));
    let identities: (String, String) =
        sqlx::query_as("SELECT session_user::text,current_user::text")
            .fetch_one(&probe)
            .await
            .unwrap();
    assert_ne!(identities.0, "learning_runtime");
    assert_eq!(identities.1, "learning_runtime");
    let claim_privilege: bool = sqlx::query_scalar(
        "SELECT has_function_privilege(current_user,'public.p0c2_claim_job(uuid,bigint)','EXECUTE')",
    )
    .fetch_one(&probe)
    .await
    .unwrap();
    assert!(
        claim_privilege,
        "current role can claim if startup accepts it"
    );
    eprintln!("identity_probe session_is_runtime=false current_is_runtime=true");
    assert!(
        ensure_restricted_runtime(&probe).await.is_err(),
        "a privileged login must be rejected even after SET ROLE learning_runtime"
    );
    probe.close().await;
    let row: (String, i32) =
        sqlx::query_as("SELECT status,attempt_count FROM public.job WHERE id=$1")
            .bind(job)
            .fetch_one(&rig.admin)
            .await
            .unwrap();
    assert_eq!(row, ("queued".into(), 0));
    no_result(&rig, job).await;
}

#[tokio::test]
async fn privileged_login_with_session_authorization_is_rejected_before_claim() {
    let rig = Rig::new().await;
    let files = Files::new();
    let (actor, space) = rig.actor_space().await;
    let job = files.job(&rig, actor, space).await;
    let super_url = std::env::var("TEST_SUPERUSER_DATABASE_URL")
        .expect("isolated test superuser DSN is required for session identity RED");
    let probe = PgPoolOptions::new()
        .max_connections(1)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET SESSION AUTHORIZATION learning_runtime")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&super_url)
        .await
        .unwrap_or_else(|_| panic!("isolated superuser must be able to set session authorization"));
    let (session, current, system): (String, String, Option<String>) =
        sqlx::query_as("SELECT session_user::text,current_user::text,system_user")
            .fetch_one(&probe)
            .await
            .unwrap();
    assert_eq!(session, "learning_runtime");
    assert_eq!(current, "learning_runtime");
    let (_, authenticated) = system
        .as_deref()
        .and_then(|value| value.split_once(':'))
        .expect("isolated test must use authenticated PostgreSQL 18 connections");
    assert_eq!(authenticated, "postgres");
    eprintln!(
        "identity_probe session_is_runtime=true current_is_runtime=true original_login_is_runtime=false"
    );
    assert!(
        ensure_restricted_runtime(&probe).await.is_err(),
        "a privileged login must be rejected after SET SESSION AUTHORIZATION"
    );
    probe.close().await;
    let row: (String, i32) =
        sqlx::query_as("SELECT status,attempt_count FROM public.job WHERE id=$1")
            .bind(job)
            .fetch_one(&rig.admin)
            .await
            .unwrap();
    assert_eq!(row, ("queued".into(), 0));
    no_result(&rig, job).await;
}

async fn asset_mount_rejected_before_claim(empty: bool) {
    let rig = Rig::new().await;
    let files = Files::new();
    let (actor, space) = rig.actor_space().await;
    let job = files.job(&rig, actor, space).await;
    let mount = files.root.join(if empty {
        "empty-assets"
    } else {
        "absent-assets"
    });
    if empty {
        fs::create_dir(&mount).unwrap();
    } else {
        assert!(!mount.exists());
    }
    let label = if empty { "empty-mount" } else { "absent-mount" };
    let stdout = File::create(files.root.join(format!("{label}.stdout.log"))).unwrap();
    let stderr = File::create(files.root.join(format!("{label}.stderr.log"))).unwrap();
    let child = worker_command()
        .args([
            "--once",
            "--job-id",
            &job.to_string(),
            "--lease-ms",
            LEASE_MS,
        ])
        .env("DATABASE_URL", &rig.runtime_url)
        .env("ASSET_ROOT", mount)
        .env("STAGING_ROOT", files.root.join("staging"))
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .unwrap();
    let mut child = WorkerChild(child);
    let status = wait_exit(&mut child).await;
    eprintln!(
        "worker_exit label={label} pid={} code={:?}",
        child.id(),
        status.code()
    );
    assert_eq!(
        status.code(),
        Some(78),
        "bad asset mount must be rejected at startup"
    );
    let row: (String, i32) =
        sqlx::query_as("SELECT status,attempt_count FROM public.job WHERE id=$1")
            .bind(job)
            .fetch_one(&rig.admin)
            .await
            .unwrap();
    assert_eq!(row, ("queued".into(), 0));
    no_result(&rig, job).await;
}

#[tokio::test]
async fn missing_asset_mount_is_rejected_before_claim() {
    asset_mount_rejected_before_claim(false).await;
}

#[tokio::test]
async fn empty_asset_mount_is_rejected_before_claim() {
    asset_mount_rejected_before_claim(true).await;
}

fn terminate(child: &Child) {
    let sent = Command::new("kill")
        .arg("-TERM")
        .arg(child.id().to_string())
        .status()
        .expect("the isolated Linux test image must provide kill");
    assert!(sent.success(), "SIGTERM was not delivered to the Worker");
}

async fn daemon_ready(files: &Files, child: &mut Child, label: &str) {
    let log = files.root.join(format!("{label}.stderr.log"));
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        if fs::read_to_string(&log)
            .unwrap()
            .contains("worker ready mode=run")
        {
            assert_restricted_child_environment(child);
            eprintln!("daemon_ready label={label} pid={}", child.id());
            return;
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!("daemon exited before startup readiness: {status}");
        }
        assert!(
            Instant::now() < deadline,
            "daemon startup readiness timed out"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test]
async fn new_database_with_empty_asset_volume_starts_and_stops_cleanly() {
    let super_url = std::env::var("TEST_SUPERUSER_DATABASE_URL")
        .expect("isolated test superuser DSN is required for empty database fixture");
    let super_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&super_url)
        .await
        .unwrap();
    let database = format!("worker_empty_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {database} OWNER learning_admin"))
        .execute(&super_pool)
        .await
        .unwrap();
    sqlx::query(&format!("REVOKE ALL ON DATABASE {database} FROM PUBLIC"))
        .execute(&super_pool)
        .await
        .unwrap();
    sqlx::query(&format!(
        "GRANT CONNECT, TEMPORARY ON DATABASE {database} TO learning_admin, learning_runtime"
    ))
    .execute(&super_pool)
    .await
    .unwrap();

    let admin_options = PgConnectOptions::from_str(
        &std::env::var("TEST_ADMIN_DATABASE_URL").expect("isolated admin DSN"),
    )
    .unwrap()
    .database(&database);
    let runtime_options = PgConnectOptions::from_str(
        &std::env::var("TEST_DATABASE_URL").expect("non-owner runtime DSN"),
    )
    .unwrap()
    .database(&database);
    let admin_url = admin_options.to_url_lossy().to_string();
    let runtime_url = runtime_options.to_url_lossy().to_string();
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(admin_options)
        .await
        .unwrap();
    MIGRATOR.run(&admin).await.unwrap();
    let runtime = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(runtime_options)
        .await
        .unwrap();
    let rig = Rig {
        admin_url,
        runtime_url,
        admin,
        runtime,
    };
    let ready_assets: i64 = sqlx::query_scalar("SELECT count(*) FROM public.asset")
        .fetch_one(&rig.runtime)
        .await
        .unwrap();
    assert_eq!(ready_assets, 0);
    let files = Files::new();
    assert_eq!(fs::read_dir(files.root.join("assets")).unwrap().count(), 0);
    let mut worker = files.spawn_daemon(&rig, "new-empty-volume");
    daemon_ready(&files, &mut worker, "new-empty-volume").await;
    terminate(&worker);
    let stopped = wait_exit(&mut worker).await;
    assert!(stopped.success(), "new empty volume must be accepted");
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM public.job")
        .fetch_one(&rig.admin)
        .await
        .unwrap();
    assert_eq!(jobs, 0);
    rig.runtime.close().await;
    rig.admin.close().await;
    drop(rig);
    sqlx::query(&format!("DROP DATABASE {database} WITH (FORCE)"))
        .execute(&super_pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn daemon_dispatches_pending_event_then_stops_and_restarts_without_duplication() {
    let rig = Rig::new().await;
    let files = Files::new();
    let (actor, space) = rig.actor_space().await;
    let key = files.pending(&rig, actor, space).await;
    let pending: (i64, bool) = sqlx::query_as(
        "SELECT count(*),bool_and(dispatched_at IS NULL) FROM public.job_outbox WHERE business_key=$1",
    )
    .bind(&key)
    .fetch_one(&rig.admin)
    .await
    .unwrap();
    assert_eq!(pending, (1, true));

    let mut first = files.spawn_daemon(&rig, "daemon-first");
    daemon_ready(&files, &mut first, "daemon-first").await;
    let deadline = Instant::now() + Duration::from_secs(20);
    let completed = loop {
        let found: Option<(Uuid, String, i32, Option<String>)> = sqlx::query_as(
            "SELECT id,status,attempt_count,output_digest FROM public.job WHERE idempotency_key=$1",
        )
        .bind(&key)
        .fetch_optional(&rig.admin)
        .await
        .unwrap();
        if let Some((id, status, attempts, digest)) = found
            && status == "succeeded"
        {
            break (id, status, attempts, digest);
        }
        if let Some(status) = first.try_wait().unwrap() {
            panic!("daemon exited before dispatch and completion: {status}");
        }
        assert!(
            Instant::now() < deadline,
            "daemon did not complete pending event"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(completed.1, "succeeded");
    assert_eq!(completed.2, 1);
    assert_eq!(completed.3.as_ref().map(String::len), Some(64));
    let results: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public.asset_integrity_result WHERE job_id=$1")
            .bind(completed.0)
            .fetch_one(&rig.admin)
            .await
            .unwrap();
    assert_eq!(results, 1);
    eprintln!(
        "daemon_completed pid={} job={} attempt=1 result_count=1",
        first.id(),
        completed.0
    );
    terminate(&first);
    let stopped = wait_exit(&mut first).await;
    eprintln!(
        "daemon_exit pid={} signal={:?} code={:?}",
        first.id(),
        stopped.signal(),
        stopped.code()
    );
    assert!(stopped.success(), "SIGTERM must stop daemon cleanly");

    let mut second = files.spawn_daemon(&rig, "daemon-restart");
    daemon_ready(&files, &mut second, "daemon-restart").await;
    terminate(&second);
    assert!(wait_exit(&mut second).await.success());
    let unchanged: (String, i32, Option<String>, i64) = sqlx::query_as(
        "SELECT j.status,j.attempt_count,j.output_digest,(SELECT count(*) FROM public.asset_integrity_result WHERE job_id=j.id) FROM public.job j WHERE j.id=$1",
    )
    .bind(completed.0)
    .fetch_one(&rig.admin)
    .await
    .unwrap();
    assert_eq!(unchanged, (completed.1, completed.2, completed.3, 1));
    let event: (i64, bool) = sqlx::query_as(
        "SELECT count(*),bool_and(dispatched_at IS NOT NULL) FROM public.job_outbox WHERE business_key=$1",
    )
    .bind(&key)
    .fetch_one(&rig.admin)
    .await
    .unwrap();
    assert_eq!(event, (1, true));
}

#[tokio::test]
async fn sigkill_after_claim_restarts_on_second_process_with_one_result() {
    let rig = Rig::new().await;
    let files = Files::new();
    let (actor, space) = rig.actor_space().await;
    let job = files.job(&rig, actor, space).await;
    let jobs = JobStore::new(rig.runtime.clone());

    let first_gate = files.gate("first");
    let mut first = files.spawn(&rig, job, "first", Some(&first_gate));
    let old_token = claimed(&rig, &mut first, &first_gate, job, 1).await;
    let first_pid = first.id();
    first
        .kill()
        .expect("OS SIGKILL must terminate the first real worker process");
    let killed = first.wait().unwrap();
    eprintln!(
        "worker_exit label=first pid={first_pid} signal={:?} code={:?}",
        killed.signal(),
        killed.code()
    );
    assert_eq!(
        killed.signal(),
        Some(9),
        "the first worker must die from SIGKILL"
    );
    no_result(&rig, job).await;
    db_clock_expired(&rig, job).await;

    let second_gate = files.gate("second");
    let mut second = files.spawn(&rig, job, "second", Some(&second_gate));
    let new_token = claimed(&rig, &mut second, &second_gate, job, 2).await;
    assert_ne!(
        new_token, old_token,
        "fresh attempt must have a new opaque token"
    );
    assert!(
        !jobs
            .renew(job, old_token, Duration::from_secs(2))
            .await
            .unwrap()
    );
    assert!(
        !jobs
            .checkpoint(job, old_token, json!({"stale": true}))
            .await
            .unwrap()
    );
    assert!(!jobs.succeed(job, old_token, &"a".repeat(64)).await.unwrap());
    assert!(
        !jobs
            .fail(job, old_token, JobFailureClass::TransientStorage)
            .await
            .unwrap()
    );
    eprintln!("stale_token_fenced job={job} all_four_mutations_rejected=true");
    no_result(&rig, job).await;

    fs::write(second_gate.join("go"), b"go").unwrap();
    let second_status = wait_exit(&mut second).await;
    eprintln!(
        "worker_exit label=second pid={} signal={:?} code={:?}",
        second.id(),
        second_status.signal(),
        second_status.code()
    );
    assert!(
        second_status.success(),
        "second process must publish the result"
    );
    let done: (String, i32, Option<String>, i64) = sqlx::query_as(
        "SELECT j.status,j.attempt_count,j.output_digest,(SELECT count(*) FROM public.asset_integrity_result WHERE job_id=j.id) FROM public.job j WHERE j.id=$1",
    ).bind(job).fetch_one(&rig.admin).await.unwrap();
    assert_eq!(done.0, "succeeded");
    assert_eq!(done.1, 2);
    assert_eq!(done.2.as_ref().map(String::len), Some(64));
    assert_eq!(done.3, 1);
    eprintln!("job_completed job={job} attempt=2 result_count=1 digest_present=true");

    let mut replay = files.spawn(&rig, job, "replay", None);
    assert!(wait_exit(&mut replay).await.success());
    jobs.dispatch_pending(256).await.unwrap();
    let same_outbox_jobs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.job WHERE outbox_id=(SELECT outbox_id FROM public.job WHERE id=$1)",
    )
    .bind(job)
    .fetch_one(&rig.admin)
    .await
    .unwrap();
    assert_eq!(same_outbox_jobs, 1);
    let after: (String, i32, Option<String>, i64) = sqlx::query_as(
        "SELECT j.status,j.attempt_count,j.output_digest,(SELECT count(*) FROM public.asset_integrity_result WHERE job_id=j.id) FROM public.job j WHERE j.id=$1",
    ).bind(job).fetch_one(&rig.admin).await.unwrap();
    assert_eq!(
        after, done,
        "replay must converge without rewriting the result"
    );
}

#[tokio::test]
async fn cancellation_after_claim_prevents_the_process_from_publishing() {
    let rig = Rig::new().await;
    let files = Files::new();
    let (actor, space) = rig.actor_space().await;
    let job = files.job(&rig, actor, space).await;
    let gate = files.gate("cancel");
    let mut worker = files.spawn(&rig, job, "cancel", Some(&gate));
    claimed(&rig, &mut worker, &gate, job, 1).await;
    assert!(
        JobStore::new(rig.runtime.clone())
            .cancel(job)
            .await
            .unwrap()
    );
    fs::write(gate.join("go"), b"go").unwrap();
    let status = wait_exit(&mut worker).await;
    eprintln!(
        "worker_exit label=cancel pid={} signal={:?} code={:?}",
        worker.id(),
        status.signal(),
        status.code()
    );
    assert!(
        status.success(),
        "a cancelled attempt should terminate cleanly"
    );
    let final_status: String = sqlx::query_scalar("SELECT status FROM public.job WHERE id=$1")
        .bind(job)
        .fetch_one(&rig.admin)
        .await
        .unwrap();
    assert_eq!(final_status, "cancelled");
    no_result(&rig, job).await;
}

#[tokio::test]
async fn revocation_after_claim_prevents_the_process_from_publishing() {
    let rig = Rig::new().await;
    let files = Files::new();
    let (actor, space) = rig.actor_space().await;
    let job = files.job(&rig, actor, space).await;
    let gate = files.gate("revoke");
    let mut worker = files.spawn(&rig, job, "revoke", Some(&gate));
    claimed(&rig, &mut worker, &gate, job, 1).await;
    sqlx::query("DELETE FROM public.space_grant WHERE actor_id=$1 AND space_id=$2")
        .bind(actor.actor_id)
        .bind(space)
        .execute(&rig.admin)
        .await
        .unwrap();
    fs::write(gate.join("go"), b"go").unwrap();
    let status = wait_exit(&mut worker).await;
    eprintln!(
        "worker_exit label=revoke pid={} signal={:?} code={:?}",
        worker.id(),
        status.signal(),
        status.code()
    );
    assert!(
        status.success(),
        "a revoked attempt should terminate cleanly"
    );
    let final_status: String = sqlx::query_scalar("SELECT status FROM public.job WHERE id=$1")
        .bind(job)
        .fetch_one(&rig.admin)
        .await
        .unwrap();
    assert_eq!(final_status, "failed");
    no_result(&rig, job).await;
}

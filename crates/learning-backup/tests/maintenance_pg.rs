#![cfg(target_os = "linux")]
use learning_assets::backup_fs::BackupDir;
use learning_assets::{FsAssetStore, UploadDeclaration};
use learning_backup::{
    GatePhase, SourceBackupConfig, SourceGateJournal, prepare_source_backup, verify_sealed,
};
use sha2::{Digest, Sha256};
use sqlx::{Connection, PgConnection, postgres::PgPoolOptions};
use std::{
    collections::HashSet,
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
use uuid::Uuid;

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} required")))
}

#[derive(Debug, PartialEq, Eq)]
struct BindingInventory {
    root_identity: (u64, u64),
    file_identity: (u64, u64),
    uid: u32,
    mode: u32,
    links: u64,
    bytes: Vec<u8>,
}

fn binding_inventory(path: &Path) -> BindingInventory {
    let root = BackupDir::open_trusted_private_root(path).unwrap();
    let file = root
        .open_file("source-binding.json")
        .expect("independently preissued binding required");
    let metadata = file.metadata().unwrap();
    assert!(metadata.len() > 0 && metadata.len() <= 4096);
    BindingInventory {
        root_identity: root.identity().unwrap(),
        file_identity: (metadata.dev(), metadata.ino()),
        uid: metadata.uid(),
        mode: metadata.mode() & 0o7777,
        links: metadata.nlink(),
        bytes: fs::read(path.join("source-binding.json")).unwrap(),
    }
}

/// Consume a record independently issued before compilation. This fixture
/// never issues/registers a binding or supplies a runtime expected digest.
fn preissued_fixture(path: &Path) -> BindingInventory {
    let _registry = learning_backup::ManagementRegistry::open_installed()
        .expect("independently provisioned current registry required")
        .try_lock()
        .expect("fixture registry not busy");
    let snapshot = binding_inventory(path);
    assert_eq!(snapshot.uid, 0);
    assert_eq!(snapshot.mode, 0o600);
    assert_eq!(snapshot.links, 1);
    let pin = match option_env!("KNOWWEAVE_C4_SOURCE_CONTROL_BINDING_SHA256") {
        Some(pin) => pin,
        None => panic!("fixture requires updated independent issuer and build pin"),
    };
    assert_eq!(format!("{:x}", Sha256::digest(&snapshot.bytes)), pin);
    let receipt_path = path
        .parent()
        .unwrap()
        .join("proofs/registry-provisioning.json");
    let receipt: serde_json::Value = serde_json::from_slice(
        &fs::read(receipt_path).expect("independent registry receipt required"),
    )
    .unwrap();
    assert_eq!(receipt["source_binding_sha256"], serde_json::json!(pin));
    assert_eq!(
        receipt["application_build_sha256"].as_str(),
        option_env!("KNOWWEAVE_BUILD_ID_SHA256")
    );
    assert_eq!(
        receipt["application_commit"].as_str(),
        option_env!("KNOWWEAVE_SOURCE_COMMIT")
    );
    assert_eq!(receipt["state"], serde_json::json!("READBACK_DURABLE"));
    assert_eq!(
        BackupDir::open_trusted_private_root(path)
            .unwrap()
            .list()
            .unwrap(),
        vec!["source-binding.json"]
    );
    snapshot
}

/// Run only against a *new*, migrated, UUID-named PG18 database inside a new
/// Compose project, with all paths private and dedicated to this batch.
/// Full dump/pin scenario is deferred until its independent driver issues the
/// source binding before the matching compile-pinned build. The bound NO_DUMP
/// public prepared gate and live admission old-session drain are current gates.
#[tokio::test]
#[ignore = "deferred: requires updated independent issuer/build pin, fresh PG18 and private Linux roots"]
async fn real_gate_waits_for_old_runtime_session_and_rejects_new_runtime_login() {
    assert!(option_env!("KNOWWEAVE_SOURCE_COMMIT").is_some());
    assert!(option_env!("KNOWWEAVE_BUILD_ID_SHA256").is_some());
    let expected = std::env::var("TEST_C4_TASK3_DATABASE_NAME").unwrap();
    let suffix = expected.strip_prefix("learning_backup_c4_task3_").unwrap();
    assert_eq!(Uuid::parse_str(suffix).unwrap().to_string(), suffix);
    let admin_url = std::env::var("TEST_ADMIN_DATABASE_URL").unwrap();
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&admin_url)
        .await
        .unwrap();
    let actual: String = sqlx::query_scalar("SELECT current_database()::text")
        .fetch_one(&admin)
        .await
        .unwrap();
    assert_eq!(actual, expected);
    // A second pre-existing session keeps the drain gate observable. The
    // root driver performs the real runtime login denial without handing its
    // runtime credential to this management process.
    let mut held = PgConnection::connect(&admin_url).await.unwrap();
    sqlx::query("BEGIN").execute(&mut held).await.unwrap();
    sqlx::query("SELECT 1").execute(&mut held).await.unwrap();

    let control = env_path("TEST_C4_CONTROL_ROOT");
    let pin = env_path("TEST_C4_PIN_ROOT");
    let asset_root = env_path("TEST_C4_ASSET_ROOT");
    let stage_root = env_path("TEST_C4_ASSET_STAGE_ROOT");
    let binding_before = preissued_fixture(&control);
    for path in [&pin, &asset_root, &stage_root] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert!(fs::read_dir(path).unwrap().next().is_none());
    }
    let assets = FsAssetStore::new(asset_root, stage_root).unwrap();
    let actor = Uuid::new_v4();
    let space = Uuid::new_v4();
    sqlx::query("INSERT INTO public.app_user(id) VALUES($1)")
        .bind(actor)
        .execute(&admin)
        .await
        .unwrap();
    sqlx::query("INSERT INTO public.space(id,owner_id) VALUES($1,$2)")
        .bind(space)
        .bind(actor)
        .execute(&admin)
        .await
        .unwrap();
    let input = control.join("fixture-original.bin");
    fs::write(&input, b"C4 nonempty immutable original").unwrap();
    let blob = assets
        .put_from_file(
            Uuid::new_v4(),
            &input,
            UploadDeclaration {
                expected_size_bytes: 30,
                max_size_bytes: 30,
            },
        )
        .unwrap();
    fs::remove_file(&input).unwrap();
    for id in [Uuid::new_v4(), Uuid::new_v4()] {
        sqlx::query("INSERT INTO public.asset(space_id,id,sha256,byte_size,storage_key,media_type,original_file_name,status) VALUES($1,$2,$3,$4,$5,'application/octet-stream','original.bin','ready')")
            .bind(space).bind(id).bind(blob.sha256()).bind(blob.size_bytes() as i64)
            .bind(blob.storage_key()).execute(&admin).await.unwrap();
    }
    let assets_for_reconcile = assets.clone();
    let config = SourceBackupConfig {
        backup_id: std::env::var("TEST_C4_BACKUP_ID").unwrap().parse().unwrap(),
        expected_database: expected,
        expected_compose_project: std::env::var("TEST_C4_COMPOSE_PROJECT").unwrap(),
        isolation_attestation: env_path("TEST_C4_ISOLATION_ATTESTATION"),
        control_root: control.clone(),
        local_pin_root: pin.clone(),
        pg_dump_executable: env_path("TEST_C4_PGDUMP_BIN"),
        pgpassfile: env_path("TEST_C4_PGPASSFILE"),
        pg_host: std::env::var("TEST_C4_PGHOST").unwrap(),
        pg_port: std::env::var("TEST_C4_PGPORT").unwrap().parse().unwrap(),
        drain_timeout: Duration::from_secs(15),
    };
    let task_config = config.clone();
    let task_admin = admin.clone();
    let task =
        tokio::spawn(
            async move { prepare_source_backup(&task_admin, &assets, &task_config).await },
        );
    let closed = control
        .join(format!("{}.control", config.backup_id))
        .join("closed.json");
    for _ in 0..100 {
        if closed.exists() {
            break;
        }
        assert!(!task.is_finished(), "gate failed before closing");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(closed.exists(), "runtime CONNECT was never revoked");
    assert!(!task.is_finished(), "existing transaction was not drained");
    let can_connect: bool = sqlx::query_scalar(
        "SELECT has_database_privilege('learning_runtime',current_database(),'CONNECT')",
    )
    .fetch_one(&mut held)
    .await
    .unwrap();
    assert!(!can_connect);
    let probe = config
        .isolation_attestation
        .parent()
        .unwrap()
        .join(format!("runtime-connect-denied-{}.json", config.backup_id));
    for _ in 0..100 {
        if probe.exists() {
            break;
        }
        assert!(
            !task.is_finished(),
            "source released before real runtime probe"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        probe.exists(),
        "root driver did not prove runtime login denied"
    );
    sqlx::query("ROLLBACK").execute(&mut held).await.unwrap();
    held.close().await.unwrap();
    let result = task.await.unwrap().unwrap();
    assert_eq!(result.manifest().backup_id, config.backup_id);
    assert_eq!(result.manifest().logical_asset_count, 2);
    assert_eq!(result.manifest().unique_asset_bytes, blob.size_bytes());
    let protected = HashSet::from([blob.storage_key().to_owned()]);
    let candidates = assets_for_reconcile
        .reconcile(&protected, SystemTime::now() + Duration::from_secs(1))
        .unwrap();
    assert!(
        !candidates
            .candidates
            .iter()
            .any(|candidate| candidate.path == blob.storage_key())
    );
    let recovered = SourceGateJournal::recover(&control, config.backup_id).unwrap();
    assert_eq!(recovered.record().phase(), GatePhase::Released);
    assert_eq!(
        verify_sealed(&pin, config.backup_id)
            .unwrap()
            .manifest_sha256(),
        result.sealed().manifest_sha256()
    );
    assert!(!pin.join(format!("{}.complete", config.backup_id)).exists());
    let can_connect: bool = sqlx::query_scalar(
        "SELECT has_database_privilege('learning_runtime',current_database(),'CONNECT')",
    )
    .fetch_one(&admin)
    .await
    .unwrap();
    assert!(can_connect);
    assert_eq!(
        binding_inventory(&control),
        binding_before,
        "full capture must preserve independent binding"
    );
}

/// Run this ignored case twice, with FAILURE_KIND=missing_original and
/// FAILURE_KIND=pg_dump_exit, each against a different fresh UUID database,
/// Compose project, roots, and root-driver lock. A source failure must never
/// release runtime CONNECT or publish a sealed/complete package.
/// Deferred full capture failures require the updated independent issuer to
/// provision source-binding.json and embed its pin before compiling this test.
#[tokio::test]
#[ignore = "deferred: requires updated independent issuer/build pin and two root-driver PG18 failure projects"]
async fn missing_original_or_pg_dump_failure_keeps_gate_closed() {
    let kind = std::env::var("TEST_C4_FAILURE_KIND").unwrap();
    assert!(matches!(kind.as_str(), "missing_original" | "pg_dump_exit"));
    let database = std::env::var("TEST_C4_TASK3_DATABASE_NAME").unwrap();
    let suffix = database.strip_prefix("learning_backup_c4_task3_").unwrap();
    assert_eq!(Uuid::parse_str(suffix).unwrap().to_string(), suffix);
    let admin_url = std::env::var("TEST_ADMIN_DATABASE_URL").unwrap();
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&admin_url)
        .await
        .unwrap();
    let control = env_path("TEST_C4_CONTROL_ROOT");
    let binding_before = preissued_fixture(&control);
    let pin = env_path("TEST_C4_PIN_ROOT");
    let assets = FsAssetStore::new(
        env_path("TEST_C4_ASSET_ROOT"),
        env_path("TEST_C4_ASSET_STAGE_ROOT"),
    )
    .unwrap();
    if kind == "missing_original" {
        let actor = Uuid::new_v4();
        let space = Uuid::new_v4();
        let digest = "a".repeat(64);
        sqlx::query("INSERT INTO public.app_user(id) VALUES($1)")
            .bind(actor)
            .execute(&admin)
            .await
            .unwrap();
        sqlx::query("INSERT INTO public.space(id,owner_id) VALUES($1,$2)")
            .bind(space)
            .bind(actor)
            .execute(&admin)
            .await
            .unwrap();
        sqlx::query("INSERT INTO public.asset(space_id,id,sha256,byte_size,storage_key,media_type,original_file_name,status) VALUES($1,$2,$3,7,$4,'application/octet-stream','missing.bin','ready')")
            .bind(space).bind(Uuid::new_v4()).bind(&digest).bind(format!("sha256/aa/{digest}"))
            .execute(&admin).await.unwrap();
    }
    let mut held = PgConnection::connect(&admin_url).await.unwrap();
    sqlx::query("BEGIN").execute(&mut held).await.unwrap();
    sqlx::query("SELECT 1").execute(&mut held).await.unwrap();
    let id: Uuid = std::env::var("TEST_C4_BACKUP_ID").unwrap().parse().unwrap();
    let config = SourceBackupConfig {
        backup_id: id,
        expected_database: database,
        expected_compose_project: std::env::var("TEST_C4_COMPOSE_PROJECT").unwrap(),
        isolation_attestation: env_path("TEST_C4_ISOLATION_ATTESTATION"),
        control_root: control.clone(),
        local_pin_root: pin.clone(),
        pg_dump_executable: if kind == "pg_dump_exit" {
            env_path("TEST_C4_PGDUMP_FAIL_BIN")
        } else {
            env_path("TEST_C4_PGDUMP_BIN")
        },
        pgpassfile: env_path("TEST_C4_PGPASSFILE"),
        pg_host: std::env::var("TEST_C4_PGHOST").unwrap(),
        pg_port: std::env::var("TEST_C4_PGPORT").unwrap().parse().unwrap(),
        drain_timeout: Duration::from_secs(15),
    };
    let task_config = config.clone();
    let task_admin = admin.clone();
    let task =
        tokio::spawn(
            async move { prepare_source_backup(&task_admin, &assets, &task_config).await },
        );
    let closed = control.join(format!("{id}.control/closed.json"));
    let probe = config
        .isolation_attestation
        .parent()
        .unwrap()
        .join(format!("runtime-connect-denied-{id}.json"));
    for _ in 0..100 {
        if closed.is_file() && probe.is_file() {
            break;
        }
        assert!(
            !task.is_finished(),
            "failure occurred before runtime rejection probe"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(closed.is_file() && probe.is_file());
    sqlx::query("ROLLBACK").execute(&mut held).await.unwrap();
    held.close().await.unwrap();
    assert!(task.await.unwrap().is_err());
    let journal = SourceGateJournal::recover(&control, id).unwrap();
    assert_eq!(
        journal.record().phase(),
        if kind == "missing_original" {
            GatePhase::DumpAndIndexDurable
        } else {
            GatePhase::Drained
        }
    );
    let can_connect: bool = sqlx::query_scalar(
        "SELECT has_database_privilege('learning_runtime',current_database(),'CONNECT')",
    )
    .fetch_one(&admin)
    .await
    .unwrap();
    assert!(!can_connect);
    assert_eq!(
        binding_inventory(&control),
        binding_before,
        "failed capture must preserve independent binding"
    );
    assert!(!pin.join(format!("{id}.sealed")).exists());
    assert!(!pin.join(format!("{id}.complete")).exists());
}

#![cfg(target_os = "linux")]
use learning_assets::FsAssetStore;
use learning_backup::{
    GatePhase, SourceBackupConfig, SourceGateJournal, prepare_source_backup, verify_sealed,
};
use sqlx::{Connection, PgConnection, postgres::PgPoolOptions};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};
use uuid::Uuid;

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} required")))
}

/// Run only against a *new*, migrated, UUID-named PG18 database inside a new
/// Compose project, with all paths private and dedicated to this batch.
#[tokio::test]
#[ignore = "requires a new isolated PG18 database and private Linux roots"]
async fn real_gate_waits_for_old_runtime_session_and_rejects_new_runtime_login() {
    assert!(option_env!("KNOWWEAVE_SOURCE_COMMIT").is_some());
    let expected = std::env::var("TEST_C4_TASK3_DATABASE_NAME").unwrap();
    let suffix = expected.strip_prefix("learning_backup_c4_task3_").unwrap();
    assert_eq!(Uuid::parse_str(suffix).unwrap().to_string(), suffix);
    let admin_url = std::env::var("TEST_ADMIN_DATABASE_URL").unwrap();
    let runtime_url = std::env::var("TEST_DATABASE_URL").unwrap();
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
    let mut runtime = PgConnection::connect(&runtime_url).await.unwrap();
    sqlx::query("BEGIN").execute(&mut runtime).await.unwrap();
    sqlx::query("SELECT 1").execute(&mut runtime).await.unwrap();

    let control = env_path("TEST_C4_CONTROL_ROOT");
    let pin = env_path("TEST_C4_PIN_ROOT");
    let asset_root = env_path("TEST_C4_ASSET_ROOT");
    let stage_root = env_path("TEST_C4_ASSET_STAGE_ROOT");
    for path in [&control, &pin, &asset_root, &stage_root] {
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert!(fs::read_dir(path).unwrap().next().is_none());
    }
    let assets = FsAssetStore::new(asset_root, stage_root).unwrap();
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
    assert!(PgConnection::connect(&runtime_url).await.is_err());
    sqlx::query("ROLLBACK").execute(&mut runtime).await.unwrap();
    runtime.close().await.unwrap();
    let result = task.await.unwrap().unwrap();
    assert_eq!(result.manifest().backup_id, config.backup_id);
    assert_eq!(result.manifest().logical_asset_count, 0);
    let recovered = SourceGateJournal::recover(&control, config.backup_id).unwrap();
    assert_eq!(recovered.record().phase(), GatePhase::Released);
    assert_eq!(
        verify_sealed(&pin, config.backup_id)
            .unwrap()
            .manifest_sha256(),
        result.sealed().manifest_sha256()
    );
    assert!(!pin.join(format!("{}.complete", config.backup_id)).exists());
    PgConnection::connect(&runtime_url)
        .await
        .unwrap()
        .close()
        .await
        .unwrap();
}

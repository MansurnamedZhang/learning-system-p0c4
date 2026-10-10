//! Actual PG18 binding gates. NO_DUMP: all phase digests are SYNTHETIC.
use super::*;
use sqlx::postgres::PgPoolOptions;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};

fn fixture_database(key: &str) -> String {
    let database = std::env::var(key).expect("binding database env missing");
    assert!(
        valid_c4_database(&database),
        "canonical fixture database required"
    );
    database
}

fn fixture_dsn(key: &str) -> String {
    let path = PathBuf::from(std::env::var(key).expect("binding DSN file env missing"));
    let metadata = std::fs::symlink_metadata(&path).expect("binding DSN metadata unavailable");
    assert!(
        metadata.is_file(),
        "private regular non-symlink DSN file required"
    );
    assert_eq!(metadata.uid(), 0, "DSN file must be root-owned");
    assert_eq!(
        metadata.permissions().mode() & 0o077,
        0,
        "DSN file must be private"
    );
    assert!(
        metadata.len() > 0 && metadata.len() <= 4096,
        "DSN file length"
    );
    std::fs::read_to_string(path)
        .expect("binding DSN file unreadable")
        .trim()
        .to_owned()
}

fn fixture_root() -> PathBuf {
    assert_eq!(
        unsafe { libc::geteuid() },
        0,
        "root-in-container fixture required"
    );
    let root = PathBuf::from(
        std::env::var("TEST_C4_BINDING_CONTROL_ROOT").expect("binding control root env missing"),
    );
    assert_eq!(root, Path::new("/var/lib/knowweave-source/control"));
    binding::preissued_fixture(&root);
    BackupDir::open_private_root(&root).expect("trusted private control root required");
    let mut entries = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    entries.sort();
    assert_eq!(
        entries,
        vec![std::ffi::OsString::from("source-binding.json")],
        "fresh root contains only independently issued binding"
    );
    root
}

fn independent_binding_bytes(root: &Path) -> Vec<u8> {
    let path = root.join("source-binding.json");
    let metadata = std::fs::symlink_metadata(&path).unwrap();
    assert!(
        metadata.is_file(),
        "binding must be a regular non-symlink file"
    );
    assert_eq!(metadata.uid(), 0, "binding must be root-owned");
    assert_eq!(metadata.permissions().mode() & 0o7777, 0o600);
    assert_eq!(metadata.nlink(), 1);
    assert!(metadata.len() > 0 && metadata.len() <= 4096);
    let bytes = std::fs::read(path).unwrap();
    // This expectation comes from the independent issuer before compilation;
    // no ordinary caller configuration or runtime hash supplies authority.
    let pin = match option_env!("KNOWWEAVE_C4_SOURCE_CONTROL_BINDING_SHA256") {
        Some(pin) => pin,
        None => panic!("independent source binding compile-time pin required"),
    };
    assert_eq!(pin.len(), 64, "compile-time pin must be SHA-256");
    assert!(
        pin.bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(&bytes)),
        pin,
        "issuer bytes match build pin"
    );
    bytes
}

#[derive(Debug, PartialEq, Eq)]
struct InventoryEntry {
    relative_path: PathBuf,
    uid: u32,
    mode: u32,
    dev: u64,
    ino: u64,
    bytes: Option<Vec<u8>>,
}

fn inventory(root: &Path) -> Vec<InventoryEntry> {
    fn visit(root: &Path, path: &Path, entries: &mut Vec<InventoryEntry>) {
        let metadata = std::fs::symlink_metadata(path).unwrap();
        assert!(
            !metadata.file_type().is_symlink(),
            "fixture inventory symlink"
        );
        assert!(
            metadata.is_dir() || metadata.is_file(),
            "fixture inventory regular entry"
        );
        entries.push(InventoryEntry {
            relative_path: path.strip_prefix(root).unwrap().to_owned(),
            uid: metadata.uid(),
            mode: metadata.permissions().mode() & 0o7777,
            dev: metadata.dev(),
            ino: metadata.ino(),
            bytes: metadata.is_file().then(|| std::fs::read(path).unwrap()),
        });
        if metadata.is_dir() {
            for entry in std::fs::read_dir(path).unwrap() {
                visit(root, &entry.unwrap().path(), entries);
            }
        }
    }
    let mut entries = Vec::new();
    visit(root, root, &mut entries);
    entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    entries
}

fn synthetic_release_ready(root: &Path, id: Uuid) {
    let control = BackupDir::open_trusted_private_root(root).unwrap();
    let database = fixture_database("TEST_C4_BINDING_DATABASE_NAME");
    binding::fixture_pending(&control, &database, id);
    let mut journal = SourceGateJournal::start(root, id).unwrap();
    journal.advance(GatePhase::Closed, None).unwrap();
    journal.advance(GatePhase::Drained, None).unwrap();
    // SYNTHETIC: no dump, full pin, or CompleteBackup is exercised.
    journal
        .advance(GatePhase::DumpAndIndexDurable, Some(&"a".repeat(64)))
        .unwrap();
    journal
        .advance(GatePhase::PinsDurable, Some(&"b".repeat(64)))
        .unwrap();
    journal.advance(GatePhase::ReleaseReady, None).unwrap();
    assert_eq!(journal.record().phase(), GatePhase::ReleaseReady);
}

async fn acl(connection: &mut PgConnection) -> (Option<String>, bool) {
    sqlx::query_as(
        "SELECT datacl::text, has_database_privilege('learning_runtime', current_database(), 'CONNECT') FROM pg_catalog.pg_database WHERE datname=current_database()",
    ).fetch_one(connection).await.expect("binding fixture ACL query failed")
}

/// Catches wrong-root recovery after the admitted backend and its lock are
/// gone. Baseline must actually REVOKE/write recovery, so preserved ACL is the
/// first behavioral assertion, ahead of the expected API refusal.
#[tokio::test]
#[ignore = "requires fresh isolated PG18 and independently pinned root-in-container binding"]
async fn alternate_root_after_session_loss_preserves_acl_and_journal() {
    let database = fixture_database("TEST_C4_BINDING_DATABASE_NAME");
    let dsn = fixture_dsn("TEST_C4_BINDING_ADMIN_DSN_FILE");
    let root = fixture_root();
    let binding = independent_binding_bytes(&root);
    let alternate = root.parent().unwrap().join("alternate");
    assert!(!alternate.exists(), "fresh sibling alternate root required");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&alternate)
        .unwrap();
    let mut copied = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(alternate.join("source-binding.json"))
        .unwrap();
    copied.write_all(&binding).unwrap();
    copied.sync_all().unwrap();
    drop(copied);
    let id = Uuid::new_v4();
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .min_connections(0)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&dsn)
        .await
        .expect("binding fixture management pool connection failed");
    let mut original = SourceAdmission::try_acquire(&pool, &database)
        .await
        .expect("original source admission must succeed");
    let version: i32 = sqlx::query_scalar("SELECT current_setting('server_version_num')::int")
        .fetch_one(original.connection())
        .await
        .unwrap();
    assert_eq!(version / 10000, 18, "actual PG18 required");
    let facts = inspect_gate(original.connection()).await.unwrap();
    assert!(facts.admin_is_database_owner && facts.runtime_can_connect);
    assert!(
        !facts.public_can_connect
            && !facts.runtime_can_inherit_admin
            && !facts.runtime_is_privileged
    );
    assert_eq!(facts.other_login_writers, 0);
    assert_eq!(
        facts.other_sessions, 0,
        "no other sessions before original capture"
    );
    // Model the final GRANT window with real ACL transitions; journal digests
    // remain synthetic and this fixture never invokes dump or backup capture.
    sqlx::query(&format!(
        "REVOKE CONNECT ON DATABASE \"{database}\" FROM PUBLIC, learning_runtime"
    ))
    .execute(original.connection())
    .await
    .unwrap();
    assert!(
        !acl(original.connection()).await.1,
        "fixture gate really closed"
    );
    synthetic_release_ready(&root, id);
    synthetic_release_ready(&alternate, id);
    sqlx::query(&format!(
        "GRANT CONNECT ON DATABASE \"{database}\" TO learning_runtime"
    ))
    .execute(original.connection())
    .await
    .unwrap();
    assert!(
        acl(original.connection()).await.1,
        "fixture CONNECT really regranted"
    );
    let old_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(original.connection())
        .await
        .unwrap();
    drop(original);
    let mut observer = pool
        .acquire()
        .await
        .expect("backend-loss observer must connect");
    let mut absent = false;
    for _ in 0..100 {
        let present: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_stat_activity WHERE datname=current_database() AND pid=$1)")
            .bind(old_pid).fetch_one(&mut *observer).await.unwrap();
        if !present {
            absent = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        absent,
        "original admitted backend must be absent before wrong-root recovery"
    );
    let free: bool = sqlx::query_scalar("SELECT pg_catalog.pg_try_advisory_lock($1, $2)")
        .bind(0x4b57_4334_i32)
        .bind(0x5352_4345_i32)
        .fetch_one(&mut *observer)
        .await
        .unwrap();
    assert!(free, "old source admission lock must be free");
    let unlocked: bool = sqlx::query_scalar("SELECT pg_catalog.pg_advisory_unlock($1, $2)")
        .bind(0x4b57_4334_i32)
        .bind(0x5352_4345_i32)
        .fetch_one(&mut *observer)
        .await
        .unwrap();
    assert!(unlocked, "observer must release its lock probe");
    let facts = inspect_gate(&mut observer).await.unwrap();
    assert_eq!(
        facts.other_sessions, 0,
        "recovery must have no competing backend"
    );
    assert!(facts.admin_is_database_owner && facts.runtime_can_connect);
    assert!(
        !facts.public_can_connect
            && !facts.runtime_can_inherit_admin
            && !facts.runtime_is_privileged
    );
    assert_eq!(facts.other_login_writers, 0);
    let acl_before = acl(&mut observer).await;
    assert!(
        acl_before.1,
        "CONNECT must be granted before wrong-root recovery"
    );
    drop(observer); // Same max=1 pool reuses this backend; no separate observer remains.
    let original_before = inventory(&root);
    let alternate_before = inventory(&alternate);
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        force_close_release_ready(&pool, &alternate, id, &database),
    )
    .await
    .expect("wrong-root recovery must return without pool deadlock");
    let acl_after = acl(&mut pool.acquire().await.unwrap()).await;
    let original_after = inventory(&root);
    let alternate_after = inventory(&alternate);
    pool.close().await;
    eprintln!(
        "NO_DUMP SYNTHETIC: original_backend_absent={absent} lock_free={free} recovery_ok={} connect_before={} connect_after={}",
        result.is_ok(),
        acl_before.1,
        acl_after.1
    );
    // Mutation caught: binding is absent/ignored or accepts copied binding bytes
    // in B, causing real REVOKE even though admission is now uncontested.
    assert_eq!(
        acl_after, acl_before,
        "alternate root recovery must preserve runtime CONNECT ACL after original backend loss"
    );
    assert!(
        result.is_err(),
        "alternate root recovery must refuse copied binding"
    );
    assert_eq!(
        original_after, original_before,
        "bound root inventory must remain unchanged"
    );
    assert_eq!(
        alternate_after, alternate_before,
        "alternate root inventory must remain unchanged"
    );
    assert_eq!(
        SourceGateJournal::recover(&root, id)
            .unwrap()
            .record()
            .phase(),
        GatePhase::ReleaseReady
    );
    assert_eq!(
        SourceGateJournal::recover(&alternate, id)
            .unwrap()
            .record()
            .phase(),
        GatePhase::ReleaseReady
    );
}

use sqlx::Connection;

async fn management_pool(dsn_key: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .min_connections(0)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&fixture_dsn(dsn_key))
        .await
        .expect("binding fixture management pool connection failed")
}

fn capture_fixture(
    pool: &PgPool,
    root: &Path,
    database: &str,
) -> (FsAssetStore, SourceBackupConfig) {
    let parent = root.parent().unwrap();
    let pin = parent.join("pins");
    let assets = parent.join("assets");
    let staging = parent.join("asset-staging");
    for path in [&pin, &assets, &staging] {
        let held = BackupDir::open_trusted_private_root(path)
            .expect("independently provisioned fixture root");
        assert!(
            held.list().unwrap().is_empty(),
            "fresh fixture root required"
        );
    }
    let options = pool.connect_options();
    let id = Uuid::new_v4();
    (
        FsAssetStore::new(assets, staging).unwrap(),
        SourceBackupConfig {
            backup_id: id,
            expected_database: database.to_owned(),
            expected_compose_project: "learning-system-p0c4-binding-test".into(),
            isolation_attestation: parent.join(format!("isolation-{id}.json")),
            control_root: root.to_owned(),
            local_pin_root: pin,
            pg_dump_executable: parent.join("NO_DUMP-not-an-executable"),
            pgpassfile: parent.join("NO_DUMP-no-pgpass"),
            pg_host: options.get_host().to_owned(),
            pg_port: options.get_port(),
            drain_timeout: Duration::from_secs(5),
        },
    )
}

async fn assert_live_preflight(pool: &PgPool) {
    let mut connection = pool.acquire().await.unwrap();
    let version: i32 = sqlx::query_scalar("SELECT current_setting('server_version_num')::int")
        .fetch_one(&mut *connection)
        .await
        .unwrap();
    assert_eq!(version / 10000, 18, "actual PG18 required");
    let facts = inspect_gate(&mut connection).await.unwrap();
    assert!(facts.admin_is_database_owner && facts.runtime_can_connect);
    assert!(
        !facts.public_can_connect
            && !facts.runtime_can_inherit_admin
            && !facts.runtime_is_privileged
    );
    assert_eq!(facts.other_login_writers, 0);
    assert_eq!(facts.other_sessions, 0);
}

#[tokio::test]
#[ignore = "requires fresh isolated PG18 and independently pinned root-in-container binding"]
async fn original_bound_root_keeps_unfinished_journal_authoritative() {
    let root = fixture_root();
    let binding = independent_binding_bytes(&root);
    let database = fixture_database("TEST_C4_BINDING_DATABASE_NAME");
    let pool = management_pool("TEST_C4_BINDING_ADMIN_DSN_FILE").await;
    assert_live_preflight(&pool).await;
    let (assets, config) = capture_fixture(&pool, &root, &database);
    // Real bound public preflight: prepared work survives its origin backend,
    // and refusal must precede the deliberately absent isolation attestation.
    let prepared_root_before = inventory(&root);
    let prepared_pin_before = inventory(&config.local_pin_root);
    let prepared_acl_before = acl(&mut pool.acquire().await.unwrap()).await;
    let prepared_id = Uuid::new_v4();
    let mut origin = PgConnection::connect(&fixture_dsn("TEST_C4_BINDING_ADMIN_DSN_FILE"))
        .await
        .expect("prepared origin connection required");
    let original_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut origin)
        .await
        .unwrap();
    sqlx::query("BEGIN").execute(&mut origin).await.unwrap();
    sqlx::query(&format!(
        "CREATE TABLE public.c4_binding_prepared_{}(id integer PRIMARY KEY)",
        prepared_id.simple()
    ))
    .execute(&mut origin)
    .await
    .unwrap();
    sqlx::query(&format!("PREPARE TRANSACTION '{prepared_id}'"))
        .execute(&mut origin)
        .await
        .unwrap();
    let origin_closed = origin.close().await;
    // Resolve only the exact synthetic work owned by this fixture before any
    // result assertions. A failed assertion must not leave prepared work behind.
    let observed: Result<_, BackupError> = async {
        let mut absent = false;
        for _ in 0..100 {
            absent = sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM pg_catalog.pg_stat_activity WHERE pid=$1)")
                .bind(original_pid).fetch_one(&pool).await?;
            if absent { break; }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let count_before: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_catalog.pg_prepared_xacts WHERE database=current_database() AND gid=$1")
            .bind(prepared_id.to_string()).fetch_one(&pool).await?;
        let capture = prepare_source_backup(&pool, &assets, &config).await;
        let count_after: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_catalog.pg_prepared_xacts WHERE database=current_database() AND gid=$1")
            .bind(prepared_id.to_string()).fetch_one(&pool).await?;
        let acl_after: (Option<String>, bool) = sqlx::query_as(
            "SELECT datacl::text, has_database_privilege('learning_runtime', current_database(), 'CONNECT') FROM pg_catalog.pg_database WHERE datname=current_database()",
        ).fetch_one(&pool).await?;
        Ok((absent, count_before, capture, count_after, acl_after, inventory(&root), inventory(&config.local_pin_root)))
    }.await;
    sqlx::query(&format!("ROLLBACK PREPARED '{prepared_id}'"))
        .execute(&pool)
        .await
        .unwrap();
    origin_closed.unwrap();
    let (
        absent,
        count_before,
        capture,
        count_after,
        prepared_acl_after,
        prepared_root_after,
        prepared_pin_after,
    ) = observed.unwrap();
    assert!(absent, "prepared origin backend must be closed");
    assert_eq!(count_before, 1, "real prepared transaction required");
    assert!(matches!(
        capture,
        Err(BackupError::Invalid(
            "source database has unsafe prepared transaction count"
        ))
    ));
    assert_eq!(
        count_after, 1,
        "public preflight must not resolve prepared work"
    );
    assert_eq!(prepared_acl_after, prepared_acl_before);
    assert_eq!(prepared_root_after, prepared_root_before);
    assert_eq!(prepared_pin_after, prepared_pin_before);
    let remaining: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_catalog.pg_prepared_xacts WHERE database=current_database()",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(remaining, 0, "owned synthetic prepared work cleaned");
    assert_live_preflight(&pool).await;
    let id = Uuid::new_v4();
    synthetic_release_ready(&root, id);
    let alternate = root.parent().unwrap().join("alternate");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&alternate)
        .unwrap();
    let alternate_handle = BackupDir::open_trusted_private_root(&alternate).unwrap();
    let mut file = alternate_handle.create_file("source-binding.json").unwrap();
    file.write_all(&binding).unwrap();
    file.sync_all().unwrap();
    drop(file);
    let mut alternate_config = config.clone();
    alternate_config.control_root = alternate.clone();
    let root_before = inventory(&root);
    let alternate_before = inventory(&alternate);
    let pin_before = inventory(&config.local_pin_root);
    let acl_before = acl(&mut pool.acquire().await.unwrap()).await;
    // The alternate capture config reaches binding before the absent driver
    // attestation, proving that omission cannot masquerade as wrong-root safety.
    let alternate_result = prepare_source_backup(&pool, &assets, &alternate_config).await;
    assert!(matches!(
        alternate_result,
        Err(BackupError::Invalid("source binding control root identity"))
    ));
    // Mutation caught: a bound root skips its durable incomplete journal,
    // or refusal is accidentally supplied only by the missing attestation.
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        prepare_source_backup(&pool, &assets, &config),
    )
    .await
    .expect("bound-root capture must not deadlock");
    assert!(matches!(
        result,
        Err(BackupError::Invalid(
            "unfinished source maintenance journal"
        ))
    ));
    assert_eq!(acl(&mut pool.acquire().await.unwrap()).await, acl_before);
    assert_eq!(inventory(&root), root_before);
    assert_eq!(inventory(&alternate), alternate_before);
    assert_eq!(inventory(&config.local_pin_root), pin_before);
    assert_eq!(
        SourceGateJournal::recover(&root, id)
            .unwrap()
            .record()
            .phase(),
        GatePhase::ReleaseReady
    );
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires fresh isolated PG18 and independently pinned root-in-container binding"]
async fn matching_bound_recovery_recloses_without_finishing() {
    let root = fixture_root();
    let binding_before = independent_binding_bytes(&root);
    let database = fixture_database("TEST_C4_BINDING_DATABASE_NAME");
    let pool = management_pool("TEST_C4_BINDING_ADMIN_DSN_FILE").await;
    assert_live_preflight(&pool).await;
    let id = Uuid::new_v4();
    let mut original = SourceAdmission::try_acquire(&pool, &database)
        .await
        .unwrap();
    sqlx::query(&format!(
        "REVOKE CONNECT ON DATABASE \"{database}\" FROM PUBLIC, learning_runtime"
    ))
    .execute(original.connection())
    .await
    .unwrap();
    assert!(!acl(original.connection()).await.1);
    synthetic_release_ready(&root, id);
    sqlx::query(&format!(
        "GRANT CONNECT ON DATABASE \"{database}\" TO learning_runtime"
    ))
    .execute(original.connection())
    .await
    .unwrap();
    assert!(acl(original.connection()).await.1);
    original.close().await.unwrap();
    let journal_path = root.join(format!("{id}.control"));
    let journal_before = inventory(&journal_path);
    // Mutation caught: correct binding always refuses, leaves CONNECT open,
    // or resolves ReleaseReady instead of retaining it as authoritative.
    tokio::time::timeout(
        Duration::from_secs(10),
        force_close_release_ready(&pool, &root, id, &database),
    )
    .await
    .expect("bound recovery must not deadlock")
    .unwrap();
    let facts = inspect_gate(&mut pool.acquire().await.unwrap())
        .await
        .unwrap();
    facts.validate().unwrap();
    assert!(!facts.runtime_can_connect);
    assert_eq!(inventory(&journal_path), journal_before);
    assert_eq!(
        std::fs::read(root.join("source-binding.json")).unwrap(),
        binding_before
    );
    assert_eq!(
        SourceGateJournal::recover(&root, id)
            .unwrap()
            .record()
            .phase(),
        GatePhase::ReleaseReady
    );
    let evidence = std::fs::read(root.join(format!("{id}.release-recovery/closed.json"))).unwrap();
    let expected = serde_json::to_vec(&serde_json::json!({
        "format_version":1,"backup_id":id,"database":database,
        "result":"runtime_connect_revoked_and_sessions_drained"
    }))
    .unwrap();
    assert_eq!(evidence, expected);
    assert!(!journal_path.join("released.json").exists());
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires fresh isolated PG18 and independently pinned root-in-container binding"]
async fn another_database_is_rejected_before_source_mutation() {
    let root = fixture_root();
    independent_binding_bytes(&root);
    let database = fixture_database("TEST_C4_BINDING_DATABASE_NAME");
    let other_database = fixture_database("TEST_C4_BINDING_OTHER_DATABASE_NAME");
    assert_ne!(database, other_database);
    let pool = management_pool("TEST_C4_BINDING_ADMIN_DSN_FILE").await;
    let other = management_pool("TEST_C4_BINDING_OTHER_ADMIN_DSN_FILE").await;
    assert_live_preflight(&pool).await;
    assert_live_preflight(&other).await;
    let identity_sql = "SELECT oid::bigint, (pg_catalog.pg_control_system()).system_identifier::text FROM pg_catalog.pg_database WHERE datname=current_database()";
    let first: (i64, String) = sqlx::query_as(identity_sql).fetch_one(&pool).await.unwrap();
    let second: (i64, String) = sqlx::query_as(identity_sql)
        .fetch_one(&other)
        .await
        .unwrap();
    assert_ne!(first.0, second.0);
    assert_eq!(
        first.1, second.1,
        "different databases on the same actual server"
    );
    let id = Uuid::new_v4();
    synthetic_release_ready(&root, id);
    let (assets, config) = capture_fixture(&other, &root, &other_database);
    let before = inventory(&root);
    let pin_before = inventory(&config.local_pin_root);
    let acl_before = acl(&mut other.acquire().await.unwrap()).await;
    let first_acl_before = acl(&mut pool.acquire().await.unwrap()).await;
    // Mutation caught: the control root binds only a cluster or caller name,
    // and accepts a second database before reading its authoritative journal.
    let recovery = force_close_release_ready(&other, &root, id, &other_database).await;
    assert!(matches!(
        recovery,
        Err(BackupError::Invalid("source binding database identity"))
    ));
    let capture = prepare_source_backup(&other, &assets, &config).await;
    assert!(matches!(
        capture,
        Err(BackupError::Invalid("source binding database identity"))
    ));
    assert_eq!(acl(&mut other.acquire().await.unwrap()).await, acl_before);
    assert_eq!(
        acl(&mut pool.acquire().await.unwrap()).await,
        first_acl_before
    );
    assert_eq!(inventory(&root), before);
    assert_eq!(inventory(&config.local_pin_root), pin_before);
    assert_eq!(
        SourceGateJournal::recover(&root, id)
            .unwrap()
            .record()
            .phase(),
        GatePhase::ReleaseReady
    );
    other.close().await;
    pool.close().await;
}

//! Actual PG18 admission fixtures. Synthetic phase digests attest no dump or pin.
use super::*;
use sqlx::{Connection, PgConnection, postgres::PgPoolOptions};
use std::os::unix::fs::{MetadataExt, PermissionsExt};

fn fixture_database(key: &str) -> String {
    let database = std::env::var(key).expect("admission database env missing");
    assert!(
        valid_c4_database(&database),
        "fresh canonical database required"
    );
    database
}

fn fixture_dsn(key: &str) -> String {
    let path = PathBuf::from(std::env::var(key).expect("admission DSN file env missing"));
    let metadata = std::fs::symlink_metadata(&path).expect("admission DSN metadata unavailable");
    assert!(
        metadata.is_file(),
        "DSN file must be a regular non-symlink file"
    );
    assert_eq!(metadata.uid(), unsafe { libc::geteuid() }, "DSN file owner");
    assert_eq!(
        metadata.permissions().mode() & 0o077,
        0,
        "DSN file must be owner-only"
    );
    assert!(
        metadata.len() > 0 && metadata.len() <= 4096,
        "DSN file length"
    );
    std::fs::read_to_string(path)
        .expect("admission DSN file unreadable")
        .trim()
        .to_owned()
}

fn fixture_root() -> PathBuf {
    let root = PathBuf::from(
        std::env::var("TEST_C4_ADMISSION_CONTROL_ROOT")
            .expect("admission control root env missing"),
    );
    BackupDir::open_private_root(&root).expect("fresh owned 0700 control root required");
    assert!(
        std::fs::read_dir(&root).unwrap().next().is_none(),
        "fresh empty control root required"
    );
    root
}

fn journal_snapshot(root: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
    fn visit(root: &Path, path: &Path, entries: &mut Vec<(PathBuf, Option<Vec<u8>>)>) {
        let metadata = std::fs::symlink_metadata(path).unwrap();
        assert!(
            !metadata.file_type().is_symlink(),
            "fixture snapshot symlink"
        );
        let relative = path.strip_prefix(root).unwrap().to_owned();
        if metadata.is_dir() {
            entries.push((relative, None));
            for entry in std::fs::read_dir(path).unwrap() {
                visit(root, &entry.unwrap().path(), entries);
            }
        } else {
            assert!(metadata.is_file(), "fixture snapshot regular file");
            entries.push((relative, Some(std::fs::read(path).unwrap())));
        }
    }
    let mut entries = Vec::new();
    visit(root, root, &mut entries);
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}

fn synthetic_release_ready(root: &Path, id: Uuid) -> SourceGateJournal {
    let mut journal = SourceGateJournal::start(root, id).unwrap();
    journal.advance(GatePhase::Closed, None).unwrap();
    journal.advance(GatePhase::Drained, None).unwrap();
    // Deliberately synthetic prior digests: no actual dump/full pin is exercised.
    journal
        .advance(GatePhase::DumpAndIndexDurable, Some(&"a".repeat(64)))
        .unwrap();
    journal
        .advance(GatePhase::PinsDurable, Some(&"b".repeat(64)))
        .unwrap();
    journal.advance(GatePhase::ReleaseReady, None).unwrap();
    journal
}

async fn acl(connection: &mut PgConnection) -> (Option<String>, bool) {
    sqlx::query_as(
        "SELECT datacl::text, has_database_privilege('learning_runtime', current_database(), 'CONNECT') FROM pg_catalog.pg_database WHERE datname=current_database()",
    ).fetch_one(connection).await.expect("fixture ACL query failed")
}

/// Catches REVOKE/journal/recovery writes before a competing live owner is
/// rejected. The independent holder makes baseline compensation refuse only
/// AFTER REVOKE; the preserved-ACL assertion must therefore be the actual RED.
#[tokio::test]
#[ignore = "requires fresh isolated PG18 admission database and private fixture files"]
async fn busy_source_recovery_preserves_acl_and_journal() {
    let database = fixture_database("TEST_C4_ADMISSION_DATABASE_NAME");
    let dsn = fixture_dsn("TEST_C4_ADMISSION_ADMIN_DSN_FILE");
    let root = fixture_root();
    let id = Uuid::new_v4();
    let journal = synthetic_release_ready(&root, id);
    assert_eq!(journal.record().phase(), GatePhase::ReleaseReady);
    drop(journal);
    let snapshot_before = journal_snapshot(&root);

    let mut holder = PgConnection::connect(&dsn)
        .await
        .expect("fixture holder connection failed");
    let (version, current, session, authenticated, actual_database, owner):
        (i32, String, String, Option<String>, String, String) = sqlx::query_as(
        "SELECT current_setting('server_version_num')::int, current_user::text, session_user::text, system_user, current_database()::text, pg_get_userbyid(datdba)::text FROM pg_catalog.pg_database WHERE datname=current_database()",
    ).fetch_one(&mut holder).await.expect("fixture identity query failed");
    assert_eq!(version / 10000, 18, "actual PG18 required");
    assert_eq!(current, "learning_admin");
    assert_eq!(session, "learning_admin");
    assert_eq!(owner, "learning_admin");
    assert_eq!(actual_database, database);
    assert!(
        authenticated
            .as_deref()
            .and_then(|v| v.split_once(':'))
            .is_some_and(|(method, user)| !method.is_empty() && user == "learning_admin"),
        "authenticated learning_admin required"
    );
    let locked: bool = sqlx::query_scalar("SELECT pg_catalog.pg_try_advisory_lock($1, $2)")
        .bind(0x4b57_4334_i32)
        .bind(0x5352_4345_i32)
        .fetch_one(&mut holder)
        .await
        .expect("fixture advisory lock query failed");
    assert!(locked, "fresh database fixed source lock must be free");
    let acl_before = acl(&mut holder).await;
    assert!(
        acl_before.1,
        "fixture runtime CONNECT must initially be granted"
    );
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .min_connections(0)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&dsn)
        .await
        .expect("fixture competing pool connection failed");

    let result = tokio::time::timeout(
        Duration::from_secs(10),
        force_close_release_ready(&pool, &root, id, &database),
    )
    .await;
    let acl_after = acl(&mut holder).await;
    let snapshot_after = journal_snapshot(&root);
    pool.close().await;
    holder.close().await.expect("fixture holder close failed");

    // Mutation caught: ignoring the live source lock allows recovery to proceed.
    assert!(
        result
            .expect("busy admission must return without pool deadlock")
            .is_err(),
        "busy source recovery must refuse"
    );
    // Mutation caught: late exclusion REVOKEs CONNECT before reporting busy.
    assert!(
        acl_after == acl_before,
        "busy source recovery must preserve runtime CONNECT ACL"
    );
    // Mutation caught: busy recovery creates/changes a journal or recovery entry.
    assert_eq!(
        snapshot_after, snapshot_before,
        "busy admission must preserve all journal bytes and entries"
    );
    assert_eq!(
        SourceGateJournal::recover(&root, id)
            .unwrap()
            .record()
            .phase(),
        GatePhase::ReleaseReady
    );
}

async fn fixture_pool(dsn: &str, minimum: u32) -> PgPool {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .min_connections(minimum)
        .acquire_timeout(Duration::from_secs(5))
        .connect(dsn)
        .await
        .expect("fresh admission pool connection failed");
    let version: i32 = sqlx::query_scalar("SELECT current_setting('server_version_num')::int")
        .fetch_one(&pool)
        .await
        .expect("fixture PG version query failed");
    assert_eq!(version / 10000, 18, "actual PG18 required");
    pool
}

async fn backend_pid(connection: &mut PgConnection) -> i32 {
    sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(connection)
        .await
        .expect("fixture backend PID query failed")
}

async fn revoke(admission: &mut SourceAdmission, database: &str) {
    sqlx::query(&format!(
        "REVOKE CONNECT ON DATABASE \"{database}\" FROM PUBLIC, learning_runtime"
    ))
    .execute(admission.connection())
    .await
    .expect("fixture REVOKE failed");
}

async fn wait_no_other_sessions(admission: &mut SourceAdmission) {
    for _ in 0..100 {
        if inspect_gate(admission.connection())
            .await
            .unwrap()
            .other_sessions
            == 0
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("fixture observation session failed to close");
}

async fn assert_competitor_busy(dsn: &str, database: &str) {
    let competitor = fixture_pool(dsn, 0).await;
    let result = SourceAdmission::try_acquire(&competitor, database).await;
    // Mutation caught: changed/early-released lock permits a concurrent owner.
    assert!(
        matches!(
            result,
            Err(BackupError::Invalid("source maintenance admission busy"))
        ),
        "admitted owner must exclude independent management pool"
    );
    competitor.close().await;
}

fn synthetic_pins_durable(root: &Path, id: Uuid) -> SourceGateJournal {
    let mut journal = SourceGateJournal::start(root, id).unwrap();
    journal.advance(GatePhase::Closed, None).unwrap();
    journal.advance(GatePhase::Drained, None).unwrap();
    // These digests prove only the synthetic release branch, no dump/full pin.
    journal
        .advance(GatePhase::DumpAndIndexDurable, Some(&"a".repeat(64)))
        .unwrap();
    journal
        .advance(GatePhase::PinsDurable, Some(&"b".repeat(64)))
        .unwrap();
    journal
}

#[tokio::test]
#[ignore = "requires two fresh isolated canonical PG18 admission databases"]
async fn same_database_attempts_share_admission_and_other_database_is_independent() {
    use std::os::unix::fs::DirBuilderExt;
    let database = fixture_database("TEST_C4_ADMISSION_DATABASE_NAME");
    let other_database = fixture_database("TEST_C4_ADMISSION_OTHER_DATABASE_NAME");
    assert_ne!(
        database, other_database,
        "fresh distinct UUID databases required"
    );
    let dsn = fixture_dsn("TEST_C4_ADMISSION_ADMIN_DSN_FILE");
    let other_dsn = fixture_dsn("TEST_C4_ADMISSION_OTHER_ADMIN_DSN_FILE");
    let root = fixture_root();
    let pool = fixture_pool(&dsn, 0).await;
    let mut owner = SourceAdmission::try_acquire(&pool, &database)
        .await
        .unwrap();
    let acl_before = acl(owner.connection()).await;
    assert_competitor_busy(&dsn, &database).await;
    // Mutation caught: keys derived from attempt/root let another recovery
    // modify the gate despite a live admitted owner on the same database.
    for name in ["attempt-a", "attempt-b"] {
        let attempt_root = root.join(name);
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&attempt_root)
            .unwrap();
        let id = Uuid::new_v4();
        drop(synthetic_release_ready(&attempt_root, id));
        let before = journal_snapshot(&attempt_root);
        let competitor = fixture_pool(&dsn, 0).await;
        let outcome = force_close_release_ready(&competitor, &attempt_root, id, &database).await;
        competitor.close().await;
        assert!(
            matches!(
                outcome,
                Err(BackupError::Invalid("source maintenance admission busy"))
            ),
            "different root/attempt must share live admission"
        );
        assert_eq!(
            journal_snapshot(&attempt_root),
            before,
            "busy different root must be unchanged"
        );
        assert_eq!(
            acl(owner.connection()).await,
            acl_before,
            "busy different attempt must preserve ACL"
        );
    }
    let other_pool = fixture_pool(&other_dsn, 0).await;
    // Mutation caught: a cluster-wide/global lock incorrectly excludes another DB.
    let other_owner = SourceAdmission::try_acquire(&other_pool, &other_database)
        .await
        .expect("different database must admit the same fixed keys");
    other_owner.close().await.unwrap();
    other_pool.close().await;
    owner.close().await.unwrap();
    // Mutation caught: returning a still-locked session to the pool leaks admission.
    let successor = SourceAdmission::try_acquire(&pool, &database)
        .await
        .unwrap();
    successor.close().await.unwrap();
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires a fresh isolated PG18 database for real catalog migration"]
async fn single_connection_catalog_and_drain_share_admitted_backend() {
    let database = fixture_database("TEST_C4_ADMISSION_DATABASE_NAME");
    let dsn = fixture_dsn("TEST_C4_ADMISSION_ADMIN_DSN_FILE");
    let pool = fixture_pool(&dsn, 1).await;
    let mut owner = SourceAdmission::try_acquire(&pool, &database)
        .await
        .unwrap();
    MIGRATOR
        .run(owner.connection())
        .await
        .expect("fixture migrations failed");
    revoke(&mut owner, &database).await;
    let pid_before = backend_pid(owner.connection()).await;
    let plan = tokio::time::timeout(
        Duration::from_secs(5),
        crate::catalog::plan_assets_on(owner.connection()),
    )
    .await
    .expect("catalog must not reacquire occupied max=1 pool")
    .unwrap();
    // Mutation caught: catalog queries a different backend or returns spurious rows.
    assert_eq!(
        plan.logical_asset_count(),
        0,
        "fresh migrated catalog is empty"
    );
    assert_eq!(
        backend_pid(owner.connection()).await,
        pid_before,
        "catalog retains admitted backend"
    );
    let inspection = inspect_gate(owner.connection()).await.unwrap();
    // Mutation caught: guard adds a spare pool backend or inspector counts itself.
    assert_eq!(
        inspection.other_sessions, 0,
        "single admitted backend adds no other sessions"
    );
    inspection.validate().unwrap();
    assert_competitor_busy(&dsn, &database).await;
    wait_no_other_sessions(&mut owner).await;
    let mut origin = PgConnection::connect(&dsn)
        .await
        .expect("fixture old session connect failed");
    sqlx::query("BEGIN").execute(&mut origin).await.unwrap();
    sqlx::query("SELECT 1").execute(&mut origin).await.unwrap();
    let blocked = inspect_gate(owner.connection()).await.unwrap();
    // Mutation caught: excluding an arbitrary extra management backend weakens drain.
    assert_eq!(
        blocked.other_sessions, 1,
        "real extra transaction still counts"
    );
    assert!(
        blocked.validate().is_err(),
        "real old transaction must block drain"
    );
    sqlx::query("ROLLBACK").execute(&mut origin).await.unwrap();
    origin.close().await.unwrap();
    wait_no_other_sessions(&mut owner).await;
    inspect_gate(owner.connection())
        .await
        .unwrap()
        .validate()
        .unwrap();
    owner.close().await.unwrap();
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires a fresh isolated PG18 database and owned private control root"]
async fn dropping_admission_closes_backend_without_reopening_gate() {
    let database = fixture_database("TEST_C4_ADMISSION_DATABASE_NAME");
    let dsn = fixture_dsn("TEST_C4_ADMISSION_ADMIN_DSN_FILE");
    let root = fixture_root();
    let pool = fixture_pool(&dsn, 0).await;
    let mut owner = SourceAdmission::try_acquire(&pool, &database)
        .await
        .unwrap();
    let id = Uuid::new_v4();
    let mut journal = SourceGateJournal::start(&root, id).unwrap();
    revoke(&mut owner, &database).await;
    journal.advance(GatePhase::Closed, None).unwrap();
    let snapshot_before = journal_snapshot(&root);
    let acl_before = acl(owner.connection()).await;
    assert!(!acl_before.1, "fixture gate must be closed before Drop");
    let old_pid = backend_pid(owner.connection()).await;
    drop(owner);
    let mut observer = PgConnection::connect(&dsn)
        .await
        .expect("fixture closure observer connect failed");
    let mut disappeared = false;
    for _ in 0..100 {
        let present: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_stat_activity WHERE datname=current_database() AND pid=$1)")
            .bind(old_pid).fetch_one(&mut observer).await.unwrap();
        if !present {
            disappeared = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let acl_after = acl(&mut observer).await;
    observer.close().await.unwrap();
    // Mutation caught: Drop recycles rather than closes a session owning the lock.
    assert!(
        disappeared,
        "Drop must close the old admitted backend within the bound"
    );
    let mut successor = SourceAdmission::try_acquire(&pool, &database)
        .await
        .unwrap();
    assert_ne!(
        backend_pid(successor.connection()).await,
        old_pid,
        "successor uses a new backend"
    );
    // Mutation caught: Drop grants CONNECT or deletes unfinished journal evidence.
    assert_eq!(
        acl_after, acl_before,
        "Drop must preserve closed runtime ACL"
    );
    let control = BackupDir::open_private_root(&root).unwrap();
    assert!(
        ensure_no_unfinished_journal(&control, &root).is_err(),
        "fixed root still refuses incomplete journal"
    );
    assert_eq!(
        journal_snapshot(&root),
        snapshot_before,
        "Drop creates no journal/recovery and deletes none"
    );
    assert_eq!(
        SourceGateJournal::recover(&root, id)
            .unwrap()
            .record()
            .phase(),
        GatePhase::Closed
    );
    inspect_gate(successor.connection())
        .await
        .unwrap()
        .validate()
        .unwrap();
    successor.close().await.unwrap();
    pool.close().await;
    // Session exclusion cannot bind a different root after a crash: trusted
    // deployment must retain its fixed root; this test makes no cross-root claim.
}

#[tokio::test]
#[ignore = "requires a fresh isolated PG18 database for synthetic release/compensation"]
async fn release_and_compensation_keep_admission_until_owner_closes() {
    let database = fixture_database("TEST_C4_ADMISSION_DATABASE_NAME");
    let dsn = fixture_dsn("TEST_C4_ADMISSION_ADMIN_DSN_FILE");
    let root = fixture_root();
    let pool = fixture_pool(&dsn, 0).await;
    let mut owner = SourceAdmission::try_acquire(&pool, &database)
        .await
        .unwrap();
    let pid = backend_pid(owner.connection()).await;
    revoke(&mut owner, &database).await;
    inspect_gate(owner.connection())
        .await
        .unwrap()
        .validate()
        .unwrap();
    let mut normal = synthetic_pins_durable(&root, Uuid::new_v4());
    release_runtime_connect(&mut owner, &database, &mut normal)
        .await
        .unwrap();
    // Mutation caught: release omits GRANT, durable Released or switches backend.
    assert_eq!(normal.record().phase(), GatePhase::Released);
    assert!(
        acl(owner.connection()).await.1,
        "normal release grants runtime CONNECT"
    );
    assert_eq!(
        backend_pid(owner.connection()).await,
        pid,
        "release retains owner backend"
    );
    assert_competitor_busy(&dsn, &database).await;
    wait_no_other_sessions(&mut owner).await;
    owner.close().await.unwrap();
    let mut owner = SourceAdmission::try_acquire(&pool, &database)
        .await
        .unwrap();
    revoke(&mut owner, &database).await;
    let pid = backend_pid(owner.connection()).await;
    let id = Uuid::new_v4();
    let mut collision_journal = synthetic_pins_durable(&root, id);
    let collision = root.join(format!("{id}.control/released.json"));
    std::fs::create_dir(&collision).unwrap();
    let outcome = release_runtime_connect(&mut owner, &database, &mut collision_journal).await;
    // Mutation caught: Released-file collision reports success or skips compensation.
    assert!(
        outcome.is_err(),
        "durable Released collision must refuse release"
    );
    assert_eq!(collision_journal.record().phase(), GatePhase::ReleaseReady);
    assert!(
        !acl(owner.connection()).await.1,
        "collision compensation revokes runtime CONNECT"
    );
    assert_eq!(
        backend_pid(owner.connection()).await,
        pid,
        "compensation retains owner backend"
    );
    assert_competitor_busy(&dsn, &database).await;
    wait_no_other_sessions(&mut owner).await;
    inspect_gate(owner.connection())
        .await
        .unwrap()
        .validate()
        .unwrap();
    owner.close().await.unwrap();
    let successor = SourceAdmission::try_acquire(&pool, &database)
        .await
        .unwrap();
    successor.close().await.unwrap();
    pool.close().await;
}

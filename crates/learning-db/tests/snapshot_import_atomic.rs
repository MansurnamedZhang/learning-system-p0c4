#[path = "support/snapshot_import.rs"]
mod fixture;
#[path = "support/relation_store.rs"]
mod relations;
mod support;
use learning_assets::{FsAssetStore, SnapshotDirectory, UploadDeclaration, stage_snapshot};
use learning_core::*;
use learning_db::{MIGRATOR, SnapshotImportStore, SnapshotPlan};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{collections::BTreeMap, fs, io::Read, path::PathBuf};
use uuid::Uuid;

struct Files {
    root: PathBuf,
    store: FsAssetStore,
}
impl Files {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("snapshot-atomic-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let store = FsAssetStore::new(root.join("assets"), root.join("uploads")).unwrap();
        Self { root, store }
    }
    fn seed(&self) {
        let source = self.root.join("original");
        fs::write(&source, b"snapshot original").unwrap();
        self.store
            .put_from_file(
                Uuid::new_v4(),
                &source,
                UploadDeclaration {
                    expected_size_bytes: 17,
                    max_size_bytes: 17,
                },
            )
            .unwrap();
    }
    fn stage(&self, plan: &SnapshotPlan) -> SnapshotDirectory {
        stage_snapshot(
            &self.root,
            Uuid::new_v4(),
            &plan.manifest,
            &plan.rows,
            &plan.assets,
            &self.store,
        )
        .unwrap()
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
struct Target {
    admin: PgPool,
    runtime: PgPool,
    files: Files,
}
impl Target {
    async fn fresh(label: &str) -> Self {
        let admin = PgPoolOptions::new()
            .max_connections(4)
            .connect(
                &std::env::var(format!("TEST_IMPORT_{label}_ADMIN_DATABASE_URL"))
                    .expect("dedicated empty import admin DSN required"),
            )
            .await
            .unwrap();
        let empty: bool = sqlx::query_scalar("SELECT to_regclass('public.block') IS NULL")
            .fetch_one(&admin)
            .await
            .unwrap();
        assert!(
            empty,
            "dedicated import target must start empty; no database reset is attempted"
        );
        MIGRATOR.run(&admin).await.unwrap();
        let runtime = PgPoolOptions::new()
            .max_connections(6)
            .connect(
                &std::env::var(format!("TEST_IMPORT_{label}_DATABASE_URL"))
                    .expect("dedicated import runtime DSN required"),
            )
            .await
            .unwrap();
        Self {
            admin,
            runtime,
            files: Files::new(),
        }
    }
    fn store(&self) -> SnapshotImportStore {
        SnapshotImportStore::new(self.runtime.clone(), self.files.store.clone())
    }
    async fn pinned_store(&self) -> (SnapshotImportStore, i32) {
        // One physical connection fixes the import transaction's waiter PID;
        // do not infer its identity from another role's query text.
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .max_lifetime(None)
            .idle_timeout(None)
            .connect_with((*self.runtime.connect_options()).clone())
            .await
            .unwrap();
        let pid = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&pool)
            .await
            .unwrap();
        (
            SnapshotImportStore::new(pool, self.files.store.clone()),
            pid,
        )
    }
    async fn provision(&self, actor: Principal, space: Uuid) {
        sqlx::query("INSERT INTO app_user(id) VALUES($1)")
            .bind(actor.actor_id)
            .execute(&self.admin)
            .await
            .unwrap();
        sqlx::query("INSERT INTO space(id,owner_id) VALUES($1,$2)")
            .bind(space)
            .bind(actor.actor_id)
            .execute(&self.admin)
            .await
            .unwrap();
        sqlx::query("INSERT INTO space_grant(actor_id,space_id,can_write) VALUES($1,$2,true)")
            .bind(actor.actor_id)
            .bind(space)
            .execute(&self.admin)
            .await
            .unwrap();
    }
}
fn table(t: SnapshotTable) -> String {
    serde_json::to_value(t).unwrap().as_str().unwrap().into()
}
async fn counts(target: &Target, plan: &SnapshotPlan) -> BTreeMap<String, i64> {
    let mut tables = plan
        .rows
        .iter()
        .map(|r| table(r.table))
        .collect::<std::collections::BTreeSet<_>>();
    tables.extend(
        [
            "snapshot_import_batch",
            "request_key",
            "mutation_receipt",
            "upload_receipt",
            "reading_receipt",
            "reading_receipt_block",
            "composition_receipt",
            "release_receipt",
            "migration_receipt",
            "lineage_receipt",
            "relation_receipt",
            "relation_review_receipt",
            "epistemic_review_receipt",
            "outbox_event",
            "job",
            "job_outbox",
        ]
        .map(str::to_owned),
    );
    let mut counts = BTreeMap::new();
    for t in tables {
        counts.insert(
            t.clone(),
            sqlx::query_scalar(&format!("SELECT count(*) FROM public.{t}"))
                .fetch_one(&target.admin)
                .await
                .unwrap(),
        );
    }
    counts
}
async fn assert_import_state(
    target: &Target,
    actor: Principal,
    plan: &SnapshotPlan,
    baseline: &BTreeMap<String, i64>,
    receipts: &BTreeMap<Uuid, String>,
) {
    let mut expected = baseline.clone();
    expected.insert("snapshot_import_batch".into(), receipts.len() as i64);
    assert_eq!(counts(target, plan).await, expected);
    let actual: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT request_id,manifest_sha256,status FROM snapshot_import_batch WHERE actor_id=$1 ORDER BY request_id",
    ).bind(actor.actor_id).fetch_all(&target.runtime).await.unwrap();
    assert_eq!(
        actual,
        receipts
            .iter()
            .map(|(request, digest)| (*request, digest.clone(), "succeeded".into()))
            .collect::<Vec<_>>()
    );
}
async fn verify_rows(target: &Target, plan: &SnapshotPlan) {
    for expected in &plan.rows {
        let all: Vec<Value> = sqlx::query_scalar(&format!(
            "SELECT to_jsonb(t) FROM public.{} t",
            table(expected.table)
        ))
        .fetch_all(&target.runtime)
        .await
        .unwrap();
        assert!(
            all.into_iter().any(|mut row| {
                for key in [
                    "head_revision_id",
                    "head_review_id",
                    "published_revision_id",
                    "last_release_id",
                ] {
                    row.as_object_mut().unwrap().remove(key);
                }
                if let Some(time) = row.get_mut("created_at") {
                    let parsed =
                        chrono::DateTime::parse_from_rfc3339(time.as_str().unwrap()).unwrap();
                    *time = json!(canonical_snapshot_timestamp(
                        parsed.with_timezone(&chrono::Utc)
                    ));
                }
                row == expected.immutable_values
            }),
            "missing exact {:?} row",
            expected.table
        );
    }
}
async fn heads(target: &Target) -> Vec<Value> {
    let mut result = vec![];
    for name in [
        "block",
        "composition",
        "overlay",
        "reading_view",
        "relation",
        "relation_review_head",
        "epistemic_stream",
    ] {
        let rows: Vec<Value> = sqlx::query_scalar(&format!(
            "SELECT to_jsonb(t) FROM public.{name} t ORDER BY to_jsonb(t)::text"
        ))
        .fetch_all(&target.runtime)
        .await
        .unwrap();
        result.extend(rows);
    }
    result
}
fn rehash(plan: &mut SnapshotPlan) {
    plan.manifest
        .files
        .retain(|f| f.path.starts_with("assets/"));
    for row in &mut plan.rows {
        row.sha256 = canonical_record_hash(&row.immutable_values);
        let bytes = canonical_json(&serde_json::to_value(&row).unwrap());
        plan.manifest.files.push(SnapshotFile {
            path: snapshot_object_path(row.table, &row.identity).unwrap(),
            size: bytes.len() as u64,
            sha256: hex_digest(bytes.as_bytes()),
        });
    }
    plan.manifest.files.sort_by(|a, b| a.path.cmp(&b.path));
}

#[tokio::test]
async fn admin_lock_observer_handles_private_runtime_query_text() {
    let rig = support::TestRig::from_env().await;
    let admin_can_read_all: bool = sqlx::query_scalar(
        "SELECT rolsuper OR pg_has_role(current_user,'pg_read_all_stats','USAGE') FROM pg_roles WHERE rolname=current_user",
    ).fetch_one(&rig.admin_pool).await.unwrap();
    assert!(
        !admin_can_read_all,
        "probe requires the ordinary isolated admin role"
    );
    let admin_role: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap();
    let mut runtime = rig.runtime_pool.begin().await.unwrap();
    let (runtime_pid, runtime_role): (i32, String) =
        sqlx::query_as("SELECT pg_backend_pid(),current_user::text")
            .fetch_one(&mut *runtime)
            .await
            .unwrap();
    assert_ne!(admin_role, runtime_role);
    let admin_inherits_runtime: bool = sqlx::query_scalar(
        "SELECT pg_has_role(current_user,(SELECT oid FROM pg_roles WHERE rolname=$1),'USAGE')",
    )
    .bind(&runtime_role)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert!(
        !admin_inherits_runtime,
        "fixture prerequisite: admin must not inherit the runtime role"
    );
    sqlx::query("SELECT 1 /* task6_private_runtime_query_probe */")
        .execute(&mut *runtime)
        .await
        .unwrap();
    let visible_to_old_probe: bool = sqlx::query_scalar(
        "SELECT coalesce(query LIKE '%task6_private_runtime_query_probe%',false) FROM pg_stat_activity WHERE pid=$1 AND datid=(SELECT oid FROM pg_database WHERE datname=current_database())",
    ).bind(runtime_pid).fetch_one(&rig.admin_pool).await.unwrap();
    assert!(
        !visible_to_old_probe,
        "cross-role query text must not be needed by the lock observer"
    );
    let key = advisory_key();
    let mut gate = rig.admin_pool.begin().await.unwrap();
    let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock($1::bigint)")
        .bind(key)
        .execute(&mut *gate)
        .await
        .unwrap();
    let waiter = tokio::spawn(async move {
        sqlx::query("SELECT pg_advisory_xact_lock($1::bigint)")
            .bind(key)
            .execute(&mut *runtime)
            .await
            .unwrap();
        runtime.commit().await.unwrap();
    });
    wait_blocked(
        &rig.admin_pool,
        runtime_pid,
        blocker,
        ExpectedLock::Advisory(key),
    )
    .await;
    let observer: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap();
    assert_ne!(observer, blocker);
    assert!(
        !is_blocked(
            &rig.admin_pool,
            runtime_pid,
            observer,
            ExpectedLock::Advisory(key)
        )
        .await
    );
    assert!(
        !is_blocked(
            &rig.admin_pool,
            observer,
            blocker,
            ExpectedLock::Advisory(key)
        )
        .await
    );
    assert!(
        !is_blocked(
            &rig.admin_pool,
            runtime_pid,
            blocker,
            ExpectedLock::Advisory(key ^ 1)
        )
        .await
    );
    assert!(
        !is_blocked(
            &rig.admin_pool,
            runtime_pid,
            blocker,
            ExpectedLock::Transaction
        )
        .await
    );
    gate.commit().await.unwrap();
    waiter.await.unwrap();

    // Row waits expose the owning transaction ID, not necessarily a tuple lock.
    let (actor, space) = rig.seed_actor_space(true).await;
    let mut deletion = rig.admin_pool.begin().await.unwrap();
    let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *deletion)
        .await
        .unwrap();
    sqlx::query("DELETE FROM space_grant WHERE actor_id=$1 AND space_id=$2")
        .bind(actor.actor_id)
        .bind(space)
        .execute(&mut *deletion)
        .await
        .unwrap();
    let mut runtime = rig.runtime_pool.begin().await.unwrap();
    let runtime_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *runtime)
        .await
        .unwrap();
    let waiter = tokio::spawn(async move {
        let grant: Option<bool> = sqlx::query_scalar("SELECT public.lock_space_grant($1,$2)")
            .bind(actor.actor_id)
            .bind(space)
            .fetch_one(&mut *runtime)
            .await
            .unwrap();
        assert_eq!(grant, Some(true));
        runtime.commit().await.unwrap();
    });
    wait_blocked(
        &rig.admin_pool,
        runtime_pid,
        blocker,
        ExpectedLock::Transaction,
    )
    .await;
    assert!(
        !is_blocked(
            &rig.admin_pool,
            runtime_pid,
            blocker,
            ExpectedLock::Advisory(key)
        )
        .await
    );
    deletion.rollback().await.unwrap();
    waiter.await.unwrap();
}

#[tokio::test]
async fn attention_exact_atomic_roundtrip_two_fresh_databases_and_failure_boundaries() {
    let (source, actor, space, plan) = fixture::attention().await;
    let original = Files::new();
    original.seed();
    let stage = original.stage(&plan);
    let a = Target::fresh("A").await;
    let b = Target::fresh("B").await;
    for target in [&a, &b] {
        target.provision(actor, space).await;
    }
    let store = a.store();
    let prepared = store.validate_exact(actor, &stage).await.unwrap();
    let request = Uuid::new_v4();
    let receipt = store.import_exact(actor, request, prepared).await.unwrap();
    assert!(!receipt.reused);
    assert_eq!(receipt.manifest_sha256, stage.manifest_sha256());
    assert_ne!(
        receipt.manifest_sha256,
        hex_digest(canonical_json(&serde_json::to_value(&plan.manifest).unwrap()).as_bytes())
    );
    verify_rows(&a, &plan).await;
    let stable = counts(&a, &plan).await;
    let mut receipts = BTreeMap::from([(request, stage.manifest_sha256().to_owned())]);
    assert_import_state(&a, actor, &plan, &stable, &receipts).await;
    let stable_heads = heads(&a).await;
    for id in [request, Uuid::new_v4()] {
        let p = store.validate_exact(actor, &stage).await.unwrap();
        assert!(store.import_exact(actor, id, p).await.unwrap().reused);
        receipts.insert(id, stage.manifest_sha256().to_owned());
        assert_import_state(&a, actor, &plan, &stable, &receipts).await;
    }
    assert_eq!(heads(&a).await, stable_heads);
    // Same request and different requests are serialized without duplicate rows.
    for same_request in [true, false] {
        let left = store.validate_exact(actor, &stage).await.unwrap();
        let right = store.validate_exact(actor, &stage).await.unwrap();
        let id = Uuid::new_v4();
        let other_id = if same_request { id } else { Uuid::new_v4() };
        let (l, r) = tokio::join!(
            store.import_exact(actor, id, left),
            store.import_exact(actor, other_id, right)
        );
        assert!(l.unwrap().reused && r.unwrap().reused);
        for request in [id, other_id] {
            receipts.insert(request, stage.manifest_sha256().to_owned());
        }
        assert_import_state(&a, actor, &plan, &stable, &receipts).await;
    }
    // Different valid package, same request: existing receipt binding is immutable.
    let mut different = plan.clone();
    different.manifest.requires_destination_assets = true;
    different
        .manifest
        .files
        .retain(|f| !f.path.starts_with("assets/"));
    let metadata = original.stage(&different);
    let p = store.validate_exact(actor, &metadata).await.unwrap();
    let before = counts(&a, &plan).await;
    assert!(matches!(
        store.import_exact(actor, request, p).await,
        Err(ContentError::IdempotencyConflict)
    ));
    assert_eq!(counts(&a, &plan).await, before);
    assert_import_state(&a, actor, &plan, &stable, &receipts).await;
    let left = store.validate_exact(actor, &stage).await.unwrap();
    let right = store.validate_exact(actor, &metadata).await.unwrap();
    let contested = Uuid::new_v4();
    let (l, r) = tokio::join!(
        store.import_exact(actor, contested, left),
        store.import_exact(actor, contested, right)
    );
    assert!(matches!(
        (&l, &r),
        (Ok(_), Err(ContentError::IdempotencyConflict))
            | (Err(ContentError::IdempotencyConflict), Ok(_))
    ));
    let saved_hash: String = sqlx::query_scalar(
        "SELECT manifest_sha256 FROM snapshot_import_batch WHERE actor_id=$1 AND request_id=$2",
    )
    .bind(actor.actor_id)
    .bind(contested)
    .fetch_one(&a.runtime)
    .await
    .unwrap();
    assert_eq!(
        saved_hash,
        l.as_ref().or(r.as_ref()).unwrap().manifest_sha256
    );
    receipts.insert(contested, saved_hash);
    assert_import_state(&a, actor, &plan, &stable, &receipts).await;
    // Metadata-only preflight cannot later restore deleted target bytes from its retained inode.
    let p = store.validate_exact(actor, &metadata).await.unwrap();
    let asset = plan
        .rows
        .iter()
        .find(|r| r.table == SnapshotTable::Asset)
        .unwrap();
    let key = asset.immutable_values["storage_key"].as_str().unwrap();
    let target_path = a.files.root.join("assets").join(key);
    fs::remove_file(&target_path).unwrap();
    let before = counts(&a, &plan).await;
    assert!(matches!(
        store.import_exact(actor, Uuid::new_v4(), p).await,
        Err(ContentError::NotFound)
    ));
    assert!(!target_path.exists());
    assert_eq!(counts(&a, &plan).await, before);
    a.files.seed();
    let p = store.validate_exact(actor, &metadata).await.unwrap();
    fs::write(&target_path, b"corrupt").unwrap();
    assert!(store.import_exact(actor, Uuid::new_v4(), p).await.is_err());
    assert_eq!(counts(&a, &plan).await, before);
    fs::remove_file(&target_path).unwrap();
    a.files.seed();
    // Grant deletion after prepare is rechecked even for a successful request replay.
    let p = store.validate_exact(actor, &stage).await.unwrap();
    sqlx::query("DELETE FROM space_grant WHERE actor_id=$1 AND space_id=$2")
        .bind(actor.actor_id)
        .bind(space)
        .execute(&a.admin)
        .await
        .unwrap();
    assert!(matches!(
        store.import_exact(actor, request, p).await,
        Err(ContentError::NotFound)
    ));
    assert_eq!(counts(&a, &plan).await, before);
    sqlx::query("INSERT INTO space_grant VALUES($1,$2,true)")
        .bind(actor.actor_id)
        .bind(space)
        .execute(&a.admin)
        .await
        .unwrap();
    // Changed canonical audit metadata after prepare is caught inside import.
    let p = store.validate_exact(actor, &stage).await.unwrap();
    let block = plan
        .rows
        .iter()
        .find(|r| r.table == SnapshotTable::BlockRevision)
        .unwrap();
    let id: Uuid = serde_json::from_value(block.immutable_values["id"].clone()).unwrap();
    sqlx::query("UPDATE block_revision SET reason='conflicting audit bytes' WHERE id=$1")
        .bind(id)
        .execute(&a.admin)
        .await
        .unwrap();
    let before = counts(&a, &plan).await;
    assert!(matches!(
        store.import_exact(actor, Uuid::new_v4(), p).await,
        Err(ContentError::IdentityConflict)
    ));
    assert_eq!(counts(&a, &plan).await, before);
    sqlx::query("UPDATE block_revision SET reason=$2 WHERE id=$1")
        .bind(id)
        .bind(block.immutable_values["reason"].as_str().unwrap())
        .execute(&a.admin)
        .await
        .unwrap();
    natural_key_collision_after_prepare(&b, actor, &plan, &stage).await;
    // Failure after many missing rows have been inserted must roll back all of them.
    let p = b.store().validate_exact(actor, &stage).await.unwrap();
    let before = counts(&b, &plan).await;
    sqlx::raw_sql("CREATE FUNCTION public.test_import_abort() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected late import failure' USING ERRCODE='23503'; END $$; CREATE TRIGGER test_import_abort BEFORE INSERT ON public.reading_view_revision FOR EACH ROW EXECUTE FUNCTION public.test_import_abort();").execute(&b.admin).await.unwrap();
    assert!(
        b.store()
            .import_exact(actor, Uuid::new_v4(), p)
            .await
            .is_err()
    );
    assert_eq!(counts(&b, &plan).await, before);
    let mut bytes = vec![];
    b.files
        .store
        .open_record(key, asset.immutable_values["sha256"].as_str().unwrap(), 17)
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    assert_eq!(bytes, b"snapshot original");
    sqlx::raw_sql("DROP TRIGGER test_import_abort ON public.reading_view_revision; DROP FUNCTION public.test_import_abort();").execute(&b.admin).await.unwrap();
    // Deferred failure at final constraint flush also rolls back every row.
    sqlx::raw_sql("CREATE FUNCTION public.test_import_abort() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected deferred failure' USING ERRCODE='23503'; END $$; CREATE CONSTRAINT TRIGGER test_import_abort AFTER INSERT ON public.reading_view_revision DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION public.test_import_abort();").execute(&b.admin).await.unwrap();
    let p = b.store().validate_exact(actor, &stage).await.unwrap();
    let before = counts(&b, &plan).await;
    assert!(
        b.store()
            .import_exact(actor, Uuid::new_v4(), p)
            .await
            .is_err()
    );
    assert_eq!(counts(&b, &plan).await, before);
    sqlx::raw_sql("DROP TRIGGER test_import_abort ON public.reading_view_revision; DROP FUNCTION public.test_import_abort();").execute(&b.admin).await.unwrap();
    // Two first imports with distinct requests: exactly one sees a prior receipt.
    let store_b = b.store();
    let left = store_b.validate_exact(actor, &stage).await.unwrap();
    let right = store_b.validate_exact(actor, &stage).await.unwrap();
    // Hold the first transaction after its complete row insertion and final
    // checks. A separate reader still sees no half-published reading or rows.
    let visibility_key = advisory_key();
    sqlx::raw_sql(&format!("CREATE FUNCTION public.test_import_visibility() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock({visibility_key}::bigint); RETURN NEW; END $$; CREATE TRIGGER test_import_visibility BEFORE INSERT ON public.snapshot_import_batch FOR EACH ROW EXECUTE FUNCTION public.test_import_visibility();")).execute(&b.admin).await.unwrap();
    let mut gate = b.admin.begin().await.unwrap();
    let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock($1::bigint)")
        .bind(visibility_key)
        .execute(&mut *gate)
        .await
        .unwrap();
    let (left_store, left_pid) = b.pinned_store().await;
    let right_store = b.store();
    let left_request = Uuid::new_v4();
    let right_request = Uuid::new_v4();
    let l = tokio::spawn(async move { left_store.import_exact(actor, left_request, left).await });
    wait_blocked(
        &b.admin,
        left_pid,
        blocker,
        ExpectedLock::Advisory(visibility_key),
    )
    .await;
    // Start the second import while the first is demonstrably uncommitted.
    let r =
        tokio::spawn(async move { right_store.import_exact(actor, right_request, right).await });
    assert!(counts(&b, &plan).await.into_values().all(|n| n == 0));
    let uncommitted = learning_db::ReadingStore::new(b.runtime.clone())
        .read_versioned(actor, plan.manifest.root.clone(), ReadingMode::Fused)
        .await
        .expect("uncommitted reading lookup must succeed with no visible reading");
    assert!(
        uncommitted.is_none(),
        "uncommitted import exposed a reading: {uncommitted:?}"
    );
    gate.commit().await.unwrap();
    let (l, r) = (l.await.unwrap(), r.await.unwrap());
    assert_ne!(l.unwrap().reused, r.unwrap().reused);
    let b_receipts = BTreeMap::from([
        (left_request, stage.manifest_sha256().to_owned()),
        (right_request, stage.manifest_sha256().to_owned()),
    ]);
    assert_import_state(&b, actor, &plan, &stable, &b_receipts).await;
    sqlx::raw_sql("DROP TRIGGER test_import_visibility ON public.snapshot_import_batch; DROP FUNCTION public.test_import_visibility();").execute(&b.admin).await.unwrap();
    verify_rows(&b, &plan).await;
    let projected = learning_db::ReadingStore::new(source.runtime_pool.clone())
        .read_versioned(actor, plan.manifest.root.clone(), ReadingMode::Fused)
        .await
        .unwrap();
    for target in [&a, &b] {
        let imported = learning_db::ReadingStore::new(target.runtime.clone())
            .read_versioned(actor, plan.manifest.root.clone(), ReadingMode::Fused)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(imported).unwrap(),
            serde_json::to_value(&projected).unwrap()
        );
        for command in [
            "UPDATE snapshot_import_batch SET status='succeeded'",
            "DELETE FROM snapshot_import_batch",
        ] {
            let error = sqlx::query(command)
                .execute(&target.runtime)
                .await
                .unwrap_err();
            assert_eq!(support::sqlstate(&error).as_deref(), Some("42501"));
        }
    }
    // No ordinary receipt or job was produced by exact publication.
    for name in [
        "request_key",
        "mutation_receipt",
        "upload_receipt",
        "reading_receipt",
        "reading_receipt_block",
        "composition_receipt",
        "release_receipt",
        "migration_receipt",
        "lineage_receipt",
        "relation_receipt",
        "relation_review_receipt",
        "epistemic_review_receipt",
        "outbox_event",
        "job",
        "job_outbox",
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM public.{name}"))
            .fetch_one(&a.admin)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
    assert_import_state(&a, actor, &plan, &stable, &receipts).await;
    assert_eq!(heads(&a).await, stable_heads);
    unexpected_registry_rejects_and_rolls_back(&a, actor, space, &plan, &original).await;
    reused_extra_index_and_private_owner_fail(&a, actor, &plan, &stage).await;
    newer_heads_and_publication_survive(&b, actor, space, &plan, &stage).await;
    grant_lock_races(&a, actor, space, &plan, &stage).await;
    repeated_digest_and_deleted_metadata_identity(&a, actor, &plan, &original).await;
    // Missing required parent is rejected by preparation without authoritative writes.
    let mut missing = plan.clone();
    let pos = missing
        .rows
        .iter()
        .position(|r| {
            r.table == SnapshotTable::BlockRevision
                && r.immutable_values["parent_revision_id"].is_null()
        })
        .unwrap();
    missing.rows.remove(pos);
    rehash(&mut missing);
    let before = counts(&a, &plan).await;
    assert!(
        store
            .validate_exact(actor, &original.stage(&missing))
            .await
            .is_err()
    );
    assert_eq!(counts(&a, &plan).await, before);
}

#[tokio::test]
async fn fresh_0014_to_0015_upgrade_preserves_checksums_and_adds_only_receipt_rights() {
    let admin = PgPoolOptions::new()
        .connect(
            &std::env::var("TEST_IMPORT_UPGRADE_ADMIN_DATABASE_URL")
                .expect("dedicated empty import upgrade DSN required"),
        )
        .await
        .unwrap();
    let empty: bool = sqlx::query_scalar("SELECT to_regclass('public.block') IS NULL")
        .fetch_one(&admin)
        .await
        .unwrap();
    assert!(empty);
    let baseline = sqlx::migrate::Migrator {
        migrations: std::borrow::Cow::Owned(
            MIGRATOR
                .iter()
                .filter(|m| m.version <= 14)
                .cloned()
                .collect(),
        ),
        ..sqlx::migrate::Migrator::DEFAULT
    };
    baseline.run(&admin).await.unwrap();
    let before: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&admin)
            .await
            .unwrap();
    assert_eq!(before.len(), 14);
    MIGRATOR.run(&admin).await.unwrap();
    let after: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations WHERE version<=14 ORDER BY version",
    )
    .fetch_all(&admin)
    .await
    .unwrap();
    assert_eq!(before, after);
    let rights:(bool,bool,bool,bool)=sqlx::query_as("SELECT has_table_privilege('learning_runtime','snapshot_import_batch','SELECT'),has_table_privilege('learning_runtime','snapshot_import_batch','INSERT'),has_table_privilege('learning_runtime','snapshot_import_batch','UPDATE'),has_table_privilege('learning_runtime','snapshot_import_batch','DELETE')").fetch_one(&admin).await.unwrap();
    assert_eq!(rights, (true, true, false, false));
}

async fn natural_key_collision_after_prepare(
    t: &Target,
    actor: Principal,
    plan: &SnapshotPlan,
    stage: &SnapshotDirectory,
) {
    let prepared = t.store().validate_exact(actor, stage).await.unwrap();
    let mut conflict_id = Uuid::nil();
    for kind in [
        SnapshotTable::Asset,
        SnapshotTable::Resource,
        SnapshotTable::ResourceVersion,
    ] {
        let mut value = plan
            .rows
            .iter()
            .find(|r| r.table == kind)
            .unwrap()
            .immutable_values
            .clone();
        if kind == SnapshotTable::ResourceVersion {
            conflict_id = Uuid::new_v4();
            value["id"] = json!(conflict_id);
        }
        let name = table(kind);
        sqlx::query(&format!(
            "INSERT INTO public.{name} SELECT * FROM jsonb_populate_record(NULL::public.{name},$1)"
        ))
        .bind(value)
        .execute(&t.admin)
        .await
        .unwrap();
    }
    let before = counts(t, plan).await;
    assert!(matches!(
        t.store()
            .import_exact(actor, Uuid::new_v4(), prepared)
            .await,
        Err(ContentError::IdentityConflict)
    ));
    assert_eq!(counts(t, plan).await, before);
    // Remove only this test's three injected rows, in dependency order.
    sqlx::query("DELETE FROM resource_version WHERE id=$1")
        .bind(conflict_id)
        .execute(&t.admin)
        .await
        .unwrap();
    for kind in [SnapshotTable::Resource, SnapshotTable::Asset] {
        let v = &plan
            .rows
            .iter()
            .find(|r| r.table == kind)
            .unwrap()
            .immutable_values;
        sqlx::query(&format!(
            "DELETE FROM public.{} WHERE space_id=$1 AND id=$2",
            table(kind)
        ))
        .bind(serde_json::from_value::<Uuid>(v["space_id"].clone()).unwrap())
        .bind(serde_json::from_value::<Uuid>(v["id"].clone()).unwrap())
        .execute(&t.admin)
        .await
        .unwrap();
    }
}
async fn unexpected_registry_rejects_and_rolls_back(
    target: &Target,
    actor: Principal,
    space: Uuid,
    plan: &SnapshotPlan,
    source: &Files,
) {
    // Obtain a legitimate spare registry row through an ordinary writer. The
    // admin-only corruption below changes its revision: INSERT-time registry
    // checks would reject a directly inserted bad triple. No trigger is disabled.
    let spare = learning_db::ContentStore::new(target.runtime.clone())
        .create(
            actor,
            space,
            support::command("registry corruption fixture"),
        )
        .await
        .unwrap();
    let registry = plan
        .rows
        .iter()
        .find(|r| {
            r.table == SnapshotTable::ReferenceObject && r.immutable_values["kind"] == "block"
        })
        .unwrap();
    let revision: Uuid =
        serde_json::from_value(registry.immutable_values["revision_id"].clone()).unwrap();
    let mut extended = plan.clone();
    let mut resource = plan
        .rows
        .iter()
        .find(|r| r.table == SnapshotTable::Resource)
        .unwrap()
        .clone();
    let resource_id = Uuid::new_v4();
    resource.immutable_values["id"] = json!(resource_id);
    resource.identity[1] = SnapshotIdentityPart::Uuid(resource_id);
    extended.rows.push(resource);
    rehash(&mut extended);
    let stage = source.stage(&extended);
    let prepared = target.store().validate_exact(actor, &stage).await.unwrap();
    sqlx::query("UPDATE reference_object SET revision_id=$1 WHERE kind='block' AND object_id=$2 AND revision_id=$3")
        .bind(revision).bind(spare.block_id).bind(spare.revision_id)
        .execute(&target.admin).await.unwrap();
    let before = counts(target, &extended).await;
    let before_heads = heads(target).await;
    let request = Uuid::new_v4();
    assert!(
        matches!(target.store().import_exact(actor, request, prepared).await,
        Err(ContentError::Invalid(message)) if message == "invalid_snapshot_import")
    );
    assert_eq!(counts(target, &extended).await, before);
    assert_eq!(heads(target).await, before_heads);
    let receipt: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM snapshot_import_batch WHERE actor_id=$1 AND request_id=$2)",
    )
    .bind(actor.actor_id)
    .bind(request)
    .fetch_one(&target.runtime)
    .await
    .unwrap();
    assert!(!receipt);
    let inserted: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM resource WHERE space_id=$1 AND id=$2)")
            .bind(space)
            .bind(resource_id)
            .fetch_one(&target.runtime)
            .await
            .unwrap();
    assert!(
        !inserted,
        "new early-table row must roll back on late registry refusal"
    );
    // Restore only the deliberately corrupted registry key.
    sqlx::query("UPDATE reference_object SET revision_id=$1 WHERE kind='block' AND object_id=$2 AND revision_id=$3")
        .bind(spare.revision_id).bind(spare.block_id).bind(revision)
        .execute(&target.admin).await.unwrap();
}

async fn reused_extra_index_and_private_owner_fail(
    t: &Target,
    actor: Principal,
    plan: &SnapshotPlan,
    stage: &SnapshotDirectory,
) {
    let revision = plan
        .rows
        .iter()
        .find(|r| {
            r.table == SnapshotTable::OverlayRevision
                && !plan.rows.iter().any(|m| {
                    m.table == SnapshotTable::PlacementManualDecision
                        && m.immutable_values["overlay_revision_id"] == r.immutable_values["id"]
                })
        })
        .unwrap();
    let overlay: Uuid =
        serde_json::from_value(revision.immutable_values["overlay_id"].clone()).unwrap();
    let id: Uuid = serde_json::from_value(revision.immutable_values["id"].clone()).unwrap();
    let group: Uuid = serde_json::from_value(
        plan.rows
            .iter()
            .find(|r| r.table == SnapshotTable::OverlayGroupIdentity)
            .unwrap()
            .immutable_values["group_id"]
            .clone(),
    )
    .unwrap();
    let prepared = t.store().validate_exact(actor, stage).await.unwrap();
    sqlx::query("INSERT INTO placement_manual_decision VALUES($1,$2,$3,$3)")
        .bind(overlay)
        .bind(id)
        .bind(group)
        .execute(&t.admin)
        .await
        .unwrap();
    let before = counts(t, plan).await;
    assert!(
        t.store()
            .import_exact(actor, Uuid::new_v4(), prepared)
            .await
            .is_err()
    );
    assert_eq!(counts(t, plan).await, before);
    sqlx::query(
        "DELETE FROM placement_manual_decision WHERE overlay_revision_id=$1 AND source_group_id=$2",
    )
    .bind(id)
    .bind(group)
    .execute(&t.admin)
    .await
    .unwrap();
    let prepared = t.store().validate_exact(actor, stage).await.unwrap();
    let other = Uuid::new_v4();
    sqlx::query("INSERT INTO app_user VALUES($1)")
        .bind(other)
        .execute(&t.admin)
        .await
        .unwrap();
    sqlx::query("UPDATE overlay SET owner_id=$1 WHERE id=$2")
        .bind(other)
        .bind(overlay)
        .execute(&t.admin)
        .await
        .unwrap();
    let before = counts(t, plan).await;
    assert!(matches!(
        t.store()
            .import_exact(actor, Uuid::new_v4(), prepared)
            .await,
        Err(ContentError::NotFound)
    ));
    assert_eq!(counts(t, plan).await, before);
    sqlx::query("UPDATE overlay SET owner_id=$1 WHERE id=$2")
        .bind(actor.actor_id)
        .bind(overlay)
        .execute(&t.admin)
        .await
        .unwrap();
}
async fn newer_heads_and_publication_survive(
    t: &Target,
    actor: Principal,
    space: Uuid,
    plan: &SnapshotPlan,
    stage: &SnapshotDirectory,
) {
    let old = plan
        .rows
        .iter()
        .find(|r| {
            r.table == SnapshotTable::BlockRevision && r.immutable_values["contract_version"] == 1
        })
        .unwrap();
    let v = &old.immutable_values;
    let block: Uuid = serde_json::from_value(v["block_id"].clone()).unwrap();
    let current: Uuid = sqlx::query_scalar("SELECT head_revision_id FROM block WHERE id=$1")
        .bind(block)
        .fetch_one(&t.admin)
        .await
        .unwrap();
    learning_db::VersionedContentStore::new(t.runtime.clone())
        .revise(
            actor,
            block,
            ReviseContent {
                request_id: Uuid::new_v4(),
                base_revision_id: current,
                draft: ContentDraft::decode(1, v["content"].clone()).unwrap(),
                reason: "newer target head".into(),
            },
        )
        .await
        .unwrap();
    let root = plan
        .rows
        .iter()
        .find(|r| r.table == SnapshotTable::Overlay)
        .unwrap();
    let composition: Uuid =
        serde_json::from_value(root.immutable_values["root_composition_id"].clone()).unwrap();
    let current: Uuid = sqlx::query_scalar("SELECT head_revision_id FROM composition WHERE id=$1")
        .bind(composition)
        .fetch_one(&t.admin)
        .await
        .unwrap();
    let next = learning_db::CompositionStore::new(t.runtime.clone())
        .save(
            actor,
            space,
            SaveComposition {
                request_id: Uuid::new_v4(),
                composition_id: Some(composition),
                base_revision_id: Some(current),
                kind: CompositionKind::Document,
                title: "newer target edition".into(),
                nodes: vec![],
                reason: "newer target head".into(),
            },
        )
        .await
        .unwrap();
    learning_db::ReleaseStore::new(t.runtime.clone())
        .publish(
            actor,
            space,
            support::assembly::publish(vec![support::assembly::root(&next, None)]),
        )
        .await
        .unwrap();
    let before = heads(t).await;
    let prepared = t.store().validate_exact(actor, stage).await.unwrap();
    assert!(
        t.store()
            .import_exact(actor, Uuid::new_v4(), prepared)
            .await
            .unwrap()
            .reused
    );
    assert_eq!(heads(t).await, before);
    // A separately requested receipt for preexisting immutable rows is not reuse
    // when an administrator has provisioned those rows without a package receipt.
    sqlx::query("DELETE FROM snapshot_import_batch WHERE actor_id=$1")
        .bind(actor.actor_id)
        .execute(&t.admin)
        .await
        .unwrap();
    let prepared = t.store().validate_exact(actor, stage).await.unwrap();
    assert!(
        !t.store()
            .import_exact(actor, Uuid::new_v4(), prepared)
            .await
            .unwrap()
            .reused
    );
    assert_eq!(heads(t).await, before);
}
#[derive(Clone, Copy, Debug)]
enum ExpectedLock {
    Advisory(i64),
    Transaction,
}
fn advisory_key() -> i64 {
    // Per invocation, not a global fixed key shared with another parallel test.
    (Uuid::new_v4().as_u128() as u64 & i64::MAX as u64) as i64
}
async fn is_blocked(admin: &PgPool, waiter: i32, blocker: i32, expected: ExpectedLock) -> bool {
    let key = match expected {
        ExpectedLock::Advisory(key) => Some(key),
        ExpectedLock::Transaction => None,
    };
    sqlx::query_scalar(
        "SELECT EXISTS(
          SELECT 1 FROM pg_locks w
          JOIN pg_stat_activity wa ON wa.pid=w.pid
          JOIN pg_locks h ON h.pid=$2 AND h.granted AND h.mode='ExclusiveLock'
          JOIN pg_stat_activity ha ON ha.pid=h.pid AND ha.datid=wa.datid
          WHERE w.pid=$1 AND w.pid<>h.pid AND NOT w.granted
            AND wa.datid=(SELECT oid FROM pg_database WHERE datname=current_database())
            AND $2=ANY(pg_blocking_pids(w.pid))
            AND (
              ($3::bigint IS NOT NULL AND w.locktype='advisory' AND h.locktype='advisory'
                AND w.mode='ExclusiveLock' AND w.database=wa.datid AND h.database=w.database
                AND w.classid::bigint=(($3::bigint >> 32) & 4294967295::bigint)
                AND w.objid::bigint=($3::bigint & 4294967295::bigint) AND w.objsubid=1
                AND h.classid=w.classid AND h.objid=w.objid AND h.objsubid=w.objsubid)
              OR ($3::bigint IS NULL AND w.locktype='transactionid' AND h.locktype='transactionid'
                AND w.mode='ShareLock' AND w.transactionid=h.transactionid)
            ))",
    )
    .bind(waiter)
    .bind(blocker)
    .bind(key)
    .fetch_one(admin)
    .await
    .unwrap()
}
async fn wait_blocked(admin: &PgPool, waiter: i32, blocker: i32, expected: ExpectedLock) {
    for _ in 0..500 {
        if is_blocked(admin, waiter, blocker, expected).await {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let locks: Vec<Value> = sqlx::query_scalar("SELECT to_jsonb(l) FROM pg_locks l WHERE pid=ANY($1) AND (NOT granted OR locktype IN ('advisory','transactionid'))")
        .bind(vec![waiter, blocker]).fetch_all(admin).await.unwrap();
    panic!(
        "expected {expected:?} wait: waiter={waiter}, blocker={blocker}, observed_locks={locks:?}"
    );
}
async fn grant_lock_races(
    t: &Target,
    actor: Principal,
    space: Uuid,
    plan: &SnapshotPlan,
    stage: &SnapshotDirectory,
) {
    // Revocation owns the row lock first; importer must fail after waiting.
    let prepared = t.store().validate_exact(actor, stage).await.unwrap();
    let before = counts(t, plan).await;
    let mut revoke = t.admin.begin().await.unwrap();
    let revoke_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *revoke)
        .await
        .unwrap();
    sqlx::query("DELETE FROM space_grant WHERE actor_id=$1 AND space_id=$2")
        .bind(actor.actor_id)
        .bind(space)
        .execute(&mut *revoke)
        .await
        .unwrap();
    let (store, import_pid) = t.pinned_store().await;
    let task =
        tokio::spawn(async move { store.import_exact(actor, Uuid::new_v4(), prepared).await });
    wait_blocked(&t.admin, import_pid, revoke_pid, ExpectedLock::Transaction).await;
    revoke.commit().await.unwrap();
    assert!(matches!(task.await.unwrap(), Err(ContentError::NotFound)));
    assert_eq!(counts(t, plan).await, before);
    sqlx::query("INSERT INTO space_grant VALUES($1,$2,true)")
        .bind(actor.actor_id)
        .bind(space)
        .execute(&t.admin)
        .await
        .unwrap();
    // Import owns the grants first; block its final receipt so a concurrent revoke
    // can be observed waiting for exactly that transaction's grant locks.
    let pause_key = advisory_key();
    sqlx::raw_sql(&format!("CREATE FUNCTION public.test_import_pause() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock({pause_key}::bigint); RETURN NEW; END $$; CREATE TRIGGER test_import_pause BEFORE INSERT ON snapshot_import_batch FOR EACH ROW EXECUTE FUNCTION public.test_import_pause();")).execute(&t.admin).await.unwrap();
    let mut gate = t.admin.begin().await.unwrap();
    let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock($1::bigint)")
        .bind(pause_key)
        .execute(&mut *gate)
        .await
        .unwrap();
    let prepared = t.store().validate_exact(actor, stage).await.unwrap();
    let (store, import_pid) = t.pinned_store().await;
    let task =
        tokio::spawn(async move { store.import_exact(actor, Uuid::new_v4(), prepared).await });
    wait_blocked(
        &t.admin,
        import_pid,
        gate_pid,
        ExpectedLock::Advisory(pause_key),
    )
    .await;
    let mut revocation = t.admin.begin().await.unwrap();
    let revoke_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *revocation)
        .await
        .unwrap();
    let revoker = tokio::spawn(async move {
        sqlx::query("DELETE FROM space_grant WHERE actor_id=$1 AND space_id=$2")
            .bind(actor.actor_id)
            .bind(space)
            .execute(&mut *revocation)
            .await
            .unwrap();
        revocation.commit().await.unwrap();
    });
    wait_blocked(&t.admin, revoke_pid, import_pid, ExpectedLock::Transaction).await;
    gate.commit().await.unwrap();
    assert!(task.await.unwrap().unwrap().reused);
    revoker.await.unwrap();
    sqlx::raw_sql("DROP TRIGGER test_import_pause ON snapshot_import_batch; DROP FUNCTION public.test_import_pause();").execute(&t.admin).await.unwrap();
    sqlx::query("INSERT INTO space_grant VALUES($1,$2,true)")
        .bind(actor.actor_id)
        .bind(space)
        .execute(&t.admin)
        .await
        .unwrap();
}

async fn repeated_digest_and_deleted_metadata_identity(
    t: &Target,
    actor: Principal,
    plan: &SnapshotPlan,
    source: &Files,
) {
    let mut duplicate = plan.clone();
    let mut asset = plan
        .rows
        .iter()
        .find(|r| r.table == SnapshotTable::Asset)
        .unwrap()
        .clone();
    let id = Uuid::new_v4();
    asset.immutable_values["id"] = json!(id);
    asset.identity[1] = SnapshotIdentityPart::Uuid(id);
    duplicate.rows.push(asset);
    rehash(&mut duplicate);
    let staged = source.stage(&duplicate);
    let prepared = t.store().validate_exact(actor, &staged).await.unwrap();
    assert_eq!(prepared.assets().count(), 2);
    t.store()
        .import_exact(actor, Uuid::new_v4(), prepared)
        .await
        .unwrap();
    duplicate.manifest.requires_destination_assets = true;
    duplicate
        .manifest
        .files
        .retain(|f| !f.path.starts_with("assets/"));
    let metadata = source.stage(&duplicate);
    let prepared = t.store().validate_exact(actor, &metadata).await.unwrap();
    sqlx::query("DELETE FROM asset WHERE id=$1")
        .bind(id)
        .execute(&t.admin)
        .await
        .unwrap();
    let before = counts(t, plan).await;
    assert!(matches!(
        t.store()
            .import_exact(actor, Uuid::new_v4(), prepared)
            .await,
        Err(ContentError::NotFound)
    ));
    assert_eq!(counts(t, plan).await, before);
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM asset WHERE id=$1)")
        .bind(id)
        .fetch_one(&t.runtime)
        .await
        .unwrap();
    assert!(
        !exists,
        "same SHA must not recreate missing metadata-only identity"
    );
}

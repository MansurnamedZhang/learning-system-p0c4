//! Isolated Task 7 management driver. Never mount this binary or its secrets in a Worker.
#[path = "c3/attention.rs"]
mod attention;
#[path = "../../learning-db/tests/support/relation_store.rs"]
mod relations;
#[path = "../../learning-db/tests/support/mod.rs"]
mod support;

use learning_assets::{FsAssetStore, SnapshotDirectory, stage_incoming};
use learning_core::*;
use learning_db::{
    JobFailureClass, JobLease, JobStore, MIGRATOR, ReadingStore, SnapshotImportStore, SnapshotStore,
};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    env, fs,
    io::{Read, Write},
    path::PathBuf,
    time::Duration,
};
use uuid::Uuid;

fn control() -> PathBuf {
    PathBuf::from(env::var_os("C3_CONTROL").expect("private control volume"))
}
fn files() -> FsAssetStore {
    FsAssetStore::new(
        env::var_os("ASSET_ROOT").unwrap().into(),
        env::var_os("STAGING_ROOT").unwrap().into(),
    )
    .unwrap()
}
async fn pool(name: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .connect(&env::var(name).expect("explicit isolated DSN"))
        .await
        .expect("isolated connection")
}
fn store(pool: &PgPool) -> SnapshotStore {
    SnapshotStore::new(pool.clone()).with_export_storage(
        PathBuf::from(env::var_os("SNAPSHOT_ROOT").unwrap()),
        files(),
    )
}
fn state() -> Value {
    serde_json::from_slice(&fs::read(control().join("case.json")).unwrap()).unwrap()
}
fn id(s: &Value, name: &str) -> Uuid {
    serde_json::from_value(s[name].clone()).unwrap()
}
fn actor(s: &Value) -> Principal {
    Principal {
        actor_id: id(s, "actor"),
    }
}
fn input(s: &Value) -> SnapshotRequest {
    serde_json::from_value(s["request"].clone()).unwrap()
}
fn save(name: &str, value: &Value) {
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(control().join(name))
        .unwrap();
    f.write_all(canonical_json(value).as_bytes()).unwrap();
    f.sync_all().unwrap();
}

async fn empty() {
    let names: std::collections::BTreeMap<String, String> = serde_json::from_str(
        include_str!("../../../deploy/c3-databases.json").trim_start_matches('\u{feff}'),
    )
    .unwrap();
    let mut actual = std::collections::BTreeSet::new();
    for (prefix, expected) in names {
        let p = pool(&format!("{prefix}_ADMIN_DATABASE_URL")).await;
        let db: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&p)
            .await
            .unwrap();
        assert_eq!(db, expected);
        assert!(actual.insert(db.clone()));
        let n:i64=sqlx::query_scalar("SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relkind IN ('r','p','v','m','S')").fetch_one(&p).await.unwrap();
        assert_eq!(
            n, 0,
            "database must be new: {db}; never reset an existing schema"
        );
    }
    println!("{}", json!({"empty_distinct_databases":actual}));
}
async fn legacy_upgrades() {
    let expected: Vec<(i64, Vec<u8>)> = MIGRATOR
        .iter()
        .map(|m| (m.version, m.checksum.to_vec()))
        .collect();
    let mut output = json!({});
    for prefix in [
        "TEST_UPGRADE",
        "TEST_B1_UPGRADE",
        "TEST_B2_UPGRADE",
        "TEST_B3_SCHEMA_UPGRADE",
    ] {
        let p = pool(&format!("{prefix}_ADMIN_DATABASE_URL")).await;
        let sums: Vec<(i64, Vec<u8>)> = sqlx::query_as(
            "SELECT version,checksum FROM _sqlx_migrations WHERE success ORDER BY version",
        )
        .fetch_all(&p)
        .await
        .unwrap();
        assert_eq!(
            sums, expected,
            "legacy DB did not upgrade through exact current migrations"
        );
        output[prefix] = json!(sums);
    }
    println!("{}", output);
}
async fn seed() {
    assert!(!control().join("case.json").exists());
    let (r, a, space, plan, request) = attention::attention().await;
    let projection = ReadingStore::new(r.runtime_pool.clone())
        .read_versioned(a, request.reading.clone(), ReadingMode::Fused)
        .await
        .unwrap()
        .unwrap();
    let job = store(&r.runtime_pool)
        .enqueue_export(a, Uuid::new_v4(), request.clone())
        .await
        .unwrap();
    let jobs = JobStore::new(r.runtime_pool.clone());
    for _ in 0..32 {
        jobs.dispatch_pending(256).await.unwrap();
        if jobs.get(job).await.unwrap().is_some() {
            break;
        }
    }
    assert_eq!(jobs.get(job).await.unwrap().unwrap().attempt_count, 0);
    save(
        "case.json",
        &json!({"actor":a.actor_id,"space":space,"job":job,"request":request,"rows":plan.rows,"projection":projection}),
    );
    println!(
        "{}",
        json!({"job_id":job,"row_count":plan.rows.len(),"originals":plan.assets.len(),"unplaced":projection.unplaced.len()})
    );
}
async fn status(p: &PgPool, s: &Value) -> Value {
    let (status,attempt,digest,expired,count):(String,i32,Option<String>,bool,i64)=sqlx::query_as("SELECT status,attempt_count,output_digest,coalesce(lease_expires_at<=clock_timestamp(),false),(SELECT count(*) FROM snapshot_export_result WHERE job_id=$1) FROM job WHERE id=$1").bind(id(s,"job")).fetch_one(p).await.unwrap();
    json!({"status":status,"attempt":attempt,"digest":digest,"expired":expired,"results":count})
}
async fn lease(p: &PgPool, s: &Value) -> JobLease {
    sqlx::query_as("SELECT id AS job_id,lease_token AS token,attempt_count,lease_expires_at FROM job WHERE id=$1 AND status='snapshot_running' AND lease_expires_at>clock_timestamp()").bind(id(s,"job")).fetch_one(p).await.unwrap()
}
async fn capture(admin: &PgPool, p: &PgPool, s: &Value) {
    let lease = lease(admin, s).await;
    assert_eq!(lease.attempt_count, 1);
    assert_eq!(status(p, s).await["results"], 0);
    let path = control().join("old-token");
    let mut opts = fs::OpenOptions::new();
    opts.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path).unwrap();
    f.write_all(lease.token.to_string().as_bytes()).unwrap();
    f.sync_all().unwrap();
    assert!(
        self::lease(admin, s).await.token == lease.token,
        "captured lease changed"
    );
    println!(
        "{}",
        json!({"captured":true,"attempt":1,"token_sha256":hex_digest(lease.token.to_string().as_bytes())})
    );
}
async fn fence(admin: &PgPool, p: &PgPool, s: &Value) {
    let live = lease(admin, s).await;
    assert_eq!(live.attempt_count, 2);
    let bytes = fs::read(control().join("old-token")).unwrap();
    let old = Uuid::parse_str(std::str::from_utf8(&bytes).unwrap()).unwrap();
    assert!(old != live.token, "replacement did not fence old token");
    let j = JobStore::new(p.clone());
    let job = id(s, "job");
    assert!(!j.renew(job, old, Duration::from_secs(30)).await.unwrap());
    assert!(!j.checkpoint(job, old, json!({})).await.unwrap());
    assert!(!j.succeed(job, old, &"a".repeat(64)).await.unwrap());
    assert!(
        !j.fail(job, old, JobFailureClass::InvalidInput)
            .await
            .unwrap()
    );
    let old_lease = JobLease {
        token: old,
        attempt_count: 1,
        ..live.clone()
    };
    assert_eq!(
        store(p).process_snapshot_export(&old_lease).await.unwrap(),
        learning_db::AssetProcessOutcome::LeaseLost
    );
    assert!(
        lease(admin, s).await.token == live.token,
        "replacement lease changed"
    );
    assert_eq!(status(p, s).await["results"], 0);
    println!(
        "{}",
        json!({"fenced":true,"attempt":2,"token_sha256":hex_digest(&bytes)})
    );
}
async fn incoming(p: &PgPool, s: &Value, root: &std::path::Path) -> SnapshotDirectory {
    let delivered = store(p)
        .deliver_export(actor(s), id(s, "job"))
        .await
        .unwrap();
    let staged = stage_incoming(
        root,
        delivered
            .files
            .into_iter()
            .map(|(n, f)| (n, Box::new(f) as Box<dyn Read>)),
    )
    .unwrap();
    assert_eq!(staged.manifest_sha256(), delivered.manifest_sha256);
    staged
}
async fn rows(p: &PgPool, expected: &[SnapshotRow]) {
    for row in expected {
        let table = serde_json::to_value(row.table).unwrap();
        let table = table.as_str().unwrap();
        let candidates: Vec<Value> =
            sqlx::query_scalar(&format!("SELECT to_jsonb(t) FROM public.{table} t"))
                .fetch_all(p)
                .await
                .unwrap();
        assert!(
            candidates.into_iter().any(|mut value| {
                for field in [
                    "head_revision_id",
                    "head_review_id",
                    "published_revision_id",
                    "last_release_id",
                ] {
                    value.as_object_mut().unwrap().remove(field);
                }
                if let Some(at) = value.get_mut("created_at") {
                    *at = json!(canonical_snapshot_timestamp(
                        chrono::DateTime::parse_from_rfc3339(at.as_str().unwrap())
                            .unwrap()
                            .with_timezone(&chrono::Utc)
                    ));
                }
                value == row.immutable_values
            }),
            "missing immutable row: {table} {:?}",
            row.identity
        );
    }
}
async fn roundtrip(p: &PgPool, s: &Value) {
    let admin = pool("C3_TARGET_ADMIN_DATABASE_URL").await;
    let n: bool = sqlx::query_scalar("SELECT to_regclass('public.block') IS NULL")
        .fetch_one(&admin)
        .await
        .unwrap();
    assert!(n, "target must be fresh");
    MIGRATOR.run(&admin).await.unwrap();
    let target = pool("C3_TARGET_DATABASE_URL").await;
    sqlx::query("INSERT INTO app_user(id) VALUES($1)")
        .bind(id(s, "actor"))
        .execute(&admin)
        .await
        .unwrap();
    sqlx::query("INSERT INTO space(id,owner_id) VALUES($1,$2)")
        .bind(id(s, "space"))
        .bind(id(s, "actor"))
        .execute(&admin)
        .await
        .unwrap();
    sqlx::query("INSERT INTO space_grant(actor_id,space_id,can_write) VALUES($1,$2,true)")
        .bind(id(s, "actor"))
        .bind(id(s, "space"))
        .execute(&admin)
        .await
        .unwrap();
    let root = PathBuf::from(env::var_os("C3_TARGET_ROOT").unwrap());
    let stranger = Principal {
        actor_id: Uuid::new_v4(),
    };
    assert!(matches!(
        store(p).deliver_export(stranger, id(s, "job")).await,
        Err(ContentError::NotFound)
    ));
    let staged = incoming(p, s, &root).await;
    let target_files = FsAssetStore::new(root.join("assets"), root.join("uploads")).unwrap();
    let importer = SnapshotImportStore::new(target.clone(), target_files.clone());
    assert!(matches!(
        importer.validate_exact(stranger, &staged).await,
        Err(ContentError::NotFound)
    ));
    let empty: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM block),(SELECT count(*) FROM snapshot_import_batch)",
    )
    .fetch_one(&admin)
    .await
    .unwrap();
    assert_eq!(empty, (0, 0));
    let prepared = importer.validate_exact(actor(s), &staged).await.unwrap();
    let expected: Vec<SnapshotRow> = serde_json::from_value(s["rows"].clone()).unwrap();
    assert_eq!(prepared.rows(), expected);
    let receipt = importer
        .import_exact(actor(s), Uuid::new_v4(), prepared)
        .await
        .unwrap();
    assert!(!receipt.reused);
    assert_eq!(receipt.manifest_sha256, staged.manifest_sha256());
    rows(&target, &expected).await;
    rows(p, &expected).await;
    let mut counts = std::collections::BTreeMap::new();
    for row in &expected {
        *counts.entry(row.table).or_insert(0_i64) += 1;
    }
    for (table, expected_count) in &counts {
        let table = serde_json::to_value(table).unwrap();
        let actual: i64 = sqlx::query_scalar(&format!(
            "SELECT count(*) FROM public.{}",
            table.as_str().unwrap()
        ))
        .fetch_one(&admin)
        .await
        .unwrap();
        assert_eq!(actual, *expected_count, "unexpected rows in {table}");
    }
    let projection = ReadingStore::new(target.clone())
        .read_versioned(actor(s), input(s).reading, ReadingMode::Fused)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(serde_json::to_value(&projection).unwrap(), s["projection"]);
    assert_eq!(projection.unplaced.len(), 2);
    let mut originals = vec![];
    for row in expected.iter().filter(|r| r.table == SnapshotTable::Asset) {
        let v = &row.immutable_values;
        let sha = v["sha256"].as_str().unwrap();
        let size = v["byte_size"].as_i64().unwrap();
        let key = v["storage_key"].as_str().unwrap();
        let mut source = vec![];
        files()
            .open_record(key, sha, size)
            .unwrap()
            .read_to_end(&mut source)
            .unwrap();
        let mut dest = vec![];
        target_files
            .open_record(key, sha, size)
            .unwrap()
            .read_to_end(&mut dest)
            .unwrap();
        assert_eq!(source, dest);
        assert_eq!(hex_digest(&dest), sha);
        originals.push(json!({"sha256":sha,"size":size}));
    }
    for table in [
        "release",
        "release_root",
        "release_reading",
        "release_manifest_object",
        "release_manifest_composition",
        "lineage_operation",
        "lineage_input",
        "lineage_output",
        "request_key",
        "mutation_receipt",
        "upload_receipt",
        "reading_receipt",
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
        "snapshot_export_request",
        "snapshot_export_result",
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM public.{table}"))
            .fetch_one(&admin)
            .await
            .unwrap();
        assert_eq!(count, 0, "excluded {table}");
        eprintln!("c3_target_excluded table={table} count={count}");
    }
    let receipts: Vec<String> =
        sqlx::query_scalar("SELECT manifest_sha256 FROM snapshot_import_batch")
            .fetch_all(&target)
            .await
            .unwrap();
    assert_eq!(receipts, vec![receipt.manifest_sha256.clone()]);
    let evidence = json!({"roundtrip":true,"wrong_actor_delivery_rejected":true,"wrong_actor_import_rejected":true,
        "manifest_sha256":receipt.manifest_sha256,"immutable_rows_sha256":canonical_record_hash(&s["rows"]),
        "projection_sha256":canonical_record_hash(&s["projection"]),"originals":originals,"receipt_count":1});
    save("roundtrip.json", &evidence);
    println!("{evidence}");
}

#[tokio::main]
async fn main() {
    let action = env::args().nth(1).expect("action");
    if action == "binary" {
        println!(
            "{}",
            json!({"sha256":hex_digest(&fs::read("/app/target/debug/learning-worker").unwrap())})
        );
        return;
    }
    if action == "legacy-upgrades" {
        legacy_upgrades().await;
        return;
    }
    if action == "empty" {
        empty().await;
        return;
    }
    if action == "seed" {
        seed().await;
        return;
    }
    let s = state();
    let p = pool("TEST_DATABASE_URL").await;
    match action.as_str() {
        "status" => println!("{}", status(&p, &s).await),
        "capture" => capture(&pool("TEST_ADMIN_DATABASE_URL").await, &p, &s).await,
        "fence" => fence(&pool("TEST_ADMIN_DATABASE_URL").await, &p, &s).await,
        "roundtrip" => roundtrip(&p, &s).await,
        "revoke" => {
            let admin = pool("TEST_ADMIN_DATABASE_URL").await;
            assert_eq!(
                sqlx::query("DELETE FROM space_grant WHERE actor_id=$1 AND space_id=$2")
                    .bind(id(&s, "actor"))
                    .bind(id(&s, "space"))
                    .execute(&admin)
                    .await
                    .unwrap()
                    .rows_affected(),
                1
            );
            assert!(matches!(
                store(&p).deliver_export(actor(&s), id(&s, "job")).await,
                Err(ContentError::NotFound)
            ));
            println!("{}", json!({"revoked_delivery":true}));
        }
        _ => panic!("unknown fixture action"),
    }
}

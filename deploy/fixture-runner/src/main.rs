//! Executes genuinely frozen B2 program code against old schema versions.
use learning_core::*;
use learning_db::{CompositionStore, ContentStore, ReleaseStore};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;
const COMMIT: &str = "b60056ba08892d69efdfa4778c976154a421e17d";
const SOURCE_HASH: &str = "2da005cdf6f2e92ea4fa2f34463b371a71c30e145e6d8927eb75db558b284225";
fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("required environment variable {name}"))
}
fn command(text: &str) -> CreateCommand {
    CreateCommand{request_id:Uuid::new_v4(),reason:"initial".into(),draft:serde_json::from_value(json!({"kind":"text","intent":"note","language":"zh-CN","title":"私有标题","payload":{"format":"markdown","text":text}})).unwrap()}
}
#[tokio::main]
async fn main() {
    let kind = std::env::args().nth(1).expect("p0a|b1|b3-schema required");
    let (admin_env, runtime_env, version) = match kind.as_str() {
        "p0a" => (
            "TEST_UPGRADE_ADMIN_DATABASE_URL",
            "TEST_UPGRADE_DATABASE_URL",
            1,
        ),
        "b1" => (
            "TEST_B1_UPGRADE_ADMIN_DATABASE_URL",
            "TEST_B1_UPGRADE_DATABASE_URL",
            2,
        ),
        "b3-schema" => (
            "TEST_B3_SCHEMA_UPGRADE_ADMIN_DATABASE_URL",
            "TEST_B3_SCHEMA_UPGRADE_DATABASE_URL",
            3,
        ),
        _ => panic!("unknown fixture kind"),
    };
    let manifest_text = include_str!("../../fixtures/b2-b60056b/sources.sha256");
    assert_eq!(hex_digest(manifest_text.as_bytes()), SOURCE_HASH);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/b2-b60056b");
    for line in manifest_text.lines() {
        let (hash, path) = line.split_once("  ").unwrap();
        assert!(!path.starts_with('/') && !path.split('/').any(|p| p == ".."));
        assert_eq!(
            hex_digest(&std::fs::read(root.join(path)).unwrap()),
            hash,
            "frozen source changed: {path}"
        );
    }
    let admin = PgPool::connect(&env(admin_env)).await.unwrap();
    let runtime = PgPool::connect(&env(runtime_env)).await.unwrap();
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT to_regclass('public._sqlx_migrations') IS NULL")
            .fetch_one(&admin)
            .await
            .unwrap(),
        "fixture runner requires empty dedicated database"
    );
    let mut old = sqlx::migrate::Migrator::DEFAULT;
    old.migrations = std::borrow::Cow::Owned(
        learning_db::MIGRATOR
            .iter()
            .filter(|m| m.version <= version)
            .cloned()
            .collect(),
    );
    old.run(&admin).await.unwrap();
    let actor = Principal {
        actor_id: Uuid::new_v4(),
    };
    let space = Uuid::new_v4();
    sqlx::query("INSERT INTO app_user VALUES($1)")
        .bind(actor.actor_id)
        .execute(&admin)
        .await
        .unwrap();
    sqlx::query("INSERT INTO space VALUES($1,$2)")
        .bind(space)
        .bind(actor.actor_id)
        .execute(&admin)
        .await
        .unwrap();
    sqlx::query("INSERT INTO space_grant VALUES($1,$2,true)")
        .bind(actor.actor_id)
        .bind(space)
        .execute(&admin)
        .await
        .unwrap();
    let store = ContentStore::new(runtime.clone());
    let cmd = command(match kind.as_str() {
        "p0a" => "historic content",
        "b1" => "B1 preserved",
        _ => "historical v1 bytes",
    });
    let before = if kind == "p0a" {
        let block = Uuid::new_v4();
        let revision = Uuid::new_v4();
        let digest=hex_digest(canonical_json(&json!({"operation":"create","target":space,"contract_version":CONTRACT_VERSION,"command":cmd})).as_bytes());
        let mut tx = admin.begin().await.unwrap();
        sqlx::query("INSERT INTO block(id,space_id,head_revision_id) VALUES($1,$2,$3)")
            .bind(block)
            .bind(space)
            .bind(revision)
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("INSERT INTO block_revision(id,space_id,block_id,content,content_sha256,author_id,reason) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(revision).bind(space).bind(block).bind(sqlx::types::Json(&cmd.draft)).bind(cmd.draft.digest()).bind(actor.actor_id).bind(&cmd.reason).execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO mutation_receipt(actor_id,request_id,request_sha256,revision_id) VALUES($1,$2,$3,$4)").bind(actor.actor_id).bind(cmd.request_id).bind(digest).bind(revision).execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        store.read(actor, revision).await.unwrap().unwrap()
    } else {
        store.create(actor, space, cmd.clone()).await.unwrap()
    };
    let mut manifest = json!({"schema_version":1,"kind":kind,"producer_commit":COMMIT,"source_manifest_sha256":SOURCE_HASH,"started_empty":true,"actor_id":actor.actor_id,"space":space,"command":cmd,"before":before});
    if kind == "b1" {
        let compositions = CompositionStore::new(runtime.clone());
        let releases = ReleaseStore::new(runtime.clone());
        let dc = SaveComposition {
            request_id: Uuid::new_v4(),
            composition_id: None,
            base_revision_id: None,
            kind: CompositionKind::Document,
            title: "Attention".into(),
            nodes: vec![NodeDraft {
                occurrence_id: None,
                target: NodeTarget::Block(BlockRef {
                    block_id: before.block_id,
                    revision_id: before.revision_id,
                }),
            }],
            reason: "initial".into(),
        };
        let doc = compositions.save(actor, space, dc.clone()).await.unwrap();
        let pc=PublishCommand{request_id:Uuid::new_v4(),roots:vec![PublishRoot{composition_id:doc.reference.composition_id,revision_id:doc.reference.revision_id,expected_head_revision_id:doc.reference.revision_id,expected_publication_token:hex_digest(canonical_json(&json!({"domain":"publication-basis-v1","composition_id":doc.reference.composition_id,"last_release_id":Value::Null})).as_bytes())}],reason:"publish".into()};
        let published = releases.publish(actor, space, pc.clone()).await.unwrap();
        let snapshot = compositions
            .read(actor, doc.reference.clone())
            .await
            .unwrap();
        let counts:(i64,i64,i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM composition_revision WHERE author_id=$1),(SELECT count(*) FROM release WHERE author_id=$1),(SELECT count(*) FROM outbox_event e JOIN release r ON r.id=e.aggregate_id WHERE r.author_id=$1),(SELECT count(*) FROM request_key WHERE actor_id=$1),(SELECT count(*) FROM composition_receipt WHERE actor_id=$1)+(SELECT count(*) FROM release_receipt WHERE actor_id=$1)").bind(actor.actor_id).fetch_one(&admin).await.unwrap();
        manifest["composition_command"] = json!(dc);
        manifest["composition"] = json!(doc);
        manifest["publish_command"] = json!(pc);
        manifest["published"] = json!(published);
        manifest["snapshot"] = json!(snapshot);
        manifest["counts"] = json!(counts);
    }
    let checksums: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&admin)
            .await
            .unwrap();
    manifest["checksums"] = json!(checksums);
    let destination = env("FIXTURE_MANIFEST_OUTPUT");
    std::fs::write(destination, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    println!("frozen {kind} fixture generated from {COMMIT}");
}

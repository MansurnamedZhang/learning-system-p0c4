use learning_core::{CONTRACT_VERSION, Principal, canonical_json, hex_digest};
use learning_db::{ContentStore, MIGRATOR};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;
mod support;

#[tokio::test]
async fn real_p0a_data_and_idempotent_receipt_survive_additive_upgrade() {
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect(
            &std::env::var("TEST_UPGRADE_ADMIN_DATABASE_URL")
                .expect("fresh upgrade admin database required"),
        )
        .await
        .unwrap();
    let runtime = PgPoolOptions::new()
        .max_connections(2)
        .connect(
            &std::env::var("TEST_UPGRADE_DATABASE_URL")
                .expect("fresh upgrade runtime database required"),
        )
        .await
        .unwrap();
    let empty: bool = sqlx::query_scalar("SELECT to_regclass('public._sqlx_migrations') IS NULL")
        .fetch_one(&admin)
        .await
        .unwrap();
    assert!(empty, "upgrade test requires its own fresh database");
    let path = std::env::var("TEST_P0A_MIGRATIONS_DIR")
        .expect("directory containing only unchanged 0001 required");
    let old = sqlx::migrate::Migrator::new(std::path::Path::new(&path))
        .await
        .unwrap();
    assert_eq!(old.iter().count(), 1);
    old.run(&admin).await.unwrap();
    let actor = Principal {
        actor_id: Uuid::new_v4(),
    };
    let space = Uuid::new_v4();
    let block = Uuid::new_v4();
    let revision = Uuid::new_v4();
    let cmd = support::command("historic content");
    let digest=hex_digest(canonical_json(&serde_json::json!({"operation":"create","target":space,"contract_version":CONTRACT_VERSION,"command":cmd})).as_bytes());
    let mut tx = admin.begin().await.unwrap();
    sqlx::query("INSERT INTO app_user VALUES($1)")
        .bind(actor.actor_id)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO space VALUES($1,$2)")
        .bind(space)
        .bind(actor.actor_id)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO space_grant VALUES($1,$2,true)")
        .bind(actor.actor_id)
        .bind(space)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO block(id,space_id,head_revision_id) VALUES($1,$2,$3)")
        .bind(block)
        .bind(space)
        .bind(revision)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO block_revision(id,space_id,block_id,content,content_sha256,author_id,reason) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(revision).bind(space).bind(block).bind(sqlx::types::Json(&cmd.draft)).bind(cmd.draft.digest()).bind(actor.actor_id).bind(&cmd.reason).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO mutation_receipt(actor_id,request_id,request_sha256,revision_id) VALUES($1,$2,$3,$4)").bind(actor.actor_id).bind(cmd.request_id).bind(&digest).bind(revision).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let store = ContentStore::new(runtime.clone());
    let before = store.read(actor, revision).await.unwrap().unwrap();
    let old_checksum: Vec<u8> =
        sqlx::query_scalar("SELECT checksum FROM _sqlx_migrations WHERE version=1")
            .fetch_one(&admin)
            .await
            .unwrap();
    MIGRATOR.run(&admin).await.unwrap();
    let new_checksum: Vec<u8> =
        sqlx::query_scalar("SELECT checksum FROM _sqlx_migrations WHERE version=1")
            .fetch_one(&admin)
            .await
            .unwrap();
    assert_eq!(new_checksum, old_checksum);
    assert_eq!(store.read(actor, revision).await.unwrap().unwrap(), before);
    assert_eq!(
        store.create(actor, space, cmd.clone()).await.unwrap(),
        before
    );
    let key: (String, String) = sqlx::query_as(
        "SELECT operation,request_sha256 FROM request_key WHERE actor_id=$1 AND request_id=$2",
    )
    .bind(actor.actor_id)
    .bind(cmd.request_id)
    .fetch_one(&admin)
    .await
    .unwrap();
    assert_eq!(key, ("content_v1".into(), digest));
    let mut next = support::assembly::doc(vec![]);
    next.request_id = cmd.request_id;
    assert!(matches!(
        learning_db::CompositionStore::new(runtime)
            .save(actor, space, next)
            .await,
        Err(learning_core::ContentError::IdempotencyConflict)
    ));
}

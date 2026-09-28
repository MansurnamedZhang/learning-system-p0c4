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
    let fixture = support::frozen_fixture::load("p0a", &admin).await;
    let actor = Principal {
        actor_id: serde_json::from_value(fixture["actor_id"].clone()).unwrap(),
    };
    let space: Uuid = serde_json::from_value(fixture["space"].clone()).unwrap();
    let cmd: learning_core::CreateCommand =
        serde_json::from_value(fixture["command"].clone()).unwrap();
    let before: learning_core::Revision =
        serde_json::from_value(fixture["before"].clone()).unwrap();
    let revision = before.revision_id;
    let digest=hex_digest(canonical_json(&serde_json::json!({"operation":"create","target":space,"contract_version":CONTRACT_VERSION,"command":cmd})).as_bytes());
    let old_checksum: Vec<u8> = serde_json::from_value(fixture["checksums"][0][1].clone()).unwrap();
    let store = ContentStore::new(runtime.clone());
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

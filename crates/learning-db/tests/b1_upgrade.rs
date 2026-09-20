mod support;
use learning_core::*;
use learning_db::{ContentStore, MIGRATOR};
use sqlx::postgres::PgPoolOptions;
use support::*;
#[tokio::test]
async fn actual_b1_stores_and_receipts_survive_b2_upgrade() {
    let admin_pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(
            &std::env::var("TEST_B1_UPGRADE_ADMIN_DATABASE_URL")
                .expect("fresh B1 upgrade admin database required"),
        )
        .await
        .unwrap();
    let runtime_pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(
            &std::env::var("TEST_B1_UPGRADE_DATABASE_URL")
                .expect("fresh B1 upgrade runtime database required"),
        )
        .await
        .unwrap();
    let fixture = frozen_fixture::load("b1", &admin_pool).await;
    let r = TestRig {
        store: ContentStore::new(runtime_pool.clone()),
        admin_pool,
        runtime_pool,
    };
    let actor = Principal {
        actor_id: serde_json::from_value(fixture["actor_id"].clone()).unwrap(),
    };
    let space = serde_json::from_value(fixture["space"].clone()).unwrap();
    let c: CreateCommand = serde_json::from_value(fixture["command"].clone()).unwrap();
    let b: Revision = serde_json::from_value(fixture["before"].clone()).unwrap();
    let dc: SaveComposition =
        serde_json::from_value(fixture["composition_command"].clone()).unwrap();
    let d: CompositionRevision = serde_json::from_value(fixture["composition"].clone()).unwrap();
    let pc: PublishCommand = serde_json::from_value(fixture["publish_command"].clone()).unwrap();
    let published: Release = serde_json::from_value(fixture["published"].clone()).unwrap();
    let before: Option<CompositionSnapshot> =
        serde_json::from_value(fixture["snapshot"].clone()).unwrap();
    let sums: Vec<(i64, Vec<u8>)> = serde_json::from_value(fixture["checksums"].clone()).unwrap();
    let counts: (i64, i64, i64, i64, i64) =
        serde_json::from_value(fixture["counts"].clone()).unwrap();
    assert_eq!(r.assembly_counts(actor).await, counts);
    MIGRATOR.run(&r.admin_pool).await.unwrap();
    let after: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations WHERE version<=2 ORDER BY version",
    )
    .fetch_all(&r.admin_pool)
    .await
    .unwrap();
    assert_eq!(sums, after);
    assert_eq!(r.store.create(actor, space, c).await.unwrap(), b);
    assert_eq!(r.compositions().save(actor, space, dc).await.unwrap(), d);
    assert_eq!(
        r.releases().publish(actor, space, pc).await.unwrap(),
        published
    );
    assert_eq!(
        r.compositions().read(actor, d.reference).await.unwrap(),
        before
    );
    assert_eq!(r.assembly_counts(actor).await, counts);
}

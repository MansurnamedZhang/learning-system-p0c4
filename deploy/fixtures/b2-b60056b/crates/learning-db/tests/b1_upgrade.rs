mod support;
use learning_db::{ContentStore, MIGRATOR};
use sqlx::postgres::PgPoolOptions;
use support::{assembly as a, *};
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
    let empty: bool = sqlx::query_scalar("SELECT to_regclass('public._sqlx_migrations') IS NULL")
        .fetch_one(&admin_pool)
        .await
        .unwrap();
    assert!(empty, "B1 upgrade needs separate empty database");
    let path =
        std::env::var("TEST_B1_MIGRATIONS_DIR").expect("unchanged 0001/0002 directory required");
    let old = sqlx::migrate::Migrator::new(std::path::Path::new(&path))
        .await
        .unwrap();
    assert_eq!(old.iter().count(), 2);
    old.run(&admin_pool).await.unwrap();
    let r = TestRig {
        store: ContentStore::new(runtime_pool.clone()),
        admin_pool,
        runtime_pool,
    };
    let (actor, space) = r.seed_actor_space(true).await;
    let c = command("B1 preserved");
    let b = r.store.create(actor, space, c.clone()).await.unwrap();
    let dc = a::doc(vec![a::block(&b)]);
    let d = r
        .compositions()
        .save(actor, space, dc.clone())
        .await
        .unwrap();
    let pc = a::publish(vec![a::root(&d, None)]);
    let published = r
        .releases()
        .publish(actor, space, pc.clone())
        .await
        .unwrap();
    let before = r
        .compositions()
        .read(actor, d.reference.clone())
        .await
        .unwrap();
    let sums: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&r.admin_pool)
            .await
            .unwrap();
    let counts = r.assembly_counts(actor).await;
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

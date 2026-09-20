//! Validates the output of the independently compiled frozen fixture program.
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;
pub async fn load(kind: &str, admin: &PgPool) -> Value {
    let name = match kind {
        "p0a" => "TEST_P0A_FIXTURE_MANIFEST",
        "b1" => "TEST_B1_FIXTURE_MANIFEST",
        "b3-schema" => "TEST_B3_SCHEMA_FIXTURE_MANIFEST",
        _ => panic!("unknown kind"),
    };
    let manifest: Value = serde_json::from_slice(
        &std::fs::read(
            std::env::var(name)
                .expect("frozen fixture manifest required; run deploy/fixture-runner first"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["schema_version"], 1);
    assert_eq!(manifest["kind"], kind);
    assert_eq!(manifest["started_empty"], true);
    assert_eq!(
        manifest["producer_commit"],
        "b60056ba08892d69efdfa4778c976154a421e17d"
    );
    const EXPECTED: &str = "2da005cdf6f2e92ea4fa2f34463b371a71c30e145e6d8927eb75db558b284225";
    assert_eq!(manifest["source_manifest_sha256"], EXPECTED);
    assert_eq!(
        learning_core::hex_digest(
            include_str!("../../../../deploy/fixtures/b2-b60056b/sources.sha256").as_bytes()
        ),
        EXPECTED
    );
    let sums: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(admin)
            .await
            .unwrap();
    assert_eq!(
        serde_json::to_value(sums).unwrap(),
        manifest["checksums"],
        "fixture database/migration mismatch"
    );
    let revision: Uuid = serde_json::from_value(manifest["before"]["revision_id"].clone()).unwrap();
    let record: (Uuid, Uuid, Uuid, String) = sqlx::query_as(
        "SELECT author_id,space_id,block_id,content_sha256 FROM block_revision WHERE id=$1",
    )
    .bind(revision)
    .fetch_one(admin)
    .await
    .unwrap();
    assert_eq!(
        serde_json::json!([record.0, record.1, record.2, record.3]),
        serde_json::json!([
            manifest["actor_id"],
            manifest["space"],
            manifest["before"]["block_id"],
            manifest["before"]["content_sha256"]
        ]),
        "fixture immutable object mismatch"
    );
    manifest
}

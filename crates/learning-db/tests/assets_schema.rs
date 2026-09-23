mod support;

use sqlx::Row;
use support::{TestRig, sqlstate};
use uuid::Uuid;

async fn insert_asset(rig: &TestRig, space: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO asset(space_id,id,sha256,byte_size,storage_key,media_type,original_file_name,status) VALUES($1,$2,$3,3,$4,'image/png','figure.png','ready')")
        .bind(space)
        .bind(id)
        .bind("a".repeat(64))
        .bind(format!("sha256/aa/{}", "a".repeat(64)))
        .execute(&rig.admin_pool)
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn asset_and_resource_keys_reject_cross_space_links() {
    let rig = TestRig::from_env().await;
    let (actor, home) = rig.seed_actor_space(true).await;
    let (_, foreign) = rig.seed_actor_space(true).await;
    let asset = insert_asset(&rig, foreign).await;
    let resource = Uuid::new_v4();
    sqlx::query("INSERT INTO resource(space_id,id,display_name) VALUES($1,$2,'source PDF')")
        .bind(home)
        .bind(resource)
        .execute(&rig.admin_pool)
        .await
        .unwrap();
    let error = sqlx::query("INSERT INTO resource_version(space_id,resource_id,id,asset_id,version_no) VALUES($1,$2,$3,$4,1)")
        .bind(home)
        .bind(resource)
        .bind(Uuid::new_v4())
        .bind(asset)
        .execute(&rig.admin_pool)
        .await
        .unwrap_err();
    assert_eq!(sqlstate(&error).as_deref(), Some("23503"));

    let owned_asset = insert_asset(&rig, home).await;
    let version = Uuid::new_v4();
    sqlx::query("INSERT INTO resource_version(space_id,resource_id,id,asset_id,version_no) VALUES($1,$2,$3,$4,1)")
        .bind(home)
        .bind(resource)
        .bind(version)
        .bind(owned_asset)
        .execute(&rig.admin_pool)
        .await
        .unwrap();
    let error = sqlx::query("INSERT INTO source_segment(space_id,resource_id,resource_version_id,id,selector) VALUES($1,$2,$3,$4,$5)")
        .bind(foreign)
        .bind(resource)
        .bind(version)
        .bind(Uuid::new_v4())
        .bind(serde_json::json!({"page": 4}))
        .execute(&rig.admin_pool)
        .await
        .unwrap_err();
    assert_eq!(sqlstate(&error).as_deref(), Some("23503"));

    let (block, revision) = rig.seed_block(actor, home).await;
    let error = sqlx::query(
        "INSERT INTO block_asset_use(space_id,block_id,revision_id,asset_id) VALUES($1,$2,$3,$4)",
    )
    .bind(home)
    .bind(block)
    .bind(revision)
    .bind(asset)
    .execute(&rig.admin_pool)
    .await
    .unwrap_err();
    assert_eq!(sqlstate(&error).as_deref(), Some("23503"));
    sqlx::query(
        "INSERT INTO block_asset_use(space_id,block_id,revision_id,asset_id) VALUES($1,$2,$3,$4)",
    )
    .bind(home)
    .bind(block)
    .bind(revision)
    .bind(owned_asset)
    .execute(&rig.admin_pool)
    .await
    .unwrap();

    let (other_block, _) = rig.seed_block(actor, home).await;
    let error = sqlx::query(
        "INSERT INTO block_asset_use(space_id,block_id,revision_id,asset_id) VALUES($1,$2,$3,$4)",
    )
    .bind(home)
    .bind(other_block)
    .bind(revision)
    .bind(owned_asset)
    .execute(&rig.admin_pool)
    .await
    .unwrap_err();
    assert_eq!(sqlstate(&error).as_deref(), Some("23503"));
}

#[tokio::test]
async fn asset_metadata_is_bounded_and_runtime_cannot_administer_it() {
    let rig = TestRig::from_env().await;
    let (_, space) = rig.seed_actor_space(true).await;
    for table in [
        "asset",
        "resource",
        "resource_version",
        "source_segment",
        "upload_receipt",
        "block_asset_use",
    ] {
        let rights: (bool, bool, bool, bool, bool) = sqlx::query_as("SELECT has_table_privilege(current_user,$1,'SELECT'),has_table_privilege(current_user,$1,'INSERT'),has_table_privilege(current_user,$1,'UPDATE'),has_table_privilege(current_user,$1,'DELETE'),has_table_privilege(current_user,$1,'TRUNCATE')")
            .bind(format!("public.{table}"))
            .fetch_one(&rig.runtime_pool)
            .await
            .unwrap();
        assert_eq!(rights, (true, true, false, false, false), "{table}");
    }
    let can_create: bool =
        sqlx::query_scalar("SELECT has_schema_privilege(current_user,'public','CREATE')")
            .fetch_one(&rig.runtime_pool)
            .await
            .unwrap();
    assert!(!can_create);
    let bad_name = sqlx::query("INSERT INTO asset(space_id,id,sha256,byte_size,storage_key,media_type,original_file_name,status) VALUES($1,$2,$3,1,$4,'image/png',$5,'ready')")
        .bind(space)
        .bind(Uuid::new_v4())
        .bind("a".repeat(64))
        .bind(format!("sha256/aa/{}", "a".repeat(64)))
        .bind("x".repeat(256))
        .execute(&rig.admin_pool)
        .await
        .unwrap_err();
    assert_eq!(sqlstate(&bad_name).as_deref(), Some("23514"));
    let bad_size = sqlx::query("INSERT INTO asset(space_id,id,sha256,byte_size,storage_key,media_type,original_file_name,status) VALUES($1,$2,$3,-1,$4,'image/png','x','ready')")
        .bind(space)
        .bind(Uuid::new_v4())
        .bind("a".repeat(64))
        .bind(format!("sha256/aa/{}", "a".repeat(64)))
        .execute(&rig.admin_pool)
        .await
        .unwrap_err();
    assert_eq!(sqlstate(&bad_size).as_deref(), Some("23514"));
}

#[tokio::test]
async fn v3_revision_and_new_request_operations_are_additive() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let block = Uuid::new_v4();
    let revision = Uuid::new_v4();
    let mut tx = rig.admin_pool.begin().await.unwrap();
    sqlx::query("INSERT INTO block(id,space_id,head_revision_id) VALUES($1,$2,$3)")
        .bind(block)
        .bind(space)
        .bind(revision)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO block_revision(id,space_id,block_id,contract_version,content,content_sha256,author_id,reason) VALUES($1,$2,$3,3,$4,$5,$6,'fixture')")
        .bind(revision)
        .bind(space)
        .bind(block)
        .bind(serde_json::json!({"kind":"text","intent":"note","language":"en","title":"v3","body":{"kind":"text","payload":{"format":"markdown","text":"fixture"}},"basis_refs":[],"requires_context":[],"source_run":null}))
        .bind("b".repeat(64))
        .bind(actor.actor_id)
        .execute(&mut *tx)
        .await
        .unwrap();
    let staged_version: i32 =
        sqlx::query_scalar("SELECT contract_version FROM block_revision WHERE id=$1")
            .bind(revision)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(staged_version, 3);
    // 0004's deferred dependency validator learns v3 in Task 4's 0010 migration.
    tx.rollback().await.unwrap();
    for operation in ["asset_register", "content_v3"] {
        sqlx::query("INSERT INTO request_key(actor_id,request_id,request_sha256,operation) VALUES($1,$2,$3,$4)")
            .bind(actor.actor_id)
            .bind(Uuid::new_v4())
            .bind("c".repeat(64))
            .bind(operation)
            .execute(&rig.admin_pool)
            .await
            .unwrap();
    }
    let count: i64 = sqlx::query("SELECT count(*) AS n FROM request_key WHERE actor_id=$1 AND operation IN ('asset_register','content_v3')")
        .bind(actor.actor_id)
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap()
        .get("n");
    assert_eq!(count, 2);
}

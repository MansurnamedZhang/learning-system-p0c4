use learning_backup::{AdminAssetCatalog, BackupError};
use learning_db::MIGRATOR;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

/// Run against a fresh isolated database with TEST_ADMIN_DATABASE_URL and
/// TEST_DATABASE_URL. Missing credentials fail explicitly instead of skipping.
#[tokio::test]
async fn admin_collects_linked_and_unlinked_ready_rows_and_runtime_is_rejected() {
    let admin_url = std::env::var("TEST_ADMIN_DATABASE_URL").expect("isolated admin DSN required");
    let runtime_url = std::env::var("TEST_DATABASE_URL").expect("isolated runtime DSN required");
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect(&admin_url)
        .await
        .unwrap();
    MIGRATOR.run(&admin).await.unwrap();
    let runtime = PgPoolOptions::new()
        .max_connections(2)
        .connect(&runtime_url)
        .await
        .unwrap();

    let actor = Uuid::new_v4();
    let space = Uuid::new_v4();
    let linked = Uuid::new_v4();
    let same_bytes = Uuid::new_v4();
    let unrelated = Uuid::new_v4();
    let resource = Uuid::new_v4();
    let version = Uuid::new_v4();
    sqlx::query("INSERT INTO public.app_user(id) VALUES($1)")
        .bind(actor)
        .execute(&admin)
        .await
        .unwrap();
    sqlx::query("INSERT INTO public.space(id,owner_id) VALUES($1,$2)")
        .bind(space)
        .bind(actor)
        .execute(&admin)
        .await
        .unwrap();
    for (id, sha, size) in [(linked, A, 3_i64), (same_bytes, A, 3), (unrelated, B, 5)] {
        sqlx::query("INSERT INTO public.asset(space_id,id,sha256,byte_size,storage_key,media_type,original_file_name,status) VALUES($1,$2,$3,$4,$5,'image/png','test.png','ready')")
            .bind(space).bind(id).bind(sha).bind(size)
            .bind(format!("sha256/{}/{}", &sha[..2], sha))
            .execute(&admin).await.unwrap();
    }
    sqlx::query("INSERT INTO public.resource(space_id,id,display_name) VALUES($1,$2,'linked')")
        .bind(space)
        .bind(resource)
        .execute(&admin)
        .await
        .unwrap();
    sqlx::query("INSERT INTO public.resource_version(space_id,resource_id,id,asset_id,version_no) VALUES($1,$2,$3,$4,1)")
        .bind(space).bind(resource).bind(version).bind(linked).execute(&admin).await.unwrap();

    let plan = AdminAssetCatalog::new(admin.clone())
        .plan_assets()
        .await
        .unwrap();
    assert!(
        plan.assets()
            .iter()
            .any(|row| row.space_id == space && row.id == linked)
    );
    assert!(
        plan.assets()
            .iter()
            .any(|row| row.space_id == space && row.id == same_bytes)
    );
    assert!(
        plan.assets()
            .iter()
            .any(|row| row.space_id == space && row.id == unrelated)
    );
    assert_eq!(
        plan.asset_files().iter().filter(|f| f.sha256 == A).count(),
        1
    );
    assert!(matches!(
        AdminAssetCatalog::new(runtime).plan_assets().await,
        Err(BackupError::Invalid("management role required"))
    ));

    sqlx::query("DELETE FROM public.resource_version WHERE space_id=$1 AND id=$2")
        .bind(space)
        .bind(version)
        .execute(&admin)
        .await
        .unwrap();
    sqlx::query("DELETE FROM public.resource WHERE space_id=$1 AND id=$2")
        .bind(space)
        .bind(resource)
        .execute(&admin)
        .await
        .unwrap();
    sqlx::query("DELETE FROM public.asset WHERE space_id=$1")
        .bind(space)
        .execute(&admin)
        .await
        .unwrap();
    sqlx::query("DELETE FROM public.space WHERE id=$1")
        .bind(space)
        .execute(&admin)
        .await
        .unwrap();
    sqlx::query("DELETE FROM public.app_user WHERE id=$1")
        .bind(actor)
        .execute(&admin)
        .await
        .unwrap();
}

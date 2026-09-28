use learning_backup::{AdminAssetCatalog, BackupError};
use learning_db::MIGRATOR;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn verify_isolated_empty_database(
    actual: &str,
    expected: &str,
    user_objects: i64,
    extra_schemas: i64,
    extra_extensions: i64,
) -> Result<(), &'static str> {
    let suffix = actual
        .strip_prefix("learning_backup_c4_task1_")
        .ok_or("test database name is outside the dedicated namespace")?;
    let marker = Uuid::parse_str(suffix).map_err(|_| "test database name lacks a UUID marker")?;
    if marker.to_string() != suffix || actual != expected {
        return Err("test database identity mismatch");
    }
    if user_objects != 0 || extra_schemas != 0 || extra_extensions != 0 {
        return Err("test database is not empty");
    }
    Ok(())
}

fn verify_session(expected: &str, current: &str, session: &str) -> Result<(), &'static str> {
    if current != expected || session != expected {
        return Err("database session has the wrong role");
    }
    Ok(())
}

#[test]
fn preflight_rejects_reused_or_misnamed_database_before_migrations() {
    let fresh = format!("learning_backup_c4_task1_{}", Uuid::new_v4());
    assert!(verify_isolated_empty_database(&fresh, &fresh, 0, 0, 0).is_ok());
    assert!(verify_isolated_empty_database("learning_test", "learning_test", 0, 0, 0).is_err());
    assert!(verify_isolated_empty_database(&fresh, "other", 0, 0, 0).is_err());
    assert!(verify_isolated_empty_database(&fresh, &fresh, 1, 0, 0).is_err());
    assert!(verify_isolated_empty_database(&fresh, &fresh, 0, 1, 0).is_err());
    assert!(verify_isolated_empty_database(&fresh, &fresh, 0, 0, 1).is_err());
}

#[test]
fn preflight_requires_real_admin_and_runtime_session_roles() {
    assert!(verify_session("learning_admin", "learning_admin", "learning_admin").is_ok());
    assert!(verify_session("learning_runtime", "learning_runtime", "learning_runtime").is_ok());
    assert!(verify_session("learning_runtime", "other", "other").is_err());
    assert!(verify_session("learning_runtime", "learning_runtime", "other").is_err());
    assert!(verify_session("learning_admin", "learning_runtime", "learning_runtime").is_err());
}

/// Provision a new database named `learning_backup_c4_task1_<UUID>` and set
/// TEST_BACKUP_EMPTY_DATABASE_NAME plus both role-specific DSNs to that exact
/// database. Existing or misnamed databases are rejected before MIGRATOR runs.
#[tokio::test]
async fn admin_collects_linked_and_unlinked_ready_rows_and_runtime_is_rejected() {
    let expected_database = std::env::var("TEST_BACKUP_EMPTY_DATABASE_NAME")
        .expect("dedicated empty backup-test database name required");
    let admin_url = std::env::var("TEST_ADMIN_DATABASE_URL").expect("isolated admin DSN required");
    let runtime_url = std::env::var("TEST_DATABASE_URL").expect("isolated runtime DSN required");
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect(&admin_url)
        .await
        .unwrap();
    let runtime = PgPoolOptions::new()
        .max_connections(2)
        .connect(&runtime_url)
        .await
        .unwrap();
    let (admin_database, admin_current, admin_session): (String, String, String) =
        sqlx::query_as("SELECT current_database()::text,current_user::text,session_user::text")
            .fetch_one(&admin)
            .await
            .unwrap();
    let (runtime_database, runtime_current, runtime_session): (String, String, String) =
        sqlx::query_as("SELECT current_database()::text,current_user::text,session_user::text")
            .fetch_one(&runtime)
            .await
            .unwrap();
    verify_session("learning_admin", &admin_current, &admin_session).unwrap();
    verify_session("learning_runtime", &runtime_current, &runtime_session).unwrap();
    assert_eq!(
        runtime_database, admin_database,
        "DSNs target different databases"
    );
    let (user_objects, extra_schemas, extra_extensions): (i64, i64, i64) = sqlx::query_as(
        "SELECT \
          (SELECT count(*) FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
            WHERE n.nspname NOT IN ('pg_catalog','information_schema') \
              AND n.nspname !~ '^pg_(toast|temp)' AND c.relkind IN ('r','p','v','m','f','S')) \
          + (SELECT count(*) FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
              WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname !~ '^pg_(toast|temp)') \
          + (SELECT count(*) FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace \
              WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname !~ '^pg_(toast|temp)' AND t.typrelid=0) \
          + (SELECT count(*) FROM pg_catalog.pg_largeobject_metadata), \
          (SELECT count(*) FROM pg_catalog.pg_namespace n \
            WHERE n.nspname NOT IN ('pg_catalog','information_schema','public') \
              AND n.nspname !~ '^pg_(toast|temp)'), \
          (SELECT count(*) FROM pg_catalog.pg_extension WHERE extname <> 'plpgsql')",
    )
    .fetch_one(&admin)
    .await
    .unwrap();
    verify_isolated_empty_database(
        &admin_database,
        &expected_database,
        user_objects,
        extra_schemas,
        extra_extensions,
    )
    .unwrap();
    MIGRATOR.run(&admin).await.unwrap();

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
    let runtime_visible: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public.asset WHERE space_id=$1")
            .bind(space)
            .fetch_one(&runtime)
            .await
            .unwrap();
    assert_eq!(runtime_visible, 3, "runtime fixture read must succeed");
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

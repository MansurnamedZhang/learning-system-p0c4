//! Explicit empty full-C schema fixture, never a production startup path.
//! Root must first provision one new isolated PG18 database and four roles.
#[cfg(target_os = "linux")]
use learning_db::{MIGRATOR, snapshot_broker};
#[cfg(target_os = "linux")]
use sqlx::postgres::PgPoolOptions;
#[cfg(target_os = "linux")]
use uuid::Uuid;

#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() {
    let result = install().await;
    // Deliberately no DSN/server errors/credentials in a terminal result.
    println!(
        "{}",
        if result.is_ok() {
            "FULL_C_EMPTY_FIXTURE_INSTALLED_NOT_CAPTURE_ACCEPTANCE"
        } else {
            "FULL_C_EMPTY_FIXTURE_REJECTED_NO_REPLAY"
        }
    );
    if result.is_err() {
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("FULL_C_FIXTURE_REQUIRES_LINUX");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
async fn install() -> Result<(), Box<dyn std::error::Error>> {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&std::env::var("KW_C4_FULL_C_CAPTURE_ADMIN_URL")?)
        .await?;
    let identity: (String, String, String, i32, i64) = sqlx::query_as(
        "SELECT session_user::text,current_user::text,current_database(),current_setting('server_version_num')::int,(SELECT count(*) FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public')")
        .fetch_one(&pool).await?;
    let suffix = identity
        .2
        .strip_prefix("learning_backup_c4_task3_")
        .ok_or("fresh fixture namespace")?;
    let id = Uuid::parse_str(suffix)?;
    if identity.0 != "learning_admin"
        || identity.1 != "learning_admin"
        || identity.3 != 180006
        || identity.4 != 0
        || id.get_version_num() != 4
        || id.to_string() != suffix
    {
        return Err("fresh fixture identity".into());
    }
    MIGRATOR.run(&pool).await?;
    snapshot_broker::install(&pool).await?;
    snapshot_broker::install_publication_observation(&pool).await?;
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relkind='r'")
        .fetch_one(&pool).await?;
    if count != 64 {
        return Err("full table count".into());
    }
    pool.close().await;
    Ok(())
}

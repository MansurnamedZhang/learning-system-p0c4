//! One-shot migration of an empty, canonical UUID-named C4 acceptance database.
//! The root-only acceptance harness runs this before the source isolation driver.

use learning_db::MIGRATOR;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let profile = std::env::var_os("TEST_C4_TASK3_FULL_SCHEMA_PROFILE");
    let full_c = selected_profile(profile.as_deref().and_then(std::ffi::OsStr::to_str))?;
    if profile.is_some() && !full_c {
        return Err("unsupported C4 source profile encoding".into());
    }
    #[cfg(not(target_os = "linux"))]
    if full_c {
        return Err("full C source installation requires Linux".into());
    }
    let database = std::env::var("TEST_C4_TASK3_DATABASE_NAME")?;
    let marker = database
        .strip_prefix("learning_backup_c4_task3_")
        .ok_or("not a dedicated C4 Task3 database")?;
    if Uuid::parse_str(marker)?.to_string() != marker {
        return Err("database marker is not canonical UUID".into());
    }
    let url = std::env::var("TEST_ADMIN_DATABASE_URL")?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await?;
    let actual: String = sqlx::query_scalar("SELECT current_database()::text")
        .fetch_one(&pool)
        .await?;
    if actual != database {
        return Err("admin connection is not in the dedicated database".into());
    }
    if full_c {
        let identity: (String, String, bool, i32, i64) = sqlx::query_as(
            "SELECT session_user::text,current_user::text,d.datdba=(SELECT oid FROM pg_catalog.pg_roles WHERE rolname=session_user),current_setting('server_version_num')::int,(SELECT count(*) FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public') FROM pg_catalog.pg_database d WHERE d.datname=current_database()")
            .fetch_one(&pool).await?;
        if identity.0 != "learning_admin"
            || identity.1 != "learning_admin"
            || !identity.2
            || identity.3 / 10000 != 18
            || identity.4 != 0
            || Uuid::parse_str(marker)?.get_version_num() != 4
        {
            return Err("full C installation requires a fresh owned PG18 database".into());
        }
    }
    MIGRATOR.run(&pool).await?;
    #[cfg(target_os = "linux")]
    if full_c {
        learning_db::snapshot_broker::install(&pool).await?;
        learning_db::snapshot_broker::install_publication_observation(&pool).await?;
    }
    pool.close().await;
    println!("C4_TASK3_MIGRATED");
    Ok(())
}

fn selected_profile(value: Option<&str>) -> Result<bool, &'static str> {
    match value {
        None => Ok(false),
        Some("snapshot_c0_d1_v1") => Ok(true),
        Some(_) => Err("unsupported C4 source schema profile"),
    }
}

#[cfg(test)]
mod tests {
    use super::selected_profile;
    #[test]
    fn optional_source_profile_has_one_explicit_literal_and_no_fallback() {
        assert!(!selected_profile(None).unwrap());
        assert!(selected_profile(Some("snapshot_c0_d1_v1")).unwrap());
        for value in [
            "",
            "base",
            "Snapshot_c0_d1_v1",
            "snapshot_c0_d1_v1\n",
            "observation_v1",
        ] {
            assert!(selected_profile(Some(value)).is_err());
        }
    }
}

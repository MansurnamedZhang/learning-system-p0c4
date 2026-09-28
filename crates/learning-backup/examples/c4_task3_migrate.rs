//! One-shot migration of an empty, canonical UUID-named C4 acceptance database.
//! The root-only acceptance harness runs this before the source isolation driver.

use learning_db::MIGRATOR;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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
    MIGRATOR.run(&pool).await?;
    println!("C4_TASK3_MIGRATED");
    Ok(())
}

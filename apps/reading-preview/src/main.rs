mod fixture;
mod web;

use learning_core::ReadingRef;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions, PgSslMode},
};
use std::{
    io::{self, Read},
    net::Ipv4Addr,
    process::ExitCode,
    time::Duration,
};
use uuid::Uuid;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const DOCUMENT_TITLE: &str = "缩放点积注意力";
const READER: &str = "kw_preview_reader";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewDb {
    port: u16,
    marker: Uuid,
}

impl PreviewDb {
    async fn connect(&self, role: &str, password: &str) -> Result<PgPool> {
        let name = format!("kw_preview_{}", self.marker.simple());
        let options = PgConnectOptions::new()
            .host("127.0.0.1")
            .port(self.port)
            .database(&name)
            .username(role)
            .password(password)
            .ssl_mode(PgSslMode::Disable);
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(5))
            .connect_with(options)
            .await?;
        let valid: bool = sqlx::query_scalar(
            "SELECT current_user = $1 AND current_database() = $2
                AND inet_server_addr() = '127.0.0.1'::inet
                AND current_setting('server_version_num')::int / 10000 = 18
                AND EXISTS (SELECT 1 FROM kw_preview.bootstrap WHERE marker = $3)",
        )
        .bind(role)
        .bind(name)
        .bind(self.marker)
        .fetch_one(&pool)
        .await?;
        if !valid {
            return Err("synthetic PostgreSQL 18 identity mismatch".into());
        }
        Ok(pool)
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    owner: Uuid,
    visitor: Uuid,
    source_space: Uuid,
    personal_space: Uuid,
    reading: ReadingRef,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SeedConfig {
    db: PreviewDb,
    admin_password: String,
    runtime_password: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ServeConfig {
    db: PreviewDb,
    fixture: Fixture,
    reader_password: String,
    owner_code: String,
    visitor_code: String,
}

fn input<T: DeserializeOwned>() -> Result<T> {
    let mut raw = Vec::new();
    io::stdin().lock().take(16_385).read_to_end(&mut raw)?;
    if raw.len() > 16_384 {
        return Err("preview config exceeds size limit".into());
    }
    Ok(serde_json::from_slice(&raw)?)
}

async fn serve(config: ServeConfig) -> Result<()> {
    let pool = config.db.connect(READER, &config.reader_password).await?;
    let read_only: bool = sqlx::query_scalar(
        "SELECT current_setting('default_transaction_read_only') = 'on'
            AND NOT has_schema_privilege(current_user, 'public', 'CREATE')
            AND NOT EXISTS (
                SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
                WHERE n.nspname = 'public' AND c.relkind IN ('r', 'p')
                AND has_table_privilege(current_user, c.oid, 'INSERT,UPDATE,DELETE,TRUNCATE,TRIGGER')
            )",
    )
    .fetch_one(&pool)
    .await?;
    if !read_only {
        return Err("preview connection must be read-only".into());
    }
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let address = listener.local_addr()?;
    let app = web::router(
        pool,
        config.fixture,
        config.owner_code,
        config.visitor_code,
        address.to_string(),
    )?;
    println!("http://{address}");
    axum::serve(listener, app).await?;
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    let result = match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [command] if command == "seed" => {
            async {
                let fixture = fixture::seed(input()?).await?;
                println!("{}", serde_json::to_string(&fixture)?);
                Ok(())
            }
            .await
        }
        [command] if command == "serve" => async { serve(input()?).await }.await,
        _ => Err("use the preview launcher to seed or serve".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Reading preview stopped: {error}");
            ExitCode::FAILURE
        }
    }
}

use crate::storage;
use learning_core::{ContentError, Principal};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

pub(crate) async fn begin(
    pool: &PgPool,
    actor: Principal,
    request: Uuid,
) -> Result<Transaction<'_, Postgres>, ContentError> {
    let mut tx = pool.begin().await.map_err(storage)?;
    for sql in [
        "SET TRANSACTION ISOLATION LEVEL READ COMMITTED",
        "SET LOCAL lock_timeout='10s'",
        "SET LOCAL statement_timeout='15s'",
    ] {
        sqlx::query(sql).execute(&mut *tx).await.map_err(storage)?;
    }
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!(
            "learning/content/request/{}:{}",
            actor.actor_id, request
        ))
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    Ok(tx)
}
pub(crate) async fn begin_read(pool: &PgPool) -> Result<Transaction<'_, Postgres>, ContentError> {
    let mut tx = pool.begin().await.map_err(storage)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    sqlx::query("SET LOCAL statement_timeout='15s'")
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    Ok(tx)
}
pub(crate) async fn check(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    request: Uuid,
    digest: &str,
    operation: &str,
) -> Result<bool, ContentError> {
    let old:Option<(String,String)>=sqlx::query_as("SELECT request_sha256,operation FROM public.request_key WHERE actor_id=$1 AND request_id=$2").bind(actor.actor_id).bind(request).fetch_optional(&mut **tx).await.map_err(storage)?;
    match old {
        None => Ok(false),
        Some((hash, op)) if hash == digest && op == operation => Ok(true),
        Some(_) => Err(ContentError::IdempotencyConflict),
    }
}
pub(crate) async fn register(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    request: Uuid,
    digest: &str,
    operation: &str,
) -> Result<(), ContentError> {
    sqlx::query("INSERT INTO public.request_key(actor_id,request_id,request_sha256,operation) VALUES($1,$2,$3,$4)").bind(actor.actor_id).bind(request).bind(digest).bind(operation).execute(&mut **tx).await.map_err(storage)?;
    Ok(())
}

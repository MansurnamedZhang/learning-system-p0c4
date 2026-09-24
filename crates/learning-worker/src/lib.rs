//! Worker startup identity validation, shared by the binary and real-PG tests.
use sqlx::PgPool;

pub async fn ensure_restricted_runtime(pool: &PgPool) -> Result<(), String> {
    let role: (String, String, Option<String>, bool, bool) = sqlx::query_as(
        "SELECT session_user::text,current_user::text,system_user,r.rolsuper,r.rolbypassrls \
         FROM pg_catalog.pg_roles r WHERE r.rolname=session_user",
    )
    .fetch_one(pool)
    .await
    .map_err(|_| "cannot inspect runtime role")?;
    let original_login_is_runtime = role
        .2
        .as_deref()
        .and_then(|identity| identity.split_once(':'))
        .is_some_and(|(method, login)| !method.is_empty() && login == "learning_runtime");
    if role.0 != "learning_runtime"
        || role.1 != "learning_runtime"
        || !original_login_is_runtime
        || role.3
        || role.4
    {
        return Err("worker requires the restricted learning_runtime role".into());
    }
    Ok(())
}

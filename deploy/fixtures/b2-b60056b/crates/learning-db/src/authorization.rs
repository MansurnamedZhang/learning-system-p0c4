use crate::storage;
use learning_core::{ContentError, Principal};
use sqlx::{Postgres, Transaction};
use std::collections::BTreeMap;
use uuid::Uuid;
pub(crate) async fn lock_grants(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    required: &[(Uuid, bool)],
) -> Result<(), ContentError> {
    let mut sorted = BTreeMap::new();
    for (space, write) in required {
        let entry = sorted.entry(*space).or_insert(false);
        *entry |= *write;
    }
    for (space, write) in sorted {
        let grant: Option<bool> = sqlx::query_scalar("SELECT public.lock_space_grant($1,$2)")
            .bind(actor.actor_id)
            .bind(space)
            .fetch_one(&mut **tx)
            .await
            .map_err(storage)?;
        if grant.is_none() || (write && grant != Some(true)) {
            return Err(ContentError::NotFound);
        }
    }
    Ok(())
}

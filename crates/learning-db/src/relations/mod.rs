mod read;
mod review;
mod write;

use crate::{authorization, references, storage};
use learning_core::*;
use sqlx::{PgPool, Postgres, Transaction};
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Clone)]
pub struct RelationStore {
    pool: PgPool,
}
impl RelationStore {
    /// The pool authenticates as the non-owner runtime role.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

pub(crate) fn scope(scope: &RelationScope) -> (Uuid, Option<Uuid>) {
    match *scope {
        RelationScope::Space { space_id } => (space_id, None),
        RelationScope::PersonalOverlay {
            space_id,
            overlay_id,
        } => (space_id, Some(overlay_id)),
    }
}
async fn check_scope(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    value: &RelationScope,
) -> Result<(), ContentError> {
    let (space, overlay) = scope(value);
    if let Some(overlay) = overlay {
        let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.overlay WHERE id=$1 AND space_id=$2 AND owner_id=$3)").bind(overlay).bind(space).bind(actor.actor_id).fetch_one(&mut **tx).await.map_err(storage)?;
        if !exists {
            return Err(ContentError::NotFound);
        }
    }
    Ok(())
}
/// Discover before locking and resolve again after sorted grant locks. The
/// returned set is also checked after identity locks, never extended backwards.
pub(crate) async fn lock_dependencies(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    value: &RelationScope,
    roots: &[ExactRef],
) -> Result<BTreeSet<Uuid>, ContentError> {
    check_scope(tx, actor, value).await?;
    let discovered = references::load(tx, actor, roots).await?;
    let mut spaces = discovered.spaces();
    spaces.push((scope(value).0, true));
    authorization::lock_grants(tx, actor, &spaces).await?;
    check_scope(tx, actor, value).await?;
    let allowed = spaces.into_iter().map(|(s, _)| s).collect();
    check_dependencies(tx, actor, roots, &allowed).await?;
    Ok(allowed)
}
pub(crate) async fn check_dependencies(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    roots: &[ExactRef],
    allowed: &BTreeSet<Uuid>,
) -> Result<references::AuthorizedClosure, ContentError> {
    let checked = references::load(tx, actor, roots).await?;
    if checked.spaces().iter().any(|(s, _)| !allowed.contains(s)) {
        return Err(ContentError::Invalid(
            "reference_authorization_changed".into(),
        ));
    }
    Ok(checked)
}
pub(crate) fn enum_text(value: serde_json::Value) -> Result<String, ContentError> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or(ContentError::Storage)
}

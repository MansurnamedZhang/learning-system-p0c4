//! Private, exact-reference authorization. No raw registry fact leaves this module.
mod closure;
mod index;
mod projection;
pub(crate) use closure::{AuthorizedClosure, Session, load};
pub(crate) use index::insert;
pub(crate) use projection::project;

/// Distinguish a hidden root from an authorized root with a hidden dependency.
/// No payload, dependency identity, or size is returned to the caller.
pub(crate) async fn directly_visible(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: learning_core::Principal,
    root: &learning_core::ExactRef,
) -> Result<bool, learning_core::ContentError> {
    Ok(index::fact(tx, actor, root).await?.is_some())
}

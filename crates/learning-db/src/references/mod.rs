//! Private, exact-reference authorization. No raw registry fact leaves this module.
mod closure;
mod index;
mod projection;
pub(crate) use closure::{AuthorizedClosure, Session, load};
pub(crate) use index::insert;
pub(crate) use projection::project;

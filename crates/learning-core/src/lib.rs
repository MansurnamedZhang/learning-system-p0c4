//! Versioned text content contracts. Authentication belongs to a trusted caller.
mod composition;
mod content;
mod digest;
mod error;
mod identity;
mod release;
pub use composition::*;
pub use content::*;
pub use digest::{canonical_content, canonical_json, content_digest, hex_digest};
pub use error::ContentError;
pub use identity::Principal;
pub use release::*;

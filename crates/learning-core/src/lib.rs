//! Versioned text content contracts. Authentication belongs to a trusted caller.
mod content;
mod digest;
mod error;
mod identity;
pub use content::*;
pub use digest::{canonical_content, canonical_json, content_digest, hex_digest};
pub use error::ContentError;
pub use identity::Principal;

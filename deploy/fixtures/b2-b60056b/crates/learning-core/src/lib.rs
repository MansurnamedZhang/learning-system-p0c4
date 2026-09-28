//! Versioned text content contracts. Authentication belongs to a trusted caller.
mod composition;
mod content;
mod digest;
mod error;
mod identity;
mod overlay;
mod reading;
mod release;
pub use reading::*;
mod placement_migration;
pub use composition::*;
pub use content::*;
pub use digest::{canonical_content, canonical_json, content_digest, hex_digest};
pub use error::ContentError;
pub use identity::Principal;
pub use overlay::*;
pub use placement_migration::*;
pub use release::*;

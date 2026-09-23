//! Exact logical asset identities and their authorized use locations.
use crate::{BlockRef, ContentError};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetRef {
    pub space_id: Uuid,
    pub asset_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceVersionRef {
    pub space_id: Uuid,
    pub resource_id: Uuid,
    pub version_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssetUseRef {
    Block(BlockRef),
    Resource(ResourceVersionRef),
}

pub(crate) fn validate_figure(
    usage: &str,
    caption: &str,
    alt: &str,
    decorative: bool,
) -> Result<(), ContentError> {
    if usage.trim().is_empty() || usage.chars().count() > 100 || usage.contains('\0') {
        return Err(ContentError::Invalid("invalid_figure_usage".into()));
    }
    if caption.chars().count() > 1_000 || caption.contains('\0') {
        return Err(ContentError::Invalid("invalid_figure_caption".into()));
    }
    if alt.chars().count() > 500 || alt.contains('\0') {
        return Err(ContentError::Invalid("invalid_figure_alt".into()));
    }
    if (decorative && !alt.is_empty()) || (!decorative && alt.trim().is_empty()) {
        return Err(ContentError::Invalid(
            "figure_alt_or_decoration_required".into(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_attachment_name(display_name: &str) -> Result<(), ContentError> {
    if display_name.trim().is_empty()
        || display_name.chars().count() > 255
        || display_name.contains('\0')
    {
        return Err(ContentError::Invalid(
            "invalid_attachment_display_name".into(),
        ));
    }
    Ok(())
}

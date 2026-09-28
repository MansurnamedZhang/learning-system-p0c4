use crate::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadingRef {
    pub view_id: Uuid,
    pub revision_id: Uuid,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadingSaved {
    pub overlay: OverlayRef,
    pub view: ReadingRef,
    pub changed_blocks: Vec<BlockRef>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadingMode {
    Original,
    Fused,
    Personal,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum GroupLocation {
    Placed { anchor: GapAnchor },
    Unplaced { origin_anchor: GapAnchor },
}
impl GroupLocation {
    pub fn anchor(&self) -> &GapAnchor {
        match self {
            Self::Placed { anchor } => anchor,
            Self::Unplaced { origin_anchor } => origin_anchor,
        }
    }
    pub fn placed(&self) -> bool {
        matches!(self, Self::Placed { .. })
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditableGroup {
    pub group_id: Uuid,
    pub location: GroupLocation,
    pub placements: Vec<Placement>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditableReading {
    pub base: CompositionRef,
    pub title: String,
    pub groups: Vec<EditableGroup>,
}
impl EditableReading {
    pub fn digest(&self) -> String {
        let mut c = self.clone();
        c.groups.sort_by_key(|g| g.group_id);
        hex_digest(
            canonical_json(&serde_json::json!({"domain":"overlay-v1","reading":c})).as_bytes(),
        )
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct ReadingState {
    pub overlay: OverlayRef,
    pub view: ReadingRef,
    pub editable: Option<EditableReading>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SourceProjection {
    Available { base: CompositionRef },
    Unavailable,
}
#[derive(Debug, Clone, Serialize)]
pub struct PersonalItem {
    pub placement_id: Uuid,
    pub revision: Revision,
    pub location: Option<GapAnchor>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ReadingItem {
    SectionStart { path: Vec<Uuid>, title: String },
    SectionEnd { path: Vec<Uuid> },
    Original { path: Vec<Uuid>, revision: Revision },
    Personal { item: PersonalItem },
}
#[derive(Debug, Clone, Serialize)]
pub struct ReadingProjection {
    pub overlay: OverlayRef,
    pub view: ReadingRef,
    pub source: SourceProjection,
    pub items: Vec<ReadingItem>,
    pub unplaced: Vec<PersonalItem>,
}

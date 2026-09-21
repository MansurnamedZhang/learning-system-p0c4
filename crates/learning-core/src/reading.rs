use crate::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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
pub struct PersonalItemData<R> {
    pub placement_id: Uuid,
    pub revision: R,
    pub location: Option<GapAnchor>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ReadingItemData<R> {
    SectionStart { path: Vec<Uuid>, title: String },
    SectionEnd { path: Vec<Uuid> },
    Original { path: Vec<Uuid>, revision: R },
    Personal { item: PersonalItemData<R> },
}
#[derive(Debug, Clone, Serialize)]
pub struct ReadingProjectionData<R> {
    pub overlay: OverlayRef,
    pub view: ReadingRef,
    pub source: SourceProjection,
    pub items: Vec<ReadingItemData<R>>,
    pub unplaced: Vec<PersonalItemData<R>>,
}

pub type PersonalItem = PersonalItemData<Revision>;
pub type VersionedPersonalItem = PersonalItemData<ContentRevision>;
pub type ReadingItem = ReadingItemData<Revision>;
pub type VersionedReadingItem = ReadingItemData<ContentRevision>;
pub type ReadingProjection = ReadingProjectionData<Revision>;
#[derive(Debug, Clone, Serialize)]
pub struct VersionedReadingProjection {
    pub contract_version: u32,
    pub overlay: OverlayRef,
    pub view: ReadingRef,
    pub source: SourceProjection,
    pub items: Vec<VersionedReadingItem>,
    pub unplaced: Vec<VersionedPersonalItem>,
    pub evidence: ReadingEvidence,
}
impl TryFrom<VersionedPersonalItem> for PersonalItem {
    type Error = ContentError;
    fn try_from(p: VersionedPersonalItem) -> Result<Self, Self::Error> {
        Ok(Self {
            placement_id: p.placement_id,
            revision: p.revision.try_into()?,
            location: p.location,
        })
    }
}
impl TryFrom<VersionedReadingProjection> for ReadingProjection {
    type Error = ContentError;
    fn try_from(p: VersionedReadingProjection) -> Result<Self, Self::Error> {
        if p.contract_version != 1 {
            return Err(ContentError::Invalid("unsupported_content_version".into()));
        }
        let items = p
            .items
            .into_iter()
            .map(|item| {
                Ok(match item {
                    VersionedReadingItem::SectionStart { path, title } => {
                        ReadingItem::SectionStart { path, title }
                    }
                    VersionedReadingItem::SectionEnd { path } => ReadingItem::SectionEnd { path },
                    VersionedReadingItem::Original { path, revision } => ReadingItem::Original {
                        path,
                        revision: revision.try_into()?,
                    },
                    VersionedReadingItem::Personal { item } => ReadingItem::Personal {
                        item: item.try_into()?,
                    },
                })
            })
            .collect::<Result<_, ContentError>>()?;
        Ok(Self {
            overlay: p.overlay,
            view: p.view,
            source: p.source,
            items,
            unplaced: p
                .unplaced
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()?,
        })
    }
}

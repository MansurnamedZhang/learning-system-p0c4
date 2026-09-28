use crate::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayRef {
    pub overlay_id: Uuid,
    pub revision_id: Uuid,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Affinity {
    AfterLeft,
    BeforeRight,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GapAnchor {
    pub base: CompositionRef,
    pub parent_occurrence_path: Vec<Uuid>,
    pub left_occurrence_id: Option<Uuid>,
    pub right_occurrence_id: Option<Uuid>,
    pub affinity: Affinity,
}
impl GapAnchor {
    pub fn validate(&self) -> Result<(), ContentError> {
        if self.parent_occurrence_path.len() > 15
            || (self.left_occurrence_id.is_some()
                && self.left_occurrence_id == self.right_occurrence_id)
        {
            return invalid_reading("invalid_anchor");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    pub placement_id: Uuid,
    pub block: BlockRef,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlacementOrder {
    pub left_placement_id: Option<Uuid>,
    pub right_placement_id: Option<Uuid>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum InsertTarget {
    ExistingGroup {
        group_id: Uuid,
        order: PlacementOrder,
    },
    NewGroup {
        anchor: GapAnchor,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextChange {
    pub block_id: Uuid,
    pub base_revision_id: Uuid,
    pub draft: TextDraft,
    pub selected_placements: Vec<Uuid>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReadingEdit {
    InsertExisting {
        block: BlockRef,
        target: InsertTarget,
    },
    InsertNew {
        drafts: Vec<TextDraft>,
        target: InsertTarget,
    },
    ReviseSelected {
        changes: Vec<TextChange>,
    },
    AdoptExisting {
        block: BlockRef,
        selected_placements: Vec<Uuid>,
    },
    Move {
        placement_id: Uuid,
        target: InsertTarget,
    },
    Remove {
        placement_id: Uuid,
    },
    PlaceUnplaced {
        group_id: Uuid,
        anchor: GapAnchor,
        merge_into: Option<Uuid>,
        merged_order: Vec<Uuid>,
    },
}
#[derive(Debug, Clone, Serialize)]
pub struct CreateReading {
    pub request_id: Uuid,
    pub base: CompositionRef,
    pub title: String,
    pub reason: String,
}
impl CreateReading {
    pub fn validate(&self) -> Result<(), ContentError> {
        if self.title.chars().count() > 300 || self.title.contains('\0') {
            return invalid_reading("invalid_title");
        }
        validate_reason(&self.reason)
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct EditReading {
    pub request_id: Uuid,
    pub expected_overlay_revision: Uuid,
    pub expected_reading_view_revision: Uuid,
    pub edit: ReadingEdit,
    pub reason: String,
}
impl EditReading {
    pub fn validate(&self) -> Result<(), ContentError> {
        validate_reason(&self.reason)?;
        match &self.edit {
            ReadingEdit::InsertExisting { target, .. } | ReadingEdit::Move { target, .. } => {
                target.validate()
            }
            ReadingEdit::InsertNew { drafts, target } => {
                if drafts.is_empty() || drafts.len() > 32 {
                    return invalid_reading("body_batch_limit");
                }
                for d in drafts {
                    d.validate()?;
                }
                target.validate()
            }
            ReadingEdit::ReviseSelected { changes } => {
                if changes.is_empty() || changes.len() > 32 {
                    return invalid_reading("body_batch_limit");
                }
                unique_ids(&changes.iter().map(|c| c.block_id).collect::<Vec<_>>(), 32)?;
                let mut all = Vec::new();
                for c in changes {
                    c.draft.validate()?;
                    unique_ids(&c.selected_placements, 2048)?;
                    all.extend(&c.selected_placements);
                }
                unique_ids(&all, 2048)
            }
            ReadingEdit::AdoptExisting {
                selected_placements,
                ..
            } => unique_ids(selected_placements, 2048),
            ReadingEdit::PlaceUnplaced {
                group_id,
                anchor,
                merge_into,
                merged_order,
            } => {
                anchor.validate()?;
                match merge_into {
                    None if !merged_order.is_empty() => {
                        invalid_reading("merge_order_without_target")
                    }
                    Some(id) if id == group_id => invalid_reading("self_merge"),
                    Some(_) => unique_ids(merged_order, 2048),
                    None => Ok(()),
                }
            }
            ReadingEdit::Remove { .. } => Ok(()),
        }
    }
    pub fn digest(&self, actor: Principal, target: Uuid) -> String {
        let mut c = self.clone();
        match &mut c.edit {
            ReadingEdit::AdoptExisting {
                selected_placements,
                ..
            } => selected_placements.sort(),
            ReadingEdit::ReviseSelected { changes } => {
                changes.sort_by_key(|c| c.block_id);
                for c in changes {
                    c.selected_placements.sort();
                }
            }
            _ => {}
        }
        reading_request_digest("reading_edit", actor, target, &c)
    }
}
impl InsertTarget {
    pub fn validate(&self) -> Result<(), ContentError> {
        match self {
            Self::NewGroup { anchor } => anchor.validate(),
            Self::ExistingGroup { order, .. } => {
                if order.left_placement_id.is_some()
                    && order.left_placement_id == order.right_placement_id
                {
                    return invalid_reading("invalid_neighbors");
                }
                Ok(())
            }
        }
    }
}
pub(crate) fn invalid_reading<T>(code: &str) -> Result<T, ContentError> {
    Err(ContentError::Invalid(code.into()))
}
pub(crate) fn validate_reason(s: &str) -> Result<(), ContentError> {
    if s.is_empty() || s.chars().count() > 1000 || s.contains('\0') {
        invalid_reading("invalid_reason")
    } else {
        Ok(())
    }
}
pub(crate) fn unique_ids(ids: &[Uuid], max: usize) -> Result<(), ContentError> {
    if ids.is_empty()
        || ids.len() > max
        || ids.iter().collect::<std::collections::BTreeSet<_>>().len() != ids.len()
    {
        invalid_reading("invalid_selection")
    } else {
        Ok(())
    }
}
pub fn reading_request_digest<T: Serialize>(
    operation: &str,
    actor: Principal,
    target: Uuid,
    command: &T,
) -> String {
    hex_digest(canonical_json(&serde_json::json!({"domain":"personal-reading-request-v1","operation":operation,"actor":actor.actor_id,"target":target,"command":command})).as_bytes())
}
macro_rules! checked_deserialize {
($name:ident { $($field:ident : $ty:ty),* $(,)? }) => {
impl<'de> serde::Deserialize<'de> for $name {
fn deserialize<D:serde::Deserializer<'de>>(deserializer:D)->Result<Self,D::Error>{
#[derive(serde::Deserialize)] #[serde(deny_unknown_fields)] struct Raw {$($field:$ty),*}
let raw=Raw::deserialize(deserializer)?;let value=Self{$($field:raw.$field),*};value.validate().map_err(serde::de::Error::custom)?;Ok(value)
}}};}
pub(crate) use checked_deserialize;
checked_deserialize!(CreateReading {
    request_id: Uuid,
    base: CompositionRef,
    title: String,
    reason: String
});
checked_deserialize!(EditReading {
    request_id: Uuid,
    expected_overlay_revision: Uuid,
    expected_reading_view_revision: Uuid,
    edit: ReadingEdit,
    reason: String
});

use super::anchor::{insertion_index, invalid};
use learning_core::*;
use uuid::Uuid;

pub(crate) fn insert(
    data: &mut EditableReading,
    target: &InsertTarget,
    placements: Vec<Placement>,
) -> Result<(), ContentError> {
    match target {
        InsertTarget::NewGroup { anchor } => {
            if data
                .groups
                .iter()
                .any(|g| g.location.placed() && g.location.anchor() == anchor)
            {
                return invalid("duplicate_gap");
            }
            data.groups.push(EditableGroup {
                group_id: Uuid::new_v4(),
                location: GroupLocation::Placed {
                    anchor: anchor.clone(),
                },
                placements,
            });
        }
        InsertTarget::ExistingGroup { group_id, order } => {
            let g = data
                .groups
                .iter_mut()
                .find(|g| g.group_id == *group_id)
                .ok_or_else(|| ContentError::Invalid("unknown_group".into()))?;
            if !g.location.placed() {
                return invalid("group_unplaced");
            }
            let i = insertion_index(
                &g.placements
                    .iter()
                    .map(|p| p.placement_id)
                    .collect::<Vec<_>>(),
                order,
            )?;
            g.placements.splice(i..i, placements);
        }
    }
    Ok(())
}
pub(crate) fn select(
    data: &mut EditableReading,
    ids: &[Uuid],
    block: &BlockRef,
) -> Result<(), ContentError> {
    for id in ids {
        let p = data
            .groups
            .iter_mut()
            .flat_map(|g| &mut g.placements)
            .find(|p| p.placement_id == *id)
            .ok_or_else(|| ContentError::Invalid("unknown_placement".into()))?;
        if p.block.block_id != block.block_id {
            return invalid("placement_target_identity");
        }
        p.block = block.clone();
    }
    Ok(())
}
pub(crate) fn ordered_merge(
    groups: &[EditableGroup],
    order: &[Uuid],
) -> Result<Vec<Placement>, ContentError> {
    let all: Vec<_> = groups
        .iter()
        .flat_map(|g| g.placements.iter().cloned())
        .collect();
    if all.len() != order.len()
        || order
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != order.len()
    {
        return invalid("merge_order");
    }
    let mut result = vec![];
    for id in order {
        result.push(
            all.iter()
                .find(|p| p.placement_id == *id)
                .ok_or_else(|| ContentError::Invalid("merge_order".into()))?
                .clone(),
        );
    }
    for g in groups {
        let filtered: Vec<_> = order
            .iter()
            .filter(|id| g.placements.iter().any(|p| &p.placement_id == *id))
            .copied()
            .collect();
        if filtered
            != g.placements
                .iter()
                .map(|p| p.placement_id)
                .collect::<Vec<_>>()
        {
            return invalid("merge_relative_order");
        }
    }
    Ok(result)
}
pub(crate) fn structure(
    data: &mut EditableReading,
    edit: &ReadingEdit,
) -> Result<Vec<(Uuid, Uuid)>, ContentError> {
    match edit {
        ReadingEdit::Move { placement_id, .. } | ReadingEdit::Remove { placement_id } => {
            let index = data
                .groups
                .iter()
                .position(|g| g.placements.iter().any(|p| p.placement_id == *placement_id))
                .ok_or_else(|| ContentError::Invalid("unknown_placement".into()))?;
            if matches!(edit, ReadingEdit::Move { .. }) && !data.groups[index].location.placed() {
                return invalid("use_place_unplaced");
            }
            let pos = data.groups[index]
                .placements
                .iter()
                .position(|p| p.placement_id == *placement_id)
                .ok_or(ContentError::Storage)?;
            let p = data.groups[index].placements.remove(pos);
            if let ReadingEdit::Move { target, .. } = edit {
                insert(data, target, vec![p])?;
            }
            data.groups.retain(|g| !g.placements.is_empty());
        }
        ReadingEdit::PlaceUnplaced {
            group_id,
            anchor,
            merge_into,
            merged_order,
        } => {
            let i = data
                .groups
                .iter()
                .position(|g| g.group_id == *group_id)
                .ok_or_else(|| ContentError::Invalid("unknown_group".into()))?;
            if data.groups[i].location.placed() {
                return invalid("group_already_placed");
            }
            let mut g = data.groups.remove(i);
            let result = if let Some(target) = merge_into {
                let j = data
                    .groups
                    .iter()
                    .position(|g| g.group_id == *target)
                    .ok_or_else(|| ContentError::Invalid("unknown_group".into()))?;
                if !data.groups[j].location.placed() || data.groups[j].location.anchor() != anchor {
                    return invalid("merge_anchor");
                }
                let other = data.groups.remove(j);
                g.placements = ordered_merge(&[other, g.clone()], merged_order)?;
                g.group_id = Uuid::new_v4();
                g.group_id
            } else {
                g.group_id
            };
            g.location = GroupLocation::Placed {
                anchor: anchor.clone(),
            };
            data.groups.push(g);
            let mut mappings = vec![(*group_id, result)];
            if let Some(target) = merge_into {
                mappings.push((*target, result));
            }
            return Ok(mappings);
        }
        _ => return invalid("unsupported_structure_edit"),
    };
    Ok(vec![])
}

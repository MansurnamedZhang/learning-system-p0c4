use crate::overlay::model::{Access, Layer};
use learning_core::*;
use uuid::Uuid;

pub(crate) fn project(
    layer: &Layer,
    access: &Access,
    mode: ReadingMode,
) -> Result<ReadingProjection, ContentError> {
    let mut result = ReadingProjection {
        overlay: OverlayRef {
            overlay_id: layer.id,
            revision_id: layer.revision,
        },
        view: layer.view.clone(),
        source: match &access.source {
            Some(_) => SourceProjection::Available {
                base: layer.data.base.clone(),
            },
            None => SourceProjection::Unavailable,
        },
        items: vec![],
        unplaced: vec![],
    };
    if let Some(source) = &access.source {
        walk(
            layer,
            access,
            source,
            &source.root,
            &[],
            mode,
            &mut result.items,
        )?;
    }
    if mode != ReadingMode::Original {
        let mut detached = vec![];
        for group in &layer.data.groups {
            if access.source.is_some() && group.location.placed() {
                continue;
            }
            for p in &group.placements {
                if let Some(item) = item(p, group, access) {
                    detached.push(item);
                }
            }
        }
        // No ordering from a hidden source is exposed.
        detached.sort_by_key(|p| p.placement_id);
        result.unplaced = detached;
    }
    Ok(result)
}
fn item(p: &Placement, group: &EditableGroup, access: &Access) -> Option<PersonalItem> {
    let (_, revision) = access.blocks.get(&p.block)?;
    Some(PersonalItem {
        placement_id: p.placement_id,
        revision: revision.clone(),
        location: (access.source.is_some()
            && access.origins.contains_key(&group.location.anchor().base))
        .then(|| group.location.anchor().clone()),
    })
}
fn walk(
    layer: &Layer,
    access: &Access,
    source: &CompositionSnapshot,
    reference: &CompositionRef,
    path: &[Uuid],
    mode: ReadingMode,
    out: &mut Vec<ReadingItem>,
) -> Result<(), ContentError> {
    let composition = source
        .compositions
        .iter()
        .find(|c| &c.reference == reference)
        .ok_or(ContentError::Storage)?;
    for index in 0..=composition.nodes.len() {
        if mode != ReadingMode::Original {
            let mut groups: Vec<_> = layer
                .data
                .groups
                .iter()
                .filter(|g| {
                    let a = g.location.anchor();
                    g.location.placed()
                        && a.parent_occurrence_path == path
                        && a.left_occurrence_id
                            == index
                                .checked_sub(1)
                                .map(|i| composition.nodes[i].occurrence_id)
                        && a.right_occurrence_id
                            == composition.nodes.get(index).map(|n| n.occurrence_id)
                })
                .collect();
            groups.sort_by_key(|g| (g.location.anchor().affinity, g.group_id));
            for group in groups {
                for p in &group.placements {
                    if let Some(item) = item(p, group, access) {
                        out.push(ReadingItem::Personal { item });
                    }
                }
            }
        }
        let Some(node) = composition.nodes.get(index) else {
            continue;
        };
        let mut next = path.to_vec();
        next.push(node.occurrence_id);
        match &node.target {
            NodeTarget::Block(reference) if mode != ReadingMode::Personal => {
                let revision = source
                    .blocks
                    .iter()
                    .find(|b| {
                        b.block_id == reference.block_id && b.revision_id == reference.revision_id
                    })
                    .ok_or(ContentError::Storage)?;
                out.push(ReadingItem::Original {
                    path: next,
                    revision: revision.clone(),
                });
            }
            NodeTarget::Composition(reference) => {
                if mode != ReadingMode::Personal {
                    let title = source
                        .compositions
                        .iter()
                        .find(|c| &c.reference == reference)
                        .ok_or(ContentError::Storage)?
                        .title
                        .clone();
                    out.push(ReadingItem::SectionStart {
                        path: next.clone(),
                        title,
                    });
                }
                walk(layer, access, source, reference, &next, mode, out)?;
                if mode != ReadingMode::Personal {
                    out.push(ReadingItem::SectionEnd { path: next });
                }
            }
            _ => {}
        }
    }
    Ok(())
}

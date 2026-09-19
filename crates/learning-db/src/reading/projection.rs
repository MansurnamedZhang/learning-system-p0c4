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
        let mut groups: Vec<_> = layer.data.groups.iter().collect();
        groups.sort_by_key(|g| g.group_id);
        for group in groups {
            if access.source.is_some() && group.location.placed() {
                continue;
            }
            for p in &group.placements {
                if let Some(item) = item(p, group, access) {
                    detached.push(item);
                }
            }
        }
        // Group identity gives a stable order independent of the hidden source;
        // visible placements retain their deliberately saved order within each group.
        result.unplaced = detached;
    }
    Ok(result)
}
fn item(p: &Placement, group: &EditableGroup, access: &Access) -> Option<PersonalItem> {
    let (_, revision) = access.blocks.get(&p.block)?;
    Some(PersonalItem {
        placement_id: p.placement_id,
        revision: revision.clone(),
        location: (group.location.placed()
            && access.source.is_some()
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    #[test]
    fn explicitly_unplaced_item_has_no_active_location() {
        let id = Uuid::from_u128(1);
        let base = CompositionRef {
            composition_id: id,
            revision_id: id,
        };
        let reference = BlockRef {
            block_id: id,
            revision_id: id,
        };
        let draft:TextDraft=serde_json::from_value(serde_json::json!({"kind":"text","intent":"note","language":"en","title":"","payload":{"format":"markdown","text":"N"}})).unwrap();
        let revision = Revision {
            block_id: id,
            revision_id: id,
            parent_revision_id: None,
            content_sha256: draft.digest(),
            draft,
            author_id: id,
            reason: "test".into(),
            created_at: chrono::Utc::now(),
        };
        let snapshot = CompositionSnapshot {
            root: base.clone(),
            blocks: vec![],
            compositions: vec![],
        };
        let access = Access {
            source: Some(snapshot.clone()),
            blocks: BTreeMap::from([(reference.clone(), (id, revision))]),
            origins: BTreeMap::from([(base.clone(), snapshot)]),
            spaces: vec![],
        };
        let placement = Placement {
            placement_id: id,
            block: reference,
        };
        let group = EditableGroup {
            group_id: id,
            location: GroupLocation::Unplaced {
                origin_anchor: GapAnchor {
                    base,
                    parent_occurrence_path: vec![],
                    left_occurrence_id: None,
                    right_occurrence_id: None,
                    affinity: Affinity::AfterLeft,
                },
            },
            placements: vec![placement.clone()],
        };
        assert!(
            item(&placement, &group, &access)
                .unwrap()
                .location
                .is_none()
        );
    }
    #[test]
    fn unplaced_projection_preserves_visible_saved_order() {
        let id = Uuid::from_u128(99);
        let base = CompositionRef {
            composition_id: id,
            revision_id: id,
        };
        let anchor = GapAnchor {
            base: base.clone(),
            parent_occurrence_path: vec![],
            left_occurrence_id: None,
            right_occurrence_id: None,
            affinity: Affinity::AfterLeft,
        };
        let snapshot = CompositionSnapshot {
            root: base.clone(),
            compositions: vec![CompositionRevision {
                reference: base.clone(),
                parent_revision_id: None,
                kind: CompositionKind::Document,
                title: "".into(),
                nodes: vec![],
                content_sha256: "0".repeat(64),
                author_id: id,
                reason: "test".into(),
                created_at: chrono::Utc::now(),
            }],
            blocks: vec![],
        };
        let mut blocks = BTreeMap::new();
        let placements=[(3,"first"),(1,"second"),(2,"third")].into_iter().map(|(n,text)|{let reference=BlockRef{block_id:Uuid::from_u128(n),revision_id:Uuid::from_u128(n)};let draft:TextDraft=serde_json::from_value(serde_json::json!({"kind":"text","intent":"note","language":"en","title":"","payload":{"format":"markdown","text":text}})).unwrap();blocks.insert(reference.clone(),(id,Revision{block_id:reference.block_id,revision_id:reference.revision_id,parent_revision_id:None,content_sha256:draft.digest(),draft,author_id:id,reason:"test".into(),created_at:chrono::Utc::now()}));Placement{placement_id:Uuid::from_u128(n),block:reference}}).collect();
        let layer = Layer {
            id,
            space: id,
            revision: id,
            view: ReadingRef {
                view_id: id,
                revision_id: id,
            },
            data: EditableReading {
                base: base.clone(),
                title: "".into(),
                groups: vec![EditableGroup {
                    group_id: id,
                    location: GroupLocation::Unplaced {
                        origin_anchor: anchor,
                    },
                    placements,
                }],
            },
        };
        for source in [Some(snapshot.clone()), None] {
            let mut access = Access {
                source,
                blocks: blocks.clone(),
                origins: BTreeMap::from([(base.clone(), snapshot.clone())]),
                spaces: vec![],
            };
            for mode in [ReadingMode::Fused, ReadingMode::Personal] {
                let result = project(&layer, &access, mode).unwrap();
                assert_eq!(
                    result
                        .unplaced
                        .iter()
                        .map(|p| p.revision.draft.payload.text.as_str())
                        .collect::<Vec<_>>(),
                    ["first", "second", "third"]
                );
            }
            access.blocks.remove(&BlockRef {
                block_id: Uuid::from_u128(1),
                revision_id: Uuid::from_u128(1),
            });
            let result = project(&layer, &access, ReadingMode::Fused).unwrap();
            assert_eq!(
                result
                    .unplaced
                    .iter()
                    .map(|p| p.revision.draft.payload.text.as_str())
                    .collect::<Vec<_>>(),
                ["first", "third"]
            );
        }
    }
}

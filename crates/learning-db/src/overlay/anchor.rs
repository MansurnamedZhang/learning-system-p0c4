use learning_core::*;
use uuid::Uuid;
pub(crate) fn invalid<T>(code: &str) -> Result<T, ContentError> {
    Err(ContentError::Invalid(code.into()))
}
pub(crate) fn insertion_index(ids: &[Uuid], order: &PlacementOrder) -> Result<usize, ContentError> {
    let index = match order.left_placement_id {
        None => 0,
        Some(left) => {
            ids.iter()
                .position(|id| *id == left)
                .ok_or_else(|| ContentError::Invalid("invalid_neighbors".into()))?
                + 1
        }
    };
    if ids.get(index).copied() != order.right_placement_id {
        return invalid("invalid_neighbors");
    }
    Ok(index)
}
pub(crate) fn parent<'a>(
    snapshot: &'a VersionedCompositionSnapshot,
    path: &[Uuid],
) -> Result<&'a CompositionRevision, ContentError> {
    let mut reference = &snapshot.root;
    for id in path {
        let node = snapshot
            .compositions
            .iter()
            .find(|c| &c.reference == reference)
            .and_then(|c| c.nodes.iter().find(|n| &n.occurrence_id == id))
            .ok_or_else(|| ContentError::Invalid("invalid_anchor".into()))?;
        match &node.target {
            NodeTarget::Composition(r) => reference = r,
            _ => return invalid("invalid_anchor"),
        }
    }
    snapshot
        .compositions
        .iter()
        .find(|c| &c.reference == reference)
        .ok_or(ContentError::Storage)
}
pub(crate) fn validate_gap(
    snapshot: &VersionedCompositionSnapshot,
    anchor: &GapAnchor,
) -> Result<(), ContentError> {
    anchor.validate()?;
    if snapshot.root != anchor.base {
        return invalid("anchor_base");
    }
    let p = parent(snapshot, &anchor.parent_occurrence_path)?;
    insertion_index(
        &p.nodes.iter().map(|n| n.occurrence_id).collect::<Vec<_>>(),
        &PlacementOrder {
            left_placement_id: anchor.left_occurrence_id,
            right_placement_id: anchor.right_occurrence_id,
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }
    fn fixture() -> (VersionedCompositionSnapshot, GapAnchor) {
        let root = CompositionRef {
            composition_id: id(1),
            revision_id: id(2),
        };
        let child = CompositionRef {
            composition_id: id(3),
            revision_id: id(4),
        };
        let revision = |reference: CompositionRef, nodes: Vec<Occurrence>| CompositionRevision {
            reference,
            parent_revision_id: None,
            kind: CompositionKind::Document,
            title: "x".into(),
            nodes,
            content_sha256: "0".repeat(64),
            author_id: id(99),
            reason: "x".into(),
            created_at: chrono::Utc::now(),
        };
        let snapshot = VersionedCompositionSnapshot {
            root: root.clone(),
            compositions: vec![
                revision(
                    root.clone(),
                    vec![
                        Occurrence {
                            occurrence_id: id(5),
                            target: NodeTarget::Composition(child.clone()),
                        },
                        Occurrence {
                            occurrence_id: id(6),
                            target: NodeTarget::Composition(child.clone()),
                        },
                    ],
                ),
                revision(child, vec![]),
            ],
            blocks: vec![],
        };
        let anchor = GapAnchor {
            base: root,
            parent_occurrence_path: vec![id(5)],
            left_occurrence_id: None,
            right_occurrence_id: None,
            affinity: Affinity::AfterLeft,
        };
        (snapshot, anchor)
    }
    #[test]
    fn insertion_requires_actual_neighbors() {
        let ids = vec![id(1), id(2)];
        for (left, right, want) in [
            (None, Some(id(1)), Some(0)),
            (Some(id(1)), Some(id(2)), Some(1)),
            (Some(id(2)), None, Some(2)),
            (None, None, None),
            (Some(id(2)), Some(id(1)), None),
            (None, Some(id(2)), None),
        ] {
            assert_eq!(
                insertion_index(
                    &ids,
                    &PlacementOrder {
                        left_placement_id: left,
                        right_placement_id: right
                    }
                )
                .ok(),
                want
            );
        }
        assert_eq!(
            insertion_index(
                &[],
                &PlacementOrder {
                    left_placement_id: None,
                    right_placement_id: None
                }
            )
            .unwrap(),
            0
        );
    }
    #[test]
    fn gap_resolves_exact_parent_occurrence_path() {
        let (s, mut a) = fixture();
        assert!(validate_gap(&s, &a).is_ok());
        a.parent_occurrence_path = vec![id(6)];
        assert!(validate_gap(&s, &a).is_ok());
        a.parent_occurrence_path = vec![id(7)];
        assert!(validate_gap(&s, &a).is_err());
        a.parent_occurrence_path = vec![];
        assert!(validate_gap(&s, &a).is_err());
        a.left_occurrence_id = Some(id(5));
        a.right_occurrence_id = Some(id(6));
        assert!(validate_gap(&s, &a).is_ok());
        a.base.revision_id = id(9);
        assert!(validate_gap(&s, &a).is_err());
    }
}

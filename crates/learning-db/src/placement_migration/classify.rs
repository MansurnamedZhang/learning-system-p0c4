use learning_core::*;
pub(crate) fn classify(
    old: &CompositionSnapshot,
    target: &CompositionSnapshot,
    group: &EditableGroup,
) -> Result<(MigrationClass, Option<GapAnchor>, String), ContentError> {
    use crate::overlay::anchor::{invalid, parent, validate_gap};
    let unresolved = |reason: &str| Ok((MigrationClass::Unresolved, None, reason.into()));
    if old.root.composition_id != target.root.composition_id {
        return invalid("migration_root");
    }
    if !group.location.placed() {
        return unresolved("already_unplaced");
    }
    let a = group.location.anchor();
    validate_gap(old, a)?;
    let previous = parent(old, &a.parent_occurrence_path)?;
    let next = match parent(target, &a.parent_occurrence_path) {
        Ok(n) => n,
        Err(ContentError::Invalid(_)) => return unresolved("parent_path_missing"),
        Err(e) => return Err(e),
    };
    if previous.reference.composition_id != next.reference.composition_id {
        return unresolved("parent_identity_changed");
    }
    let mut candidate = a.clone();
    candidate.base = target.root.clone();
    if validate_gap(target, &candidate).is_ok() {
        return Ok((
            MigrationClass::Exact,
            Some(candidate),
            "neighbors_unchanged".into(),
        ));
    }
    let left = a
        .left_occurrence_id
        .and_then(|id| next.nodes.iter().position(|n| n.occurrence_id == id));
    let right = a
        .right_occurrence_id
        .and_then(|id| next.nodes.iter().position(|n| n.occurrence_id == id));
    if matches!((left,right),(Some(l),Some(r)) if l>=r) {
        return unresolved("neighbors_reversed");
    }
    let preferred = match a.affinity {
        Affinity::AfterLeft => left.map(|l| l + 1),
        Affinity::BeforeRight => right,
    };
    let fallback = match a.affinity {
        Affinity::AfterLeft => right,
        Affinity::BeforeRight => left.map(|l| l + 1),
    };
    let Some(index) = preferred.or(fallback) else {
        return unresolved("neighbors_missing");
    };
    candidate.left_occurrence_id = index.checked_sub(1).map(|i| next.nodes[i].occurrence_id);
    candidate.right_occurrence_id = next.nodes.get(index).map(|n| n.occurrence_id);
    validate_gap(target, &candidate)?;
    Ok((
        MigrationClass::Candidate,
        Some(candidate),
        if preferred.is_some() {
            "neighbors_changed"
        } else {
            "preferred_side_missing"
        }
        .into(),
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }
    fn snap(rev: u128, nodes: &[u128]) -> CompositionSnapshot {
        let reference = CompositionRef {
            composition_id: id(1),
            revision_id: id(rev),
        };
        CompositionSnapshot {
            root: reference.clone(),
            blocks: vec![],
            compositions: vec![CompositionRevision {
                reference,
                parent_revision_id: None,
                kind: CompositionKind::Document,
                title: "doc".into(),
                nodes: nodes
                    .iter()
                    .map(|n| Occurrence {
                        occurrence_id: id(*n),
                        target: NodeTarget::Block(BlockRef {
                            block_id: id(*n + 100),
                            revision_id: id(*n + 200),
                        }),
                    })
                    .collect(),
                content_sha256: "0".repeat(64),
                author_id: id(99),
                reason: "fixture".into(),
                created_at: chrono::Utc::now(),
            }],
        }
    }
    fn group(
        old: &CompositionSnapshot,
        l: Option<u128>,
        r: Option<u128>,
        affinity: Affinity,
    ) -> EditableGroup {
        EditableGroup {
            group_id: id(9),
            location: GroupLocation::Placed {
                anchor: GapAnchor {
                    base: old.root.clone(),
                    parent_occurrence_path: vec![],
                    left_occurrence_id: l.map(id),
                    right_occurrence_id: r.map(id),
                    affinity,
                },
            },
            placements: vec![],
        }
    }
    #[test]
    fn fixed_neighbors_classify_without_text_similarity() {
        let old = snap(2, &[10, 20]);
        let g = group(&old, Some(10), Some(20), Affinity::AfterLeft);
        for (nodes, class, left, right) in [
            (vec![10, 20], MigrationClass::Exact, Some(10), Some(20)),
            (
                vec![10, 15, 20],
                MigrationClass::Candidate,
                Some(10),
                Some(15),
            ),
            (vec![20], MigrationClass::Candidate, None, Some(20)),
            (vec![10], MigrationClass::Candidate, Some(10), None),
            (vec![20, 10], MigrationClass::Unresolved, None, None),
            (vec![30], MigrationClass::Unresolved, None, None),
        ] {
            let target = snap(3, &nodes);
            let (c, a, _) = classify(&old, &target, &g).unwrap();
            assert_eq!(c, class);
            if class == MigrationClass::Unresolved {
                assert!(a.is_none());
            } else {
                let a = a.unwrap();
                assert_eq!(a.base, target.root);
                assert_eq!(a.left_occurrence_id, left.map(id));
                assert_eq!(a.right_occurrence_id, right.map(id));
            }
        }
    }
    #[test]
    fn empty_boundaries_and_unplaced_do_not_guess() {
        let old = snap(2, &[]);
        let mut g = group(&old, None, None, Affinity::BeforeRight);
        assert_eq!(
            classify(&old, &snap(3, &[]), &g).unwrap().0,
            MigrationClass::Exact
        );
        assert_eq!(
            classify(&old, &snap(3, &[10]), &g).unwrap().0,
            MigrationClass::Unresolved
        );
        g.location = GroupLocation::Unplaced {
            origin_anchor: g.location.anchor().clone(),
        };
        assert_eq!(
            classify(&old, &snap(3, &[]), &g).unwrap().0,
            MigrationClass::Unresolved
        );
        let old = snap(2, &[10, 20]);
        let g = group(&old, None, Some(10), Affinity::BeforeRight);
        let (c, a, _) = classify(&old, &snap(3, &[5, 10, 20]), &g).unwrap();
        assert_eq!(c, MigrationClass::Candidate);
        assert_eq!(a.unwrap().left_occurrence_id, Some(id(5)));
    }
}

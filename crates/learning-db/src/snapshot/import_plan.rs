//! Deterministic target defaults, never reconstruction of source mutable heads.
use super::import_rows::{Package, id, invalid};
use learning_core::*;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub(super) struct HeadPlan(BTreeMap<String, (&'static str, Uuid)>);
const HEADS: &[(SnapshotTable, SnapshotTable, &str, &str)] = &[
    (
        SnapshotTable::Block,
        SnapshotTable::BlockRevision,
        "block_id",
        "head_revision_id",
    ),
    (
        SnapshotTable::Composition,
        SnapshotTable::CompositionRevision,
        "composition_id",
        "head_revision_id",
    ),
    (
        SnapshotTable::Overlay,
        SnapshotTable::OverlayRevision,
        "overlay_id",
        "head_revision_id",
    ),
    (
        SnapshotTable::ReadingView,
        SnapshotTable::ReadingViewRevision,
        "view_id",
        "head_revision_id",
    ),
    (
        SnapshotTable::Relation,
        SnapshotTable::RelationRevision,
        "relation_id",
        "head_revision_id",
    ),
    (
        SnapshotTable::RelationReviewHead,
        SnapshotTable::RelationReview,
        "relation_revision_id",
        "head_review_id",
    ),
    (
        SnapshotTable::EpistemicStream,
        SnapshotTable::EpistemicReview,
        "stream_id",
        "head_review_id",
    ),
];
impl HeadPlan {
    pub fn build(package: &Package<'_>, root: &ReadingRef) -> Result<Self, ContentError> {
        let view = package.one(
            SnapshotTable::ReadingViewRevision,
            &["view_id", "id"],
            &[json!(root.view_id), json!(root.revision_id)],
        )?;
        let overlay = package.one(
            SnapshotTable::OverlayRevision,
            &["overlay_id", "id"],
            &[
                view["overlay_id"].clone(),
                view["overlay_revision_id"].clone(),
            ],
        )?;
        let mut heads = BTreeMap::new();
        for &(container, revision, owner, column) in HEADS {
            for row in package.rows.iter().filter(|r| r.table == container) {
                let v = &row.immutable_values;
                let owner_id = if container == SnapshotTable::RelationReviewHead {
                    id(v, "relation_revision_id")?
                } else {
                    id(v, "id")?
                };
                let candidates = package
                    .rows
                    .iter()
                    .filter(|r| r.table == revision && r.immutable_values[owner] == json!(owner_id))
                    .collect::<Vec<_>>();
                let mut ids = BTreeSet::new();
                let mut parents = BTreeSet::new();
                for candidate in &candidates {
                    ids.insert(id(&candidate.immutable_values, "id")?);
                    for parent in ["parent_revision_id", "previous_review_id"] {
                        if candidate
                            .immutable_values
                            .get(parent)
                            .is_some_and(|p| !p.is_null())
                        {
                            parents.insert(id(&candidate.immutable_values, parent)?);
                        }
                    }
                }
                let default = *ids.difference(&parents).next().ok_or_else(invalid)?;
                let selected =
                    if container == SnapshotTable::ReadingView && owner_id == root.view_id {
                        root.revision_id
                    } else if container == SnapshotTable::Overlay && v["id"] == view["overlay_id"] {
                        id(view, "overlay_revision_id")?
                    } else if container == SnapshotTable::Composition
                        && v["id"] == overlay["root_composition_id"]
                    {
                        id(overlay, "base_revision_id")?
                    } else {
                        default
                    };
                if !ids.contains(&selected) {
                    return Err(invalid());
                }
                heads.insert(
                    snapshot_object_path(container, &row.identity)?,
                    (column, selected),
                );
            }
        }
        Ok(Self(heads))
    }
    pub fn value(&self, row: &SnapshotRow) -> Result<Value, ContentError> {
        let mut value = row.immutable_values.clone();
        if let Some((column, head)) = self.0.get(&snapshot_object_path(row.table, &row.identity)?) {
            value[*column] = json!(head);
        }
        Ok(value)
    }
}

/// Fixed table order followed by parent-first order within each table. All
/// immediate foreign keys precede their users; necessary-reference cycles
/// remain legal because their registry/dependency constraints are deferred.
pub(super) fn insertion_order<'a>(
    package: &Package<'a>,
) -> Result<Vec<&'a SnapshotRow>, ContentError> {
    use SnapshotTable::*;
    const ORDER: &[SnapshotTable] = &[
        Asset,
        Resource,
        ResourceVersion,
        SourceSegment,
        Block,
        BlockRevision,
        BlockAssetUse,
        Composition,
        CompositionRevision,
        CompositionOccurrence,
        Overlay,
        OverlayGroupIdentity,
        OverlayPlacementIdentity,
        OverlayRevision,
        OverlayGroup,
        OverlayPlacement,
        PlacementManualDecision,
        Relation,
        RelationRevision,
        RelationReviewHead,
        RelationReview,
        EpistemicStream,
        EpistemicReview,
        ReadingView,
        ReadingViewRevision,
        ReadingRelationSelection,
        ReadingEpistemicSelection,
        ReferenceDependency,
    ];
    let mut result = vec![];
    for table in ORDER {
        let mut pending = package
            .rows
            .iter()
            .filter(|r| r.table == *table)
            .collect::<Vec<_>>();
        pending.sort_by_key(|r| {
            snapshot_object_path(r.table, &r.identity).expect("validated identity")
        });
        while !pending.is_empty() {
            let index = pending
                .iter()
                .position(|row| {
                    ["parent_revision_id", "previous_review_id"]
                        .iter()
                        .all(|key| {
                            row.immutable_values.get(*key).is_none_or(|p| {
                                p.is_null()
                                    || !pending
                                        .iter()
                                        .any(|other| other.immutable_values["id"] == *p)
                            })
                        })
                })
                .ok_or_else(invalid)?;
            result.push(pending.remove(index));
        }
    }
    if result.len()
        + package
            .rows
            .iter()
            .filter(|r| r.table == ReferenceObject)
            .count()
        != package.rows.len()
    {
        return Err(invalid());
    }
    Ok(result)
}

pub(super) fn receipt_policy(
    prior_request: Option<&str>,
    manifest: &str,
    prior_manifest: bool,
) -> Result<bool, ContentError> {
    if prior_request.is_some_and(|old| old != manifest) {
        return Err(ContentError::IdempotencyConflict);
    }
    Ok(prior_request.is_some() || prior_manifest)
}

#[cfg(test)]
mod tests {
    use super::super::rows;
    use super::*;
    fn fixture() -> (Vec<SnapshotRow>, ReadingRef, Principal) {
        super::super::import_tests::fixture()
    }
    #[test]
    fn heads_and_insertion_order_follow_ancestry_not_uuid_or_input_order() {
        let (mut data, root, _) = fixture();
        let original = data
            .iter()
            .find(|r| r.table == SnapshotTable::BlockRevision)
            .unwrap()
            .immutable_values
            .clone();
        for (n, parent) in [(2, 4), (1, 2), (8, 4)] {
            let mut v = original.clone();
            v["id"] = json!(Uuid::from_u128(n));
            v["parent_revision_id"] = json!(Uuid::from_u128(parent));
            data.push(rows::record(SnapshotTable::BlockRevision, v).unwrap());
        }
        let mut expected = None;
        for _ in 0..2 {
            let package = Package { rows: &data };
            let plan = HeadPlan::build(&package, &root).unwrap();
            let block = data
                .iter()
                .find(|r| r.table == SnapshotTable::Block)
                .unwrap();
            assert_eq!(
                plan.value(block).unwrap()["head_revision_id"],
                json!(Uuid::from_u128(1))
            );
            let ordered = insertion_order(&package)
                .unwrap()
                .into_iter()
                .filter(|r| r.table == SnapshotTable::BlockRevision)
                .map(|r| id(&r.immutable_values, "id").unwrap())
                .collect::<Vec<_>>();
            assert_eq!(ordered, [4, 2, 1, 8].map(Uuid::from_u128));
            if let Some(prior) = &expected {
                assert_eq!(prior, &ordered);
            } else {
                expected = Some(ordered);
            }
            data.reverse();
        }
    }
    #[test]
    fn review_heads_use_previous_review_terminals_and_all_seven_heads_are_planned() {
        let (mut data, root, _) = super::super::import_tests::epistemic_fixture(
            Intent::Conjecture,
            RelationType::Supports,
            12,
            true,
        );
        for table in [
            SnapshotTable::RelationReview,
            SnapshotTable::EpistemicReview,
        ] {
            let original = data
                .iter()
                .find(|r| r.table == table)
                .unwrap()
                .immutable_values
                .clone();
            let mut branch = original.clone();
            branch["previous_review_id"] = original["id"].clone();
            branch["id"] = json!(Uuid::from_u128(900));
            data.push(rows::record(table, branch.clone()).unwrap());
            branch["id"] = json!(Uuid::from_u128(800));
            data.push(rows::record(table, branch).unwrap());
        }
        for _ in 0..2 {
            let package = Package { rows: &data };
            let plan = HeadPlan::build(&package, &root).unwrap();
            for &(table, _, _, column) in HEADS {
                for row in data.iter().filter(|r| r.table == table) {
                    assert!(!plan.value(row).unwrap()[column].is_null());
                }
            }
            for table in [
                SnapshotTable::RelationReviewHead,
                SnapshotTable::EpistemicStream,
            ] {
                let row = data.iter().find(|r| r.table == table).unwrap();
                assert_eq!(
                    plan.value(row).unwrap()["head_review_id"],
                    json!(Uuid::from_u128(800))
                );
            }
            for table in [
                SnapshotTable::RelationReview,
                SnapshotTable::EpistemicReview,
            ] {
                let ordered = insertion_order(&package)
                    .unwrap()
                    .into_iter()
                    .filter(|r| r.table == table)
                    .collect::<Vec<_>>();
                assert!(ordered[0].immutable_values["previous_review_id"].is_null());
                assert_eq!(
                    id(&ordered[1].immutable_values, "id").unwrap(),
                    Uuid::from_u128(800)
                );
                assert_eq!(
                    id(&ordered[2].immutable_values, "id").unwrap(),
                    Uuid::from_u128(900)
                );
            }
            data.reverse();
        }
    }
    #[test]
    fn root_heads_keep_exact_selected_ancestors_and_missing_candidates_fail() {
        let (mut data, root, _) = fixture();
        for table in [
            SnapshotTable::ReadingViewRevision,
            SnapshotTable::OverlayRevision,
            SnapshotTable::CompositionRevision,
        ] {
            let mut v = data
                .iter()
                .find(|r| r.table == table)
                .unwrap()
                .immutable_values
                .clone();
            v["parent_revision_id"] = v["id"].clone();
            v["id"] = json!(Uuid::new_v4());
            data.push(rows::record(table, v).unwrap());
        }
        let plan = HeadPlan::build(&Package { rows: &data }, &root).unwrap();
        for (table, expected) in [
            (SnapshotTable::ReadingView, 10),
            (SnapshotTable::Overlay, 8),
            (SnapshotTable::Composition, 6),
        ] {
            assert_eq!(
                plan.value(data.iter().find(|r| r.table == table).unwrap())
                    .unwrap()["head_revision_id"],
                json!(Uuid::from_u128(expected))
            );
        }
        data.retain(|r| r.table != SnapshotTable::BlockRevision);
        assert!(HeadPlan::build(&Package { rows: &data }, &root).is_err());
    }
    #[test]
    fn receipt_reuse_depends_on_committed_package_receipt_not_business_rows() {
        assert!(!receipt_policy(None, "a", false).unwrap());
        assert!(receipt_policy(None, "a", true).unwrap());
        assert!(receipt_policy(Some("a"), "a", false).unwrap());
        assert!(matches!(
            receipt_policy(Some("b"), "a", true),
            Err(ContentError::IdempotencyConflict)
        ));
    }
}

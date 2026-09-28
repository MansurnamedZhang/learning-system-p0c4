//! Package-only checks, independent of target state and filesystem names.
use super::{import_schema as schema, rows};
use learning_core::*;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub(super) fn invalid() -> ContentError {
    ContentError::Invalid("invalid_snapshot_import".into())
}
pub(super) fn decode<T: DeserializeOwned>(v: Value) -> Result<T, ContentError> {
    serde_json::from_value(v).map_err(|_| invalid())
}
pub(super) fn id(v: &Value, k: &str) -> Result<Uuid, ContentError> {
    rows::uuid(v, k).map_err(|_| invalid())
}
fn text<'a>(v: &'a Value, k: &str) -> Result<&'a str, ContentError> {
    rows::string(v, k).map_err(|_| invalid())
}
pub(super) struct Package<'a> {
    pub rows: &'a [SnapshotRow],
}
impl<'a> Package<'a> {
    pub fn matching(&self, t: SnapshotTable, columns: &[&str], values: &[Value]) -> Vec<&'a Value> {
        self.rows
            .iter()
            .filter(|r| {
                r.table == t
                    && columns
                        .iter()
                        .zip(values)
                        .all(|(k, v)| r.immutable_values[*k] == *v)
            })
            .map(|r| &r.immutable_values)
            .collect()
    }
    pub fn one(
        &self,
        t: SnapshotTable,
        columns: &[&str],
        values: &[Value],
    ) -> Result<&'a Value, ContentError> {
        let found = self.matching(t, columns, values);
        if found.len() != 1 {
            return Err(invalid());
        }
        Ok(found[0])
    }
    pub fn validate(&self, root: &ReadingRef, actor: Principal) -> Result<(), ContentError> {
        let mut paths = BTreeSet::new();
        let mut unique = BTreeSet::new();
        let mut body = 0usize;
        for row in self.rows {
            let v = &row.immutable_values;
            let object = v.as_object().ok_or_else(invalid)?;
            let columns = schema::columns(row.table);
            if columns.len() != object.len() {
                return Err(invalid());
            }
            for (key, kind, nullable) in columns {
                let value = object.get(*key).ok_or_else(invalid)?;
                if value.is_null() {
                    if !nullable {
                        return Err(invalid());
                    }
                    continue;
                }
                let valid = match *kind {
                    "uuid" => value.as_str().is_some_and(valid_uuid),
                    "uuid[]" => value
                        .as_array()
                        .is_some_and(|a| a.iter().all(|v| v.as_str().is_some_and(valid_uuid))),
                    "text" => value.as_str().is_some_and(|s| !s.contains('\0')),
                    "integer" => value.as_i64().is_some_and(|n| i32::try_from(n).is_ok()),
                    "bigint" => value.as_i64().is_some(),
                    "boolean" => value.is_boolean(),
                    "jsonb" => true,
                    "timestamptz" => value.as_str().is_some_and(|s| {
                        chrono::DateTime::parse_from_rfc3339(s).is_ok_and(|t| {
                            canonical_snapshot_timestamp(t.with_timezone(&chrono::Utc)) == s
                        })
                    }),
                    _ => false,
                };
                if !valid {
                    return Err(invalid());
                }
            }
            if rows::record(row.table, v.clone()).map_err(|_| invalid())? != *row
                || !paths.insert(snapshot_object_path(row.table, &row.identity)?)
            {
                return Err(invalid());
            }
            let keys = schema::unique_key(row.table);
            if !keys.is_empty() && (row.table != SnapshotTable::OverlayGroup || v["placed"] == true)
            {
                let key = canonical_json(&json!([
                    row.table,
                    keys.iter().map(|k| &v[*k]).collect::<Vec<_>>()
                ]));
                if !unique.insert(key) {
                    return Err(invalid());
                }
            }
            for link in schema::links(row.table) {
                let values: Vec<_> = link.columns.iter().map(|k| v[*k].clone()).collect();
                if values.iter().any(Value::is_null) {
                    continue;
                }
                if link.target == "space" || link.target == "app_user" {
                    continue;
                }
                let table = decode(json!(link.target))?;
                self.one(table, link.keys, &values)?;
            }
            if matches!(
                row.table,
                SnapshotTable::BlockRevision
                    | SnapshotTable::RelationRevision
                    | SnapshotTable::RelationReview
                    | SnapshotTable::EpistemicReview
            ) {
                body = body.saturating_add(
                    canonical_json(if row.table == SnapshotTable::BlockRevision {
                        &v["content"]
                    } else {
                        v
                    })
                    .len(),
                );
            }
            if v.get("parent_revision_id")
                .is_some_and(|p| !p.is_null() && p == &v["id"])
                || v.get("previous_review_id")
                    .is_some_and(|p| !p.is_null() && p == &v["id"])
            {
                return Err(invalid());
            }
        }
        if body > SNAPSHOT_MAX_BODY_BYTES || self.rows.len() > SNAPSHOT_MAX_FILES {
            return Err(invalid());
        }
        let rootrow = self.one(
            SnapshotTable::ReadingViewRevision,
            &["view_id", "id"],
            &[json!(root.view_id), json!(root.revision_id)],
        )?;
        let overlay = id(rootrow, "overlay_id")?;
        for row in self.rows {
            let v = &row.immutable_values;
            if row.table == SnapshotTable::Overlay
                && (id(v, "id")? != overlay || id(v, "owner_id")? != actor.actor_id)
            {
                return Err(ContentError::NotFound);
            }
            if v.get("overlay_id")
                .is_some_and(|x| !x.is_null() && x != &json!(overlay))
            {
                return Err(ContentError::NotFound);
            }
            self.semantic(row)?;
        }
        self.graph_limits()?;
        Ok(())
    }
    fn ordered(
        &self,
        t: SnapshotTable,
        column: &str,
        value: &Value,
    ) -> Result<Vec<&'a Value>, ContentError> {
        let mut found = self.matching(t, &[column], std::slice::from_ref(value));
        found.sort_by_key(|r| r["position"].as_u64());
        if found
            .iter()
            .enumerate()
            .any(|(n, r)| r["position"].as_u64() != Some(n as u64))
        {
            return Err(invalid());
        }
        Ok(found)
    }
    fn scope(&self, v: &Value) -> Result<RelationScope, ContentError> {
        let space_id = id(v, "space_id")?;
        Ok(if v["overlay_id"].is_null() {
            RelationScope::Space { space_id }
        } else {
            RelationScope::PersonalOverlay {
                space_id,
                overlay_id: id(v, "overlay_id")?,
            }
        })
    }
    fn block_ref(&self, v: &Value, prefix: &str) -> Result<BlockRef, ContentError> {
        Ok(BlockRef {
            block_id: id(v, &format!("{prefix}block_id"))?,
            revision_id: id(v, &format!("{prefix}revision_id"))?,
        })
    }
    // Validate stable writer semantics against the imported immutable closure.
    // Current review heads may have changed since this historical judgment.
    fn judgment(&self, command: &AppendEpistemicReview) -> Result<(), ContentError> {
        let target = self.one(
            SnapshotTable::BlockRevision,
            &["block_id", "id"],
            &[
                serde_json::json!(command.target.block_id),
                serde_json::json!(command.target.revision_id),
            ],
        )?;
        let draft = ContentDraft::decode(
            target["contract_version"].as_u64().ok_or_else(invalid)? as u32,
            target["content"].clone(),
        )
        .map_err(|_| invalid())?;
        let intent = match draft {
            ContentDraft::V1(d) => d.intent,
            ContentDraft::V2(d) => d.intent,
            ContentDraft::V3(d) => d.intent,
        };
        if !matches!(intent, Intent::Conjecture | Intent::Conclusion) {
            return Err(invalid());
        }
        let direction = match command.state {
            EpistemicState::SupportedWithinScope => Some(RelationType::Supports),
            EpistemicState::RefutedWithinScope => Some(RelationType::Opposes),
            _ => None,
        };
        let mut has_reviewed_basis = false;
        for selection in &command.relations {
            let identity = self.one(
                SnapshotTable::Relation,
                &["id"],
                &[serde_json::json!(selection.relation.relation_id)],
            )?;
            let revision = self.one(
                SnapshotTable::RelationRevision,
                &["relation_id", "id"],
                &[
                    serde_json::json!(selection.relation.relation_id),
                    serde_json::json!(selection.relation.revision_id),
                ],
            )?;
            let kind: RelationType = decode(identity["type"].clone())?;
            let from = self.block_ref(revision, "from_")?;
            let to = self.block_ref(revision, "to_")?;
            if matches!(kind, RelationType::Supports | RelationType::Opposes) {
                if to != command.target || !command.evidence.contains(&from) {
                    return Err(invalid());
                }
            } else if from != command.target && to != command.target {
                return Err(invalid());
            }
            if direction == Some(kind)
                && let Some(selected) = &selection.review
            {
                let review = self.one(
                    SnapshotTable::RelationReview,
                    &["relation_id", "relation_revision_id", "id"],
                    &[
                        json!(selected.relation.relation_id),
                        json!(selected.relation.revision_id),
                        json!(selected.review_id),
                    ],
                )?;
                let state: RelationReviewState = decode(review["state"].clone())?;
                has_reviewed_basis |= state == RelationReviewState::Reviewed;
            }
        }
        if direction.is_some() && !has_reviewed_basis {
            return Err(invalid());
        }
        Ok(())
    }
    fn semantic(&self, row: &SnapshotRow) -> Result<(), ContentError> {
        use SnapshotTable::*;
        let v = &row.immutable_values;
        let mut exact = None;
        let mut deps = vec![];
        match row.table {
            BlockRevision => {
                let draft = ContentDraft::decode(
                    v["contract_version"].as_u64().ok_or_else(invalid)? as u32,
                    v["content"].clone(),
                )
                .map_err(|_| invalid())?;
                exact = Some(ExactRef::Block(BlockRef {
                    block_id: id(v, "block_id")?,
                    revision_id: id(v, "id")?,
                }));
                deps = draft.dependencies();
                let uses = self.matching(
                    BlockAssetUse,
                    &["block_id", "revision_id"],
                    &[v["block_id"].clone(), v["id"].clone()],
                );
                if let Some(asset) = draft.asset_ref() {
                    if uses.len() != 1
                        || uses[0]["space_id"] != json!(asset.space_id)
                        || uses[0]["asset_id"] != json!(asset.asset_id)
                    {
                        return Err(invalid());
                    }
                } else if !uses.is_empty() {
                    return Err(invalid());
                }
            }
            CompositionRevision => {
                let kind: CompositionKind = decode(v["kind"].clone())?;
                let nodes = self.occurrences(v)?;
                let container = self.one(Composition, &["id"], &[v["composition_id"].clone()])?;
                if container["kind"] != v["kind"] {
                    return Err(invalid());
                }
                if composition_digest(kind, text(v, "title")?, &nodes) != text(v, "content_sha256")?
                {
                    return Err(invalid());
                }
            }
            OverlayRevision => {
                let mut groups = vec![];
                for g in self.matching(OverlayGroup, &["overlay_revision_id"], &[v["id"].clone()]) {
                    let anchor = GapAnchor {
                        base: CompositionRef {
                            composition_id: id(g, "root_composition_id")?,
                            revision_id: id(g, "base_revision_id")?,
                        },
                        parent_occurrence_path: decode(g["parent_path"].clone())?,
                        left_occurrence_id: decode(g["left_id"].clone())?,
                        right_occurrence_id: decode(g["right_id"].clone())?,
                        affinity: decode(g["affinity"].clone())?,
                    };
                    anchor.validate()?;
                    let placed = g["placed"].as_bool().ok_or_else(invalid)?;
                    if placed {
                        if anchor.base.revision_id != id(v, "base_revision_id")? {
                            return Err(invalid());
                        }
                        self.validate_gap(&anchor)?;
                    }
                    if (placed && g["placed_base_revision_id"] != g["base_revision_id"])
                        || (!placed && !g["placed_base_revision_id"].is_null())
                    {
                        return Err(invalid());
                    }
                    let mut ps = self.matching(
                        OverlayPlacement,
                        &["overlay_revision_id", "group_id"],
                        &[v["id"].clone(), g["group_id"].clone()],
                    );
                    ps.sort_by_key(|p| p["position"].as_u64());
                    let mut placements = vec![];
                    for (n, p) in ps.iter().enumerate() {
                        if p["position"].as_u64() != Some(n as u64) {
                            return Err(invalid());
                        }
                        placements.push(Placement {
                            placement_id: id(p, "placement_id")?,
                            block: BlockRef {
                                block_id: id(p, "block_id")?,
                                revision_id: id(p, "block_revision_id")?,
                            },
                        });
                    }
                    groups.push(EditableGroup {
                        group_id: id(g, "group_id")?,
                        location: if placed {
                            GroupLocation::Placed { anchor }
                        } else {
                            GroupLocation::Unplaced {
                                origin_anchor: anchor,
                            }
                        },
                        placements,
                    });
                }
                let reading = EditableReading {
                    base: CompositionRef {
                        composition_id: id(v, "root_composition_id")?,
                        revision_id: id(v, "base_revision_id")?,
                    },
                    title: text(v, "title")?.into(),
                    groups,
                };
                let mut gaps = BTreeSet::new();
                if reading.groups.len() > 512
                    || reading
                        .groups
                        .iter()
                        .map(|g| g.placements.len())
                        .sum::<usize>()
                        > 2048
                {
                    return Err(invalid());
                }
                for group in &reading.groups {
                    if group.placements.is_empty()
                        || (group.location.placed()
                            && !gaps.insert(canonical_json(&json!(group.location.anchor()))))
                    {
                        return Err(invalid());
                    }
                }
                if reading.digest() != text(v, "content_sha256")? {
                    return Err(invalid());
                }
            }
            RelationRevision => {
                let identity = self.one(Relation, &["id"], &[v["relation_id"].clone()])?;
                let from = self.block_ref(v, "from_")?;
                let to = self.block_ref(v, "to_")?;
                let kind: RelationType = decode(identity["type"].clone())?;
                let scope = self.scope(identity)?;
                SaveRelation {
                    request_id: Uuid::nil(),
                    scope: scope.clone(),
                    relation_id: None,
                    expected_revision: None,
                    relation_type: kind,
                    from: from.clone(),
                    to: to.clone(),
                    rationale: text(v, "rationale")?.into(),
                    conditions: text(v, "conditions")?.into(),
                }
                .validate()
                .map_err(|_| invalid())?;
                let hash = canonical_record_hash(
                    &json!({"domain":"relation-content-v1","scope":scope,"type":kind,"from":from,"to":to,"rationale":v["rationale"],"conditions":v["conditions"]}),
                );
                if hash != text(v, "content_sha256")? {
                    return Err(invalid());
                }
                exact = Some(ExactRef::Relation(RelationRef {
                    relation_id: id(v, "relation_id")?,
                    revision_id: id(v, "id")?,
                }));
                deps = vec![
                    Dependency {
                        role: DependencyRole::Target,
                        target: ExactRef::Block(from),
                    },
                    Dependency {
                        role: DependencyRole::Target,
                        target: ExactRef::Block(to),
                    },
                ];
            }
            RelationReview => {
                let relation = RelationRef {
                    relation_id: id(v, "relation_id")?,
                    revision_id: id(v, "relation_revision_id")?,
                };
                let _: RelationReviewState = decode(v["state"].clone())?;
                exact = Some(ExactRef::RelationReview(RelationReviewRef {
                    relation: relation.clone(),
                    review_id: id(v, "id")?,
                }));
                deps.push(Dependency {
                    role: DependencyRole::Target,
                    target: ExactRef::Relation(relation),
                });
            }
            EpistemicReview => {
                let stream = self.one(EpistemicStream, &["id"], &[v["stream_id"].clone()])?;
                let command = AppendEpistemicReview {
                    request_id: Uuid::nil(),
                    scope: self.scope(stream)?,
                    target: self.block_ref(stream, "target_")?,
                    expected_previous: None,
                    state: decode(v["state"].clone())?,
                    relations: decode(v["relations"].clone())?,
                    evidence: decode(v["evidence"].clone())?,
                    conditions: text(v, "conditions")?.into(),
                    explanation: text(v, "explanation")?.into(),
                };
                command.validate().map_err(|_| invalid())?;
                self.judgment(&command)?;
                deps = command.dependencies();
                exact = Some(ExactRef::EpistemicReview(EpistemicReviewRef {
                    stream_id: id(v, "stream_id")?,
                    review_id: id(v, "id")?,
                }));
            }
            ReadingViewRevision => {
                let choices: ReadingSelections = match v["contract_version"].as_i64() {
                    Some(1) if v["evidence"].is_null() => ReadingSelections::default(),
                    Some(2) => decode(v["evidence"].clone())?,
                    _ => return Err(invalid()),
                };
                choices.validate().map_err(|_| invalid())?;
                let relations =
                    self.ordered(ReadingRelationSelection, "view_revision_id", &v["id"])?;
                let epistemic =
                    self.ordered(ReadingEpistemicSelection, "view_revision_id", &v["id"])?;
                if relations.len() != choices.selections.len()
                    || epistemic.len() != choices.epistemic_reviews.len()
                {
                    return Err(invalid());
                }
                for (r, c) in relations.iter().zip(choices.selections) {
                    if r["relation_id"] != json!(c.relation.relation_id)
                        || r["relation_revision_id"] != json!(c.relation.revision_id)
                        || r["review_id"] != json!(c.review.map(|x| x.review_id))
                    {
                        return Err(invalid());
                    }
                }
                for (r, c) in epistemic.iter().zip(choices.epistemic_reviews) {
                    if r["stream_id"] != json!(c.stream_id) || r["review_id"] != json!(c.review_id)
                    {
                        return Err(invalid());
                    }
                }
            }
            Asset => {
                let hash = text(v, "sha256")?;
                if hash.len() != 64
                    || !hash
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    || v["byte_size"].as_i64().is_none_or(|n| n < 0)
                    || text(v, "storage_key")? != format!("sha256/{}/{}", &hash[..2], hash)
                    || v["status"] != "ready"
                {
                    return Err(invalid());
                }
            }
            ReferenceObject => {
                let _ = self.exact_row(v)?;
            }
            _ => {}
        }
        if let Some(exact) = exact {
            let (kind, object, revision) = crate::references::key(&exact);
            let registry = self.one(
                ReferenceObject,
                &["kind", "object_id", "revision_id"],
                &[json!(kind), json!(object), json!(revision)],
            )?;
            if registry["space_id"] != v["space_id"] {
                return Err(invalid());
            }
            let mut stored = self.matching(
                ReferenceDependency,
                &["source_kind", "source_object_id", "source_revision_id"],
                &[json!(kind), json!(object), json!(revision)],
            );
            stored.sort_by_key(|r| r["position"].as_u64());
            if stored.len() != deps.len() {
                return Err(invalid());
            }
            for (n, (r, d)) in stored.iter().zip(deps).enumerate() {
                let (kind, obj, rev) = crate::references::key(&d.target);
                if r["position"] != json!(n)
                    || r["role"] != json!(d.role)
                    || r["target_kind"] != json!(kind)
                    || r["target_object_id"] != json!(obj)
                    || r["target_revision_id"] != json!(rev)
                {
                    return Err(invalid());
                }
            }
        }
        Ok(())
    }
    fn exact_row(&self, v: &Value) -> Result<&'a Value, ContentError> {
        let (table, object) = match text(v, "kind")? {
            "block" => (SnapshotTable::BlockRevision, "block_id"),
            "relation" => (SnapshotTable::RelationRevision, "relation_id"),
            "relation_review" => (SnapshotTable::RelationReview, "relation_revision_id"),
            "epistemic_review" => (SnapshotTable::EpistemicReview, "stream_id"),
            _ => return Err(invalid()),
        };
        self.one(
            table,
            &[object, "id"],
            &[v["object_id"].clone(), v["revision_id"].clone()],
        )
    }
    fn occurrences(&self, v: &Value) -> Result<Vec<Occurrence>, ContentError> {
        self.ordered(
            SnapshotTable::CompositionOccurrence,
            "composition_revision_id",
            &v["id"],
        )?
        .into_iter()
        .map(|o| {
            let block = !o["block_id"].is_null();
            if block == !o["child_composition_id"].is_null() {
                return Err(invalid());
            }
            Ok(Occurrence {
                occurrence_id: id(o, "occurrence_id")?,
                target: if block {
                    NodeTarget::Block(BlockRef {
                        block_id: id(o, "block_id")?,
                        revision_id: id(o, "block_revision_id")?,
                    })
                } else {
                    NodeTarget::Composition(CompositionRef {
                        composition_id: id(o, "child_composition_id")?,
                        revision_id: id(o, "child_revision_id")?,
                    })
                },
            })
        })
        .collect()
    }
    fn validate_gap(&self, anchor: &GapAnchor) -> Result<(), ContentError> {
        let mut reference = anchor.base.clone();
        for occurrence in &anchor.parent_occurrence_path {
            let composition = self.one(
                SnapshotTable::CompositionRevision,
                &["composition_id", "id"],
                &[
                    json!(reference.composition_id),
                    json!(reference.revision_id),
                ],
            )?;
            let nodes = self.occurrences(composition)?;
            let node = nodes
                .iter()
                .find(|n| n.occurrence_id == *occurrence)
                .ok_or_else(invalid)?;
            let NodeTarget::Composition(child) = &node.target else {
                return Err(invalid());
            };
            reference = child.clone();
        }
        let composition = self.one(
            SnapshotTable::CompositionRevision,
            &["composition_id", "id"],
            &[
                json!(reference.composition_id),
                json!(reference.revision_id),
            ],
        )?;
        crate::overlay::anchor::insertion_index(
            &self
                .occurrences(composition)?
                .iter()
                .map(|n| n.occurrence_id)
                .collect::<Vec<_>>(),
            &PlacementOrder {
                left_placement_id: anchor.left_occurrence_id,
                right_placement_id: anchor.right_occurrence_id,
            },
        )
        .map(|_| ())
        .map_err(|_| invalid())
    }
    fn graph_limits(&self) -> Result<(), ContentError> {
        let mut graph: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut count = 0;
        let mut history = BTreeMap::new();
        for row in self.rows {
            let v = &row.immutable_values;
            if matches!(
                row.table,
                SnapshotTable::ReferenceObject | SnapshotTable::CompositionRevision
            ) {
                count += 1;
            }
            for key in ["parent_revision_id", "previous_review_id"] {
                if let Some(parent) = v.get(key).filter(|x| !x.is_null()) {
                    let pair = match row.table {
                        SnapshotTable::BlockRevision => Some(("block", "block_id")),
                        SnapshotTable::RelationRevision => Some(("relation", "relation_id")),
                        SnapshotTable::RelationReview => {
                            Some(("relation_review", "relation_revision_id"))
                        }
                        SnapshotTable::EpistemicReview => Some(("epistemic_review", "stream_id")),
                        _ => None,
                    };
                    let (source, target) = if let Some((kind, object)) = pair {
                        (
                            format!("{}:{}:{}", json!(kind), v[object], v["id"]),
                            format!("{}:{}:{}", json!(kind), v[object], parent),
                        )
                    } else {
                        (
                            format!("{:?}:{}", row.table, v["id"]),
                            format!("{:?}:{}", row.table, parent),
                        )
                    };
                    history.insert(source.clone(), target.clone());
                    graph.entry(source).or_default().push(target);
                }
            }
            if row.table == SnapshotTable::ReferenceDependency {
                graph
                    .entry(format!(
                        "{}:{}:{}",
                        v["source_kind"], v["source_object_id"], v["source_revision_id"]
                    ))
                    .or_default()
                    .push(format!(
                        "{}:{}:{}",
                        v["target_kind"], v["target_object_id"], v["target_revision_id"]
                    ));
            }
        }
        if count > SNAPSHOT_MAX_OBJECTS
            || graph.values().map(Vec::len).sum::<usize>() > SNAPSHOT_MAX_EDGES
        {
            return Err(invalid());
        }
        // Audit ancestry must be acyclic. Necessary reference cycles are
        // legal under the existing resolver, and use bounded simple paths.
        for root in history.keys() {
            let mut seen = BTreeSet::new();
            let mut next = root;
            while let Some(parent) = history.get(next) {
                if !seen.insert(next) {
                    return Err(invalid());
                }
                next = parent;
            }
        }
        let mut incoming = BTreeMap::<String, usize>::new();
        for (source, targets) in &graph {
            incoming.entry(source.clone()).or_default();
            for t in targets {
                *incoming.entry(t.clone()).or_default() += 1;
            }
        }
        let mut queue: std::collections::VecDeque<_> = incoming
            .iter()
            .filter(|(_, n)| **n == 0)
            .map(|(k, _)| k.clone())
            .collect();
        let mut depths = BTreeMap::<String, usize>::new();
        let mut processed = 0;
        while let Some(node) = queue.pop_front() {
            processed += 1;
            let depth = *depths.entry(node.clone()).or_insert(1);
            if depth > SNAPSHOT_MAX_REFERENCE_DEPTH {
                return Err(invalid());
            }
            if let Some(edges) = graph.get(&node) {
                for edge in edges {
                    let target = depths.entry(edge.clone()).or_insert(1);
                    *target = (*target).max(depth + 1);
                    let n = incoming.get_mut(edge).ok_or_else(invalid)?;
                    *n -= 1;
                    if *n == 0 {
                        queue.push_back(edge.clone());
                    }
                }
            }
        }
        if processed != incoming.len() {
            let mut work = 0usize;
            for root in graph.keys() {
                let mut pending = vec![(root.clone(), Vec::<String>::new())];
                while let Some((node, mut path)) = pending.pop() {
                    work += 1;
                    if work > 131_072 {
                        return Err(invalid());
                    }
                    if path.contains(&node) {
                        continue;
                    }
                    if path.len() >= SNAPSHOT_MAX_REFERENCE_DEPTH {
                        return Err(invalid());
                    }
                    path.push(node.clone());
                    if let Some(edges) = graph.get(&node) {
                        pending.extend(edges.iter().cloned().map(|e| (e, path.clone())));
                    }
                }
            }
        }
        let mut comps = BTreeMap::new();
        for row in self
            .rows
            .iter()
            .filter(|r| r.table == SnapshotTable::CompositionRevision)
        {
            comps.insert(
                id(&row.immutable_values, "id")?,
                self.occurrences(&row.immutable_values)?,
            );
        }
        fn expand(
            id: Uuid,
            comps: &BTreeMap<Uuid, Vec<Occurrence>>,
            depth: usize,
            seen: &mut usize,
        ) -> Result<(), ContentError> {
            if depth > SNAPSHOT_MAX_COMPOSITION_DEPTH {
                return Err(invalid());
            }
            for o in comps.get(&id).ok_or_else(invalid)? {
                *seen += 1;
                if *seen > SNAPSHOT_MAX_OCCURRENCES {
                    return Err(invalid());
                }
                if let NodeTarget::Composition(c) = &o.target {
                    expand(c.revision_id, comps, depth + 1, seen)?;
                }
            }
            Ok(())
        }
        for key in comps.keys() {
            expand(*key, &comps, 1, &mut 0)?;
        }
        let mut roots = BTreeSet::new();
        for row in self.rows {
            let v = &row.immutable_values;
            if matches!(
                row.table,
                SnapshotTable::OverlayRevision | SnapshotTable::OverlayGroup
            ) {
                roots.insert(id(v, "base_revision_id")?);
            }
            if row.table == SnapshotTable::CompositionRevision && !v["parent_revision_id"].is_null()
            {
                roots.insert(id(v, "parent_revision_id")?);
            }
        }
        let mut total = 0;
        for root in roots {
            expand(root, &comps, 1, &mut total)?;
        }

        Ok(())
    }
}
fn valid_uuid(s: &str) -> bool {
    Uuid::parse_str(s).is_ok_and(|id| !id.is_nil() && id.to_string() == s)
}

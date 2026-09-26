use super::{
    SnapshotPlan,
    rows::{self, Tx, optional_uuid, string, uuid},
};
use crate::{
    assets, composition::closure, overlay::model, reading::selection, references, storage,
};
use learning_core::*;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Node {
    Reading(ReadingRef),
    Overlay(Uuid, Uuid),
    Composition(CompositionRef),
    Exact(ExactRef),
    Resource(ResourceVersionRef),
    Segment(SourceSegmentRef),
}

struct Collector {
    actor: Principal,
    root_overlay: Uuid,
    session: references::Session,
    budget: SnapshotBudget,
    pending: VecDeque<Node>,
    visited: BTreeSet<Node>,
    rows: BTreeMap<String, SnapshotRow>,
    assets: BTreeMap<AssetUseRef, SnapshotAssetUse>,
    asset_files: BTreeMap<String, SnapshotFile>,
    include_originals: bool,
    required_spaces: BTreeSet<Uuid>,
    // Historical/necessary edges are distinct from composition occurrence walks.
    exact_edges: BTreeMap<Node, BTreeSet<Node>>,
    history_parents: BTreeMap<Node, Node>,
    compositions: BTreeMap<CompositionRef, CompositionRevision>,
    composition_roots: BTreeSet<CompositionRef>,
}

fn limit() -> ContentError {
    ContentError::Invalid("snapshot_limit_exceeded".into())
}

pub(super) async fn collect(
    tx: &mut Tx<'_>,
    actor: Principal,
    input: &SnapshotRequest,
) -> Result<SnapshotPlan, ContentError> {
    collect_authorized(tx, actor, input)
        .await
        .map(|(plan, _)| plan)
}

pub(super) async fn collect_authorized(
    tx: &mut Tx<'_>,
    actor: Principal,
    input: &SnapshotRequest,
) -> Result<(SnapshotPlan, Vec<(Uuid, bool)>), ContentError> {
    let root = model::load_view(tx, actor, input.reading.clone()).await?;
    let mut collector = Collector {
        actor,
        root_overlay: root.id,
        session: references::Session::default(),
        budget: SnapshotBudget::default(),
        pending: VecDeque::from([Node::Reading(input.reading.clone())]),
        visited: BTreeSet::new(),
        rows: BTreeMap::new(),
        assets: BTreeMap::new(),
        asset_files: BTreeMap::new(),
        include_originals: input.include_originals,
        required_spaces: BTreeSet::new(),
        exact_edges: BTreeMap::new(),
        history_parents: BTreeMap::new(),
        compositions: BTreeMap::new(),
        composition_roots: BTreeSet::new(),
    };
    // Reserve manifest and required validation.json, whose actual bytes the file
    // writer charges again while assembling the final package manifest.
    collector
        .budget
        .charge(SnapshotBudgetKind::JsonFile, 0, 0)?;
    collector
        .budget
        .charge(SnapshotBudgetKind::JsonFile, 0, 0)?;
    collector
        .pending
        .extend(input.resource_versions.iter().cloned().map(Node::Resource));
    collector
        .pending
        .extend(input.source_segments.iter().cloned().map(Node::Segment));
    while let Some(node) = collector.pending.pop_front() {
        if !collector.visited.insert(node.clone()) {
            continue;
        }
        match node {
            Node::Reading(view) => collector.reading(tx, &view).await?,
            Node::Overlay(id, revision) => collector.overlay(tx, id, revision).await?,
            Node::Composition(reference) => collector.composition(tx, &reference).await?,
            Node::Exact(reference) => collector.exact(tx, &reference).await?,
            Node::Resource(reference) => collector.resource(tx, &reference).await?,
            Node::Segment(reference) => collector.segment(tx, &reference).await?,
        }
    }
    collector.check_depth()?;
    collector.check_occurrences()?;
    let mut files = collector
        .rows
        .iter()
        .map(|(path, row)| {
            let bytes = canonical_json(&serde_json::to_value(row).map_err(storage)?);
            Ok(SnapshotFile {
                path: path.clone(),
                size: bytes.len() as u64,
                sha256: hex_digest(bytes.as_bytes()),
            })
        })
        .collect::<Result<Vec<_>, ContentError>>()?;
    files.extend(collector.asset_files.into_values());
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let manifest = ExactSnapshotManifest {
        format_version: SNAPSHOT_FORMAT_VERSION,
        root: input.reading.clone(),
        files,
        requires_destination_assets: !input.include_originals,
    };
    // Include the actual deterministic manifest bytes in the aggregate JSON
    // budget; file count was already reserved. Final package adds report bytes.
    let json_size = collector
        .rows
        .values()
        .map(|row| {
            canonical_json(&serde_json::to_value(row).expect("snapshot row serializes")).len()
        })
        .sum::<usize>();
    let manifest_size = canonical_json(&serde_json::to_value(&manifest).map_err(storage)?).len();
    if manifest_size > SNAPSHOT_MAX_JSON_FILE_BYTES
        || json_size.saturating_add(manifest_size) > SNAPSHOT_MAX_JSON_BYTES
    {
        return Err(limit());
    }
    let required_spaces = collector
        .required_spaces
        .iter()
        .map(|s| (*s, false))
        .chain(collector.session.spaces())
        .collect();
    Ok((
        SnapshotPlan {
            manifest,
            rows: collector.rows.into_values().collect(),
            assets: collector.assets.into_values().collect(),
        },
        required_spaces,
    ))
}

impl Collector {
    fn add(&mut self, table: SnapshotTable, value: Value) -> Result<(), ContentError> {
        let row = rows::record(table, value)?;
        let path = snapshot_object_path(table, &row.identity)?;
        if let Some(existing) = self.rows.get(&path) {
            return if existing == &row {
                Ok(())
            } else {
                Err(ContentError::Storage)
            };
        }
        let bytes = canonical_json(&serde_json::to_value(&row).map_err(storage)?).len();
        self.budget.charge(SnapshotBudgetKind::JsonFile, bytes, 0)?;
        self.rows.insert(path, row);
        Ok(())
    }
    fn enqueue(&mut self, node: Node) {
        self.pending.push_back(node);
    }
    fn edge(&mut self, from: Node, to: Node) -> Result<(), ContentError> {
        let edges = self.exact_edges.entry(from).or_default();
        if edges.insert(to.clone()) {
            self.budget
                .charge(SnapshotBudgetKind::ReferenceEdge, 0, 1)?;
        }
        self.enqueue(to);
        Ok(())
    }
    fn parent(&mut self, from: Node, to: Node) -> Result<(), ContentError> {
        self.history_parents.insert(from.clone(), to.clone());
        self.edge(from, to)
    }
    fn composition_root(&mut self, reference: CompositionRef) {
        self.composition_roots.insert(reference.clone());
        self.enqueue(Node::Composition(reference));
    }
    async fn grant(&mut self, tx: &mut Tx<'_>, space: Uuid) -> Result<(), ContentError> {
        let present: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM public.space_grant WHERE actor_id=$1 AND space_id=$2)",
        )
        .bind(self.actor.actor_id)
        .bind(space)
        .fetch_one(&mut **tx)
        .await
        .map_err(storage)?;
        if present {
            self.required_spaces.insert(space);
            Ok(())
        } else {
            Err(ContentError::NotFound)
        }
    }
    async fn attached(
        &mut self,
        tx: &mut Tx<'_>,
        table: SnapshotTable,
        columns: &[&str],
        ids: &[Uuid],
    ) -> Result<(), ContentError> {
        for value in rows::many(tx, table, columns, ids).await? {
            self.add(table, value)?;
        }
        Ok(())
    }

    async fn reading(&mut self, tx: &mut Tx<'_>, view: &ReadingRef) -> Result<(), ContentError> {
        let layer = model::load_view(tx, self.actor, view.clone()).await?;
        if layer.id != self.root_overlay {
            return Err(ContentError::NotFound);
        }
        let row = rows::one(
            tx,
            SnapshotTable::ReadingViewRevision,
            &["view_id", "id"],
            &[view.view_id, view.revision_id],
        )
        .await?;
        let identity = rows::one(tx, SnapshotTable::ReadingView, &["id"], &[view.view_id]).await?;
        self.add(SnapshotTable::ReadingView, identity)?;
        if let Some(parent) = optional_uuid(&row, "parent_revision_id")? {
            self.parent(
                Node::Reading(view.clone()),
                Node::Reading(ReadingRef {
                    view_id: view.view_id,
                    revision_id: parent,
                }),
            )?;
        }
        self.enqueue(Node::Overlay(layer.id, layer.revision));
        let (_, choices) = selection::choices(tx, view).await?;
        selection::authorize(tx, self.actor, layer.id, &choices, &mut self.session).await?;
        for exact in choices.references() {
            self.enqueue(Node::Exact(exact));
        }
        self.attached(
            tx,
            SnapshotTable::ReadingRelationSelection,
            &["view_id", "view_revision_id"],
            &[view.view_id, view.revision_id],
        )
        .await?;
        self.attached(
            tx,
            SnapshotTable::ReadingEpistemicSelection,
            &["view_id", "view_revision_id"],
            &[view.view_id, view.revision_id],
        )
        .await?;
        self.add(SnapshotTable::ReadingViewRevision, row)
    }

    async fn overlay(
        &mut self,
        tx: &mut Tx<'_>,
        id: Uuid,
        revision: Uuid,
    ) -> Result<(), ContentError> {
        // Exact overlays are selected by saved reading revisions/parent pointers,
        // never by following the container's mutable head.
        let identity = rows::one(tx, SnapshotTable::Overlay, &["id"], &[id]).await?;
        self.grant(tx, uuid(&identity, "space_id")?).await?;
        if uuid(&identity, "owner_id")? != self.actor.actor_id || id != self.root_overlay {
            return Err(ContentError::NotFound);
        }
        let row = rows::one(
            tx,
            SnapshotTable::OverlayRevision,
            &["overlay_id", "id"],
            &[id, revision],
        )
        .await?;
        self.add(SnapshotTable::Overlay, identity)?;
        if let Some(parent) = optional_uuid(&row, "parent_revision_id")? {
            self.parent(Node::Overlay(id, revision), Node::Overlay(id, parent))?;
        }
        self.composition_root(CompositionRef {
            composition_id: uuid(&row, "root_composition_id")?,
            revision_id: uuid(&row, "base_revision_id")?,
        });
        for group in rows::many(
            tx,
            SnapshotTable::OverlayGroup,
            &["overlay_id", "overlay_revision_id"],
            &[id, revision],
        )
        .await?
        {
            let group_id = uuid(&group, "group_id")?;
            self.group_identity(tx, id, group_id).await?;
            self.composition_root(CompositionRef {
                composition_id: uuid(&group, "root_composition_id")?,
                revision_id: uuid(&group, "base_revision_id")?,
            });
            self.add(SnapshotTable::OverlayGroup, group)?;
        }
        for placement in rows::many(
            tx,
            SnapshotTable::OverlayPlacement,
            &["overlay_id", "overlay_revision_id"],
            &[id, revision],
        )
        .await?
        {
            let placement_id = uuid(&placement, "placement_id")?;
            let identity = rows::one(
                tx,
                SnapshotTable::OverlayPlacementIdentity,
                &["overlay_id", "placement_id"],
                &[id, placement_id],
            )
            .await?;
            self.add(SnapshotTable::OverlayPlacementIdentity, identity)?;
            self.enqueue(Node::Exact(ExactRef::Block(BlockRef {
                block_id: uuid(&placement, "block_id")?,
                revision_id: uuid(&placement, "block_revision_id")?,
            })));
            self.add(SnapshotTable::OverlayPlacement, placement)?;
        }
        // Manual placement facts have hard FKs to both group identities, even
        // when a source group has no active row in this particular revision.
        for decision in rows::many(
            tx,
            SnapshotTable::PlacementManualDecision,
            &["overlay_id", "overlay_revision_id"],
            &[id, revision],
        )
        .await?
        {
            self.group_identity(tx, id, uuid(&decision, "source_group_id")?)
                .await?;
            self.group_identity(tx, id, uuid(&decision, "result_group_id")?)
                .await?;
            self.add(SnapshotTable::PlacementManualDecision, decision)?;
        }
        self.add(SnapshotTable::OverlayRevision, row)
    }
    async fn group_identity(
        &mut self,
        tx: &mut Tx<'_>,
        overlay: Uuid,
        group: Uuid,
    ) -> Result<(), ContentError> {
        let row = rows::one(
            tx,
            SnapshotTable::OverlayGroupIdentity,
            &["overlay_id", "group_id"],
            &[overlay, group],
        )
        .await?;
        self.add(SnapshotTable::OverlayGroupIdentity, row)
    }

    async fn composition(
        &mut self,
        tx: &mut Tx<'_>,
        reference: &CompositionRef,
    ) -> Result<(), ContentError> {
        // Reuse the existing direct authorization/materialization primitive. The
        // package owns its cumulative occurrence/depth/object accounting.
        let (space, typed) = closure::load_composition(tx, self.actor, reference).await?;
        self.required_spaces.insert(space);
        let row = rows::one(
            tx,
            SnapshotTable::CompositionRevision,
            &["composition_id", "id"],
            &[reference.composition_id, reference.revision_id],
        )
        .await?;
        self.budget
            .charge(SnapshotBudgetKind::CompositionObject, 0, 1)?;
        let identity = rows::one(
            tx,
            SnapshotTable::Composition,
            &["id"],
            &[reference.composition_id],
        )
        .await?;
        self.add(SnapshotTable::Composition, identity)?;
        if let Some(parent) = optional_uuid(&row, "parent_revision_id")? {
            let previous = CompositionRef {
                composition_id: reference.composition_id,
                revision_id: parent,
            };
            self.parent(
                Node::Composition(reference.clone()),
                Node::Composition(previous.clone()),
            )?;
            self.composition_roots.insert(previous);
        }
        for node in &typed.nodes {
            match &node.target {
                NodeTarget::Block(block) => {
                    self.enqueue(Node::Exact(ExactRef::Block(block.clone())))
                }
                NodeTarget::Composition(child) => self.enqueue(Node::Composition(child.clone())),
            }
        }
        self.attached(
            tx,
            SnapshotTable::CompositionOccurrence,
            &["composition_revision_id"],
            &[reference.revision_id],
        )
        .await?;
        self.compositions.insert(reference.clone(), typed);
        self.add(SnapshotTable::CompositionRevision, row)
    }

    async fn exact(&mut self, tx: &mut Tx<'_>, reference: &ExactRef) -> Result<(), ContentError> {
        if !self.session.authorize(tx, self.actor, reference).await? {
            return Err(ContentError::NotFound);
        }
        self.budget.charge(
            SnapshotBudgetKind::ReferenceObject,
            self.session.bytes(reference)?,
            1,
        )?;
        let registry = rows::references(tx, SnapshotTable::ReferenceObject, reference).await?;
        if registry.len() != 1 {
            return Err(ContentError::NotFound);
        }
        for row in registry {
            self.add(SnapshotTable::ReferenceObject, row)?;
        }
        for dependency in
            rows::references(tx, SnapshotTable::ReferenceDependency, reference).await?
        {
            let target = rows::dependency_target(tx, &dependency).await?;
            // Different dependency roles may point to the same exact target.
            // Count every stored edge, while graph reachability deduplicates it.
            self.budget
                .charge(SnapshotBudgetKind::ReferenceEdge, 0, 1)?;
            self.exact_edges
                .entry(Node::Exact(reference.clone()))
                .or_default()
                .insert(Node::Exact(target.clone()));
            self.enqueue(Node::Exact(target));
            self.add(SnapshotTable::ReferenceDependency, dependency)?;
        }
        match reference {
            ExactRef::Block(block) => self.block(tx, block).await,
            ExactRef::Relation(relation) => self.relation(tx, relation).await,
            ExactRef::RelationReview(review) => self.relation_review(tx, review).await,
            ExactRef::EpistemicReview(review) => self.epistemic(tx, review).await,
        }
    }

    async fn block(&mut self, tx: &mut Tx<'_>, block: &BlockRef) -> Result<(), ContentError> {
        let row = rows::one(
            tx,
            SnapshotTable::BlockRevision,
            &["block_id", "id"],
            &[block.block_id, block.revision_id],
        )
        .await?;
        if let Some(parent) = optional_uuid(&row, "parent_revision_id")? {
            self.parent(
                Node::Exact(ExactRef::Block(block.clone())),
                Node::Exact(ExactRef::Block(BlockRef {
                    block_id: block.block_id,
                    revision_id: parent,
                })),
            )?;
        }
        let identity = rows::one(tx, SnapshotTable::Block, &["id"], &[block.block_id]).await?;
        self.add(SnapshotTable::Block, identity)?;
        let draft = ContentDraft::decode(
            row["contract_version"]
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .ok_or(ContentError::Storage)?,
            row["content"].clone(),
        )?;
        if draft.digest() != string(&row, "content_sha256")? {
            return Err(ContentError::Storage);
        }
        let uses = rows::many(
            tx,
            SnapshotTable::BlockAssetUse,
            &["block_id", "revision_id"],
            &[block.block_id, block.revision_id],
        )
        .await?;
        if let Some(asset) = draft.asset_ref() {
            if uses.len() != 1
                || uuid(&uses[0], "asset_id")? != asset.asset_id
                || uuid(&uses[0], "space_id")? != asset.space_id
            {
                return Err(ContentError::Storage);
            }
            self.asset(tx, AssetUseRef::Block(block.clone())).await?;
        } else if !uses.is_empty() {
            return Err(ContentError::Storage);
        }
        for usage in uses {
            self.add(SnapshotTable::BlockAssetUse, usage)?;
        }
        self.add(SnapshotTable::BlockRevision, row)
    }

    fn personal_scope(&self, identity: &Value) -> Result<(), ContentError> {
        if let Some(overlay) = optional_uuid(identity, "overlay_id")?
            && overlay != self.root_overlay
        {
            // Scope has only a container ID: do not chase an unrelated head to
            // fabricate an exact overlay closure for a newly imported container.
            return Err(ContentError::NotFound);
        }
        Ok(())
    }
    async fn relation(
        &mut self,
        tx: &mut Tx<'_>,
        reference: &RelationRef,
    ) -> Result<(), ContentError> {
        let identity = rows::one(
            tx,
            SnapshotTable::Relation,
            &["id"],
            &[reference.relation_id],
        )
        .await?;
        self.personal_scope(&identity)?;
        self.add(SnapshotTable::Relation, identity)?;
        let row = rows::one(
            tx,
            SnapshotTable::RelationRevision,
            &["relation_id", "id"],
            &[reference.relation_id, reference.revision_id],
        )
        .await?;
        if let Some(parent) = optional_uuid(&row, "parent_revision_id")? {
            self.parent(
                Node::Exact(ExactRef::Relation(reference.clone())),
                Node::Exact(ExactRef::Relation(RelationRef {
                    relation_id: reference.relation_id,
                    revision_id: parent,
                })),
            )?;
        }
        self.add(SnapshotTable::RelationRevision, row)
    }
    async fn relation_review(
        &mut self,
        tx: &mut Tx<'_>,
        reference: &RelationReviewRef,
    ) -> Result<(), ContentError> {
        let row = rows::one(
            tx,
            SnapshotTable::RelationReview,
            &["relation_id", "relation_revision_id", "id"],
            &[
                reference.relation.relation_id,
                reference.relation.revision_id,
                reference.review_id,
            ],
        )
        .await?;
        let identity = rows::one(
            tx,
            SnapshotTable::RelationReviewHead,
            &["relation_id", "relation_revision_id"],
            &[
                reference.relation.relation_id,
                reference.relation.revision_id,
            ],
        )
        .await?;
        self.add(SnapshotTable::RelationReviewHead, identity)?;
        if let Some(parent) = optional_uuid(&row, "previous_review_id")? {
            self.parent(
                Node::Exact(ExactRef::RelationReview(reference.clone())),
                Node::Exact(ExactRef::RelationReview(RelationReviewRef {
                    relation: reference.relation.clone(),
                    review_id: parent,
                })),
            )?;
        }
        self.add(SnapshotTable::RelationReview, row)
    }
    async fn epistemic(
        &mut self,
        tx: &mut Tx<'_>,
        reference: &EpistemicReviewRef,
    ) -> Result<(), ContentError> {
        let identity = rows::one(
            tx,
            SnapshotTable::EpistemicStream,
            &["id"],
            &[reference.stream_id],
        )
        .await?;
        self.personal_scope(&identity)?;
        self.add(SnapshotTable::EpistemicStream, identity)?;
        let row = rows::one(
            tx,
            SnapshotTable::EpistemicReview,
            &["stream_id", "id"],
            &[reference.stream_id, reference.review_id],
        )
        .await?;
        if let Some(parent) = optional_uuid(&row, "previous_review_id")? {
            self.parent(
                Node::Exact(ExactRef::EpistemicReview(reference.clone())),
                Node::Exact(ExactRef::EpistemicReview(EpistemicReviewRef {
                    stream_id: reference.stream_id,
                    review_id: parent,
                })),
            )?;
        }
        self.add(SnapshotTable::EpistemicReview, row)
    }

    async fn resource(
        &mut self,
        tx: &mut Tx<'_>,
        reference: &ResourceVersionRef,
    ) -> Result<(), ContentError> {
        self.grant(tx, reference.space_id).await?;
        let row = rows::one(
            tx,
            SnapshotTable::ResourceVersion,
            &["space_id", "resource_id", "id"],
            &[
                reference.space_id,
                reference.resource_id,
                reference.version_id,
            ],
        )
        .await?;
        let resource = rows::one(
            tx,
            SnapshotTable::Resource,
            &["space_id", "id"],
            &[reference.space_id, reference.resource_id],
        )
        .await?;
        self.add(SnapshotTable::Resource, resource)?;
        self.asset(tx, AssetUseRef::Resource(reference.clone()))
            .await?;
        self.add(SnapshotTable::ResourceVersion, row)
    }
    async fn segment(
        &mut self,
        tx: &mut Tx<'_>,
        reference: &SourceSegmentRef,
    ) -> Result<(), ContentError> {
        self.grant(tx, reference.space_id).await?;
        let row = rows::one(
            tx,
            SnapshotTable::SourceSegment,
            &["space_id", "resource_id", "resource_version_id", "id"],
            &[
                reference.space_id,
                reference.resource_id,
                reference.version_id,
                reference.segment_id,
            ],
        )
        .await?;
        self.enqueue(Node::Resource(ResourceVersionRef {
            space_id: reference.space_id,
            resource_id: reference.resource_id,
            version_id: reference.version_id,
        }));
        self.add(SnapshotTable::SourceSegment, row)
    }
    async fn asset(&mut self, tx: &mut Tx<'_>, use_ref: AssetUseRef) -> Result<(), ContentError> {
        if self.assets.contains_key(&use_ref) {
            return Ok(());
        }
        let asset = assets::read_for_use_in_tx(tx, self.actor, use_ref.clone(), &mut self.session)
            .await?
            .ok_or(ContentError::NotFound)?;
        let row = rows::one(
            tx,
            SnapshotTable::Asset,
            &["space_id", "id"],
            &[asset.reference.space_id, asset.reference.asset_id],
        )
        .await?;
        let size = u64::try_from(asset.byte_size).map_err(storage)?;
        let path = format!("assets/{}", asset.storage_key);
        let byte_size = usize::try_from(size).map_err(storage)?;
        if byte_size > SNAPSHOT_MAX_ASSET_FILE_BYTES {
            return Err(limit());
        }
        if self.include_originals && !self.asset_files.contains_key(&path) {
            self.budget
                .charge(SnapshotBudgetKind::AssetFile, byte_size, 0)?;
            self.asset_files.insert(
                path.clone(),
                SnapshotFile {
                    path: format!("assets/{}", asset.storage_key),
                    size,
                    sha256: asset.sha256.clone(),
                },
            );
        }
        if self
            .asset_files
            .get(&path)
            .is_some_and(|file| file.size != size || file.sha256 != asset.sha256)
        {
            return Err(ContentError::Storage);
        }
        self.assets.insert(
            use_ref.clone(),
            SnapshotAssetUse {
                use_ref,
                asset: asset.reference,
                sha256: asset.sha256,
                byte_size: size,
                storage_key: asset.storage_key,
            },
        );
        self.add(SnapshotTable::Asset, row)
    }

    fn check_depth(&self) -> Result<(), ContentError> {
        // Audit predecessors must terminate at a root even though semantic
        // reference graphs are allowed to contain bounded cycles.
        for root in self.history_parents.keys() {
            let mut path = BTreeSet::new();
            let mut node = root;
            loop {
                if !path.insert(node) || path.len() > SNAPSHOT_MAX_REFERENCE_DEPTH {
                    return Err(limit());
                }
                match self.history_parents.get(node) {
                    Some(parent) => node = parent,
                    None => break,
                }
            }
        }
        // Depth is evaluated after all roots are loaded, so revisiting a shared
        // ancestor never makes a deeper path escape the package limit.
        // Kahn's longest-path pass keeps a diamond DAG linear in its edges.
        let mut incoming = BTreeMap::<Node, usize>::new();
        for (node, edges) in &self.exact_edges {
            incoming.entry(node.clone()).or_default();
            for target in edges {
                *incoming.entry(target.clone()).or_default() += 1;
            }
        }
        let mut queue: VecDeque<_> = incoming
            .iter()
            .filter(|(_, count)| **count == 0)
            .map(|(node, _)| node.clone())
            .collect();
        let mut depths: BTreeMap<_, _> = incoming
            .keys()
            .cloned()
            .map(|node| (node, 1usize))
            .collect();
        let mut count = 0;
        while let Some(node) = queue.pop_front() {
            count += 1;
            let depth = depths[&node];
            if depth > SNAPSHOT_MAX_REFERENCE_DEPTH {
                return Err(limit());
            }
            if let Some(edges) = self.exact_edges.get(&node) {
                for target in edges {
                    depths
                        .entry(target.clone())
                        .and_modify(|old| *old = (*old).max(depth + 1));
                    let remaining = incoming.get_mut(target).ok_or(ContentError::Storage)?;
                    *remaining -= 1;
                    if *remaining == 0 {
                        queue.push_back(target.clone());
                    }
                }
            }
        }
        if count == incoming.len() {
            return Ok(());
        }
        // Necessary reference cycles are valid; bounded simple paths retain the
        // existing Session semantics without unbounded cycle expansion.
        let mut work = 0usize;
        for root in self.exact_edges.keys() {
            let mut pending = vec![(root.clone(), Vec::<Node>::new())];
            while let Some((node, mut path)) = pending.pop() {
                work += 1;
                if work > 131_072 {
                    return Err(limit());
                }
                if path.contains(&node) {
                    continue;
                }
                if path.len() >= SNAPSHOT_MAX_REFERENCE_DEPTH {
                    return Err(limit());
                }
                path.push(node.clone());
                if let Some(edges) = self.exact_edges.get(&node) {
                    pending.extend(edges.iter().cloned().map(|edge| (edge, path.clone())));
                }
            }
        }
        Ok(())
    }
    fn check_occurrences(&mut self) -> Result<(), ContentError> {
        for root in &self.composition_roots {
            let mut stack = vec![(root.clone(), Vec::<Uuid>::new())];
            while let Some((reference, mut path)) = stack.pop() {
                if path.contains(&reference.composition_id)
                    || path.len() >= SNAPSHOT_MAX_COMPOSITION_DEPTH
                {
                    return Err(limit());
                }
                path.push(reference.composition_id);
                let revision = self
                    .compositions
                    .get(&reference)
                    .ok_or(ContentError::Storage)?;
                for occurrence in &revision.nodes {
                    self.budget
                        .charge(SnapshotBudgetKind::CompositionOccurrence, 0, path.len())?;
                    if let NodeTarget::Composition(child) = &occurrence.target {
                        stack.push((child.clone(), path.clone()));
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collector() -> Collector {
        Collector {
            actor: Principal {
                actor_id: Uuid::from_u128(1),
            },
            root_overlay: Uuid::from_u128(2),
            session: references::Session::default(),
            budget: SnapshotBudget::default(),
            pending: VecDeque::new(),
            visited: BTreeSet::new(),
            rows: BTreeMap::new(),
            assets: BTreeMap::new(),
            asset_files: BTreeMap::new(),
            include_originals: false,
            required_spaces: BTreeSet::new(),
            exact_edges: BTreeMap::new(),
            history_parents: BTreeMap::new(),
            compositions: BTreeMap::new(),
            composition_roots: BTreeSet::new(),
        }
    }
    fn block(n: u128) -> Node {
        Node::Exact(ExactRef::Block(BlockRef {
            block_id: Uuid::from_u128(n),
            revision_id: Uuid::from_u128(n),
        }))
    }
    #[test]
    fn shared_diamond_depth_is_bounded_without_enumerating_paths() {
        let mut c = collector();
        for layer in 0..31 {
            for branch in 0..2 {
                c.edge(block(2 * layer + branch), block(2 * layer + 2))
                    .unwrap();
                c.edge(block(2 * layer + branch), block(2 * layer + 3))
                    .unwrap();
            }
        }
        assert!(c.check_depth().is_ok());
        c.edge(block(63), block(64)).unwrap();
        assert!(
            matches!(c.check_depth(), Err(ContentError::Invalid(ref code)) if code == "snapshot_limit_exceeded")
        );
    }
    #[test]
    fn ancestors_and_required_edges_share_depth_limit() {
        let mut c = collector();
        for n in 1..32 {
            c.edge(block(n), block(n + 1)).unwrap();
        }
        assert!(c.check_depth().is_ok());
        c.edge(block(32), block(33)).unwrap();
        assert!(c.check_depth().is_err());
    }
    #[test]
    fn historical_cycles_are_rejected_while_reference_cycles_remain_bounded() {
        let mut c = collector();
        c.parent(block(1), block(2)).unwrap();
        c.parent(block(2), block(1)).unwrap();
        assert!(c.check_depth().is_err());
        let mut references = collector();
        references.edge(block(1), block(2)).unwrap();
        references.edge(block(2), block(1)).unwrap();
        assert!(references.check_depth().is_ok());
    }
}

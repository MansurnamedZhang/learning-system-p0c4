//! Public shape and visible-only limits for bounded revision impact queries.
use crate::{
    BlockRef, CompositionRef, ContentError, EpistemicReviewRef, ExactRef, ReadingMode, ReadingRef,
    RelationRef, RelationReviewRef, RelationType,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub const MAX_IMPACT_DEPTH: u8 = 8;
pub const MAX_IMPACT_CONSUMERS: u16 = 200;
pub const MAX_VISIBLE_EDGES: u32 = 8192;
pub const MAX_VISIBLE_NODES: usize = 2048;
pub const MAX_EXPLANATION_PATHS: u32 = 4096;
pub const MAX_PROJECTION_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_VISIBLE_WORK: u32 = 131072;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImpactFamily {
    Structural,
    Necessary,
    Semantic,
    ReviewSelection,
    Lineage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImpactScope {
    Reading { view: ReadingRef, mode: ReadingMode },
    Release { release_id: Uuid },
}

/// Membership does not by itself imply display or a direct citation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImpactMembership {
    Displayed,
    Selected,
    Context,
}

// Existing B3 refs have deliberately permissive wire decoding. The new query
// boundary uses exact wrappers so extra nested keys cannot become authority.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum StrictExactRef {
    Block {
        block_id: Uuid,
        revision_id: Uuid,
    },
    Relation {
        relation_id: Uuid,
        revision_id: Uuid,
    },
    RelationReview {
        relation: StrictRelationRef,
        review_id: Uuid,
    },
    EpistemicReview {
        stream_id: Uuid,
        review_id: Uuid,
    },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictRelationRef {
    relation_id: Uuid,
    revision_id: Uuid,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictReadingRef {
    view_id: Uuid,
    revision_id: Uuid,
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum StrictScope {
    Reading {
        view: StrictReadingRef,
        mode: ReadingMode,
    },
    Release {
        release_id: Uuid,
    },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawImpactQuery {
    start: StrictExactRef,
    scope: StrictScope,
    #[serde(default, deserialize_with = "present_families")]
    families: Option<Vec<ImpactFamily>>,
    #[serde(default = "default_depth")]
    max_depth: u8,
    #[serde(default = "default_limit")]
    limit: u16,
    #[serde(default = "default_work")]
    work_limit: u32,
    #[serde(default)]
    after: Option<ImpactSortKey>,
}
const fn default_depth() -> u8 {
    3
}
const fn default_limit() -> u16 {
    50
}
const fn default_work() -> u32 {
    4096
}
fn present_families<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<ImpactFamily>>, D::Error> {
    Vec::<ImpactFamily>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawImpactQuery")]
pub struct ImpactQuery {
    pub start: ExactRef,
    pub scope: ImpactScope,
    pub families: Vec<ImpactFamily>,
    pub max_depth: u8,
    pub limit: u16,
    pub work_limit: u32,
    pub after: Option<ImpactSortKey>,
}
impl TryFrom<RawImpactQuery> for ImpactQuery {
    type Error = ContentError;
    fn try_from(raw: RawImpactQuery) -> Result<Self, Self::Error> {
        let start = match raw.start {
            StrictExactRef::Block {
                block_id,
                revision_id,
            } => ExactRef::Block(BlockRef {
                block_id,
                revision_id,
            }),
            StrictExactRef::Relation {
                relation_id,
                revision_id,
            } => ExactRef::Relation(RelationRef {
                relation_id,
                revision_id,
            }),
            StrictExactRef::RelationReview {
                relation,
                review_id,
            } => ExactRef::RelationReview(RelationReviewRef {
                relation: RelationRef {
                    relation_id: relation.relation_id,
                    revision_id: relation.revision_id,
                },
                review_id,
            }),
            StrictExactRef::EpistemicReview {
                stream_id,
                review_id,
            } => ExactRef::EpistemicReview(EpistemicReviewRef {
                stream_id,
                review_id,
            }),
        };
        let scope = match raw.scope {
            StrictScope::Reading { view, mode } => ImpactScope::Reading {
                view: ReadingRef {
                    view_id: view.view_id,
                    revision_id: view.revision_id,
                },
                mode,
            },
            StrictScope::Release { release_id } => ImpactScope::Release { release_id },
        };
        let families = raw
            .families
            .unwrap_or_else(|| vec![ImpactFamily::Structural, ImpactFamily::Necessary]);
        if families.is_empty()
            || families.len() > 5
            || families.iter().collect::<BTreeSet<_>>().len() != families.len()
            || !(1..=MAX_IMPACT_DEPTH).contains(&raw.max_depth)
            || !(1..=MAX_IMPACT_CONSUMERS).contains(&raw.limit)
            || !(1..=MAX_VISIBLE_WORK).contains(&raw.work_limit)
        {
            return Err(ContentError::Invalid("invalid_impact_query_limits".into()));
        }
        Ok(Self {
            start,
            scope,
            families,
            max_depth: raw.max_depth,
            limit: raw.limit,
            work_limit: raw.work_limit,
            after: raw.after,
        })
    }
}
impl ImpactQuery {
    pub fn validate(&self) -> Result<(), ContentError> {
        let value = serde_json::to_value(self)
            .map_err(|_| ContentError::Invalid("invalid_impact_query".into()))?;
        serde_json::from_value::<Self>(value)
            .map(|_| ())
            .map_err(|_| ContentError::Invalid("invalid_impact_query".into()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ImpactNode {
    Block(BlockRef),
    Composition(CompositionRef),
    Reading { view_id: Uuid, revision_id: Uuid },
    Relation(RelationRef),
    RelationReview(RelationReviewRef),
    EpistemicReview(EpistemicReviewRef),
    Lineage { operation_id: Uuid },
    Release { release_id: Uuid },
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ImpactLocation {
    Occurrence {
        path: Vec<Uuid>,
    },
    Placement {
        view_id: Uuid,
        revision_id: Uuid,
        placement_id: Uuid,
    },
    Unplaced {
        view_id: Uuid,
        revision_id: Uuid,
        placement_id: Uuid,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TraversalDirection {
    SavedForward,
    SavedReverse,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImpactProvenance {
    Stored,
    FixedReadingSelection,
    FixedReleaseManifest,
    DynamicWorking,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImpactReason {
    ReferencesOldRevision,
    RequiresExactRevision,
    SuggestReview,
    SelectedEvidence,
    DerivedFrom,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImpactStep {
    pub from: ImpactNode,
    pub to: ImpactNode,
    pub family: ImpactFamily,
    pub direction: Option<TraversalDirection>,
    pub relation_type: Option<RelationType>,
    pub provenance: ImpactProvenance,
    pub location: Option<ImpactLocation>,
    pub reason: ImpactReason,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImpactExplanation {
    pub steps: Vec<ImpactStep>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImpactConsumerGroup {
    pub consumer: ImpactNode,
    pub locations: Vec<ImpactLocation>,
    pub explanations: Vec<ImpactExplanation>,
}

/// Stable visible ordering key. It contains only already-authorized path data.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct ImpactSortKey {
    pub hops: usize,
    pub path: Vec<String>,
    pub consumer: ImpactNode,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawImpactSortKey {
    hops: usize,
    path: Vec<String>,
    consumer: ImpactNode,
}
impl<'de> Deserialize<'de> for ImpactSortKey {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawImpactSortKey::deserialize(deserializer)?;
        if raw.hops != raw.path.len()
            || raw.hops > usize::from(MAX_IMPACT_DEPTH)
            || raw
                .path
                .iter()
                .any(|s| s.len() > 2048 || !s.starts_with("0"))
        {
            return Err(serde::de::Error::custom("invalid_impact_cursor"));
        }
        Ok(Self {
            hops: raw.hops,
            path: raw.path,
            consumer: raw.consumer,
        })
    }
}
fn step_key(step: &ImpactStep) -> String {
    let family_order = match step.family {
        ImpactFamily::Structural => 0,
        ImpactFamily::Necessary => 1,
        ImpactFamily::Semantic => 2,
        ImpactFamily::ReviewSelection => 3,
        ImpactFamily::Lineage => 4,
    };
    format!(
        "0{family_order}|{}",
        serde_json::to_string(step).expect("impact step serializes")
    )
}
impl ImpactConsumerGroup {
    pub fn sort_key(&self) -> ImpactSortKey {
        let best = self
            .explanations
            .iter()
            .map(|e| {
                let path = e.steps.iter().map(step_key).collect::<Vec<_>>();
                (path.len(), path)
            })
            .min()
            .unwrap_or((usize::MAX, vec![]));
        ImpactSortKey {
            hops: best.0,
            path: best.1,
            consumer: self.consumer.clone(),
        }
    }
    fn normalize(&mut self) {
        self.locations.sort();
        self.locations.dedup();
        self.explanations
            .sort_by_key(|e| e.steps.iter().map(step_key).collect::<Vec<_>>());
        self.explanations.dedup();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PageStatus {
    Complete,
    Truncated { after: ImpactSortKey },
    BudgetExceeded,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImpactPage {
    pub consumers: Vec<ImpactConsumerGroup>,
    pub status: PageStatus,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImpactResult {
    start: ExactRef,
    start_membership: ImpactMembership,
    actual_scope: ImpactScope,
    consumers: Vec<ImpactConsumerGroup>,
    status: PageStatus,
}
impl ImpactResult {
    pub fn from_page(
        start: ExactRef,
        start_membership: ImpactMembership,
        actual_scope: ImpactScope,
        page: ImpactPage,
    ) -> Self {
        let consumers = if matches!(&page.status, PageStatus::BudgetExceeded) {
            vec![]
        } else {
            page.consumers
        };
        Self {
            start,
            start_membership,
            actual_scope,
            consumers,
            status: page.status,
        }
    }
    pub fn budget_exceeded(
        start: ExactRef,
        start_membership: ImpactMembership,
        actual_scope: ImpactScope,
    ) -> Self {
        Self {
            start,
            start_membership,
            actual_scope,
            consumers: vec![],
            status: PageStatus::BudgetExceeded,
        }
    }
    pub fn start(&self) -> &ExactRef {
        &self.start
    }
    pub fn start_membership(&self) -> ImpactMembership {
        self.start_membership
    }
    pub fn actual_scope(&self) -> &ImpactScope {
        &self.actual_scope
    }
    pub fn consumers(&self) -> &[ImpactConsumerGroup] {
        &self.consumers
    }
    pub fn status(&self) -> &PageStatus {
        &self.status
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetExceeded;
#[derive(Debug, Clone)]
pub struct VisibleWorkBudget {
    work_limit: u32,
    work: u32,
    edges: u32,
    nodes: BTreeSet<ImpactNode>,
    paths: u32,
    bytes: usize,
}
impl VisibleWorkBudget {
    pub fn new(work_limit: u32) -> Self {
        Self {
            work_limit: work_limit.min(MAX_VISIBLE_WORK),
            work: 0,
            edges: 0,
            nodes: BTreeSet::new(),
            paths: 0,
            bytes: 0,
        }
    }
    pub fn charge_step(&mut self) -> Result<(), BudgetExceeded> {
        if self.work >= self.work_limit {
            return Err(BudgetExceeded);
        }
        self.work += 1;
        Ok(())
    }
    pub fn charge_edge(&mut self) -> Result<(), BudgetExceeded> {
        if self.edges >= MAX_VISIBLE_EDGES {
            return Err(BudgetExceeded);
        }
        self.charge_step()?;
        self.edges += 1;
        Ok(())
    }
    pub fn charge_node(&mut self, node: ImpactNode) -> Result<(), BudgetExceeded> {
        if self.nodes.contains(&node) {
            return Ok(());
        }
        if self.nodes.len() >= MAX_VISIBLE_NODES {
            return Err(BudgetExceeded);
        }
        self.charge_step()?;
        self.nodes.insert(node);
        Ok(())
    }
    pub fn charge_path(&mut self) -> Result<(), BudgetExceeded> {
        if self.paths >= MAX_EXPLANATION_PATHS {
            return Err(BudgetExceeded);
        }
        self.charge_step()?;
        self.paths += 1;
        Ok(())
    }
    pub fn charge_payload_bytes(&mut self, bytes: usize) -> Result<(), BudgetExceeded> {
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .filter(|n| *n <= MAX_PROJECTION_BYTES)
            .ok_or(BudgetExceeded)?;
        Ok(())
    }
    pub fn edges(&self) -> u32 {
        self.edges
    }
    pub fn nodes(&self) -> usize {
        self.nodes.len()
    }
    pub fn work(&self) -> u32 {
        self.work
    }
    pub fn paths(&self) -> u32 {
        self.paths
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

/// Input must already contain *all and only* authorized visible groups from
/// the fixed scope. The caller charges traversal work before passing them here.
pub fn paginate_visible_groups(
    mut groups: Vec<ImpactConsumerGroup>,
    after: Option<&ImpactSortKey>,
    limit: u16,
    budget: &mut VisibleWorkBudget,
) -> Result<ImpactPage, BudgetExceeded> {
    if limit == 0 || limit > MAX_IMPACT_CONSUMERS {
        return Err(BudgetExceeded);
    }
    let mut merged: BTreeMap<ImpactNode, ImpactConsumerGroup> = BTreeMap::new();
    for group in groups.drain(..) {
        let entry = merged
            .entry(group.consumer.clone())
            .or_insert_with(|| ImpactConsumerGroup {
                consumer: group.consumer.clone(),
                locations: vec![],
                explanations: vec![],
            });
        entry.locations.extend(group.locations);
        entry.explanations.extend(group.explanations);
    }
    let mut groups: Vec<_> = merged
        .into_values()
        .filter(|g| !g.explanations.is_empty())
        .collect();
    for group in &mut groups {
        group.normalize();
    }
    groups.sort_by_key(ImpactConsumerGroup::sort_key);
    let mut eligible = groups
        .into_iter()
        .filter(|g| after.is_none_or(|a| g.sort_key() > *a))
        .peekable();
    let mut consumers = Vec::new();
    while consumers.len() < usize::from(limit) {
        let Some(group) = eligible.next() else {
            break;
        };
        // Reserve the entire consumer before exposing any of it.
        for _ in &group.explanations {
            budget.charge_path()?;
        }
        budget.charge_payload_bytes(
            serde_json::to_vec(&group)
                .expect("impact group serializes")
                .len(),
        )?;
        consumers.push(group);
    }
    let status = if eligible.peek().is_some() {
        PageStatus::Truncated {
            after: consumers.last().expect("positive limit").sort_key(),
        }
    } else {
        PageStatus::Complete
    };
    Ok(ImpactPage { consumers, status })
}

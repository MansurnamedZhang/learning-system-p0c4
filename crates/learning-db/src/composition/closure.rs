use crate::{references, storage};
use chrono::{DateTime, Utc};
use learning_core::*;
use sqlx::{Postgres, Transaction};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub(crate) const BODY_LIMIT: usize = 8 * 1024 * 1024;
pub(crate) struct Closure {
    pub compositions: BTreeMap<CompositionRef, (Uuid, CompositionRevision)>,
    pub blocks: BTreeMap<BlockRef, (Uuid, ContentRevision)>,
    reference_spaces: Vec<(Uuid, bool)>,
}
impl Closure {
    pub fn spaces(&self) -> Vec<(Uuid, bool)> {
        self.compositions
            .values()
            .map(|(s, _)| (*s, false))
            .chain(self.blocks.values().map(|(s, _)| (*s, false)))
            .chain(self.reference_spaces.iter().copied())
            .collect()
    }
    pub fn snapshot(self, root: CompositionRef) -> VersionedCompositionSnapshot {
        VersionedCompositionSnapshot {
            root,
            compositions: self.compositions.into_values().map(|(_, r)| r).collect(),
            blocks: self.blocks.into_values().map(|(_, r)| r).collect(),
        }
    }
    pub fn target_space(&self, target: &NodeTarget) -> Result<Uuid, ContentError> {
        match target {
            NodeTarget::Block(r) => self.blocks.get(r).map(|(s, _)| *s),
            NodeTarget::Composition(r) => self.compositions.get(r).map(|(s, _)| *s),
        }
        .ok_or(ContentError::Storage)
    }
}
#[derive(sqlx::FromRow)]
struct CompositionRow {
    id: Uuid,
    space_id: Uuid,
    composition_id: Uuid,
    parent_revision_id: Option<Uuid>,
    kind: String,
    title: String,
    content_sha256: String,
    author_id: Uuid,
    reason: String,
    created_at: DateTime<Utc>,
}
#[derive(sqlx::FromRow)]
struct NodeRow {
    occurrence_id: Uuid,
    block_id: Option<Uuid>,
    block_revision_id: Option<Uuid>,
    child_composition_id: Option<Uuid>,
    child_revision_id: Option<Uuid>,
}

pub(crate) async fn load_composition(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    reference: &CompositionRef,
) -> Result<(Uuid, CompositionRevision), ContentError> {
    let row=sqlx::query_as::<_,CompositionRow>("SELECT r.* FROM public.composition_revision r JOIN public.space_grant g ON g.space_id=r.space_id WHERE g.actor_id=$1 AND r.composition_id=$2 AND r.id=$3").bind(actor.actor_id).bind(reference.composition_id).bind(reference.revision_id).fetch_optional(&mut **tx).await.map_err(storage)?.ok_or(ContentError::NotFound)?;
    let rows=sqlx::query_as::<_,NodeRow>("SELECT occurrence_id,block_id,block_revision_id,child_composition_id,child_revision_id FROM public.composition_occurrence WHERE composition_revision_id=$1 ORDER BY position").bind(row.id).fetch_all(&mut **tx).await.map_err(storage)?;
    let mut nodes = Vec::with_capacity(rows.len());
    for n in rows {
        let target = match (
            n.block_id,
            n.block_revision_id,
            n.child_composition_id,
            n.child_revision_id,
        ) {
            (Some(block_id), Some(revision_id), None, None) => NodeTarget::Block(BlockRef {
                block_id,
                revision_id,
            }),
            (None, None, Some(composition_id), Some(revision_id)) => {
                NodeTarget::Composition(CompositionRef {
                    composition_id,
                    revision_id,
                })
            }
            _ => return Err(ContentError::Storage),
        };
        nodes.push(Occurrence {
            occurrence_id: n.occurrence_id,
            target,
        });
    }
    let kind = match row.kind.as_str() {
        "document" => CompositionKind::Document,
        "section" => CompositionKind::Section,
        _ => return Err(ContentError::Storage),
    };
    Ok((
        row.space_id,
        CompositionRevision {
            reference: CompositionRef {
                composition_id: row.composition_id,
                revision_id: row.id,
            },
            parent_revision_id: row.parent_revision_id,
            kind,
            title: row.title,
            nodes,
            content_sha256: row.content_sha256,
            author_id: row.author_id,
            reason: row.reason,
            created_at: row.created_at,
        },
    ))
}
fn invalid<T>(code: &str) -> Result<T, ContentError> {
    Err(ContentError::Invalid(code.into()))
}

/// Cache object reads, but walk/count every occurrence along its own ancestor path.
pub(crate) async fn load(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    roots: &[CompositionRef],
    draft: Option<(Uuid, &[NodeDraft])>,
) -> Result<Closure, ContentError> {
    load_with_session(tx, actor, roots, draft, &mut references::Session::default()).await
}
pub(crate) async fn load_with_session(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    roots: &[CompositionRef],
    draft: Option<(Uuid, &[NodeDraft])>,
    session: &mut references::Session,
) -> Result<Closure, ContentError> {
    let mut result = Closure {
        compositions: BTreeMap::new(),
        blocks: BTreeMap::new(),
        reference_spaces: vec![],
    };
    let mut stack: Vec<(NodeTarget, Vec<Uuid>)> = roots
        .iter()
        .rev()
        .map(|r| (NodeTarget::Composition(r.clone()), vec![]))
        .collect();
    let mut occurrences = 0usize;
    let mut bytes = 0usize;
    let synthetic = usize::from(draft.is_some());
    if let Some((id, nodes)) = draft {
        occurrences = nodes.len();
        for n in nodes.iter().rev() {
            stack.push((n.target.clone(), vec![id]));
        }
    }
    while let Some((target, path)) = stack.pop() {
        match target {
            NodeTarget::Block(reference) => {
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    result.blocks.entry(reference.clone())
                {
                    let (space, size, revision, spaces) = session
                        .assembly_block(tx, actor, &reference)
                        .await?
                        .ok_or(ContentError::NotFound)?;
                    result.reference_spaces.extend(spaces);
                    bytes += size;
                    if bytes > BODY_LIMIT {
                        return invalid("composition_body_limit");
                    }
                    entry.insert((space, revision));
                }
            }
            NodeTarget::Composition(reference) => {
                if !result.compositions.contains_key(&reference) {
                    let row = load_composition(tx, actor, &reference).await?;
                    result.compositions.insert(reference.clone(), row);
                }
                if path.contains(&reference.composition_id) {
                    return invalid("composition_cycle");
                }
                if path.len() >= 16 {
                    return invalid("composition_depth_limit");
                }
                let revision = &result.compositions[&reference].1;
                occurrences += revision.nodes.len();
                if occurrences > 4096 {
                    return invalid("composition_occurrences_limit");
                }
                let mut next = path;
                next.push(reference.composition_id);
                for n in revision.nodes.iter().rev() {
                    stack.push((n.target.clone(), next.clone()));
                }
            }
        }
        if result.compositions.len() + result.blocks.len() + synthetic > 2048 {
            return invalid("composition_objects_limit");
        }
    }
    Ok(result)
}
pub(crate) fn same_identity(a: &NodeTarget, b: &NodeTarget) -> bool {
    match (a, b) {
        (NodeTarget::Block(a), NodeTarget::Block(b)) => a.block_id == b.block_id,
        (NodeTarget::Composition(a), NodeTarget::Composition(b)) => {
            a.composition_id == b.composition_id
        }
        _ => false,
    }
}
pub(crate) fn merge_spaces(closures: &[&Closure], write_space: Uuid) -> Vec<(Uuid, bool)> {
    let mut set = BTreeSet::new();
    for c in closures {
        for (s, _) in c.spaces() {
            set.insert(s);
        }
    }
    let mut out: Vec<_> = set.into_iter().map(|s| (s, s == write_space)).collect();
    if !out.iter().any(|(s, _)| *s == write_space) {
        out.push((write_space, true));
    }
    out
}

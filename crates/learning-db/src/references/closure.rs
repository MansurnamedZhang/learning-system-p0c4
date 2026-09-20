use super::{
    index::{self, Fact},
    projection::{self, AuthorizedObject},
};
use learning_core::*;
use sqlx::{Postgres, Transaction};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

const OBJECTS: usize = 2048;
const EDGES: usize = 4096;
const BYTES: usize = 8 * 1024 * 1024;
const DEPTH: usize = 32;
const WORK: usize = 131072;
fn budget<T>() -> Result<T, ContentError> {
    Err(ContentError::Invalid("reference_budget_exceeded".into()))
}

pub(crate) struct AuthorizedClosure {
    pub objects: BTreeMap<ExactRef, (Uuid, AuthorizedObject)>,
    required_spaces: Vec<(Uuid, bool)>,
}
impl AuthorizedClosure {
    pub fn spaces(&self) -> Vec<(Uuid, bool)> {
        self.required_spaces.clone()
    }
}
/// One operation owns one session, including omitted roots and audit checks.
/// Only metadata is cached until a whole necessary closure is authorized.
#[derive(Default)]
pub(crate) struct Session {
    facts: BTreeMap<ExactRef, Option<Fact>>,
    selected: BTreeSet<ExactRef>,
    validated_roots: BTreeSet<ExactRef>,
    work: usize,
}
impl Session {
    pub fn checkpoint(&self) -> BTreeSet<ExactRef> {
        self.selected.clone()
    }
    pub fn restore(&mut self, selected: BTreeSet<ExactRef>) {
        self.selected = selected;
    }
    /// B1/B2 keep their separate root-body budgets. Only structured v2 roots
    /// join the additional operation-wide dependency budget; reachable v1 does.
    pub async fn assembly_block(
        &mut self,
        tx: &mut Transaction<'_, Postgres>,
        actor: Principal,
        r: &BlockRef,
    ) -> Result<Option<(Uuid, usize, ContentRevision, Vec<(Uuid, bool)>)>, ContentError> {
        let version:Option<i32>=sqlx::query_scalar("SELECT r.contract_version FROM public.block_revision r JOIN public.space_grant g ON g.space_id=r.space_id AND g.actor_id=$1 WHERE r.block_id=$2 AND r.id=$3")
            .bind(actor.actor_id).bind(r.block_id).bind(r.revision_id).fetch_optional(&mut **tx).await.map_err(crate::storage)?;
        let Some(version) = version else {
            return Ok(None);
        };
        let mut legacy = Session::default();
        let active = if version == 1 { &mut legacy } else { self };
        let exact = ExactRef::Block(r.clone());
        let Some(object) = active.object(tx, actor, &exact).await? else {
            return Ok(None);
        };
        Ok(Some((
            active.space(&exact)?,
            active.bytes(&exact)?,
            object.block()?,
            active.spaces(),
        )))
    }
    fn step(&mut self) -> Result<(), ContentError> {
        self.work += 1;
        if self.work > WORK {
            return budget();
        }
        Ok(())
    }
    pub async fn authorize(
        &mut self,
        tx: &mut Transaction<'_, Postgres>,
        actor: Principal,
        root: &ExactRef,
    ) -> Result<bool, ContentError> {
        self.step()?;
        // Reachability from another root does not prove this root's depth in
        // a cyclic graph. Restored budget checkpoints must also retain it.
        if self.selected.contains(root) && self.validated_roots.contains(root) {
            return Ok(true);
        }
        let mut reached = BTreeSet::new();
        let mut pending = vec![root.clone()];
        let mut edges = 0;
        while let Some(r) = pending.pop() {
            self.step()?;
            if !reached.insert(r.clone()) {
                continue;
            }
            if !self.facts.contains_key(&r) {
                let value = index::fact(tx, actor, &r).await?;
                self.facts.insert(r.clone(), value);
            }
            let Some(fact) = &self.facts[&r] else {
                return Ok(false);
            };
            edges += fact.edges.len();
            if reached.len() > OBJECTS || edges > EDGES {
                return budget();
            }
            pending.extend(fact.edges.iter().map(|(_, target)| target.clone()));
        }
        self.depth(root, &reached)?;
        let mut merged = self.selected.clone();
        merged.extend(reached);
        let mut bytes = 0;
        let mut edges = 0;
        for r in &merged {
            let fact = self.facts[r].as_ref().ok_or(ContentError::Storage)?;
            bytes += fact.bytes;
            edges += fact.edges.len();
        }
        if merged.len() > OBJECTS || edges > EDGES || bytes > BYTES {
            return budget();
        }
        self.selected = merged;
        self.validated_roots.insert(root.clone());
        Ok(true)
    }
    fn depth(&mut self, root: &ExactRef, reached: &BTreeSet<ExactRef>) -> Result<(), ContentError> {
        // Kahn order gives an exact longest path for DAGs without enumerating
        // diamond paths. Cyclic graphs use a separately hard-bounded path walk.
        let mut incoming: BTreeMap<_, usize> = reached.iter().map(|r| (r.clone(), 0)).collect();
        for r in reached {
            for (_, t) in &self.facts[r].as_ref().ok_or(ContentError::Storage)?.edges {
                *incoming.get_mut(t).ok_or(ContentError::Storage)? += 1;
            }
        }
        let mut queue: Vec<_> = incoming
            .iter()
            .filter(|(_, n)| **n == 0)
            .map(|(r, _)| r.clone())
            .collect();
        let mut lengths = BTreeMap::from([(root.clone(), 1usize)]);
        let mut visited = 0;
        while let Some(r) = queue.pop() {
            self.step()?;
            visited += 1;
            let depth = lengths.get(&r).copied().unwrap_or(1);
            if depth > DEPTH {
                return budget();
            }
            let targets: Vec<_> = self.facts[&r]
                .as_ref()
                .ok_or(ContentError::Storage)?
                .edges
                .iter()
                .map(|(_, t)| t.clone())
                .collect();
            for target in targets {
                self.step()?;
                lengths
                    .entry(target.clone())
                    .and_modify(|n| *n = (*n).max(depth + 1))
                    .or_insert(depth + 1);
                let n = incoming.get_mut(&target).ok_or(ContentError::Storage)?;
                *n -= 1;
                if *n == 0 {
                    queue.push(target);
                }
            }
        }
        if visited == reached.len() {
            return Ok(());
        }
        let mut stack = vec![(root.clone(), Vec::<ExactRef>::new())];
        while let Some((r, mut path)) = stack.pop() {
            self.step()?;
            if path.contains(&r) {
                continue;
            }
            if path.len() >= DEPTH {
                return budget();
            }
            path.push(r.clone());
            for (_, target) in &self.facts[&r].as_ref().ok_or(ContentError::Storage)?.edges {
                stack.push((target.clone(), path.clone()));
            }
        }
        Ok(())
    }
    pub async fn object(
        &mut self,
        tx: &mut Transaction<'_, Postgres>,
        actor: Principal,
        root: &ExactRef,
    ) -> Result<Option<AuthorizedObject>, ContentError> {
        if !self.authorize(tx, actor, root).await? {
            return Ok(None);
        }
        self.selected_object(tx, actor, root).await.map(Some)
    }
    /// Materialize a member of an authorized closure without promoting a
    /// dependency to a new operation root. Audit references remain independent.
    async fn selected_object(
        &mut self,
        tx: &mut Transaction<'_, Postgres>,
        actor: Principal,
        root: &ExactRef,
    ) -> Result<AuthorizedObject, ContentError> {
        if !self.selected.contains(root) {
            return Err(ContentError::Storage);
        }
        let mut object = projection::materialize(tx, root).await?;
        if let Some(previous) = object.previous()
            && !self.authorize(tx, actor, &previous).await?
        {
            object.redact_previous();
        }
        Ok(object)
    }
    pub fn space(&self, r: &ExactRef) -> Result<Uuid, ContentError> {
        self.facts
            .get(r)
            .and_then(Option::as_ref)
            .map(|f| f.space)
            .ok_or(ContentError::Storage)
    }
    pub fn bytes(&self, r: &ExactRef) -> Result<usize, ContentError> {
        self.facts
            .get(r)
            .and_then(Option::as_ref)
            .map(|f| f.bytes)
            .ok_or(ContentError::Storage)
    }
    pub fn spaces(&self) -> Vec<(Uuid, bool)> {
        self.selected
            .iter()
            .filter_map(|r| {
                self.facts
                    .get(r)
                    .and_then(Option::as_ref)
                    .map(|f| (f.space, false))
            })
            .collect()
    }
}
pub(crate) async fn load(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    roots: &[ExactRef],
) -> Result<AuthorizedClosure, ContentError> {
    let mut session = Session::default();
    for r in roots {
        if !session.authorize(tx, actor, r).await? {
            return Err(ContentError::NotFound);
        }
    }
    let selected: Vec<_> = session.selected.iter().cloned().collect();
    let mut objects = BTreeMap::new();
    for r in selected {
        let object = session.selected_object(tx, actor, &r).await?;
        objects.insert(r.clone(), (session.space(&r)?, object));
    }
    Ok(AuthorizedClosure {
        objects,
        required_spaces: session.spaces(),
    })
}

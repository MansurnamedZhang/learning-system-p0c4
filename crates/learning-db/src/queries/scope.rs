use crate::{composition::closure, overlay::model, reading::selection, references, release};
use learning_core::*;
use sqlx::{Postgres, Transaction};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Default)]
pub(super) struct FixedScope {
    pub members: BTreeMap<ImpactStart, ImpactMembership>,
    /// Authorized, exact saved reading revisions. Fixed selections are
    /// subsequently projected per view, preserving each review pairing.
    pub reading_views: BTreeSet<ReadingRef>,
    /// Every root-to-composition occurrence prefix, including repeated child
    /// compositions. An empty prefix is the selected root itself.
    pub composition_paths: BTreeMap<CompositionRef, BTreeSet<Vec<Uuid>>>,
    /// The exact saved base of a readable fixed view. Manifest anchors never
    /// add this edge, and personal-only access does not expose the base.
    pub reading_bases: BTreeMap<CompositionRef, BTreeSet<ImpactNode>>,
    pub placements: BTreeMap<BlockRef, Vec<ImpactLocation>>,
    pub reading_locations: BTreeMap<BlockRef, Vec<(ImpactNode, ImpactLocation)>>,
    pub compositions: BTreeMap<CompositionRef, CompositionRevision>,
}
impl FixedScope {
    fn member(&mut self, reference: ImpactStart, level: ImpactMembership) {
        use ImpactMembership::*;
        self.members
            .entry(reference)
            .and_modify(|old| {
                if matches!(level, Displayed)
                    || matches!(level, Selected) && matches!(*old, Context)
                {
                    *old = level;
                }
            })
            .or_insert(level);
    }
    fn add_snapshot(
        &mut self,
        snapshot: VersionedCompositionSnapshot,
        displayed: bool,
    ) -> Result<(), ContentError> {
        let revisions: BTreeMap<_, _> = snapshot
            .compositions
            .into_iter()
            .map(|c| (c.reference.clone(), c))
            .collect();
        let mut stack = vec![(snapshot.root, Vec::<Uuid>::new())];
        let mut seen_occurrences = 0usize;
        while let Some((reference, path)) = stack.pop() {
            let composition = revisions.get(&reference).ok_or(ContentError::Storage)?;
            self.member(
                ImpactStart::Composition(reference.clone()),
                ImpactMembership::Context,
            );
            self.composition_paths
                .entry(reference.clone())
                .or_default()
                .insert(path.clone());
            self.compositions.insert(reference, composition.clone());
            for node in composition.nodes.iter().rev() {
                seen_occurrences += 1;
                if seen_occurrences > 4096 {
                    return Err(ContentError::Storage);
                }
                let mut next = path.clone();
                next.push(node.occurrence_id);
                match &node.target {
                    NodeTarget::Composition(child) => stack.push((child.clone(), next)),
                    NodeTarget::Block(block) if displayed => self.member(
                        ImpactStart::Block(block.clone()),
                        ImpactMembership::Displayed,
                    ),
                    NodeTarget::Block(_) => {}
                }
            }
        }
        Ok(())
    }
    async fn add_reading(
        &mut self,
        tx: &mut Transaction<'_, Postgres>,
        actor: Principal,
        view: ReadingRef,
        mode: ReadingMode,
        session: &mut references::Session,
    ) -> Result<(), ContentError> {
        let layer = model::load_view(tx, actor, view.clone()).await?;
        let access = model::access_with_session(tx, actor, &layer, session).await?;
        let projection = crate::reading::project_for_impact(&layer, &access, mode)?;
        let reading_node = ImpactNode::Reading {
            view_id: view.view_id,
            revision_id: view.revision_id,
        };
        if mode != ReadingMode::Personal
            && let Some(source) = access.source
        {
            // The B3 item projection below alone decides reading display.
            self.reading_bases
                .entry(source.root.clone())
                .or_default()
                .insert(reading_node.clone());
            self.add_snapshot(source, false)?;
        }
        for item in projection.items {
            match item {
                VersionedReadingItem::Original { path, revision } => {
                    let block = BlockRef {
                        block_id: revision.block_id,
                        revision_id: revision.revision_id,
                    };
                    self.member(
                        ImpactStart::Block(block.clone()),
                        ImpactMembership::Displayed,
                    );
                    self.reading_locations
                        .entry(block)
                        .or_default()
                        .push((reading_node.clone(), ImpactLocation::Occurrence { path }));
                }
                VersionedReadingItem::Personal { item } => {
                    self.add_personal(&view, &reading_node, item, false);
                }
                _ => {}
            }
        }
        for item in projection.unplaced {
            self.add_personal(&view, &reading_node, item, true);
        }
        let (_, choices) = selection::choices(tx, &view).await?;
        let evidence = selection::project(tx, actor, &choices, session).await?;
        for selected in evidence.selections {
            self.member(
                ImpactStart::Relation(selected.relation.reference),
                ImpactMembership::Selected,
            );
            if let Some(ReviewProjection::Available(review)) = selected.review {
                self.member(
                    ImpactStart::RelationReview(review.reference),
                    ImpactMembership::Selected,
                );
            }
        }
        for selected in evidence.epistemic_reviews {
            if let ReviewProjection::Available(review) = selected {
                self.member(
                    ImpactStart::EpistemicReview(review.reference),
                    ImpactMembership::Selected,
                );
            }
        }
        self.reading_views.insert(view);
        Ok(())
    }
    fn add_personal(
        &mut self,
        view: &ReadingRef,
        reading_node: &ImpactNode,
        item: VersionedPersonalItem,
        unplaced: bool,
    ) {
        let block = BlockRef {
            block_id: item.revision.block_id,
            revision_id: item.revision.revision_id,
        };
        let location = if unplaced || item.location.is_none() {
            ImpactLocation::Unplaced {
                view_id: view.view_id,
                revision_id: view.revision_id,
                placement_id: item.placement_id,
            }
        } else {
            ImpactLocation::Placement {
                view_id: view.view_id,
                revision_id: view.revision_id,
                placement_id: item.placement_id,
            }
        };
        self.member(
            ImpactStart::Block(block.clone()),
            ImpactMembership::Displayed,
        );
        self.placements
            .entry(block.clone())
            .or_default()
            .push(location.clone());
        self.reading_locations
            .entry(block)
            .or_default()
            .push((reading_node.clone(), location));
    }
}

pub(super) async fn load(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    scope: &ImpactScope,
    session: &mut references::Session,
) -> Result<FixedScope, ContentError> {
    let mut fixed = FixedScope::default();
    match scope {
        ImpactScope::Reading { view, mode } => {
            fixed
                .add_reading(tx, actor, view.clone(), *mode, session)
                .await?
        }
        ImpactScope::Release { release_id } => {
            let released = release::load_for_impact(tx, actor, *release_id).await?;
            for root in released.roots {
                let closure = closure::load_with_session(
                    tx,
                    actor,
                    std::slice::from_ref(&root),
                    None,
                    session,
                )
                .await?;
                fixed.add_snapshot(closure.snapshot(root), true)?;
            }
            for view in released.readings {
                fixed
                    .add_reading(tx, actor, view, ReadingMode::Fused, session)
                    .await?;
            }
            // These rows were validated against the immutable manifest by B3.
            // They confer context membership but no display or direct edge.
            for reference in released.manifest_compositions {
                fixed.member(
                    ImpactStart::Composition(reference),
                    ImpactMembership::Context,
                );
            }
            for (kind, object, revision) in released.manifest_objects {
                let reference = match kind.as_str() {
                    "block" => ImpactStart::Block(BlockRef {
                        block_id: object,
                        revision_id: revision,
                    }),
                    "relation" => ImpactStart::Relation(RelationRef {
                        relation_id: object,
                        revision_id: revision,
                    }),
                    "relation_review" => {
                        let relation_id: Uuid = sqlx::query_scalar("SELECT relation_id FROM public.relation_review WHERE relation_revision_id=$1 AND id=$2")
                            .bind(object).bind(revision).fetch_one(&mut **tx).await.map_err(crate::storage)?;
                        ImpactStart::RelationReview(RelationReviewRef {
                            relation: RelationRef {
                                relation_id,
                                revision_id: object,
                            },
                            review_id: revision,
                        })
                    }
                    "epistemic_review" => ImpactStart::EpistemicReview(EpistemicReviewRef {
                        stream_id: object,
                        review_id: revision,
                    }),
                    _ => return Err(ContentError::Storage),
                };
                fixed.member(reference, ImpactMembership::Context);
            }
        }
    }
    // Rebuild the necessary closure from visible display/selection roots. The
    // B3 access loader may have inspected other anchors, which are not members.
    let roots: BTreeSet<_> = fixed
        .members
        .keys()
        .filter_map(ImpactStart::exact)
        .collect();
    let mut members = references::Session::default();
    for root in roots {
        if !members.authorize(tx, actor, &root).await? {
            return Err(ContentError::Storage);
        }
    }
    for reference in members.checkpoint() {
        fixed.member(reference.into(), ImpactMembership::Context);
    }
    // The operational session is reused by direct candidates; no authorization
    // result is carried across calls or snapshots.
    *session = members;
    Ok(fixed)
}

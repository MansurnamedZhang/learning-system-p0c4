use crate::composition::closure;
use crate::{references, storage};
use learning_core::*;
use sqlx::{Postgres, Row, Transaction};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;
pub(crate) type Tx<'a> = Transaction<'a, Postgres>;
#[derive(Clone)]
pub(crate) struct Layer {
    pub id: Uuid,
    pub space: Uuid,
    pub revision: Uuid,
    pub view: ReadingRef,
    pub data: EditableReading,
}
pub(crate) struct Access {
    pub source: Option<VersionedCompositionSnapshot>,
    pub blocks: BTreeMap<BlockRef, (Uuid, ContentRevision)>,
    pub origins: BTreeMap<CompositionRef, VersionedCompositionSnapshot>,
    pub spaces: Vec<(Uuid, bool)>,
}
impl Access {
    pub fn complete(&self, layer: &Layer) -> bool {
        self.source.is_some()
            && layer.data.groups.iter().all(|g| {
                self.origins.contains_key(&g.location.anchor().base)
                    && g.placements
                        .iter()
                        .all(|p| self.blocks.contains_key(&p.block))
            })
    }
}
pub(crate) async fn load(
    tx: &mut Tx<'_>,
    actor: Principal,
    id: Uuid,
    revision: Option<Uuid>,
) -> Result<Layer, ContentError> {
    let row=sqlx::query("SELECT o.id,o.space_id,o.head_revision_id FROM overlay o JOIN space_grant g ON g.space_id=o.space_id AND g.actor_id=$1 WHERE o.id=$2 AND o.owner_id=$1").bind(actor.actor_id).bind(id).fetch_optional(&mut **tx).await.map_err(storage)?.ok_or(ContentError::NotFound)?;
    let rev = revision.unwrap_or(row.get("head_revision_id"));
    let r=sqlx::query("SELECT r.*,v.view_id,v.id AS view_revision FROM overlay_revision r JOIN reading_view_revision v ON v.overlay_id=r.overlay_id AND v.overlay_revision_id=r.id JOIN reading_view h ON h.id=v.view_id WHERE r.overlay_id=$1 AND r.id=$2 ORDER BY (v.id=h.head_revision_id) DESC,v.created_at,v.id LIMIT 1").bind(id).bind(rev).fetch_optional(&mut **tx).await.map_err(storage)?.ok_or(ContentError::NotFound)?;
    let rows=sqlx::query("SELECT * FROM overlay_group WHERE overlay_id=$1 AND overlay_revision_id=$2 ORDER BY group_id").bind(id).bind(rev).fetch_all(&mut **tx).await.map_err(storage)?;
    let mut groups = vec![];
    for g in rows {
        let group_id = g.get("group_id");
        let anchor = GapAnchor {
            base: CompositionRef {
                composition_id: g.get("root_composition_id"),
                revision_id: g.get("base_revision_id"),
            },
            parent_occurrence_path: g.get("parent_path"),
            left_occurrence_id: g.get("left_id"),
            right_occurrence_id: g.get("right_id"),
            affinity: match g.get::<String, _>("affinity").as_str() {
                "after_left" => Affinity::AfterLeft,
                "before_right" => Affinity::BeforeRight,
                _ => return Err(ContentError::Storage),
            },
        };
        let ps=sqlx::query("SELECT placement_id,block_id,block_revision_id FROM overlay_placement WHERE overlay_revision_id=$1 AND group_id=$2 ORDER BY position").bind(rev).bind(group_id).fetch_all(&mut **tx).await.map_err(storage)?;
        groups.push(EditableGroup {
            group_id,
            location: if g.get("placed") {
                GroupLocation::Placed { anchor }
            } else {
                GroupLocation::Unplaced {
                    origin_anchor: anchor,
                }
            },
            placements: ps
                .iter()
                .map(|p| Placement {
                    placement_id: p.get("placement_id"),
                    block: BlockRef {
                        block_id: p.get("block_id"),
                        revision_id: p.get("block_revision_id"),
                    },
                })
                .collect(),
        });
    }
    Ok(Layer {
        id,
        space: row.get("space_id"),
        revision: rev,
        view: ReadingRef {
            view_id: r.get("view_id"),
            revision_id: r.get("view_revision"),
        },
        data: EditableReading {
            base: CompositionRef {
                composition_id: r.get("root_composition_id"),
                revision_id: r.get("base_revision_id"),
            },
            title: r.get("title"),
            groups,
        },
    })
}
pub(crate) async fn load_view(
    tx: &mut Tx<'_>,
    actor: Principal,
    view: ReadingRef,
) -> Result<Layer, ContentError> {
    let pair:Option<(Uuid,Uuid)>=sqlx::query_as("SELECT v.overlay_id,v.overlay_revision_id FROM reading_view_revision v JOIN overlay o ON o.id=v.overlay_id JOIN space_grant g ON g.space_id=o.space_id AND g.actor_id=$1 WHERE v.view_id=$2 AND v.id=$3 AND o.owner_id=$1")
        .bind(actor.actor_id).bind(view.view_id).bind(view.revision_id).fetch_optional(&mut **tx).await.map_err(storage)?;
    let (id, rev) = pair.ok_or(ContentError::NotFound)?;
    let mut layer = load(tx, actor, id, Some(rev)).await?;
    layer.view = view;
    Ok(layer)
}
pub(crate) async fn snapshot(
    tx: &mut Tx<'_>,
    actor: Principal,
    r: &CompositionRef,
) -> Result<(Vec<(Uuid, bool)>, Option<VersionedCompositionSnapshot>), ContentError> {
    snapshot_with_session(tx, actor, r, &mut references::Session::default()).await
}
pub(crate) async fn snapshot_with_session(
    tx: &mut Tx<'_>,
    actor: Principal,
    r: &CompositionRef,
    session: &mut references::Session,
) -> Result<(Vec<(Uuid, bool)>, Option<VersionedCompositionSnapshot>), ContentError> {
    let checkpoint = session.checkpoint();
    match closure::load_with_session(tx, actor, std::slice::from_ref(r), None, session).await {
        Ok(c) => Ok((c.spaces(), Some(c.snapshot(r.clone())))),
        Err(ContentError::NotFound) => {
            session.restore(checkpoint);
            Ok((vec![], None))
        }
        Err(e) => Err(e),
    }
}
pub(crate) async fn blocks_with_session(
    tx: &mut Tx<'_>,
    actor: Principal,
    refs: impl IntoIterator<Item = BlockRef>,
    session: &mut references::Session,
) -> Result<
    (
        BTreeMap<BlockRef, (Uuid, ContentRevision)>,
        Vec<(Uuid, bool)>,
    ),
    ContentError,
> {
    let refs: BTreeSet<_> = refs.into_iter().collect();
    let mut result = BTreeMap::new();
    let mut bytes = 0;
    let mut spaces = vec![];
    for r in refs {
        if let Some((space, size, revision, ss)) = session.assembly_block(tx, actor, &r).await? {
            bytes += size;
            if bytes > closure::BODY_LIMIT {
                return super::anchor::invalid("personal_body_limit");
            }
            spaces.extend(ss);
            result.insert(r, (space, revision));
        }
    }
    Ok((result, spaces))
}
pub(crate) async fn access(
    tx: &mut Tx<'_>,
    actor: Principal,
    layer: &Layer,
) -> Result<Access, ContentError> {
    access_with_session(tx, actor, layer, &mut references::Session::default()).await
}
pub(crate) async fn access_with_session(
    tx: &mut Tx<'_>,
    actor: Principal,
    layer: &Layer,
    session: &mut references::Session,
) -> Result<Access, ContentError> {
    let (mut spaces, source) = snapshot_with_session(tx, actor, &layer.data.base, session).await?;
    let mut origins = BTreeMap::new();
    if let Some(s) = &source {
        origins.insert(layer.data.base.clone(), s.clone());
    }
    for reference in layer
        .data
        .groups
        .iter()
        .map(|g| g.location.anchor().base.clone())
        .collect::<BTreeSet<_>>()
    {
        if origins.contains_key(&reference) {
            continue;
        }
        let (ss, s) = snapshot_with_session(tx, actor, &reference, session).await?;
        spaces.extend(ss);
        if let Some(s) = s {
            origins.insert(reference, s);
        }
    }
    let (blocks, block_spaces) = blocks_with_session(
        tx,
        actor,
        layer
            .data
            .groups
            .iter()
            .flat_map(|g| g.placements.iter().map(|p| p.block.clone()))
            .collect::<Vec<_>>(),
        session,
    )
    .await?;
    spaces.extend(block_spaces);
    spaces.push((layer.space, false));
    Ok(Access {
        source,
        blocks,
        origins,
        spaces,
    })
}
/// Full immutable view authorization for writes/replays. Display projections
/// intentionally retain their separate omission behavior in access_with_session.
pub(crate) async fn authorize_view(
    tx: &mut Tx<'_>,
    actor: Principal,
    layer: &Layer,
    session: &mut references::Session,
) -> Result<Access, ContentError> {
    let mut access = access_with_session(tx, actor, layer, session).await?;
    if !access.complete(layer) {
        return Err(ContentError::NotFound);
    }
    let (_, choices) = crate::reading::selection::choices(tx, &layer.view).await?;
    crate::reading::selection::authorize(tx, actor, layer.id, &choices, session).await?;
    access.spaces.extend(session.spaces());
    Ok(access)
}
pub(crate) fn require_locked_spaces(
    locked: &[(Uuid, bool)],
    required: &[(Uuid, bool)],
) -> Result<(), ContentError> {
    if required.iter().any(|(space, write)| {
        !locked
            .iter()
            .any(|(held, can_write)| held == space && (!write || *can_write))
    }) {
        return Err(ContentError::NotFound);
    }
    Ok(())
}
pub(crate) async fn lock_heads(
    tx: &mut Tx<'_>,
    actor: Principal,
    layer: &Layer,
    expected: Uuid,
    view_expected: Uuid,
) -> Result<(), ContentError> {
    let current: Uuid =
        sqlx::query_scalar("SELECT head_revision_id FROM overlay WHERE id=$1 FOR UPDATE")
            .bind(layer.id)
            .fetch_one(&mut **tx)
            .await
            .map_err(storage)?;
    let view: Uuid =
        sqlx::query_scalar("SELECT head_revision_id FROM reading_view WHERE id=$1 FOR UPDATE")
            .bind(layer.view.view_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(storage)?;
    if current != expected || view != view_expected {
        // Conflict metadata is a historical read, too. Never disclose the new
        // head when any of its necessary reading/evidence dependencies is hidden.
        // This failing operation takes no additional grant locks after heads.
        let current_layer = load_view(
            tx,
            actor,
            ReadingRef {
                view_id: layer.view.view_id,
                revision_id: view,
            },
        )
        .await?;
        let mut session = references::Session::default();
        authorize_view(tx, actor, &current_layer, &mut session).await?;
        return Err(ContentError::ReadingConflict {
            current_overlay_revision: current,
            current_reading_view_revision: view,
        });
    }
    Ok(())
}

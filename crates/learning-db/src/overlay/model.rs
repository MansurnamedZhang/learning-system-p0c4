use crate::composition::closure;
use crate::{COLUMNS, RevisionRow, storage};
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
    pub source: Option<CompositionSnapshot>,
    pub blocks: BTreeMap<BlockRef, (Uuid, Revision)>,
    pub origins: BTreeMap<CompositionRef, CompositionSnapshot>,
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
    let r=sqlx::query("SELECT r.*,v.view_id,v.id AS view_revision FROM overlay_revision r JOIN reading_view_revision v ON v.overlay_id=r.overlay_id AND v.overlay_revision_id=r.id WHERE r.overlay_id=$1 AND r.id=$2").bind(id).bind(rev).fetch_optional(&mut **tx).await.map_err(storage)?.ok_or(ContentError::NotFound)?;
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
pub(crate) async fn snapshot(
    tx: &mut Tx<'_>,
    actor: Principal,
    r: &CompositionRef,
) -> Result<(Vec<(Uuid, bool)>, Option<CompositionSnapshot>), ContentError> {
    match closure::load(tx, actor, std::slice::from_ref(r), None).await {
        Ok(c) => Ok((c.spaces(), Some(c.snapshot(r.clone())))),
        Err(ContentError::NotFound) => Ok((vec![], None)),
        Err(e) => Err(e),
    }
}
pub(crate) async fn blocks(
    tx: &mut Tx<'_>,
    actor: Principal,
    refs: impl IntoIterator<Item = BlockRef>,
) -> Result<BTreeMap<BlockRef, (Uuid, Revision)>, ContentError> {
    let refs: BTreeSet<_> = refs.into_iter().collect();
    let mut result = BTreeMap::new();
    let mut bytes = 0usize;
    for r in refs {
        let meta:Option<(Uuid,i32)>=sqlx::query_as("SELECT r.space_id,octet_length(r.content::text) FROM block_revision r JOIN space_grant g ON g.space_id=r.space_id AND g.actor_id=$1 WHERE r.block_id=$2 AND r.id=$3").bind(actor.actor_id).bind(r.block_id).bind(r.revision_id).fetch_optional(&mut **tx).await.map_err(storage)?;
        if let Some((space, size)) = meta {
            bytes += size as usize;
            if bytes > closure::BODY_LIMIT {
                return super::anchor::invalid("personal_body_limit");
            }
            let row=sqlx::query_as::<_,RevisionRow>(&format!("SELECT {COLUMNS} FROM block_revision r JOIN space_grant g ON g.space_id=r.space_id AND g.actor_id=$1 WHERE r.block_id=$2 AND r.id=$3")).bind(actor.actor_id).bind(r.block_id).bind(r.revision_id).fetch_optional(&mut **tx).await.map_err(storage)?.ok_or(ContentError::NotFound)?;
            result.insert(r, (space, row.into()));
        }
    }
    Ok(result)
}
pub(crate) async fn access(
    tx: &mut Tx<'_>,
    actor: Principal,
    layer: &Layer,
) -> Result<Access, ContentError> {
    let (mut spaces, source) = snapshot(tx, actor, &layer.data.base).await?;
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
        let (ss, s) = snapshot(tx, actor, &reference).await?;
        spaces.extend(ss);
        if let Some(s) = s {
            origins.insert(reference, s);
        }
    }
    let blocks = blocks(
        tx,
        actor,
        layer
            .data
            .groups
            .iter()
            .flat_map(|g| g.placements.iter().map(|p| p.block.clone()))
            .collect::<Vec<_>>(),
    )
    .await?;
    spaces.extend(blocks.values().map(|(s, _)| (*s, false)));
    spaces.push((layer.space, false));
    Ok(Access {
        source,
        blocks,
        origins,
        spaces,
    })
}
pub(crate) async fn lock_heads(
    tx: &mut Tx<'_>,
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
        return Err(ContentError::ReadingConflict {
            current_overlay_revision: current,
            current_reading_view_revision: view,
        });
    }
    Ok(())
}

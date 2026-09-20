use super::{
    anchor::{invalid, validate_gap},
    model::*,
};
use crate::storage;
use learning_core::*;
use sqlx::Row;
use uuid::Uuid;

pub(crate) fn validate(layer: &Layer, access: &Access) -> Result<(), ContentError> {
    if !access.complete(layer) {
        return Err(ContentError::NotFound);
    }
    if layer.data.groups.len() > 512 {
        return invalid("group_limit");
    }
    let count: usize = layer.data.groups.iter().map(|g| g.placements.len()).sum();
    if count > 2048 {
        return invalid("placement_limit");
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut gaps = std::collections::BTreeSet::new();
    for g in &layer.data.groups {
        if g.placements.is_empty() {
            return invalid("empty_group");
        }
        let a = g.location.anchor();
        if a.base.composition_id != layer.data.base.composition_id {
            return invalid("anchor_root");
        }
        if g.location.placed() {
            validate_gap(access.source.as_ref().ok_or(ContentError::NotFound)?, a)?;
            if !gaps.insert(canonical_json(&serde_json::to_value(a).map_err(storage)?)) {
                return invalid("duplicate_gap");
            }
        }
        for p in &g.placements {
            if !ids.insert(p.placement_id) {
                return invalid("duplicate_placement");
            }
        }
    }
    Ok(())
}
pub(crate) async fn save(
    tx: &mut Tx<'_>,
    actor: Principal,
    layer: &mut Layer,
    parent: Option<(Uuid, Uuid)>,
    reason: &str,
    access: &Access,
) -> Result<ReadingSaved, ContentError> {
    validate(layer, access)?;
    let rev = Uuid::new_v4();
    let view_rev = Uuid::new_v4();
    let source_space: Uuid = sqlx::query_scalar(
        "SELECT space_id FROM composition_revision WHERE composition_id=$1 AND id=$2",
    )
    .bind(layer.data.base.composition_id)
    .bind(layer.data.base.revision_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(storage)?;
    sqlx::query("INSERT INTO overlay_revision(id,overlay_id,space_id,root_composition_id,source_space_id,base_revision_id,parent_revision_id,title,content_sha256,author_id,reason) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)").bind(rev).bind(layer.id).bind(layer.space).bind(layer.data.base.composition_id).bind(source_space).bind(layer.data.base.revision_id).bind(parent.map(|p|p.0)).bind(&layer.data.title).bind(layer.data.digest()).bind(actor.actor_id).bind(reason).execute(&mut **tx).await.map_err(storage)?;
    for g in &layer.data.groups {
        sqlx::query("INSERT INTO overlay_group_identity(overlay_id,group_id) VALUES($1,$2) ON CONFLICT DO NOTHING").bind(layer.id).bind(g.group_id).execute(&mut **tx).await.map_err(storage)?;
        let a = g.location.anchor();
        let gs: Uuid = sqlx::query_scalar(
            "SELECT space_id FROM composition_revision WHERE composition_id=$1 AND id=$2",
        )
        .bind(a.base.composition_id)
        .bind(a.base.revision_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(storage)?;
        sqlx::query("INSERT INTO overlay_group(overlay_id,overlay_revision_id,group_id,placed,source_space_id,root_composition_id,base_revision_id,placed_base_revision_id,parent_path,left_id,right_id,affinity) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)").bind(layer.id).bind(rev).bind(g.group_id).bind(g.location.placed()).bind(gs).bind(a.base.composition_id).bind(a.base.revision_id).bind(if g.location.placed(){Some(a.base.revision_id)}else{None}).bind(&a.parent_occurrence_path).bind(a.left_occurrence_id).bind(a.right_occurrence_id).bind(match a.affinity{Affinity::AfterLeft=>"after_left",Affinity::BeforeRight=>"before_right"}).execute(&mut **tx).await.map_err(storage)?;
        for (i, p) in g.placements.iter().enumerate() {
            sqlx::query("INSERT INTO overlay_placement_identity(overlay_id,placement_id,block_id) VALUES($1,$2,$3) ON CONFLICT DO NOTHING").bind(layer.id).bind(p.placement_id).bind(p.block.block_id).execute(&mut **tx).await.map_err(storage)?;
            let space = access.blocks.get(&p.block).ok_or(ContentError::NotFound)?.0;
            sqlx::query("INSERT INTO overlay_placement(overlay_id,overlay_revision_id,placement_id,group_id,position,block_space_id,block_id,block_revision_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8)").bind(layer.id).bind(rev).bind(p.placement_id).bind(g.group_id).bind(i as i32).bind(space).bind(p.block.block_id).bind(p.block.revision_id).execute(&mut **tx).await.map_err(storage)?;
        }
    }
    sqlx::query("INSERT INTO reading_view_revision(id,view_id,overlay_id,overlay_revision_id,parent_revision_id,author_id) VALUES($1,$2,$3,$4,$5,$6)").bind(view_rev).bind(layer.view.view_id).bind(layer.id).bind(rev).bind(parent.map(|p|p.1)).bind(actor.actor_id).execute(&mut **tx).await.map_err(storage)?;
    sqlx::query("UPDATE overlay SET head_revision_id=$1 WHERE id=$2")
        .bind(rev)
        .bind(layer.id)
        .execute(&mut **tx)
        .await
        .map_err(storage)?;
    sqlx::query("UPDATE reading_view SET head_revision_id=$1 WHERE id=$2")
        .bind(view_rev)
        .bind(layer.view.view_id)
        .execute(&mut **tx)
        .await
        .map_err(storage)?;
    layer.revision = rev;
    layer.view.revision_id = view_rev;
    Ok(ReadingSaved {
        overlay: OverlayRef {
            overlay_id: layer.id,
            revision_id: rev,
        },
        view: layer.view.clone(),
        changed_blocks: vec![],
    })
}
pub(crate) async fn receipt(
    tx: &mut Tx<'_>,
    actor: Principal,
    request: Uuid,
) -> Result<Option<ReadingSaved>, ContentError> {
    let r = sqlx::query("SELECT * FROM reading_receipt WHERE actor_id=$1 AND request_id=$2")
        .bind(actor.actor_id)
        .bind(request)
        .fetch_optional(&mut **tx)
        .await
        .map_err(storage)?;
    let Some(r) = r else { return Ok(None) };
    let rows=sqlx::query("SELECT block_id,revision_id FROM reading_receipt_block WHERE actor_id=$1 AND request_id=$2 ORDER BY position").bind(actor.actor_id).bind(request).fetch_all(&mut **tx).await.map_err(storage)?;
    Ok(Some(ReadingSaved {
        overlay: OverlayRef {
            overlay_id: r.get("overlay_id"),
            revision_id: r.get("overlay_revision_id"),
        },
        view: ReadingRef {
            view_id: r.get("view_id"),
            revision_id: r.get("view_revision_id"),
        },
        changed_blocks: rows
            .iter()
            .map(|r| BlockRef {
                block_id: r.get("block_id"),
                revision_id: r.get("revision_id"),
            })
            .collect(),
    }))
}
pub(crate) async fn record(
    tx: &mut Tx<'_>,
    actor: Principal,
    request: Uuid,
    digest: &str,
    op: &str,
    saved: &ReadingSaved,
) -> Result<(), ContentError> {
    crate::request::register(tx, actor, request, digest, op).await?;
    sqlx::query("INSERT INTO reading_receipt(actor_id,request_id,overlay_id,overlay_revision_id,view_id,view_revision_id) VALUES($1,$2,$3,$4,$5,$6)").bind(actor.actor_id).bind(request).bind(saved.overlay.overlay_id).bind(saved.overlay.revision_id).bind(saved.view.view_id).bind(saved.view.revision_id).execute(&mut **tx).await.map_err(storage)?;
    for (i, b) in saved.changed_blocks.iter().enumerate() {
        let s: Uuid = sqlx::query_scalar("SELECT space_id FROM block WHERE id=$1")
            .bind(b.block_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(storage)?;
        sqlx::query("INSERT INTO reading_receipt_block(actor_id,request_id,position,block_space_id,block_id,revision_id) VALUES($1,$2,$3,$4,$5,$6)").bind(actor.actor_id).bind(request).bind(i as i32).bind(s).bind(b.block_id).bind(b.revision_id).execute(&mut **tx).await.map_err(storage)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    #[test]
    fn resulting_group_and_placement_limits_are_inclusive() {
        let id = Uuid::new_v4();
        let base = CompositionRef {
            composition_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
        };
        let block = BlockRef {
            block_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
        };
        let draft:TextDraft=serde_json::from_value(serde_json::json!({"kind":"text","intent":"note","language":"en","title":"","payload":{"format":"markdown","text":"N"}})).unwrap();
        let body = Revision {
            block_id: block.block_id,
            revision_id: block.revision_id,
            parent_revision_id: None,
            content_sha256: draft.digest(),
            draft,
            author_id: id,
            reason: "test".into(),
            created_at: chrono::Utc::now(),
        };
        let snapshot = CompositionSnapshot {
            root: base.clone(),
            blocks: vec![],
            compositions: vec![],
        };
        let access = Access {
            source: Some(snapshot.clone()),
            blocks: BTreeMap::from([(block.clone(), (id, body))]),
            origins: BTreeMap::from([(base.clone(), snapshot)]),
            spaces: vec![],
        };
        let group = || EditableGroup {
            group_id: Uuid::new_v4(),
            location: GroupLocation::Unplaced {
                origin_anchor: GapAnchor {
                    base: base.clone(),
                    parent_occurrence_path: vec![],
                    left_occurrence_id: None,
                    right_occurrence_id: None,
                    affinity: Affinity::AfterLeft,
                },
            },
            placements: vec![Placement {
                placement_id: Uuid::new_v4(),
                block: block.clone(),
            }],
        };
        let mut layer = Layer {
            id,
            space: id,
            revision: id,
            view: ReadingRef {
                view_id: id,
                revision_id: id,
            },
            data: EditableReading {
                base: base.clone(),
                title: "".into(),
                groups: (0..512).map(|_| group()).collect(),
            },
        };
        assert!(validate(&layer, &access).is_ok());
        layer.data.groups.push(group());
        assert!(
            matches!(validate(&layer,&access),Err(ContentError::Invalid(ref s)) if s=="group_limit")
        );
        layer.data.groups = vec![group()];
        layer.data.groups[0].placements = (0..2048)
            .map(|_| Placement {
                placement_id: Uuid::new_v4(),
                block: block.clone(),
            })
            .collect();
        assert!(validate(&layer, &access).is_ok());
        layer.data.groups[0].placements.push(Placement {
            placement_id: Uuid::new_v4(),
            block,
        });
        assert!(
            matches!(validate(&layer,&access),Err(ContentError::Invalid(ref s)) if s=="placement_limit")
        );
    }
}

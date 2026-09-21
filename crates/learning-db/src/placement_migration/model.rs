use crate::{
    authorization,
    overlay::model::{self, Access, Layer, Tx},
    storage,
};
use learning_core::*;
use sqlx::{Row, types::Json};
use uuid::Uuid;
pub(crate) struct Proposal {
    pub id: Uuid,
    pub layer: Layer,
    pub target: CompositionRef,
    pub groups: Vec<Suggestion>,
}
pub(crate) struct Suggestion {
    pub group_id: Uuid,
    pub class: MigrationClass,
    pub anchor: Option<GapAnchor>,
    pub reason: String,
}
pub(crate) async fn load(
    tx: &mut Tx<'_>,
    actor: Principal,
    id: Uuid,
) -> Result<Proposal, ContentError> {
    let row=sqlx::query("SELECT p.* FROM placement_migration p JOIN overlay o ON o.id=p.overlay_id JOIN space_grant g ON g.space_id=o.space_id AND g.actor_id=$1 WHERE p.id=$2 AND o.owner_id=$1").bind(actor.actor_id).bind(id).fetch_optional(&mut **tx).await.map_err(storage)?.ok_or(ContentError::NotFound)?;
    let mut layer = model::load(
        tx,
        actor,
        row.get("overlay_id"),
        Some(row.get("old_overlay_revision_id")),
    )
    .await?;
    let fixed = if let Some(id) = row.get::<Option<Uuid>, _>("old_view_revision_id") {
        id
    } else {
        sqlx::query_scalar("SELECT id FROM reading_view_revision WHERE overlay_id=$1 AND overlay_revision_id=$2 AND contract_version=1 ORDER BY created_at,id LIMIT 1")
            .bind(layer.id).bind(layer.revision).fetch_one(&mut **tx).await.map_err(storage)?
    };
    layer.view.revision_id = fixed;
    let rows = sqlx::query(
        "SELECT * FROM placement_migration_group WHERE proposal_id=$1 ORDER BY group_id",
    )
    .bind(id)
    .fetch_all(&mut **tx)
    .await
    .map_err(storage)?;
    let mut groups = vec![];
    for r in rows {
        groups.push(Suggestion {
            group_id: r.get("group_id"),
            class: match r.get::<String, _>("classification").as_str() {
                "exact" => MigrationClass::Exact,
                "candidate" => MigrationClass::Candidate,
                "unresolved" => MigrationClass::Unresolved,
                _ => return Err(ContentError::Storage),
            },
            anchor: r
                .get::<Option<Json<GapAnchor>>, _>("candidate")
                .map(|j| j.0),
            reason: r.get("reason_code"),
        });
    }
    Ok(Proposal {
        id,
        layer,
        target: CompositionRef {
            composition_id: row.get("target_root"),
            revision_id: row.get("target_revision_id"),
        },
        groups,
    })
}
pub(crate) async fn lock_access(
    tx: &mut Tx<'_>,
    actor: Principal,
    p: &Proposal,
    result: Option<&Layer>,
) -> Result<(Access, VersionedCompositionSnapshot), ContentError> {
    let (access, _) = access_for_write(tx, actor, p, result).await?;
    authorization::lock_grants(tx, actor, &access.spaces).await?;
    let (checked, target) = access_for_write(tx, actor, p, result).await?;
    model::require_locked_spaces(&access.spaces, &checked.spaces)?;
    Ok((checked, target))
}
async fn access_for_write(
    tx: &mut Tx<'_>,
    actor: Principal,
    p: &Proposal,
    result: Option<&Layer>,
) -> Result<(Access, VersionedCompositionSnapshot), ContentError> {
    let mut session = crate::references::Session::default();
    let mut access = model::authorize_view(tx, actor, &p.layer, &mut session).await?;
    let (spaces, target) = model::snapshot_with_session(tx, actor, &p.target, &mut session).await?;
    access.spaces.extend(spaces);
    if let Some(result) = result {
        let saved = model::authorize_view(tx, actor, result, &mut session).await?;
        access.spaces.extend(saved.spaces);
    }
    access.spaces.push((p.layer.space, true));
    Ok((access, target.ok_or(ContentError::NotFound)?))
}
pub(crate) async fn project(
    tx: &mut Tx<'_>,
    actor: Principal,
    p: &Proposal,
) -> Result<MigrationProjection, ContentError> {
    let access = model::access(tx, actor, &p.layer).await?;
    let (_, target) = model::snapshot(tx, actor, &p.target).await?;
    let mut groups = vec![];
    for suggestion in &p.groups {
        let g = p
            .layer
            .data
            .groups
            .iter()
            .find(|g| g.group_id == suggestion.group_id)
            .ok_or(ContentError::Storage)?;
        let placement_ids: Vec<_> = g
            .placements
            .iter()
            .filter(|p| access.blocks.contains_key(&p.block))
            .map(|p| p.placement_id)
            .collect();
        if placement_ids.is_empty() {
            continue;
        }
        let old_visible =
            access.source.is_some() && access.origins.contains_key(&g.location.anchor().base);
        let both = old_visible && target.is_some();
        groups.push(MigrationGroupProjection {
            group_id: g.group_id,
            placement_ids,
            old_anchor: old_visible.then(|| g.location.anchor().clone()),
            suggested_anchor: if target.is_some() {
                suggestion.anchor.clone()
            } else {
                None
            },
            classification: both.then_some(suggestion.class),
            reason_code: both.then(|| suggestion.reason.clone()),
        });
    }
    let rows=sqlx::query("SELECT id,adopted,author_id,created_at FROM placement_migration_decision WHERE proposal_id=$1 ORDER BY id").bind(p.id).fetch_all(&mut **tx).await.map_err(storage)?;
    Ok(MigrationProjection {
        proposal_id: p.id,
        overlay: OverlayRef {
            overlay_id: p.layer.id,
            revision_id: p.layer.revision,
        },
        target: target.map(|_| p.target.clone()),
        groups,
        decisions: rows
            .iter()
            .map(|r| MigrationDecisionSummary {
                decision_id: r.get("id"),
                adopted: r.get("adopted"),
                author_id: r.get("author_id"),
                created_at: r.get("created_at"),
            })
            .collect(),
    })
}

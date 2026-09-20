use super::{
    MigrationStore,
    model::{self, Proposal},
};
use crate::{
    overlay::{
        anchor::{invalid, validate_gap},
        edit::ordered_merge,
        model as overlay, persist,
    },
    request, storage,
};
use learning_core::*;
use sqlx::Row;
use std::collections::BTreeSet;
use uuid::Uuid;
impl MigrationStore {
    pub async fn decide(
        &self,
        actor: Principal,
        id: Uuid,
        command: DecideMigration,
    ) -> Result<MigrationDecided, ContentError> {
        command.validate()?;
        let digest = command.digest(actor, id);
        let mut tx = request::begin(&self.pool, actor, command.request_id).await?;
        let p = model::load(&mut tx, actor, command.proposal_id).await?;
        if p.layer.id != id {
            return Err(ContentError::NotFound);
        }
        model::lock_access(&mut tx, actor, &p).await?;
        if request::check(
            &mut tx,
            actor,
            command.request_id,
            &digest,
            "migration_decide",
        )
        .await?
        {
            let row=sqlx::query("SELECT d.* FROM migration_receipt r JOIN placement_migration_decision d ON d.id=r.decision_id WHERE r.actor_id=$1 AND r.request_id=$2").bind(actor.actor_id).bind(command.request_id).fetch_one(&mut *tx).await.map_err(storage)?;
            let adopted = if row.get("adopted") {
                let layer = overlay::load(
                    &mut tx,
                    actor,
                    id,
                    Some(row.get("result_overlay_revision_id")),
                )
                .await?;
                if !overlay::access(&mut tx, actor, &layer)
                    .await?
                    .complete(&layer)
                {
                    return Err(ContentError::NotFound);
                }
                Some(ReadingSaved {
                    overlay: OverlayRef {
                        overlay_id: id,
                        revision_id: layer.revision,
                    },
                    view: layer.view,
                    changed_blocks: vec![],
                })
            } else {
                None
            };
            let result = MigrationDecided {
                proposal_id: p.id,
                decision_id: row.get("id"),
                adopted,
            };
            tx.commit().await.map_err(storage)?;
            return Ok(result);
        }
        overlay::lock_heads(
            &mut tx,
            &p.layer,
            command.expected_overlay_revision,
            command.expected_reading_view_revision,
        )
        .await?;
        if command.expected_overlay_revision != p.layer.revision
            || command.expected_reading_view_revision != p.layer.view.revision_id
        {
            return invalid("stale_proposal");
        }
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM placement_migration_decision WHERE proposal_id=$1)",
        )
        .bind(p.id)
        .fetch_one(&mut *tx)
        .await
        .map_err(storage)?;
        if exists {
            return invalid("proposal_decided");
        }
        let decision_id = Uuid::new_v4();
        let mut mappings = vec![];
        let adopted = match &command.action {
            MigrationAction::Reject => None,
            MigrationAction::Adopt { groups, merges } => {
                let (_, target) = overlay::snapshot(&mut tx, actor, &p.target).await?;
                let target = target.ok_or(ContentError::NotFound)?;
                let (mut layer, map) = apply(&p, &target, groups, merges)?;
                mappings = map;
                let access = overlay::access(&mut tx, actor, &layer).await?;
                Some(
                    persist::save(
                        &mut tx,
                        actor,
                        &mut layer,
                        Some((p.layer.revision, p.layer.view.revision_id)),
                        &command.reason,
                        &access,
                    )
                    .await?,
                )
            }
        };
        sqlx::query("INSERT INTO placement_migration_decision(id,overlay_id,proposal_id,adopted,result_overlay_revision_id,result_view_revision_id,author_id,reason) VALUES($1,$2,$3,$4,$5,$6,$7,$8)").bind(decision_id).bind(id).bind(p.id).bind(adopted.is_some()).bind(adopted.as_ref().map(|s|s.overlay.revision_id)).bind(adopted.as_ref().map(|s|s.view.revision_id)).bind(actor.actor_id).bind(&command.reason).execute(&mut *tx).await.map_err(storage)?;
        for (source, result, disposition, order) in mappings {
            sqlx::query("INSERT INTO placement_migration_mapping(decision_id,overlay_id,source_group_id,result_group_id,disposition,placement_order) VALUES($1,$2,$3,$4,$5,$6)").bind(decision_id).bind(id).bind(source).bind(result).bind(disposition).bind(order).execute(&mut *tx).await.map_err(storage)?;
        }
        request::register(
            &mut tx,
            actor,
            command.request_id,
            &digest,
            "migration_decide",
        )
        .await?;
        sqlx::query(
            "INSERT INTO migration_receipt(actor_id,request_id,decision_id) VALUES($1,$2,$3)",
        )
        .bind(actor.actor_id)
        .bind(command.request_id)
        .bind(decision_id)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        Ok(MigrationDecided {
            proposal_id: p.id,
            decision_id,
            adopted,
        })
    }
}
type Mapping = (Uuid, Uuid, &'static str, Vec<Uuid>);
fn apply(
    p: &Proposal,
    target: &VersionedCompositionSnapshot,
    decisions: &[GroupDecision],
    merges: &[MergeOrder],
) -> Result<(overlay::Layer, Vec<Mapping>), ContentError> {
    let expected: BTreeSet<_> = p.layer.data.groups.iter().map(|g| g.group_id).collect();
    let provided: BTreeSet<_> = decisions.iter().map(GroupDecision::group_id).collect();
    if expected != provided || decisions.len() != expected.len() {
        return invalid("incomplete_group_decisions");
    }
    let mut layer = p.layer.clone();
    layer.data.base = p.target.clone();
    let mut mappings = vec![];
    for g in &mut layer.data.groups {
        let decision = decisions
            .iter()
            .find(|d| d.group_id() == g.group_id)
            .ok_or(ContentError::Storage)?;
        let suggestion = p
            .groups
            .iter()
            .find(|s| s.group_id == g.group_id)
            .ok_or(ContentError::Storage)?;
        let (anchor, disposition) = match decision {
            GroupDecision::KeepUnplaced { .. } => (None, "unplaced"),
            GroupDecision::Manual { anchor, .. } => {
                validate_gap(target, anchor)?;
                (Some(anchor.clone()), "manual")
            }
            GroupDecision::Exact { .. } => {
                if suggestion.class != MigrationClass::Exact {
                    return invalid("classification_mismatch");
                }
                (
                    Some(suggestion.anchor.clone().ok_or(ContentError::Storage)?),
                    "exact",
                )
            }
            GroupDecision::AcceptCandidate { .. } => {
                if suggestion.class != MigrationClass::Candidate {
                    return invalid("classification_mismatch");
                }
                (
                    Some(suggestion.anchor.clone().ok_or(ContentError::Storage)?),
                    "candidate",
                )
            }
        };
        g.location = match anchor {
            Some(anchor) => GroupLocation::Placed { anchor },
            None => GroupLocation::Unplaced {
                origin_anchor: g.location.anchor().clone(),
            },
        };
        mappings.push((
            g.group_id,
            g.group_id,
            disposition,
            g.placements.iter().map(|p| p.placement_id).collect(),
        ));
    }
    for merge in merges {
        let groups: Vec<_> = layer
            .data
            .groups
            .iter()
            .filter(|g| merge.source_group_ids.contains(&g.group_id))
            .cloned()
            .collect();
        if groups.len() != merge.source_group_ids.len() || groups.len() < 2 {
            return invalid("invalid_merge");
        }
        let anchor = groups[0].location.anchor();
        if groups
            .iter()
            .any(|g| !g.location.placed() || g.location.anchor() != anchor)
        {
            return invalid("merge_anchor");
        }
        let placements = ordered_merge(&groups, &merge.placement_ids)?;
        let new_id = Uuid::new_v4();
        let location = groups[0].location.clone();
        for m in &mut mappings {
            if merge.source_group_ids.contains(&m.0) {
                m.1 = new_id;
                m.3 = merge.placement_ids.clone();
            }
        }
        layer
            .data
            .groups
            .retain(|g| !merge.source_group_ids.contains(&g.group_id));
        layer.data.groups.push(EditableGroup {
            group_id: new_id,
            location,
            placements,
        });
    }
    let mut gaps = BTreeSet::new();
    for g in &layer.data.groups {
        if g.location.placed()
            && !gaps.insert(canonical_json(
                &serde_json::to_value(g.location.anchor()).map_err(storage)?,
            ))
        {
            return invalid("merge_required");
        }
    }
    Ok((layer, mappings))
}

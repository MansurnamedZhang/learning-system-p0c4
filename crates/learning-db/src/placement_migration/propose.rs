use super::{
    MigrationStore,
    model::{self, Proposal},
};
use crate::{
    overlay::{anchor::invalid, model as overlay},
    request, storage,
};
use learning_core::*;
use sqlx::types::Json;
use uuid::Uuid;
impl MigrationStore {
    pub async fn propose(
        &self,
        actor: Principal,
        id: Uuid,
        command: ProposeMigration,
    ) -> Result<MigrationProjection, ContentError> {
        command.validate()?;
        let digest = reading_request_digest("migration_propose", actor, id, &command);
        let mut tx = request::begin(&self.pool, actor, command.request_id).await?;
        let mut layer =
            overlay::load(&mut tx, actor, id, Some(command.expected_overlay_revision)).await?;
        layer = overlay::load_view(
            &mut tx,
            actor,
            ReadingRef {
                view_id: layer.view.view_id,
                revision_id: command.expected_reading_view_revision,
            },
        )
        .await?;
        if layer.id != id || layer.revision != command.expected_overlay_revision {
            return Err(ContentError::NotFound);
        }
        if let Some(prior) = sqlx::query_scalar::<_, Option<Uuid>>(
            "SELECT proposal_id FROM migration_receipt WHERE actor_id=$1 AND request_id=$2",
        )
        .bind(actor.actor_id)
        .bind(command.request_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage)?
        .flatten()
        {
            let p = model::load(&mut tx, actor, prior).await?;
            model::lock_access(&mut tx, actor, &p).await?;
            request::check(
                &mut tx,
                actor,
                command.request_id,
                &digest,
                "migration_propose",
            )
            .await?;
            let result = model::project(&mut tx, actor, &p).await?;
            tx.commit().await.map_err(storage)?;
            return Ok(result);
        }
        if command.target.composition_id != layer.data.base.composition_id {
            return invalid("migration_root");
        }
        if command.target == layer.data.base {
            return invalid("no_change");
        }
        let mut p = Proposal {
            id: Uuid::new_v4(),
            layer,
            target: command.target,
            groups: vec![],
        };
        let (access, target) = model::lock_access(&mut tx, actor, &p).await?;
        if request::check(
            &mut tx,
            actor,
            command.request_id,
            &digest,
            "migration_propose",
        )
        .await?
        {
            return Err(ContentError::Storage);
        }
        overlay::lock_heads(
            &mut tx,
            actor,
            &p.layer,
            command.expected_overlay_revision,
            command.expected_reading_view_revision,
        )
        .await?;
        let space: Uuid = sqlx::query_scalar(
            "SELECT space_id FROM composition_revision WHERE composition_id=$1 AND id=$2",
        )
        .bind(p.target.composition_id)
        .bind(p.target.revision_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(storage)?;
        sqlx::query("INSERT INTO placement_migration(id,overlay_id,old_overlay_revision_id,target_space_id,target_root,target_revision_id,author_id,reason,old_view_revision_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)").bind(p.id).bind(id).bind(p.layer.revision).bind(space).bind(p.target.composition_id).bind(p.target.revision_id).bind(actor.actor_id).bind(&command.reason).bind(p.layer.view.revision_id).execute(&mut *tx).await.map_err(storage)?;
        for g in &p.layer.data.groups {
            let old = access
                .origins
                .get(&g.location.anchor().base)
                .ok_or(ContentError::NotFound)?;
            let (class, anchor, reason) = super::classify::classify(old, &target, g)?;
            sqlx::query("INSERT INTO placement_migration_group(proposal_id,old_overlay_revision_id,group_id,classification,candidate,reason_code) VALUES($1,$2,$3,$4,$5,$6)").bind(p.id).bind(p.layer.revision).bind(g.group_id).bind(match class{MigrationClass::Exact=>"exact",MigrationClass::Candidate=>"candidate",MigrationClass::Unresolved=>"unresolved"}).bind(anchor.as_ref().map(Json)).bind(&reason).execute(&mut *tx).await.map_err(storage)?;
            p.groups.push(model::Suggestion {
                group_id: g.group_id,
                class,
                anchor,
                reason,
            });
        }
        request::register(
            &mut tx,
            actor,
            command.request_id,
            &digest,
            "migration_propose",
        )
        .await?;
        sqlx::query(
            "INSERT INTO migration_receipt(actor_id,request_id,proposal_id) VALUES($1,$2,$3)",
        )
        .bind(actor.actor_id)
        .bind(command.request_id)
        .bind(p.id)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
        let result = model::project(&mut tx, actor, &p).await?;
        tx.commit().await.map_err(storage)?;
        Ok(result)
    }
    pub async fn read(
        &self,
        actor: Principal,
        id: Uuid,
    ) -> Result<Option<MigrationProjection>, ContentError> {
        let mut tx = request::begin_read(&self.pool).await?;
        let p = match model::load(&mut tx, actor, id).await {
            Ok(p) => p,
            Err(ContentError::NotFound) => return Ok(None),
            Err(e) => return Err(e),
        };
        let result = model::project(&mut tx, actor, &p).await?;
        tx.commit().await.map_err(storage)?;
        Ok(Some(result))
    }
}

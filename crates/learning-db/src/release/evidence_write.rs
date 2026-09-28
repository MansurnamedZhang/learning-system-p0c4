use super::{ReleaseStore, evidence_read::load, manifest::collect};
use crate::{authorization, composition::closure, request, storage};
use learning_core::*;
use std::collections::BTreeSet;
use uuid::Uuid;
fn space_set(spaces: &[(Uuid, bool)]) -> BTreeSet<Uuid> {
    spaces.iter().map(|p| p.0).collect()
}
impl ReleaseStore {
    pub async fn publish_evidence(
        &self,
        actor: Principal,
        space: Uuid,
        mut command: PublishEvidence,
    ) -> Result<EvidenceRelease, ContentError> {
        command.validate()?;
        let digest = command.digest(space);
        command.roots.sort_by_key(|r| r.composition_id);
        command.readings.sort();
        let roots: Vec<_> = command
            .roots
            .iter()
            .map(|r| CompositionRef {
                composition_id: r.composition_id,
                revision_id: r.revision_id,
            })
            .collect();
        let mut tx = request::begin(&self.pool, actor, command.request_id).await?;
        let discovered = collect(&mut tx, actor, &roots, &command.readings).await?;
        if roots
            .iter()
            .any(|r| discovered.compositions.get(r) != Some(&space))
        {
            return Err(ContentError::NotFound);
        }
        let mut spaces = discovered.spaces;
        spaces.push((space, true));
        authorization::lock_grants(&mut tx, actor, &spaces).await?;
        let locked = collect(&mut tx, actor, &roots, &command.readings).await?;
        if !space_set(&locked.spaces).is_subset(&space_set(&spaces)) {
            return Err(ContentError::NotFound);
        }
        if request::check(
            &mut tx,
            actor,
            command.request_id,
            &digest,
            "release_publish_evidence",
        )
        .await?
        {
            let id: Uuid = sqlx::query_scalar(
                "SELECT release_id FROM release_receipt WHERE actor_id=$1 AND request_id=$2",
            )
            .bind(actor.actor_id)
            .bind(command.request_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(storage)?;
            let result = load(&mut tx, actor, id).await?;
            tx.commit().await.map_err(storage)?;
            return Ok(result);
        }
        for root in &command.roots {
            let (head,release):(Uuid,Option<Uuid>)=sqlx::query_as("SELECT head_revision_id,last_release_id FROM composition WHERE id=$1 AND space_id=$2 FOR UPDATE").bind(root.composition_id).bind(space).fetch_optional(&mut *tx).await.map_err(storage)?.ok_or(ContentError::NotFound)?;
            if head != root.expected_head_revision_id {
                closure::load(
                    &mut tx,
                    actor,
                    &[CompositionRef {
                        composition_id: root.composition_id,
                        revision_id: head,
                    }],
                    None,
                )
                .await?;
                return Err(ContentError::Conflict {
                    current_revision_id: head,
                });
            }
            if super::publication_token(root.composition_id, release)
                != root.expected_publication_token
            {
                return Err(ContentError::PublicationConflict {
                    composition_id: root.composition_id,
                });
            }
        }
        for view in &command.readings {
            sqlx::query("SELECT id FROM reading_view WHERE id=$1 FOR UPDATE")
                .bind(view.view_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(storage)?;
        }
        let manifest = collect(&mut tx, actor, &roots, &command.readings).await?;
        if !space_set(&manifest.spaces).is_subset(&space_set(&spaces)) {
            return Err(ContentError::NotFound);
        }
        let sha = manifest.digest();
        let id = Uuid::new_v4();
        let created_at=sqlx::query_scalar("INSERT INTO release(id,space_id,author_id,reason,contract_version,manifest_sha256) VALUES($1,$2,$3,$4,2,$5) RETURNING created_at")
            .bind(id).bind(space).bind(actor.actor_id).bind(&command.reason).bind(&sha).fetch_one(&mut *tx).await.map_err(storage)?;
        for root in &roots {
            sqlx::query("INSERT INTO release_root(release_id,space_id,composition_id,revision_id) VALUES($1,$2,$3,$4)").bind(id).bind(space).bind(root.composition_id).bind(root.revision_id).execute(&mut *tx).await.map_err(storage)?;
            sqlx::query("UPDATE composition SET published_revision_id=$1,last_release_id=$2 WHERE id=$3 AND space_id=$4").bind(root.revision_id).bind(id).bind(root.composition_id).bind(space).execute(&mut *tx).await.map_err(storage)?;
        }
        for layer in &manifest.readings {
            sqlx::query("INSERT INTO release_reading(release_id,view_id,view_revision_id,overlay_id,overlay_revision_id) VALUES($1,$2,$3,$4,$5)")
                .bind(id).bind(layer.view.view_id).bind(layer.view.revision_id).bind(layer.id).bind(layer.revision).execute(&mut *tx).await.map_err(storage)?;
        }
        for (r, s) in &manifest.compositions {
            sqlx::query("INSERT INTO release_manifest_composition(release_id,space_id,composition_id,revision_id) VALUES($1,$2,$3,$4)")
                .bind(id).bind(s).bind(r.composition_id).bind(r.revision_id).execute(&mut *tx).await.map_err(storage)?;
        }
        for (kind, object, revision) in &manifest.objects {
            sqlx::query("INSERT INTO release_manifest_object(release_id,kind,object_id,revision_id) VALUES($1,$2,$3,$4)")
                .bind(id).bind(kind).bind(object).bind(revision).execute(&mut *tx).await.map_err(storage)?;
        }
        sqlx::query("INSERT INTO outbox_event(id,event_type,aggregate_id,payload_version) VALUES($1,'composition_released',$2,2)").bind(Uuid::new_v4()).bind(id).execute(&mut *tx).await.map_err(storage)?;
        request::register(
            &mut tx,
            actor,
            command.request_id,
            &digest,
            "release_publish_evidence",
        )
        .await?;
        sqlx::query("INSERT INTO release_receipt(actor_id,request_id,release_id) VALUES($1,$2,$3)")
            .bind(actor.actor_id)
            .bind(command.request_id)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        Ok(EvidenceRelease {
            release_id: id,
            space_id: space,
            roots,
            author_id: actor.actor_id,
            reason: command.reason,
            created_at,
            readings: command.readings,
            manifest_sha256: sha,
        })
    }
}

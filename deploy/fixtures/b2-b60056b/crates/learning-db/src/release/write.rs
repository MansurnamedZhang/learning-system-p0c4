use super::{ReleaseStore, read};
use crate::{authorization, composition::closure, request, storage};
use learning_core::*;
use uuid::Uuid;
impl ReleaseStore {
    pub async fn publish(
        &self,
        actor: Principal,
        space: Uuid,
        mut command: PublishCommand,
    ) -> Result<Release, ContentError> {
        command.validate()?;
        let digest = command.digest(space);
        command.roots.sort_by_key(|r| r.composition_id);
        let roots: Vec<_> = command
            .roots
            .iter()
            .map(|r| CompositionRef {
                composition_id: r.composition_id,
                revision_id: r.revision_id,
            })
            .collect();
        let mut tx = request::begin(&self.pool, actor, command.request_id).await?;
        let discovered = closure::load(&mut tx, actor, &roots, None).await?;
        for root in &roots {
            if discovered.compositions.get(root).map(|(s, _)| *s) != Some(space) {
                return Err(ContentError::NotFound);
            }
        }
        authorization::lock_grants(
            &mut tx,
            actor,
            &closure::merge_spaces(&[&discovered], space),
        )
        .await?;
        closure::load(&mut tx, actor, &roots, None).await?;
        if request::check(
            &mut tx,
            actor,
            command.request_id,
            &digest,
            "release_publish",
        )
        .await?
        {
            let id: Option<Uuid> = sqlx::query_scalar(
                "SELECT release_id FROM public.release_receipt WHERE actor_id=$1 AND request_id=$2",
            )
            .bind(actor.actor_id)
            .bind(command.request_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(storage)?;
            let result = read::load(&mut tx, actor, id.ok_or(ContentError::Storage)?).await?;
            closure::load(&mut tx, actor, &result.roots, None).await?;
            tx.commit().await.map_err(storage)?;
            return Ok(result);
        }
        for root in &command.roots {
            let (head,release):(Uuid,Option<Uuid>)=sqlx::query_as("SELECT head_revision_id,last_release_id FROM public.composition WHERE id=$1 AND space_id=$2 FOR UPDATE").bind(root.composition_id).bind(space).fetch_optional(&mut *tx).await.map_err(storage)?.ok_or(ContentError::NotFound)?;
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
        let release_id = Uuid::new_v4();
        let created_at=sqlx::query_scalar("INSERT INTO public.release(id,space_id,author_id,reason) VALUES($1,$2,$3,$4) RETURNING created_at").bind(release_id).bind(space).bind(actor.actor_id).bind(&command.reason).fetch_one(&mut *tx).await.map_err(storage)?;
        for root in &roots {
            sqlx::query("INSERT INTO public.release_root(release_id,space_id,composition_id,revision_id) VALUES($1,$2,$3,$4)").bind(release_id).bind(space).bind(root.composition_id).bind(root.revision_id).execute(&mut *tx).await.map_err(storage)?;
            sqlx::query("UPDATE public.composition SET published_revision_id=$1,last_release_id=$2 WHERE id=$3 AND space_id=$4").bind(root.revision_id).bind(release_id).bind(root.composition_id).bind(space).execute(&mut *tx).await.map_err(storage)?;
        }
        sqlx::query("INSERT INTO public.outbox_event(id,event_type,aggregate_id,payload_version) VALUES($1,'composition_released',$2,1)").bind(Uuid::new_v4()).bind(release_id).execute(&mut *tx).await.map_err(storage)?;
        request::register(
            &mut tx,
            actor,
            command.request_id,
            &digest,
            "release_publish",
        )
        .await?;
        sqlx::query(
            "INSERT INTO public.release_receipt(actor_id,request_id,release_id) VALUES($1,$2,$3)",
        )
        .bind(actor.actor_id)
        .bind(command.request_id)
        .bind(release_id)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        Ok(Release {
            release_id,
            space_id: space,
            roots,
            author_id: actor.actor_id,
            reason: command.reason,
            created_at,
        })
    }
}

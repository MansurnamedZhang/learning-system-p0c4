use super::ReadingStore;
use crate::{overlay::model, request, storage};
use learning_core::*;
use uuid::Uuid;

impl ReadingStore {
    pub async fn state(
        &self,
        actor: Principal,
        id: Uuid,
    ) -> Result<Option<ReadingState>, ContentError> {
        let mut tx = request::begin_read(&self.pool).await?;
        let layer = match model::load(&mut tx, actor, id, None).await {
            Ok(layer) => layer,
            Err(ContentError::NotFound) => return Ok(None),
            Err(e) => return Err(e),
        };
        let access = model::access(&mut tx, actor, &layer).await?;
        let editable = access.complete(&layer).then_some(layer.data);
        tx.commit().await.map_err(storage)?;
        Ok(Some(ReadingState {
            overlay: OverlayRef {
                overlay_id: id,
                revision_id: layer.revision,
            },
            view: layer.view,
            editable,
        }))
    }

    pub async fn read(
        &self,
        actor: Principal,
        reference: ReadingRef,
        mode: ReadingMode,
    ) -> Result<Option<ReadingProjection>, ContentError> {
        let mut tx = request::begin_read(&self.pool).await?;
        let ids: Option<(Uuid, Uuid)> = sqlx::query_as("SELECT v.overlay_id,v.overlay_revision_id FROM reading_view_revision v JOIN overlay o ON o.id=v.overlay_id JOIN space_grant g ON g.space_id=o.space_id AND g.actor_id=$1 WHERE v.view_id=$2 AND v.id=$3 AND o.owner_id=$1")
            .bind(actor.actor_id).bind(reference.view_id).bind(reference.revision_id).fetch_optional(&mut *tx).await.map_err(storage)?;
        let Some((id, rev)) = ids else {
            return Ok(None);
        };
        let layer = model::load(&mut tx, actor, id, Some(rev)).await?;
        let access = model::access(&mut tx, actor, &layer).await?;
        let projection = super::projection::project(&layer, &access, mode)?;
        tx.commit().await.map_err(storage)?;
        Ok(Some(projection))
    }
}

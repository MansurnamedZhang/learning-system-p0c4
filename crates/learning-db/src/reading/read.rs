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

    pub async fn read_versioned(
        &self,
        actor: Principal,
        reference: ReadingRef,
        mode: ReadingMode,
    ) -> Result<Option<VersionedReadingProjection>, ContentError> {
        let mut tx = request::begin_read(&self.pool).await?;
        let projection = project_in_transaction(&mut tx, actor, reference, mode).await?;
        if projection.is_none() {
            return Ok(None);
        }
        tx.commit().await.map_err(storage)?;
        Ok(projection)
    }
}

// Shared by the unchanged runtime endpoint and the isolated management recovery
// reader. The latter cannot substitute a hand-written admin SELECT for these
// authorization, reference traversal and evidence-selection checks.
pub(super) async fn project_in_transaction(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: Principal,
    reference: ReadingRef,
    mode: ReadingMode,
) -> Result<Option<VersionedReadingProjection>, ContentError> {
    let layer = match model::load_view(tx, actor, reference).await {
        Ok(layer) => layer,
        Err(ContentError::NotFound) => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut session = crate::references::Session::default();
    let access = model::access_with_session(tx, actor, &layer, &mut session).await?;
    let mut projection = super::projection::project(&layer, &access, mode)?;
    let (version, choices) = super::selection::choices(tx, &layer.view).await?;
    projection.contract_version = version;
    projection.evidence = super::selection::project(tx, actor, &choices, &mut session).await?;
    Ok(Some(projection))
}

impl ReadingStore {
    pub async fn read(
        &self,
        actor: Principal,
        reference: ReadingRef,
        mode: ReadingMode,
    ) -> Result<Option<ReadingProjection>, ContentError> {
        self.read_versioned(actor, reference, mode)
            .await?
            .map(TryInto::try_into)
            .transpose()
    }
}

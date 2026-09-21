use super::LineageStore;
use crate::{references, request, storage};
use learning_core::*;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

impl LineageStore {
    pub async fn read(
        &self,
        actor: Principal,
        operation_id: Uuid,
    ) -> Result<Option<LineageSaved>, ContentError> {
        let mut tx = request::begin_read(&self.pool).await?;
        let saved = match load(&mut tx, actor, operation_id).await {
            Ok(saved) => Some(saved),
            Err(ContentError::NotFound) => None,
            Err(e) => return Err(e),
        };
        tx.commit().await.map_err(storage)?;
        Ok(saved)
    }
}
pub(super) async fn load(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    operation_id: Uuid,
) -> Result<LineageSaved, ContentError> {
    let (kind, author_id, created_at): (String, Uuid, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
        "SELECT o.operation,o.author_id,o.created_at FROM public.lineage_operation o JOIN public.space_grant g ON g.space_id=o.space_id AND g.actor_id=$1 WHERE o.id=$2"
    ).bind(actor.actor_id).bind(operation_id).fetch_optional(&mut **tx).await.map_err(storage)?.ok_or(ContentError::NotFound)?;
    let inputs = members(tx, operation_id, false).await?;
    let outputs = members(tx, operation_id, true).await?;
    let roots: Vec<_> = inputs
        .iter()
        .chain(&outputs)
        .cloned()
        .map(ExactRef::Block)
        .collect();
    references::load(tx, actor, &roots).await?;
    Ok(LineageSaved {
        operation_id,
        operation: serde_json::from_value(serde_json::Value::String(kind)).map_err(storage)?,
        inputs,
        outputs,
        author_id,
        created_at,
    })
}
async fn members(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    outputs: bool,
) -> Result<Vec<BlockRef>, ContentError> {
    let query = if outputs {
        "SELECT block_id,revision_id FROM public.lineage_output WHERE operation_id=$1 ORDER BY position"
    } else {
        "SELECT block_id,revision_id FROM public.lineage_input WHERE operation_id=$1 ORDER BY position"
    };
    Ok(sqlx::query_as::<_, (Uuid, Uuid)>(query)
        .bind(operation_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(storage)?
        .into_iter()
        .map(|(block_id, revision_id)| BlockRef {
            block_id,
            revision_id,
        })
        .collect())
}

use crate::{ContentStore, references, request, storage};
use chrono::{DateTime, Utc};
use learning_core::*;
use uuid::Uuid;

const AUTHORIZED_FROM: &str = "FROM public.block_revision r JOIN public.block b ON (b.id,b.space_id)=(r.block_id,r.space_id) JOIN public.space_grant g ON g.space_id=b.space_id";
const PAGE_SIZE: usize = 100;
const SCAN_LIMIT: usize = 2048;

impl ContentStore {
    pub async fn read(
        &self,
        actor: Principal,
        revision_id: Uuid,
    ) -> Result<Option<Revision>, ContentError> {
        Ok(self
            .read_many(actor, &[revision_id])
            .await?
            .into_iter()
            .next())
    }
    pub async fn read_many(
        &self,
        actor: Principal,
        revision_ids: &[Uuid],
    ) -> Result<Vec<Revision>, ContentError> {
        if revision_ids.len() > 200 {
            return Err(ContentError::Invalid("批量读取最多 200 个输入 ID".into()));
        }
        let mut tx = request::begin_read(&self.pool).await?;
        let rows:Vec<(Uuid,Uuid)>=sqlx::query_as(&format!("SELECT r.block_id,r.id {AUTHORIZED_FROM} WHERE g.actor_id=$1 AND r.id=ANY($2) ORDER BY array_position($2,r.id)"))
            .bind(actor.actor_id).bind(revision_ids).fetch_all(&mut *tx).await.map_err(storage)?;
        let mut session = references::Session::default();
        let mut result = vec![];
        for (block_id, revision_id) in rows {
            if let Some(object) = session
                .object(
                    &mut tx,
                    actor,
                    &ExactRef::Block(BlockRef {
                        block_id,
                        revision_id,
                    }),
                )
                .await?
            {
                result.push(object.block()?.try_into()?);
            }
        }
        tx.commit().await.map_err(storage)?;
        Ok(result)
    }
    pub async fn list(
        &self,
        actor: Principal,
        space_id: Uuid,
        cursor: Option<PageCursor>,
    ) -> Result<RevisionPage, ContentError> {
        self.page(actor, space_id, cursor, false).await?.try_into()
    }
    pub async fn history(
        &self,
        actor: Principal,
        block_id: Uuid,
        cursor: Option<PageCursor>,
    ) -> Result<RevisionPage, ContentError> {
        self.page(actor, block_id, cursor, true).await?.try_into()
    }
    pub(crate) async fn page(
        &self,
        actor: Principal,
        target: Uuid,
        cursor: Option<PageCursor>,
        history: bool,
    ) -> Result<ContentRevisionPage, ContentError> {
        let (filter, time, id) = if history {
            ("r.block_id=$2", "r.created_at", "r.id")
        } else {
            (
                "b.space_id=$2 AND r.id=b.head_revision_id",
                "b.created_at",
                "b.id",
            )
        };
        let query = format!(
            "SELECT r.block_id,r.id AS revision_id,{time} AS cursor_time,{id} AS cursor_id {AUTHORIZED_FROM} WHERE g.actor_id=$1 AND {filter} AND ($3::timestamptz IS NULL OR ({time},{id})<($3,$4::uuid)) ORDER BY {time} DESC,{id} DESC LIMIT 2049"
        );
        let mut tx = request::begin_read(&self.pool).await?;
        let rows = sqlx::query_as::<_, PageRow>(&query)
            .bind(actor.actor_id)
            .bind(target)
            .bind(cursor.as_ref().map(|c| c.created_at))
            .bind(cursor.as_ref().map(|c| c.id))
            .fetch_all(&mut *tx)
            .await
            .map_err(storage)?;
        let mut session = references::Session::default();
        let mut visible = vec![];
        for (index, row) in rows.into_iter().enumerate() {
            if index >= SCAN_LIMIT {
                return Err(ContentError::Invalid("reference_budget_exceeded".into()));
            }
            if let Some(object) = session
                .object(
                    &mut tx,
                    actor,
                    &ExactRef::Block(BlockRef {
                        block_id: row.block_id,
                        revision_id: row.revision_id,
                    }),
                )
                .await?
            {
                visible.push((
                    object.block()?,
                    PageCursor {
                        created_at: row.cursor_time,
                        id: row.cursor_id,
                    },
                ));
                if visible.len() > PAGE_SIZE {
                    break;
                }
            }
        }
        let more = visible.len() > PAGE_SIZE;
        visible.truncate(PAGE_SIZE);
        let next_cursor = if more {
            visible.last().map(|(_, c)| c.clone())
        } else {
            None
        };
        let items = visible.into_iter().map(|(r, _)| r).collect::<Vec<_>>();
        tx.commit().await.map_err(storage)?;
        Ok(ContentRevisionPage { items, next_cursor })
    }
}
#[derive(sqlx::FromRow)]
struct PageRow {
    block_id: Uuid,
    revision_id: Uuid,
    cursor_time: DateTime<Utc>,
    cursor_id: Uuid,
}

use crate::{COLUMNS, ContentStore, RevisionRow, storage};
use chrono::{DateTime, Utc};
use learning_core::{ContentError, PageCursor, Principal, Revision, RevisionPage};
use uuid::Uuid;

const AUTHORIZED_FROM: &str = "FROM public.block_revision r JOIN public.block b ON (b.id,b.space_id)=(r.block_id,r.space_id) JOIN public.space_grant g ON g.space_id=b.space_id";
const PAGE_SIZE: usize = 100;

impl ContentStore {
    pub async fn read(
        &self,
        actor: Principal,
        revision_id: Uuid,
    ) -> Result<Option<Revision>, ContentError> {
        let query = format!("SELECT {COLUMNS} {AUTHORIZED_FROM} WHERE g.actor_id=$1 AND r.id=$2");
        let row = sqlx::query_as::<_, RevisionRow>(&query)
            .bind(actor.actor_id)
            .bind(revision_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(storage)?;
        Ok(row.map(Into::into))
    }
    pub async fn read_many(
        &self,
        actor: Principal,
        revision_ids: &[Uuid],
    ) -> Result<Vec<Revision>, ContentError> {
        if revision_ids.len() > 200 {
            return Err(ContentError::Invalid("批量读取最多 200 个输入 ID".into()));
        }
        if revision_ids.is_empty() {
            return Ok(Vec::new());
        }
        let query = format!(
            "SELECT {COLUMNS} {AUTHORIZED_FROM} WHERE g.actor_id=$1 AND r.id=ANY($2) ORDER BY array_position($2,r.id)"
        );
        let rows = sqlx::query_as::<_, RevisionRow>(&query)
            .bind(actor.actor_id)
            .bind(revision_ids)
            .fetch_all(&self.pool)
            .await
            .map_err(storage)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }
    pub async fn list(
        &self,
        actor: Principal,
        space_id: Uuid,
        cursor: Option<PageCursor>,
    ) -> Result<RevisionPage, ContentError> {
        self.page(actor, space_id, cursor, PageKind::Blocks).await
    }
    pub async fn history(
        &self,
        actor: Principal,
        block_id: Uuid,
        cursor: Option<PageCursor>,
    ) -> Result<RevisionPage, ContentError> {
        self.page(actor, block_id, cursor, PageKind::History).await
    }
    async fn page(
        &self,
        actor: Principal,
        target: Uuid,
        cursor: Option<PageCursor>,
        kind: PageKind,
    ) -> Result<RevisionPage, ContentError> {
        let (filter, time, id) = match kind {
            PageKind::Blocks => (
                "b.space_id=$2 AND r.id=b.head_revision_id",
                "b.created_at",
                "b.id",
            ),
            PageKind::History => ("r.block_id=$2", "r.created_at", "r.id"),
        };
        // Identifiers above are private constants, never caller-controlled SQL.
        let query = format!(
            "SELECT {COLUMNS},{time} AS cursor_time,{id} AS cursor_id {AUTHORIZED_FROM} WHERE g.actor_id=$1 AND {filter} AND ($3::timestamptz IS NULL OR ({time},{id})<($3,$4::uuid)) ORDER BY {time} DESC,{id} DESC LIMIT 101"
        );
        let mut rows = sqlx::query_as::<_, PageRow>(&query)
            .bind(actor.actor_id)
            .bind(target)
            .bind(cursor.as_ref().map(|c| c.created_at))
            .bind(cursor.as_ref().map(|c| c.id))
            .fetch_all(&self.pool)
            .await
            .map_err(storage)?;
        let more = rows.len() > PAGE_SIZE;
        rows.truncate(PAGE_SIZE);
        let next_cursor = if more {
            rows.last().map(|r| PageCursor {
                created_at: r.cursor_time,
                id: r.cursor_id,
            })
        } else {
            None
        };
        Ok(RevisionPage {
            items: rows.into_iter().map(|r| r.revision.into()).collect(),
            next_cursor,
        })
    }
}
enum PageKind {
    Blocks,
    History,
}
#[derive(sqlx::FromRow)]
struct PageRow {
    #[sqlx(flatten)]
    revision: RevisionRow,
    cursor_time: DateTime<Utc>,
    cursor_id: Uuid,
}

//! Read-only job inspection. Transfer, claim, and completion live in later tasks.
use learning_core::{ContentError, JobInput, JobStatus};
use sqlx::{PgPool, types::Json};
use uuid::Uuid;

#[derive(Clone)]
pub struct JobStore {
    pool: PgPool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRecord {
    pub id: Uuid,
    pub outbox_id: Uuid,
    pub idempotency_key: String,
    pub input: JobInput,
    pub status: JobStatus,
    pub attempt_count: i32,
}

#[derive(sqlx::FromRow)]
struct JobRow {
    id: Uuid,
    outbox_id: Uuid,
    idempotency_key: String,
    input: Json<serde_json::Value>,
    status: String,
    attempt_count: i32,
}

#[derive(sqlx::FromRow)]
struct PendingEvent {
    id: Uuid,
    business_key: String,
}

impl JobStore {
    /// Construct with the restricted runtime pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Move a bounded batch of pending business events to queued jobs.
    /// Row locks partition the batch among concurrent dispatchers; insertion
    /// and the dispatch marker commit together.
    pub async fn dispatch_pending(&self, limit: usize) -> Result<usize, ContentError> {
        let limit = limit.min(256);
        if limit == 0 {
            return Ok(0);
        }
        let mut tx = self.pool.begin().await.map_err(|_| ContentError::Storage)?;
        sqlx::query("SET LOCAL statement_timeout='15s'")
            .execute(&mut *tx)
            .await
            .map_err(|_| ContentError::Storage)?;
        let pending: Vec<PendingEvent> = sqlx::query_as(
            "SELECT id,business_key FROM public.job_outbox \
             WHERE dispatched_at IS NULL ORDER BY created_at,id \
             LIMIT $1 FOR UPDATE SKIP LOCKED",
        )
        .bind(limit as i64)
        .fetch_all(&mut *tx)
        .await
        .map_err(|_| ContentError::Storage)?;
        for event in &pending {
            let inserted: Option<Uuid> = sqlx::query_scalar(
                "INSERT INTO public.job(id,outbox_id,idempotency_key,status,attempt_count) \
                 VALUES($1,$2,$3,'queued',0) \
                 ON CONFLICT (outbox_id) DO NOTHING RETURNING id",
            )
            .bind(Uuid::new_v4())
            .bind(event.id)
            .bind(&event.business_key)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|_| ContentError::Storage)?;
            if inserted.is_none() {
                let existing: Option<(Uuid, String)> =
                    sqlx::query_as("SELECT id,idempotency_key FROM public.job WHERE outbox_id=$1")
                        .bind(event.id)
                        .fetch_optional(&mut *tx)
                        .await
                        .map_err(|_| ContentError::Storage)?;
                match existing {
                    Some((_, key)) if key == event.business_key => {}
                    _ => {
                        return Err(ContentError::Invalid("job_outbox_identity_conflict".into()));
                    }
                }
            }
            let marked = sqlx::query(
                "UPDATE public.job_outbox SET dispatched_at=clock_timestamp() \
                 WHERE id=$1 AND dispatched_at IS NULL",
            )
            .bind(event.id)
            .execute(&mut *tx)
            .await
            .map_err(|_| ContentError::Storage)?;
            if marked.rows_affected() != 1 {
                return Err(ContentError::Storage);
            }
        }
        tx.commit().await.map_err(|_| ContentError::Storage)?;
        Ok(pending.len())
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<JobRecord>, ContentError> {
        let row: Option<JobRow> = sqlx::query_as(
            "SELECT j.id,j.outbox_id,j.idempotency_key,o.payload AS input,j.status,j.attempt_count
               FROM public.job j JOIN public.job_outbox o ON o.id=j.outbox_id
              WHERE j.id=$1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| ContentError::Storage)?;
        row.map(|row| {
            let input = JobInput::from_value(row.input.0)?;
            if input.business_key() != row.idempotency_key {
                return Err(ContentError::Invalid("job_input_key_mismatch".into()));
            }
            Ok(JobRecord {
                id: row.id,
                outbox_id: row.outbox_id,
                idempotency_key: row.idempotency_key,
                input,
                status: JobStatus::parse(&row.status)?,
                attempt_count: row.attempt_count,
            })
        })
        .transpose()
    }
}

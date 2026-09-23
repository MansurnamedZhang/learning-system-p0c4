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

impl JobStore {
    /// Construct with the restricted runtime pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
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

use super::{BROKER_ROLE, ObservationError, catalog};
use crate::snapshot_observation::{JobObservation, OutboxObservation, SnapshotObservation};
use chrono::{DateTime, Utc};
use learning_assets::root_journal::SharedJournalBudget;
use learning_core::JobInput;
use sqlx::{PgPool, Postgres, Transaction, types::Json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::{Instant, timeout_at};
use uuid::Uuid;

const ROW_CAP: u64 = 1_048_576;
const HEADER_CHARGE: u64 = 512;
const MAX_BUDGET: u64 = 67_108_864;
struct Meter {
    used: u64,
    limit: u64,
    exhausted: bool,
}
/// Caller-owned shared cumulative accounting, never reset on clone/begin.
#[derive(Clone)]
pub struct ObservationBudget {
    local: Arc<Mutex<Meter>>,
    work: SharedJournalBudget,
}
impl ObservationBudget {
    pub fn new(limit: u64) -> Result<Self, ObservationError> {
        Self::with_shared(limit, SharedJournalBudget::default())
    }
    pub fn with_shared(limit: u64, work: SharedJournalBudget) -> Result<Self, ObservationError> {
        if limit > MAX_BUDGET {
            return Err(ObservationError::Capacity);
        }
        Ok(Self {
            local: Arc::new(Mutex::new(Meter {
                used: 0,
                limit,
                exhausted: false,
            })),
            work,
        })
    }
    pub fn charged_bytes(&self) -> Result<u64, ObservationError> {
        Ok(self
            .local
            .lock()
            .map_err(|_| ObservationError::Capacity)?
            .used)
    }
    fn charge(&self, amount: u64) -> Result<(), ObservationError> {
        let mut meter = self.local.lock().map_err(|_| ObservationError::Capacity)?;
        if meter.exhausted {
            return Err(ObservationError::Capacity);
        }
        let Some(next) = meter.used.checked_add(amount).filter(|n| *n <= meter.limit) else {
            meter.exhausted = true;
            return Err(ObservationError::Capacity);
        };
        if self.work.charge(amount, 0).is_err() {
            meter.exhausted = true;
            return Err(ObservationError::Capacity);
        }
        meter.used = next;
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct Raw {
    job_octets: i64,
    outbox_octets: i64,
    observed_at: DateTime<Utc>,
    job_row: Option<Json<JobObservation>>,
    outbox_row: Option<Json<OutboxObservation>>,
}

/// Owns its original top-level transaction. Only observation and ending rollback
/// are exposed; no executor/connection/commit/claim or write-capability API.
/// Timeouts poison reuse and do not assert that queued Drop rollback completed.
pub struct ObservationTransaction {
    tx: Transaction<'static, Postgres>,
    budget: ObservationBudget,
    deadline: Instant,
    poisoned: bool,
}
impl ObservationTransaction {
    pub async fn begin(
        pool: &PgPool,
        budget: ObservationBudget,
        duration: Duration,
    ) -> Result<Self, ObservationError> {
        if duration.is_zero() || duration > Duration::from_secs(60) {
            return Err(ObservationError::Invalid(
                "snapshot claim observation duration",
            ));
        }
        let deadline = Instant::now() + duration;
        budget.charge(HEADER_CHARGE + catalog::profile()?.len() as u64)?;
        timeout_at(deadline, async {
            let mut tx = pool.begin().await?;
            for sql in [
                "SET TRANSACTION ISOLATION LEVEL READ COMMITTED",
                "SET LOCAL search_path=pg_catalog",
                "SET LOCAL lock_timeout='5s'",
                "SET LOCAL statement_timeout='15s'",
                "SET LOCAL TimeZone='UTC'",
            ] {
                sqlx::query(sql).execute(&mut *tx).await?;
            }
            let identity: (String, String) =
                sqlx::query_as("SELECT session_user::text,current_user::text")
                    .fetch_one(&mut *tx)
                    .await?;
            if identity != (BROKER_ROLE.into(), BROKER_ROLE.into()) {
                return Err(ObservationError::Invalid(
                    "snapshot claim observation requires real broker login",
                ));
            }
            catalog::verify(&mut tx, catalog::Phase::Observe).await?;
            Ok(Self {
                tx,
                budget,
                deadline,
                poisoned: false,
            })
        })
        .await
        .map_err(|_| ObservationError::Timeout)?
    }

    pub async fn observe(
        &mut self,
        id: Uuid,
    ) -> Result<Option<SnapshotObservation>, ObservationError> {
        if self.poisoned {
            return Err(ObservationError::Invalid(
                "snapshot claim observation permanently poisoned",
            ));
        }
        if id.is_nil() {
            return Err(ObservationError::Invalid("nil snapshot claim observation"));
        }
        // Set before the first await/SQL. Dropping the borrowed future must
        // leave the original observer unusable even though it still exists.
        self.poisoned = true;
        let result = timeout_at(self.deadline, self.observe_inner(id))
            .await
            .map_err(|_| ObservationError::Timeout)
            .and_then(|value| value);
        if result.is_ok() {
            if Instant::now() >= self.deadline {
                return Err(ObservationError::Timeout);
            }
            self.poisoned = false;
        }
        result
    }
    async fn observe_inner(
        &mut self,
        id: Uuid,
    ) -> Result<Option<SnapshotObservation>, ObservationError> {
        let profile_size = catalog::profile()?.len() as u64;
        self.budget.charge(HEADER_CHARGE + profile_size)?;
        catalog::verify(&mut self.tx, catalog::Phase::Observe).await?;
        let header: Option<Raw> =
            sqlx::query_as("SELECT * FROM public.p0c4_snapshot_claim_locked_observation($1,false)")
                .bind(id)
                .fetch_optional(&mut *self.tx)
                .await?;
        let Some(header) = header else {
            return Ok(None);
        };
        if header.job_row.is_some() || header.outbox_row.is_some() {
            return Err(ObservationError::Invalid(
                "snapshot claim observation header contains body",
            ));
        }
        let job_size = u64::try_from(header.job_octets).map_err(|_| ObservationError::Capacity)?;
        let event_size =
            u64::try_from(header.outbox_octets).map_err(|_| ObservationError::Capacity)?;
        if job_size == 0 || event_size == 0 || job_size > ROW_CAP || event_size > ROW_CAP {
            return Err(ObservationError::Capacity);
        }
        let bytes = job_size
            .checked_add(event_size)
            .and_then(|bytes| bytes.checked_add(HEADER_CHARGE + profile_size))
            .ok_or(ObservationError::Capacity)?;
        self.budget.charge(bytes)?;
        let row: Raw =
            sqlx::query_as("SELECT * FROM public.p0c4_snapshot_claim_locked_observation($1,true)")
                .bind(id)
                .fetch_one(&mut *self.tx)
                .await?;
        if (row.job_octets, row.outbox_octets) != (header.job_octets, header.outbox_octets) {
            return Err(ObservationError::Invalid(
                "snapshot claim observation changed under lock",
            ));
        }
        let (Some(Json(job)), Some(Json(outbox))) = (row.job_row, row.outbox_row) else {
            return Err(ObservationError::Invalid(
                "snapshot claim observation body missing",
            ));
        };
        validate_binding(id, &job, &outbox)?;
        catalog::verify(&mut self.tx, catalog::Phase::Observe).await?;
        Ok(Some(SnapshotObservation {
            job,
            outbox,
            observed_at: row.observed_at,
        }))
    }

    pub async fn rollback(self) -> Result<(), ObservationError> {
        timeout_at(self.deadline, self.tx.rollback())
            .await
            .map_err(|_| ObservationError::Timeout)??;
        Ok(())
    }
}

fn validate_binding(
    id: Uuid,
    job: &JobObservation,
    outbox: &OutboxObservation,
) -> Result<(), ObservationError> {
    let input = JobInput::from_value(outbox.payload.clone())
        .map_err(|_| ObservationError::Invalid("snapshot claim payload invalid"))?;
    if job.id != id
        || job.id != job.outbox_id
        || job.outbox_id != outbox.id
        || job.idempotency_key != outbox.business_key
        || outbox.event_type != "snapshot_export_requested"
        || outbox.payload_version != 1
        || outbox.processor_version != 1
        || outbox.dispatched_at.is_none()
        || input.business_key() != outbox.business_key
        || !matches!(input, JobInput::SnapshotExport { actor_id, .. } if actor_id==outbox.actor_id)
    {
        return Err(ObservationError::Invalid(
            "snapshot claim observation binding mismatch",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use learning_core::{ReadingMode, ReadingRef, SnapshotRequest};

    fn rows() -> (JobObservation, OutboxObservation) {
        let actor = Uuid::new_v4();
        let id = Uuid::new_v4();
        let input = JobInput::snapshot_export(
            actor,
            Uuid::new_v4(),
            SnapshotRequest {
                reading: ReadingRef {
                    view_id: Uuid::new_v4(),
                    revision_id: Uuid::new_v4(),
                },
                mode: ReadingMode::Fused,
                include_personal: true,
                include_originals: true,
                resource_versions: vec![],
                source_segments: vec![],
            },
        )
        .unwrap();
        let now = Utc::now();
        let event = OutboxObservation {
            id,
            business_key: input.business_key(),
            event_type: "snapshot_export_requested".into(),
            payload_version: 1,
            payload: input.to_value(),
            actor_id: actor,
            processor_version: 1,
            created_at: now,
            dispatched_at: Some(now),
        };
        let job = JobObservation {
            id,
            outbox_id: id,
            idempotency_key: event.business_key.clone(),
            status: "snapshot_queued".into(),
            attempt_count: 0,
            lease_token: None,
            lease_expires_at: None,
            next_attempt_at: None,
            last_error_class: None,
            output_digest: None,
            checkpoint: None,
            created_at: now,
            updated_at: now,
        };
        (job, event)
    }

    #[test]
    fn binding_accepts_real_payload_and_rejects_actor_or_event_rebinding() {
        let (job, mut event) = rows();
        validate_binding(job.id, &job, &event).unwrap();
        let actor = event.actor_id;
        event.actor_id = Uuid::new_v4();
        assert!(validate_binding(job.id, &job, &event).is_err());
        event.actor_id = actor;
        event.event_type = "asset_integrity_requested".into();
        assert!(validate_binding(job.id, &job, &event).is_err());
    }

    #[test]
    fn binding_rejects_other_job_id_undispatched_or_unbound_business_key() {
        let (mut job, mut event) = rows();
        assert!(validate_binding(Uuid::new_v4(), &job, &event).is_err());
        let dispatched = event.dispatched_at.take();
        assert!(validate_binding(job.id, &job, &event).is_err());
        event.dispatched_at = dispatched;
        event.business_key.push('x');
        job.idempotency_key = event.business_key.clone();
        assert!(validate_binding(job.id, &job, &event).is_err());
    }
    #[test]
    fn shared_budget_stays_exhausted_across_clones_and_smaller_later_requests() {
        let budget = ObservationBudget::new(100).unwrap();
        let clone = budget.clone();
        budget.charge(60).unwrap();
        assert!(clone.charge(41).is_err());
        assert!(budget.charge(1).is_err());
        assert_eq!(clone.charged_bytes().unwrap(), 60);
    }
    #[test]
    fn two_observation_budgets_charge_the_same_work_meter() {
        let shared = SharedJournalBudget::default();
        let a = ObservationBudget::with_shared(100, shared.clone()).unwrap();
        let b = ObservationBudget::with_shared(100, shared.clone()).unwrap();
        a.charge(60).unwrap();
        b.charge(50).unwrap();
        assert_eq!(shared.usage().unwrap().charged_bytes, 110);
    }

    #[test]
    fn external_work_budget_exhaustion_poison_is_not_reset_by_new_observation_budget() {
        let shared = SharedJournalBudget::default();
        shared.charge(512 * 1024 * 1024, 0).unwrap();
        let a = ObservationBudget::with_shared(100, shared.clone()).unwrap();
        assert!(a.charge(1).is_err());
        let b = ObservationBudget::with_shared(100, shared.clone()).unwrap();
        assert!(b.charge(0).is_err());
        assert!(a.charge(0).is_err());
        assert!(shared.usage().unwrap().exhausted);
    }
}

//! Versioned actual claim/transition mechanisms. No lease or JSON admission.
use super::{
    catalog,
    history::{
        self, Claim, RecordRef, Result, Rows,
        runtime::{self, Original, Slot},
    },
    transaction,
    transition::Claimed,
};
use chrono::{DateTime, TimeDelta, Utc};
use serde::Serialize;
use sqlx::{Postgres, Transaction};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Serialize)]
pub(super) struct Event<'a, T: Serialize> {
    format_version: u8,
    capability: &'static str,
    epoch_id: Uuid,
    binding_id: Uuid,
    sequence: u64,
    previous_sha256: [u8; 32],
    kind: &'static str,
    data: &'a T,
}
pub(super) async fn event<T: Serialize>(
    owner: &Arc<Original>,
    slot: &Slot,
    kind: &'static str,
    data: &T,
) -> Result<Vec<u8>> {
    let head = owner.head(slot).await?.head();
    history::encode(
        &Event {
            format_version: 2,
            capability: "snapshot_effect_journal_event_v2",
            epoch_id: owner.epoch,
            binding_id: owner.binding,
            sequence: head
                .sequence
                .checked_add(1)
                .ok_or(super::ObservationError::Capacity)?,
            previous_sha256: head.sha256,
            kind,
            data,
        },
        &owner.budget,
    )
}
pub(super) fn rows_digest(rows: &Rows, owner: &Original) -> Result<String> {
    let bytes = history::encode(&(&rows.job, &rows.outbox), &owner.budget)?;
    history::digest(&bytes, &owner.budget)
}
pub(super) async fn begin<'connection>(
    owner: &Arc<Original>,
    slot: &Slot,
    connection: &'connection mut Option<sqlx::pool::PoolConnection<Postgres>>,
) -> Result<Transaction<'connection, Postgres>> {
    // BEGIN returns into its permanent owner before any setup can fail.
    let mut tx = runtime::settle_started(slot, async {
        use sqlx::Acquire;
        match connection.as_mut() {
            Some(connection) => Ok((&mut *connection).begin().await?),
            None => Ok(owner.pool.begin().await?),
        }
    })
    .await?;
    let initialized = runtime::settle_started(slot, transaction::initialize_owned(&mut tx)).await;
    if let Err(error) = initialized {
        if runtime::settle_started(slot, async { Ok(tx.rollback().await?) })
            .await
            .is_err()
        {
            slot.poison();
        }
        return Err(error);
    }
    if let Err(error) = slot.live() {
        let _ = runtime::settle_started(slot, async { Ok(tx.rollback().await?) }).await;
        return Err(error);
    }
    Ok(tx)
}
async fn clock(tx: &mut Transaction<'_, Postgres>, slot: &Slot) -> Result<DateTime<Utc>> {
    runtime::settle(slot, async {
        Ok(sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
            .fetch_one(&mut **tx)
            .await?)
    })
    .await
}
async fn observe(
    tx: &mut Transaction<'_, Postgres>,
    owner: &Arc<Original>,
    slot: &Slot,
    job: Uuid,
) -> Result<Rows> {
    runtime::settle(slot, transaction::read_locked(tx, &owner.observation, job))
        .await?
        .map(Into::into)
        .ok_or_else(|| history::rejected("v2 actual snapshot row missing"))
}
pub(super) struct KnownAttemptCommit {
    _private: (),
}
async fn commit(tx: Transaction<'_, Postgres>, slot: &Slot) -> Result<KnownAttemptCommit> {
    runtime::settle_started(slot, async { Ok(tx.commit().await?) }).await?;
    slot.live()?;
    Ok(KnownAttemptCommit { _private: () })
}
pub(super) enum ClaimResult {
    Acquired(Box<Claimed>),
    NotAcquired {
        after: Box<Rows>,
        reference: RecordRef,
    },
}
#[derive(Serialize)]
struct Intent<'a> {
    request_id: Uuid,
    worker_request_id: Uuid,
    job_id: Uuid,
    candidate_attempt: i32,
    previous_attempt: i32,
    before_job: &'a crate::snapshot_observation::JobObservation,
    source_job_binding_sha256: &'a str,
    lease_ms: i64,
    identity: &'a history::DatabaseIdentity,
}
#[derive(sqlx::FromRow)]
pub(super) struct BusinessRequest {
    request_id: Uuid,
    job_id: Uuid,
    outbox_id: Uuid,
    actor_id: Uuid,
}
impl BusinessRequest {
    pub(super) fn request_id(&self) -> Uuid {
        self.request_id
    }
    fn check(&self, rows: &Rows) -> Result<()> {
        if self.request_id.is_nil()
            || self.job_id != rows.job.id
            || self.outbox_id != rows.job.outbox_id
            || self.outbox_id != rows.outbox.id
            || self.actor_id != rows.outbox.actor_id
        {
            return Err(history::rejected("actual business request binding differs"));
        }
        Ok(())
    }
}
// Selection describes one existing business request. The original C0 locked
// before/after rows check all these keys, and D1 later rechecks request/key rows
// in its original runtime transaction. A worker nonce is never a fallback.
pub(super) const BUSINESS_REQUEST: &str = "SELECT r.request_id,r.job_id,r.outbox_id,r.actor_id FROM public.snapshot_export_request r JOIN public.job j ON j.id=r.job_id AND j.outbox_id=r.outbox_id JOIN public.job_outbox o ON o.id=j.outbox_id AND o.actor_id=r.actor_id WHERE j.id=$1 ORDER BY r.request_id LIMIT 1";
#[derive(Serialize)]
struct Committed<'a> {
    request_id: Uuid,
    job_id: Uuid,
    attempt: i32,
    claim_token: Uuid,
    lease_expires_at: DateTime<Utc>,
    before_job: &'a crate::snapshot_observation::JobObservation,
    after_job: &'a crate::snapshot_observation::JobObservation,
    source_job_binding_sha256: &'a str,
    lease_ms: i64,
    sql_before: DateTime<Utc>,
    sql_after: DateTime<Utc>,
    identity: &'a history::DatabaseIdentity,
}
#[derive(Serialize)]
struct NotAcquired<'a> {
    request_id: Uuid,
    job_id: Uuid,
    before_job: &'a crate::snapshot_observation::JobObservation,
    after_job: &'a crate::snapshot_observation::JobObservation,
    reason: &'static str,
    sql_before: DateTime<Utc>,
    sql_after: DateTime<Utc>,
    identity: &'a history::DatabaseIdentity,
}

fn invariant(before: &Rows, after: &Rows) -> bool {
    let b = &before.job;
    let a = &after.job;
    before.outbox == after.outbox
        && a.id == b.id
        && a.outbox_id == b.outbox_id
        && a.idempotency_key == b.idempotency_key
        && a.created_at == b.created_at
        && a.checkpoint == b.checkpoint
}
fn acquired_delta(
    before: &Rows,
    after: &Rows,
    c: &Claim,
    lease_ms: i64,
    low: DateTime<Utc>,
    high: DateTime<Utc>,
) -> Result<()> {
    let b = &before.job;
    let a = &after.job;
    if !invariant(before, after)
        || low > high
        || c.attempts != b.attempt_count + 1
        || !(1..=3).contains(&c.attempts)
        || c.job_id != b.id
        || c.token.get_version_num() != 4
        || c.token.get_variant() != uuid::Variant::RFC4122
        || a.status != "snapshot_running"
        || a.attempt_count != c.attempts
        || a.lease_token != Some(c.token)
        || a.lease_expires_at != Some(c.expires_at)
        || a.next_attempt_at.is_some()
        || a.last_error_class.is_some()
        || a.output_digest.is_some()
        || a.updated_at < low
        || a.updated_at > high
        || c.expires_at < low + TimeDelta::milliseconds(lease_ms)
        || c.expires_at > high + TimeDelta::milliseconds(lease_ms)
    {
        return Err(history::rejected("v2 canonical claim delta differs"));
    }
    Ok(())
}

pub(super) async fn claim(
    owner: Arc<Original>,
    slot: Arc<Slot>,
    selected: BusinessRequest,
    worker_request_id: Uuid,
    lease_ms: i64,
    expected: Option<String>,
) -> Result<ClaimResult> {
    if !(1..=300_000).contains(&lease_ms) {
        return Err(history::rejected("v2 lease policy"));
    }
    owner
        .observation
        .charge(1024 + catalog::profile()?.len() as u64)?;
    let mut connection = owner.acquire_broker(&slot).await?;
    let result = async {
        let mut tx = Some(begin(&owner, &slot, &mut connection).await?);
        let request = selected.request_id();
        let result = claim_inner(
            &owner,
            &slot,
            tx.as_mut().expect("owned transaction"),
            &selected,
            worker_request_id,
            lease_ms,
            expected,
        )
        .await;
        // claim_inner returns the prepared original output; COMMIT ownership stays
        // here so every error path can explicitly await its original rollback.
        let result = match result {
            Ok(prepared) => {
                let known = commit(tx.take().expect("original claim transaction"), &slot).await?;
                let reference = owner.append(&slot, prepared.bytes).await?;
                slot.live()?;
                match prepared.claim {
                    Some(c) => Ok(ClaimResult::Acquired(Box::new(Claimed::from_versioned(
                        known,
                        history::Outcome {
                            schema: history::OutcomeSchema::V2,
                            version: 2,
                            intent: prepared.intent,
                            after: prepared.after,
                            claim: c,
                            sql_before: prepared.low,
                            sql_after: prepared.high,
                            identity: prepared.identity,
                            commit: history::CommitState::OriginalKnown,
                        },
                        (prepared.intent, reference),
                        owner.clone(),
                        slot.clone(),
                        request,
                        lease_ms,
                    )))),
                    None => Ok(ClaimResult::NotAcquired {
                        after: Box::new(prepared.after),
                        reference,
                    }),
                }
            }
            Err(error) => Err(error),
        };
        if let Some(tx) = tx.take()
            && runtime::settle_started(&slot, async { Ok(tx.rollback().await?) })
                .await
                .is_err()
        {
            slot.poison();
        }
        result
    }
    .await;
    owner.close_connection(&slot, connection, result).await
}
struct Prepared {
    after: Rows,
    claim: Option<Claim>,
    bytes: Vec<u8>,
    intent: RecordRef,
    low: DateTime<Utc>,
    high: DateTime<Utc>,
    identity: history::DatabaseIdentity,
}

#[derive(Clone, Copy)]
pub(super) enum Transition {
    Renew(i64),
    TransientFailure,
    TerminalFailure,
}
pub(super) struct KnownTransition {
    pub(super) after: Rows,
    pub(super) reference: RecordRef,
    _private: (),
}
#[derive(Serialize)]
struct TransitionRecord<'a> {
    job_id: Uuid,
    attempt: i32,
    claim_record_sha256: String,
    operation: &'static str,
    lease_ms: Option<i64>,
    error_class: Option<&'static str>,
    retry: Option<bool>,
    sql_before: DateTime<Utc>,
    sql_after: DateTime<Utc>,
    identity: &'a history::DatabaseIdentity,
    before_job: &'a crate::snapshot_observation::JobObservation,
    after_job: &'a crate::snapshot_observation::JobObservation,
    sql_return: bool,
}

pub(super) async fn transition(
    owner: &Arc<Original>,
    slot: &Arc<Slot>,
    claimed: &mut Claimed,
    operation: Transition,
) -> Result<(RecordRef, bool)> {
    claimed.check_original(owner, slot)?;
    let mut connection = owner.acquire_broker(slot).await?;
    let result = async {
        let mut tx = Some(begin(owner, slot, &mut connection).await?);
        let result = transition_inner(
            owner,
            slot,
            claimed,
            operation,
            tx.as_mut().expect("owned transition"),
        )
        .await;
        let result = match result {
            Ok((after, encoded, returned)) => {
                let _known = commit(tx.take().expect("original transition commit"), slot).await?;
                let reference = owner.append(slot, encoded).await?;
                slot.live()?;
                claimed.apply_known_transition(KnownTransition {
                    after,
                    reference,
                    _private: (),
                });
                Ok((reference, returned))
            }
            Err(error) => Err(error),
        };
        if let Some(tx) = tx.take()
            && runtime::settle_started(slot, async { Ok(tx.rollback().await?) })
                .await
                .is_err()
        {
            slot.poison();
        }
        result
    }
    .await;
    owner.close_connection(slot, connection, result).await
}
async fn transition_inner(
    owner: &Arc<Original>,
    slot: &Arc<Slot>,
    claimed: &Claimed,
    operation: Transition,
    tx: &mut Transaction<'_, Postgres>,
) -> Result<(Rows, Vec<u8>, bool)> {
    let lease = claimed.lease();
    let before = observe(tx, owner, slot, lease.job_id).await?;
    let identity = super::transition::identity(tx).await?;
    if before.job != claimed.output().after.job || before.outbox != claimed.output().after.outbox {
        return Err(history::rejected("v2 transition history discontinuity"));
    }
    let low = clock(tx, slot).await?;
    let returned: bool = match operation {
        Transition::Renew(ms) => {
            if !(1..=300_000).contains(&ms) {
                return Err(history::rejected("v2 renewal duration"));
            }
            runtime::settle(slot, async {
                Ok(
                    sqlx::query_scalar("SELECT public.p0c3_renew_snapshot_job($1,$2,$3)")
                        .bind(lease.job_id)
                        .bind(lease.token)
                        .bind(ms)
                        .fetch_one(&mut **tx)
                        .await?,
                )
            })
            .await?
        }
        Transition::TransientFailure | Transition::TerminalFailure => {
            let retry = matches!(operation, Transition::TransientFailure);
            runtime::settle(slot, async {
                Ok(
                    sqlx::query_scalar("SELECT public.p0c3_fail_snapshot_job($1,$2,$3,$4)")
                        .bind(lease.job_id)
                        .bind(lease.token)
                        .bind(if retry {
                            "transient_storage"
                        } else {
                            "invalid_input"
                        })
                        .bind(retry)
                        .fetch_one(&mut **tx)
                        .await?,
                )
            })
            .await?
        }
    };
    let high = clock(tx, slot).await?;
    let after = observe(tx, owner, slot, lease.job_id).await?;
    let b = &before.job;
    let a = &after.job;
    if low > high || !invariant(&before, &after) || a.attempt_count != b.attempt_count {
        return Err(history::rejected("v2 transition invariant changed"));
    }
    if !returned {
        if a != b {
            return Err(history::rejected("v2 rejected transition changed row"));
        }
    } else {
        if b.status != "snapshot_running"
            || b.lease_token != Some(lease.token)
            || b.lease_expires_at.is_none_or(|expiry| expiry <= low)
            || a.updated_at < low
            || a.updated_at > high
            || a.output_digest.is_some()
        {
            return Err(history::rejected("v2 transition canonical delta rejected"));
        }
        match operation {
            Transition::Renew(ms) => {
                if a.status != b.status
                    || a.lease_token != b.lease_token
                    || a.next_attempt_at != b.next_attempt_at
                    || a.last_error_class != b.last_error_class
                    || a.lease_expires_at.is_none_or(|expiry| {
                        expiry < lease.lease_expires_at
                            || expiry < low + TimeDelta::milliseconds(ms)
                            || expiry > high + TimeDelta::milliseconds(ms)
                    })
                {
                    return Err(history::rejected("v2 canonical renewal differs"));
                }
            }
            Transition::TransientFailure | Transition::TerminalFailure => {
                let retry =
                    matches!(operation, Transition::TransientFailure) && lease.attempt_count < 3;
                let seconds = if lease.attempt_count == 1 { 1 } else { 2 };
                if a.status
                    != if retry {
                        "snapshot_retry_wait"
                    } else {
                        "failed"
                    }
                    || a.lease_token.is_some()
                    || a.lease_expires_at.is_some()
                    || a.last_error_class.as_deref()
                        != Some(if matches!(operation, Transition::TransientFailure) {
                            "transient_storage"
                        } else {
                            "invalid_input"
                        })
                    || if retry {
                        a.next_attempt_at.is_none_or(|next| {
                            next < low + TimeDelta::seconds(seconds)
                                || next > high + TimeDelta::seconds(seconds)
                        })
                    } else {
                        a.next_attempt_at.is_some()
                    }
                {
                    return Err(history::rejected("v2 canonical failure differs"));
                }
            }
        }
    }
    if super::transition::identity(tx).await? != identity {
        return Err(history::rejected("v2 transition transaction changed"));
    }
    let claim = claimed.records().1.head();
    owner
        .budget
        .charge(64, 0)
        .map_err(|_| super::ObservationError::Capacity)?;
    let encoded = event(
        owner,
        slot,
        "JOB_TRANSITION_KNOWN",
        &TransitionRecord {
            job_id: lease.job_id,
            attempt: lease.attempt_count,
            claim_record_sha256: claim.sha256.iter().map(|b| format!("{b:02x}")).collect(),
            operation: if matches!(operation, Transition::Renew(_)) {
                "renew"
            } else {
                "fail"
            },
            lease_ms: match operation {
                Transition::Renew(ms) => Some(ms),
                _ => None,
            },
            error_class: match operation {
                Transition::Renew(_) => None,
                Transition::TransientFailure => Some("transient_storage"),
                Transition::TerminalFailure => Some("invalid_input"),
            },
            retry: match operation {
                Transition::Renew(_) => None,
                Transition::TransientFailure => Some(true),
                Transition::TerminalFailure => Some(false),
            },
            sql_before: low,
            sql_after: high,
            identity: &identity,
            before_job: b,
            after_job: a,
            sql_return: returned,
        },
    )
    .await?;
    owner.preflight()?;
    Ok((after, encoded, returned))
}
async fn claim_inner(
    owner: &Arc<Original>,
    slot: &Arc<Slot>,
    tx: &mut Transaction<'_, Postgres>,
    selected: &BusinessRequest,
    worker_request_id: Uuid,
    lease_ms: i64,
    expected: Option<String>,
) -> Result<Prepared> {
    let job = selected.job_id;
    let before = observe(tx, owner, slot, job).await?;
    selected.check(&before)?;
    let request = selected.request_id();
    let b = &before.job;
    if let Some(expected) = expected {
        if rows_digest(&before, owner)? != expected {
            return Err(history::rejected("v2 history row discontinuity"));
        }
    } else if b.status != "snapshot_queued"
        || b.attempt_count != 0
        || b.lease_token.is_some()
        || b.lease_expires_at.is_some()
        || b.next_attempt_at.is_some()
        || b.checkpoint.is_some()
        || b.output_digest.is_some()
        || b.last_error_class.is_some()
    {
        return Err(history::rejected("v2 unknown prior claim history"));
    }
    if !(0..=3).contains(&b.attempt_count) {
        return Err(history::rejected("v2 attempt range"));
    }
    let identity = super::transition::identity(tx).await?;
    let encoded_outbox = history::encode(&before.outbox, &owner.budget)?;
    let binding = history::digest(&encoded_outbox, &owner.budget)?;
    let intent_bytes = event(
        owner,
        slot,
        "CLAIM_INTENT",
        &Intent {
            request_id: request,
            worker_request_id,
            job_id: job,
            candidate_attempt: (b.attempt_count + 1).min(3),
            previous_attempt: b.attempt_count,
            before_job: b,
            source_job_binding_sha256: &binding,
            lease_ms,
            identity: &identity,
        },
    )
    .await?;
    owner.preflight()?;
    let intent = owner.append(slot, intent_bytes).await?;
    let low = clock(tx, slot).await?;
    let returned: Vec<Claim> = runtime::settle(slot, async {
        Ok(sqlx::query_as(
            "SELECT job_id,token,attempts,expires_at FROM public.p0c3_claim_snapshot_job($1,$2)",
        )
        .bind(job)
        .bind(lease_ms)
        .fetch_all(&mut **tx)
        .await?)
    })
    .await?;
    let high = clock(tx, slot).await?;
    if returned.len() > 1 {
        return Err(history::rejected("v2 claim returned multiple rows"));
    }
    let after = observe(tx, owner, slot, job).await?;
    selected.check(&after)?;
    if super::transition::identity(tx).await? != identity {
        return Err(history::rejected("v2 claim transaction changed"));
    }
    let claim = returned.into_iter().next();
    let bytes = if let Some(c) = &claim {
        acquired_delta(&before, &after, c, lease_ms, low, high)?;
        event(
            owner,
            slot,
            "CLAIM_COMMITTED",
            &Committed {
                request_id: request,
                job_id: job,
                attempt: c.attempts,
                claim_token: c.token,
                lease_expires_at: c.expires_at,
                before_job: &before.job,
                after_job: &after.job,
                source_job_binding_sha256: &binding,
                lease_ms,
                sql_before: low,
                sql_after: high,
                identity: &identity,
            },
        )
        .await?
    } else {
        let ceiling = b.status == "snapshot_running"
            && b.attempt_count == 3
            && b.lease_expires_at.is_some_and(|v| v <= low);
        let a = &after.job;
        if ceiling {
            if !invariant(&before, &after)
                || a.status != "failed"
                || a.attempt_count != 3
                || a.lease_token.is_some()
                || a.lease_expires_at.is_some()
                || a.next_attempt_at != b.next_attempt_at
                || a.output_digest.is_some()
                || a.last_error_class.as_deref() != Some("lease_expired")
                || a.updated_at < low
                || a.updated_at > high
            {
                return Err(history::rejected("v2 ceiling None delta differs"));
            }
        } else if before.job != after.job || before.outbox != after.outbox {
            return Err(history::rejected("v2 ineligible None changed rows"));
        }
        event(
            owner,
            slot,
            "CLAIM_NOT_ACQUIRED",
            &NotAcquired {
                request_id: request,
                job_id: job,
                before_job: &before.job,
                after_job: &after.job,
                reason: if ceiling {
                    "expired_at_ceiling"
                } else {
                    "ineligible"
                },
                sql_before: low,
                sql_after: high,
                identity: &identity,
            },
        )
        .await?
    };
    owner.preflight()?;
    Ok(Prepared {
        after,
        claim,
        bytes,
        intent,
        low,
        high,
        identity,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn formal_business_request_binding_never_uses_the_worker_dispatch_nonce() {
        let job = Uuid::new_v4();
        let actor = Uuid::new_v4();
        let request = Uuid::new_v4();
        let worker = Uuid::new_v4();
        assert_ne!(request, worker);
        let at: DateTime<Utc> = "2026-10-23T00:00:00Z".parse().unwrap();
        let rows: Rows = serde_json::from_value(serde_json::json!({
            "job":{"id":job,"outbox_id":job,"idempotency_key":"binding","status":"snapshot_queued","attempt_count":0,"lease_token":null,"lease_expires_at":null,"next_attempt_at":null,"last_error_class":null,"checkpoint":null,"output_digest":null,"created_at":at,"updated_at":at},
            "outbox":{"id":job,"business_key":"binding","event_type":"snapshot_export_requested","payload_version":1,"payload":{},"actor_id":actor,"processor_version":1,"created_at":at,"dispatched_at":at},"observed_at":at
        })).unwrap();
        let selected = BusinessRequest {
            request_id: request,
            job_id: job,
            outbox_id: job,
            actor_id: actor,
        };
        selected.check(&rows).unwrap();
        assert_eq!(selected.request_id(), request);
        for field in 0..4 {
            let mut wrong = BusinessRequest { ..selected };
            match field {
                0 => wrong.request_id = Uuid::nil(),
                1 => wrong.job_id = worker,
                2 => wrong.outbox_id = worker,
                _ => wrong.actor_id = worker,
            }
            assert!(wrong.check(&rows).is_err());
        }
        let identity = history::DatabaseIdentity {
            database: "db".into(),
            oid: 1,
            postmaster: at,
            session_user: super::super::BROKER_ROLE.into(),
            current_user: super::super::BROKER_ROLE.into(),
            backend: 123,
            transaction: "42".into(),
        };
        let body = serde_json::to_value(Intent {
            request_id: selected.request_id(),
            worker_request_id: worker,
            job_id: job,
            candidate_attempt: 1,
            previous_attempt: 0,
            before_job: &rows.job,
            source_job_binding_sha256: "binding",
            lease_ms: 30_000,
            identity: &identity,
        })
        .unwrap();
        assert_eq!(body["request_id"], request.to_string());
        assert_eq!(body["worker_request_id"], worker.to_string());
        assert_ne!(body["request_id"], body["worker_request_id"]);
    }
    #[test]
    fn formal_acquired_claim_checks_exact_clock_bounds_and_full_immutable_rows() {
        let job = Uuid::new_v4();
        let token = Uuid::new_v4();
        let actor = Uuid::new_v4();
        let low: DateTime<Utc> = "2026-10-23T00:00:00Z".parse().unwrap();
        let high = low + TimeDelta::milliseconds(10);
        let expires = low + TimeDelta::milliseconds(30_005);
        let before = serde_json::json!({"job":{"id":job,"outbox_id":job,"idempotency_key":"binding","status":"snapshot_queued","attempt_count":0,"lease_token":null,"lease_expires_at":null,"next_attempt_at":null,"last_error_class":null,"checkpoint":null,"output_digest":null,"created_at":low,"updated_at":low},
            "outbox":{"id":job,"business_key":"binding","event_type":"snapshot_export_requested","payload_version":1,"payload":{"actor_id":actor},"actor_id":actor,"processor_version":1,"created_at":low,"dispatched_at":low},"observed_at":low});
        let mut after = before.clone();
        after["job"]["status"] = "snapshot_running".into();
        after["job"]["attempt_count"] = 1.into();
        after["job"]["lease_token"] = serde_json::to_value(token).unwrap();
        after["job"]["lease_expires_at"] = serde_json::to_value(expires).unwrap();
        after["job"]["updated_at"] = serde_json::to_value(high).unwrap();
        after["observed_at"] = serde_json::to_value(high).unwrap();
        let before: Rows = serde_json::from_value(before).unwrap();
        let claim = Claim {
            job_id: job,
            token,
            attempts: 1,
            expires_at: expires,
        };
        acquired_delta(
            &before,
            &serde_json::from_value(after.clone()).unwrap(),
            &claim,
            30_000,
            low,
            high,
        )
        .unwrap();
        for expiry in [
            low + TimeDelta::milliseconds(29_999),
            high + TimeDelta::milliseconds(30_001),
        ] {
            let mut changed = after.clone();
            changed["job"]["lease_expires_at"] = serde_json::to_value(expiry).unwrap();
            let claim = Claim {
                job_id: job,
                token,
                attempts: 1,
                expires_at: expiry,
            };
            assert!(
                acquired_delta(
                    &before,
                    &serde_json::from_value(changed).unwrap(),
                    &claim,
                    30_000,
                    low,
                    high
                )
                .is_err()
            );
        }
        for (table, key, value) in [
            ("job", "idempotency_key", serde_json::json!("other")),
            ("job", "checkpoint", serde_json::json!({"changed":true})),
            ("outbox", "payload", serde_json::json!({"changed":true})),
            ("outbox", "processor_version", serde_json::json!(2)),
            (
                "job",
                "last_error_class",
                serde_json::json!("lease_expired"),
            ),
            (
                "job",
                "next_attempt_at",
                serde_json::to_value(high).unwrap(),
            ),
        ] {
            let mut changed = after.clone();
            changed[table][key] = value;
            assert!(
                acquired_delta(
                    &before,
                    &serde_json::from_value(changed).unwrap(),
                    &claim,
                    30_000,
                    low,
                    high
                )
                .is_err()
            );
        }
    }
}

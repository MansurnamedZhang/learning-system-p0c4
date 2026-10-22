//! Only fresh snapshot_queued attempt0 -> canonical Some1. No public entry.
use super::{
    catalog,
    history::{
        self, Claim, DatabaseIdentity, Intent, Outcome, RecordRef, Result, Rows,
        runtime::{self, Original, Slot, Stage},
    },
    transaction,
};
use chrono::{DateTime, TimeDelta, Utc};
use sqlx::{Postgres, Transaction};
use std::sync::{Arc, atomic::Ordering};
use uuid::Uuid;

use history::LEASE_MS;
const SQL: &str =
    "SELECT job_id,token,attempts,expires_at FROM public.p0c3_claim_snapshot_job($1,$2)";
struct KnownCommit;
// Only this module can construct the result, after consuming KnownCommit and
// receiving the original durable outcome. No ByteHead/JSON/JobLease constructor.
pub(super) struct DurableTransition {
    intent: RecordRef,
    outcome: RecordRef,
}
// Constructed only after this same original operation's known commit and both
// durable frames. Neither serialization nor a JobLease can construct it.
pub(super) struct Claimed {
    transition: DurableTransition,
    output: Outcome,
    original: Arc<Original>,
    slot: Arc<Slot>,
    request: Uuid,
    continuation: RecordRef,
    lease_ms: i64,
}
impl Claimed {
    pub(super) fn from_versioned(
        _known: super::attempts::KnownAttemptCommit,
        output: Outcome,
        references: (RecordRef, RecordRef),
        original: Arc<Original>,
        slot: Arc<Slot>,
        request: Uuid,
        lease_ms: i64,
    ) -> Self {
        Self {
            transition: DurableTransition {
                intent: references.0,
                outcome: references.1,
            },
            output,
            original,
            slot,
            request,
            continuation: references.1,
            lease_ms,
        }
    }
    pub(super) fn continuation(&self) -> RecordRef {
        self.continuation
    }
    pub(super) fn lease_ms(&self) -> i64 {
        self.lease_ms
    }
    pub(super) fn apply_known_transition(&mut self, known: super::attempts::KnownTransition) {
        if let Some(expiry) = known.after.job.lease_expires_at {
            self.output.claim.expires_at = expiry;
        }
        self.output.after = known.after;
        self.continuation = known.reference;
    }
    pub(super) fn check_original(&self, owner: &Arc<Original>, slot: &Arc<Slot>) -> Result<()> {
        if !Arc::ptr_eq(&self.original, owner) || !Arc::ptr_eq(&self.slot, slot) {
            return Err(history::rejected("C2 original claim owner mismatch"));
        }
        slot.live()
    }
    pub(super) fn output(&self) -> &Outcome {
        &self.output
    }
    pub(super) fn records(&self) -> (RecordRef, RecordRef) {
        (self.transition.intent, self.transition.outcome)
    }
    pub(super) fn request(&self) -> Uuid {
        self.request
    }
    pub(super) fn lease(&self) -> crate::JobLease {
        let claim = &self.output.claim;
        crate::JobLease {
            job_id: claim.job_id,
            token: claim.token,
            attempt_count: claim.attempts,
            lease_expires_at: claim.expires_at,
        }
    }
}
#[cfg(test)]
impl DurableTransition {
    pub(super) fn sequences(&self) -> (u64, u64) {
        (self.intent.sequence(), self.outcome.sequence())
    }
}
fn fresh(rows: &Rows) -> Result<()> {
    let j = &rows.job;
    if j.status != "snapshot_queued"
        || j.attempt_count != 0
        || j.lease_token.is_some()
        || j.lease_expires_at.is_some()
        || j.next_attempt_at.is_some()
        || j.last_error_class.is_some()
        || j.output_digest.is_some()
        || j.checkpoint.is_some()
    {
        return Err(history::rejected("P1 unsupported nonfresh row"));
    }
    Ok(())
}
pub(super) async fn identity(tx: &mut Transaction<'static, Postgres>) -> Result<DatabaseIdentity> {
    let row:(String,i64,DateTime<Utc>,String,String,i32,String)=sqlx::query_as("SELECT current_database(),(SELECT oid::bigint FROM pg_catalog.pg_database WHERE datname=current_database()),pg_catalog.pg_postmaster_start_time(),session_user::text,current_user::text,pg_catalog.pg_backend_pid(),pg_catalog.pg_current_xact_id()::text").fetch_one(&mut **tx).await?;
    Ok(DatabaseIdentity {
        database: row.0,
        oid: row.1,
        postmaster: row.2,
        session_user: row.3,
        current_user: row.4,
        backend: row.5,
        transaction: row.6,
    })
}
fn delta(
    intent: &Intent,
    after: &Rows,
    claim: &Claim,
    low: DateTime<Utc>,
    high: DateTime<Utc>,
) -> Result<()> {
    let b = &intent.before.job;
    let a = &after.job;
    if low > high
        || intent.before.outbox != after.outbox
        || a.id != b.id
        || a.outbox_id != b.outbox_id
        || a.idempotency_key != b.idempotency_key
        || a.created_at != b.created_at
        || a.checkpoint != b.checkpoint
        || a.status != "snapshot_running"
        || a.attempt_count != 1
        || a.lease_token != Some(claim.token)
        || a.lease_expires_at != Some(claim.expires_at)
        || a.next_attempt_at.is_some()
        || a.last_error_class.is_some()
        || a.output_digest.is_some()
        || claim.job_id != b.id
        || claim.attempts != 1
        || claim.token.get_version_num() != 4
        || claim.token.get_variant() != uuid::Variant::RFC4122
        || a.updated_at < low
        || a.updated_at > high
        || claim.expires_at < low + TimeDelta::milliseconds(LEASE_MS)
        || claim.expires_at > high + TimeDelta::milliseconds(LEASE_MS)
    {
        return Err(history::rejected("P1 canonical claim delta rejected"));
    }
    Ok(())
}
async fn consume_commit(tx: Transaction<'static, Postgres>, slot: &Slot) -> Result<KnownCommit> {
    runtime::settle_started(slot, async {
        tx.commit().await?;
        Ok(())
    })
    .await?;
    slot.live()?;
    Ok(KnownCommit)
}
// Only the consumed original commit token permits construction of this body.
async fn append_committed(
    _known: KnownCommit,
    owner: &Arc<Original>,
    slot: &Slot,
    prepared: Vec<u8>,
) -> Result<RecordRef> {
    owner.append(slot, prepared).await
}
pub(super) async fn drive(
    owner: Arc<Original>,
    slot: Arc<Slot>,
    job: Uuid,
    request: Uuid,
) -> Result<DurableTransition> {
    Ok(claim(owner, slot, job, request).await?.transition)
}
pub(super) async fn claim(
    owner: Arc<Original>,
    slot: Arc<Slot>,
    job: Uuid,
    request: Uuid,
) -> Result<Claimed> {
    slot.live()?;
    owner
        .observation
        .charge(512 + catalog::profile()?.len() as u64)?;
    let mut tx = Some(runtime::settle(&slot, transaction::begin_owned(&owner.pool)).await?);
    let result = inner(&owner, &slot, &mut tx, job, request).await;
    if let Some(tx) = tx.take() {
        // Original owner retains and settles rollback even after authority has
        // become HOLD; rollback is not a new operation or result adoption.
        if tx.rollback().await.is_err() {
            slot.poison();
        }
    }
    result
}
async fn inner(
    owner: &Arc<Original>,
    slot: &Arc<Slot>,
    owned: &mut Option<Transaction<'static, Postgres>>,
    job: Uuid,
    request: Uuid,
) -> Result<Claimed> {
    slot.live()?;
    let tx = owned
        .as_mut()
        .ok_or_else(|| history::rejected("P1 original transaction missing"))?;
    let before = runtime::settle(slot, transaction::read_locked(tx, &owner.observation, job))
        .await?
        .ok_or_else(|| history::rejected("P1 unsupported missing row"))?;
    let before = Rows::from(before);
    fresh(&before)?;
    let db = runtime::settle(slot, identity(tx)).await?;
    let previous = owner.head(slot).await?;
    let profile_sha256 = history::digest(catalog::profile()?.as_bytes(), &owner.budget)?;
    let intent = Intent {
        schema: history::IntentSchema::V1,
        version: 1,
        epoch: owner.epoch,
        binding: owner.binding,
        request,
        job,
        previous,
        profile_sha256,
        source_build: owner.source_build.clone(),
        identity: db,
        branch: history::Branch::Fresh,
        lease_ms: LEASE_MS,
        before,
    };
    let bytes = history::encode(&intent, &owner.budget)?;
    if bytes
        .len()
        .checked_add(4096)
        .is_none_or(|n| n > learning_assets::root_journal::MAX_FRAME_BYTES - 120)
    {
        return Err(super::ObservationError::Capacity);
    }
    if owner
        .observation
        .charged_bytes()?
        .checked_add(2 * 1_048_576 + 1024 + 2 * catalog::profile()?.len() as u64)
        .is_none_or(|n| n > 64 * 1024 * 1024)
    {
        return Err(super::ObservationError::Capacity);
    }
    owner.preflight()?;
    owner.stage(slot, Stage::BeforeIntent).await?;
    let intent_ref = owner.append(slot, bytes).await?;
    owner.stage(slot, Stage::AfterIntent).await?;
    owner.stage(slot, Stage::BeforeClaim).await?;
    let low: DateTime<Utc> = runtime::settle(slot, async {
        Ok(sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
            .fetch_one(&mut **tx)
            .await?)
    })
    .await?;
    slot.live()?;
    owner.sql_calls.fetch_add(1, Ordering::SeqCst);
    let rows: Vec<Claim> = runtime::settle(slot, async {
        Ok(sqlx::query_as(SQL)
            .bind(job)
            .bind(LEASE_MS)
            .fetch_all(&mut **tx)
            .await?)
    })
    .await?;
    let mut rows = rows.into_iter();
    let claim = rows
        .next()
        .ok_or_else(|| history::rejected("P1 expected Some1, never None"))?;
    if rows.next().is_some() {
        return Err(history::rejected("P1 multiple SQL rows"));
    }
    let high: DateTime<Utc> = runtime::settle(slot, async {
        Ok(sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
            .fetch_one(&mut **tx)
            .await?)
    })
    .await?;
    owner.stage(slot, Stage::AfterClaim).await?;
    let after = Rows::from(
        runtime::settle(slot, transaction::read_locked(tx, &owner.observation, job))
            .await?
            .ok_or_else(|| history::rejected("P1 after row missing"))?,
    );
    delta(&intent, &after, &claim, low, high)?;
    let after_identity = runtime::settle(slot, identity(tx)).await?;
    if after_identity != intent.identity {
        return Err(history::rejected(
            "P1 original transaction identity changed",
        ));
    }
    // Encode the actual OUTCOME before commit; predictable capacity errors must
    // not be discovered only after a successful DB commit. Its private commit
    // token is still required before these bytes may ever be appended.
    let output = Outcome {
        schema: history::OutcomeSchema::V1,
        version: 1,
        intent: intent_ref,
        after,
        claim,
        sql_before: low,
        sql_after: high,
        identity: after_identity,
        commit: history::CommitState::OriginalKnown,
    };
    let output_bytes = history::encode(&output, &owner.budget)?;
    owner.stage(slot, Stage::BeforeCommit).await?;
    let known = consume_commit(
        owned
            .take()
            .ok_or_else(|| history::rejected("P1 original commit missing"))?,
        slot,
    )
    .await?;
    owner.stage(slot, Stage::AfterCommit).await?;
    #[cfg(test)]
    if owner
        .hooks
        .as_ref()
        .is_some_and(|h| h.lose_commit_knowledge)
    {
        slot.poison();
        return Err(history::rejected("P1 commit knowledge deliberately lost"));
    }
    owner.stage(slot, Stage::BeforeOutcome).await?;
    let outcome_ref = append_committed(known, owner, slot, output_bytes).await?;
    owner.stage(slot, Stage::AfterOutcome).await?;
    slot.live()?;
    Ok(Claimed {
        transition: DurableTransition {
            intent: intent_ref,
            outcome: outcome_ref,
        },
        output,
        original: owner.clone(),
        slot: slot.clone(),
        request,
        continuation: outcome_ref,
        lease_ms: LEASE_MS,
    })
}

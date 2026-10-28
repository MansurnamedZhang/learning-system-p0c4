//! D1 continues the original owned stage. Disk/rows cannot construct its commit.
pub(super) mod observation;

use super::first_write::{self, RuntimeIdentity, StagedAttempt};
use crate::snapshot_broker::{
    ObservationError,
    history::{
        self, RecordRef, Result,
        runtime::{self, Original, Slot, Stage},
    },
};
use crate::{
    SnapshotStore,
    snapshot_observation::{JobObservation, OutboxObservation},
};
use chrono::{DateTime, Utc};
use learning_assets::CustodyWriter;
use learning_core::{JobInput, SnapshotManifest};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction, types::Json};
use std::sync::Arc;
use uuid::Uuid;

pub(super) async fn validate_observation(
    tx: &mut Transaction<'_, Postgres>,
) -> std::result::Result<(), learning_core::ContentError> {
    observation::validate(tx).await
}

// No SH, directory, Prepared, lease capability, Clone or deserialize remains in
// the result. Construction occurs only after both original durable appends.
pub(in super::super) struct PublishedComponent {
    _business: RecordRef,
    _closed: RecordRef,
    issue: RecordRef,
    package: RecordRef,
    manifest: String,
    final_lease_expiry: DateTime<Utc>,
}
impl PublishedComponent {
    pub(in crate::snapshot_broker) fn final_lease_expiry(&self) -> DateTime<Utc> {
        self.final_lease_expiry
    }
    pub(in crate::snapshot_broker) fn references(
        &self,
    ) -> (RecordRef, RecordRef, RecordRef, RecordRef, &str) {
        (
            self.issue,
            self.package,
            self._business,
            self._closed,
            &self.manifest,
        )
    }
}
struct KnownBusinessCommit;
struct DurableBusinessCommit(RecordRef);

#[derive(Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct RequestRow {
    actor_id: Uuid,
    request_id: Uuid,
    outbox_id: Uuid,
    job_id: Uuid,
}
#[derive(Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct KeyRow {
    actor_id: Uuid,
    request_id: Uuid,
    request_sha256: String,
    operation: String,
}
#[derive(Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Receipt {
    job_id: Uuid,
    stage_token: Uuid,
    capability: String,
    manifest_sha256: String,
    plan_sha256: String,
    required_spaces: Vec<Uuid>,
    created_at: DateTime<Utc>,
}
#[derive(Serialize)]
struct FullRows {
    job: JobObservation,
    outbox: OutboxObservation,
    request: RequestRow,
    key: KeyRow,
    receipt: Option<Receipt>,
    observed_at: DateTime<Utc>,
}
#[derive(sqlx::FromRow)]
struct Header {
    job_octets: i64,
    outbox_octets: i64,
    request_octets: i64,
    key_octets: i64,
    receipt_octets: i64,
}
#[derive(sqlx::FromRow)]
struct Body {
    job: Json<JobObservation>,
    outbox: Json<OutboxObservation>,
    request: Json<RequestRow>,
    key: Json<KeyRow>,
    receipt: Option<Json<Receipt>>,
    observed_at: DateTime<Utc>,
    job_octets: i64,
    outbox_octets: i64,
    request_octets: i64,
    key_octets: i64,
    receipt_octets: i64,
}
const HEADER: &str = "SELECT job_octets,outbox_octets,request_octets,key_octets,receipt_octets FROM public.p0c4_snapshot_publication_observe_v1($1,$2,$3,$4,$5,false)";
const BODY: &str = "SELECT job_row AS job,outbox_row AS outbox,request_row AS request,key_row AS key,receipt_row AS receipt,observed_at,job_octets,outbox_octets,request_octets,key_octets,receipt_octets FROM public.p0c4_snapshot_publication_observe_v1($1,$2,$3,$4,$5,true)";

enum ReadPhase {
    Before,
    After,
}
impl ReadPhase {
    fn fixed_value(&self) -> &'static str {
        match self {
            Self::Before => "before",
            Self::After => "after",
        }
    }
}

async fn rows(
    tx: &mut Transaction<'_, Postgres>,
    owner: &Original,
    slot: &Slot,
    lease: &crate::JobLease,
    request: Uuid,
    phase: ReadPhase,
) -> Result<FullRows> {
    observer_profile(tx, owner, slot).await?;
    owner.observation.charge(512)?;
    let head: Header = runtime::settle(slot, async {
        sqlx::query_as(HEADER)
            .bind(lease.job_id)
            .bind(lease.token)
            .bind(request)
            .bind(lease.lease_expires_at)
            .bind(phase.fixed_value())
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| history::rejected("D1 full input binding missing"))
    })
    .await?;
    observer_profile(tx, owner, slot).await?;
    let sizes = [
        head.job_octets,
        head.outbox_octets,
        head.request_octets,
        head.key_octets,
        head.receipt_octets,
    ];
    if sizes[..4].iter().any(|n| *n <= 0 || *n > 1_048_576) || !(0..=1_048_576).contains(&sizes[4])
    {
        return Err(ObservationError::Capacity);
    }
    let bytes = sizes
        .iter()
        .try_fold(512u64, |sum, n| sum.checked_add(*n as u64))
        .ok_or(ObservationError::Capacity)?;
    owner.observation.charge(bytes)?;
    observer_profile(tx, owner, slot).await?;
    let body: Body = runtime::settle(slot, async {
        Ok(sqlx::query_as(BODY)
            .bind(lease.job_id)
            .bind(lease.token)
            .bind(request)
            .bind(lease.lease_expires_at)
            .bind(phase.fixed_value())
            .fetch_one(&mut **tx)
            .await?)
    })
    .await?;
    observer_profile(tx, owner, slot).await?;
    if sizes
        != [
            body.job_octets,
            body.outbox_octets,
            body.request_octets,
            body.key_octets,
            body.receipt_octets,
        ]
    {
        return Err(history::rejected(
            "D1 full rows changed after charged header",
        ));
    }
    Ok(FullRows {
        job: body.job.0,
        outbox: body.outbox.0,
        request: body.request.0,
        key: body.key.0,
        receipt: body.receipt.map(|r| r.0),
        observed_at: body.observed_at,
    })
}

#[derive(Serialize)]
struct TransactionIdentity {
    runtime: RuntimeIdentity,
    backend: i32,
    transaction: String,
}
async fn identity(
    tx: &mut Transaction<'_, Postgres>,
    owner: &Original,
    slot: &Slot,
    attempt: &StagedAttempt,
) -> Result<TransactionIdentity> {
    owner.observation.charge(2048)?;
    let actual: RuntimeIdentity = runtime::settle(slot, async {
        Ok(sqlx::query_as(first_write::RUNTIME_IDENTITY)
            .fetch_one(&mut **tx)
            .await?)
    })
    .await?;
    first_write::restricted_runtime_for_owner(owner, &actual)?;
    first_write::original_database(&actual, &attempt.copied.claimed.output().identity)?;
    if actual.server_address != attempt.runtime_identity.server_address
        || actual.server_port != attempt.runtime_identity.server_port
    {
        return Err(history::rejected("D1 original runtime endpoint changed"));
    }
    let (backend, transaction): (i32, String) = runtime::settle(slot, async {
        Ok(sqlx::query_as(
            "SELECT pg_catalog.pg_backend_pid(),pg_catalog.pg_current_xact_id()::text",
        )
        .fetch_one(&mut **tx)
        .await?)
    })
    .await?;
    Ok(TransactionIdentity {
        runtime: actual,
        backend,
        transaction,
    })
}

const MIGRATION: &str = include_str!("../../../../../migrations/0014_snapshot_jobs.sql");
fn function_body(name: &str) -> Result<&'static str> {
    let marker = match name {
        "complete" => "CREATE FUNCTION public.p0c3_complete_snapshot_job(",
        "succeed" => "CREATE FUNCTION public.p0c3_succeed_snapshot_job(",
        _ => return Err(history::rejected("D1 fixed function source required")),
    };
    let mut parts = MIGRATION.split(marker);
    parts.next();
    let tail = parts
        .next()
        .ok_or_else(|| history::rejected("D1 canonical function missing"))?;
    if parts.next().is_some() {
        return Err(history::rejected("D1 duplicate canonical function"));
    }
    tail.split_once("AS $$")
        .and_then(|(_, body)| body.split_once("$$;"))
        .map(|(body, _)| body)
        .ok_or_else(|| history::rejected("D1 canonical function delimiters changed"))
}
async fn observer_profile(
    tx: &mut Transaction<'_, Postgres>,
    owner: &Original,
    slot: &Slot,
) -> Result<()> {
    owner
        .observation
        .charge(4096 + (observation::SOURCE.len() + observation::SIGNATURE.len()) as u64 * 2)?;
    runtime::settle(slot, async {
        observation::validate(tx)
            .await
            .map_err(|_| history::rejected("D1 publication observation profile changed"))
    })
    .await
}
async fn canonical(
    tx: &mut Transaction<'_, Postgres>,
    owner: &Original,
    slot: &Slot,
) -> Result<()> {
    observer_profile(tx, owner, slot).await?;
    // Also bind the directly invoked nested succeed function. These are SELECT
    // comparisons only: C0's three-function profile and all grants stay intact.
    const PROFILE: &str = "SELECT count(*)=1 AND COALESCE(bool_and(p.proowner='learning_admin'::regrole AND p.prosecdef AND NOT p.proisstrict AND NOT p.proleakproof AND p.provolatile='v' AND p.proparallel='u' AND p.prokind='f' AND NOT p.proretset AND p.prorettype='boolean'::regtype AND p.pronargdefaults=0 AND p.provariadic=0 AND p.prolang=(SELECT oid FROM pg_catalog.pg_language WHERE lanname='plpgsql') AND p.proargtypes=$3::oidvector AND p.proargnames=$4::text[] AND p.proconfig=ARRAY['search_path=pg_catalog, public, pg_temp']::text[] AND p.prosrc=$2 AND (SELECT count(*)=2 AND bool_and(a.privilege_type='EXECUTE' AND NOT a.is_grantable AND a.grantor=p.proowner AND a.grantee IN (p.proowner,'learning_runtime'::regrole)) FROM pg_catalog.aclexplode(p.proacl) a)),false) FROM pg_catalog.pg_proc p WHERE p.oid=pg_catalog.to_regprocedure($1)";
    for (name, signature, types, names) in [
        (
            "complete",
            "public.p0c3_complete_snapshot_job(uuid,uuid,text,text,text,uuid[])",
            "2950 2950 25 25 25 2951",
            vec![
                "p_job_id",
                "p_token",
                "p_capability",
                "p_manifest",
                "p_plan",
                "p_spaces",
            ],
        ),
        (
            "succeed",
            "public.p0c3_succeed_snapshot_job(uuid,uuid,text)",
            "2950 2950 25",
            vec!["p_job_id", "p_token", "p_digest"],
        ),
    ] {
        let body = function_body(name)?;
        owner.observation.charge(2048 + body.len() as u64 * 2)?;
        let valid: bool = runtime::settle(slot, async {
            Ok(sqlx::query_scalar(PROFILE)
                .bind(signature)
                .bind(body)
                .bind(types)
                .bind(names)
                .fetch_one(&mut **tx)
                .await?)
        })
        .await?;
        if !valid {
            return Err(history::rejected(
                "D1 canonical publication function boundary changed",
            ));
        }
    }
    Ok(())
}

fn before_matches(owner: &Original, attempt: &StagedAttempt, before: &FullRows) -> Result<()> {
    let original = attempt.copied.claimed.output();
    let encoded = history::encode(&before.outbox.payload, &owner.budget)?;
    owner
        .budget
        .charge(
            (encoded.len() as u64)
                .checked_mul(8)
                .ok_or(ObservationError::Capacity)?,
            0,
        )
        .map_err(|_| ObservationError::Capacity)?;
    let input = JobInput::from_value(before.outbox.payload.clone())
        .map_err(|_| history::rejected("D1 original input invalid"))?;
    if before.job != original.after.job
        || before.outbox != original.after.outbox
        || before.receipt.is_some()
        || before.observed_at < original.after.observed_at
        || before.observed_at >= original.claim.expires_at
        || before.request.actor_id != before.outbox.actor_id
        || before.request.request_id != attempt.copied.claimed.request()
        || before.request.job_id != before.job.id
        || before.request.outbox_id != before.outbox.id
        || before.key.actor_id != before.request.actor_id
        || before.key.request_id != before.request.request_id
        || before.key.operation != "snapshot_export"
        || before.key.request_sha256 != history::digest(&encoded, &owner.budget)?
        || !attempt
            .copied
            .prepared
            .broker_matches(&attempt.copied.claimed.lease(), &input)
    {
        return Err(history::rejected(
            "D1 original full claim/request/key changed",
        ));
    }
    Ok(())
}
fn after_matches(
    before: &FullRows,
    after: &FullRows,
    attempt: &StagedAttempt,
    plan: (&str, &str, &[Uuid]),
    low: DateTime<Utc>,
    high: DateTime<Utc>,
) -> Result<()> {
    let Some(receipt) = &after.receipt else {
        return Err(history::rejected("D1 complete receipt missing"));
    };
    let b = &before.job;
    let a = &after.job;
    if low > high
        || after.observed_at < high
        || before.outbox != after.outbox
        || before.request != after.request
        || before.key != after.key
        || a.id != b.id
        || a.outbox_id != b.outbox_id
        || a.idempotency_key != b.idempotency_key
        || a.created_at != b.created_at
        || a.checkpoint != b.checkpoint
        || a.attempt_count != b.attempt_count
        || a.status != "succeeded"
        || a.lease_token.is_some()
        || a.lease_expires_at.is_some()
        || a.next_attempt_at.is_some()
        || a.last_error_class.is_some()
        || a.output_digest.as_deref()
            != Some(attempt.copied.staged()?.directory().manifest_sha256())
        || a.updated_at < low
        || a.updated_at > high
        || receipt.job_id != b.id
        || receipt.stage_token != attempt.copied.claimed.lease().token
        || receipt.capability != plan.0
        || receipt.manifest_sha256 != attempt.copied.staged()?.directory().manifest_sha256()
        || receipt.plan_sha256 != plan.1
        || receipt.required_spaces != plan.2
        || receipt.created_at < low
        || receipt.created_at > high
    {
        return Err(history::rejected(
            "D1 canonical full completion delta rejected",
        ));
    }
    Ok(())
}

#[derive(Serialize)]
enum BusinessSchema {
    #[serde(rename = "knowweave.snapshot-publication.d1.business-commit-known")]
    V1,
}
#[derive(Serialize)]
enum CloseSchema {
    #[serde(rename = "knowweave.snapshot-publication.d1.write-closed")]
    V1,
}
#[derive(Serialize)]
enum BusinessEvent {
    #[serde(rename = "BUSINESS_COMMIT_KNOWN")]
    Known,
}
#[derive(Serialize)]
enum CloseEvent {
    #[serde(rename = "WRITE_CLOSED")]
    Closed,
}
#[derive(Serialize)]
enum BusinessKnowledge {
    #[serde(rename = "same_original_business_commit_returned_success")]
    Original,
}
#[derive(Serialize)]
enum ClosedOutcome {
    #[serde(rename = "succeeded")]
    Succeeded,
}
#[derive(Serialize)]
struct BusinessRecord<'a> {
    schema: BusinessSchema,
    version: u8,
    event: BusinessEvent,
    epoch: Uuid,
    binding: Uuid,
    request: Uuid,
    previous: RecordRef,
    issue: RecordRef,
    package: RecordRef,
    claim_committed: RecordRef,
    permit_nonce: Uuid,
    before: &'a FullRows,
    after: &'a FullRows,
    runtime: &'a TransactionIdentity,
    sql_before: DateTime<Utc>,
    sql_after: DateTime<Utc>,
    commit: BusinessKnowledge,
}
#[derive(Serialize)]
struct CloseRecord<'a> {
    schema: CloseSchema,
    version: u8,
    event: CloseEvent,
    epoch: Uuid,
    binding: Uuid,
    request: Uuid,
    previous: RecordRef,
    business_commit: RecordRef,
    issue: RecordRef,
    package: RecordRef,
    permit_nonce: Uuid,
    manifest_sha256: &'a str,
    outcome: ClosedOutcome,
}

async fn clock(tx: &mut Transaction<'_, Postgres>, slot: &Slot) -> Result<DateTime<Utc>> {
    runtime::settle(slot, async {
        Ok(sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
            .fetch_one(&mut **tx)
            .await?)
    })
    .await
}
async fn commit(
    tx: Transaction<'_, Postgres>,
    owner: &Original,
    slot: &Slot,
) -> Result<KnownBusinessCommit> {
    runtime::settle_started(slot, async {
        tx.commit().await?;
        // The fixture pauses the already-executed original COMMIT return,
        // never a mock network error or a replacement/reopened transaction.
        owner.stage(slot, Stage::AfterBusinessCommit).await?;
        Ok(())
    })
    .await?;
    slot.live()?;
    #[cfg(test)]
    if owner
        .hooks
        .as_ref()
        .is_some_and(|h| h.stage == Stage::LoseBusinessCommitKnowledge)
    {
        slot.poison();
        return Err(history::rejected(
            "D1 original business commit knowledge deliberately lost",
        ));
    }
    Ok(KnownBusinessCommit)
}
async fn append_business(
    _known: KnownBusinessCommit,
    owner: &Arc<Original>,
    slot: &Slot,
    body: Vec<u8>,
) -> Result<DurableBusinessCommit> {
    Ok(DurableBusinessCommit(owner.append(slot, body).await?))
}

async fn transaction(
    owner: &Arc<Original>,
    slot: &Arc<Slot>,
    store: &SnapshotStore,
    attempt: &mut StagedAttempt,
) -> Result<(KnownBusinessCommit, Vec<u8>)> {
    let mut connection = owner.acquire_runtime(slot).await?;
    let result = async {
        let mut tx = Some(
            runtime::settle(slot, async {
                match connection.as_mut() {
                    Some(connection) => {
                        SnapshotStore::broker_begin_publication_on(connection).await
                    }
                    None => store.broker_begin_publication().await,
                }
                .map_err(|_| history::rejected("D1 original runtime begin failed"))
            })
            .await?,
        );
        let result = transaction_inner(owner, slot, attempt, &mut tx).await;
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
async fn transaction_inner(
    owner: &Arc<Original>,
    slot: &Arc<Slot>,
    attempt: &mut StagedAttempt,
    owned: &mut Option<Transaction<'_, Postgres>>,
) -> Result<(KnownBusinessCommit, Vec<u8>)> {
    attempt.copied.claimed.check_original(owner, slot)?;
    let tx = owned
        .as_mut()
        .ok_or_else(|| history::rejected("D1 original publication transaction missing"))?;
    owner.observation.charge(2048)?;
    runtime::settle(slot, async {
        attempt
            .copied
            .prepared
            .broker_initialize_publication(tx)
            .await
            .map_err(|_| history::rejected("D1 original runtime initialization failed"))
    })
    .await?;
    let original_identity = identity(tx, owner, slot, attempt).await?;
    canonical(tx, owner, slot).await?;
    let original_lease = attempt.copied.claimed.lease();
    let before = rows(
        tx,
        owner,
        slot,
        &original_lease,
        attempt.copied.claimed.request(),
        ReadPhase::Before,
    )
    .await?;
    before_matches(owner, attempt, &before)?;
    let plan = runtime::settle(slot, async {
        attempt
            .copied
            .prepared
            .broker_recheck(tx, &owner.budget)
            .await
            .map_err(|_| history::rejected("D1 original actor ACL or plan recheck failed"))
    })
    .await?;
    slot.live()?;
    // Fresh no-follow fixed-four-file metadata is charged before actual reads
    // by the assets verifier; historical sizes never pay a later changed file.
    attempt
        .copied
        .staged()?
        .revalidate()
        .map_err(|_| history::rejected("D1 original custody changed"))?;
    // The actual blocking verifier owns the same staged package+SH. Only it can
    // return that value; no directory/guard extraction or unregistered writer.
    let staged = attempt
        .copied
        .staged
        .take()
        .ok_or_else(|| history::rejected("D1 original staged value missing before verify"))?;
    let verify_budget = owner.budget.clone();
    let task = tokio::task::spawn_blocking(move || {
        let result = staged
            .directory()
            .verify_reading_copy_with_budget(&verify_budget);
        (staged, result)
    });
    let (staged, verified) = runtime::settle_started(slot, async {
        task.await
            .map_err(|_| history::rejected("D1 original verifier panicked"))
    })
    .await?;
    attempt.copied.staged = Some(staged);
    slot.live()?;
    let verified = verified
        .map_err(|_| history::rejected("D1 actual publication package verification failed"))?;
    if !plan.accepts(&verified.manifest)
        || !matches!(&verified.manifest, SnapshotManifest::ReadingCopyV1(m) if m.copy_id==before.job.id)
        || verified.files.len() as u64 != attempt.copied.verified_files
        || verified.files.iter().map(|f| f.size).sum::<u64>() != attempt.copied.verified_bytes
    {
        return Err(history::rejected("D1 actual package binding changed"));
    }
    attempt
        .copied
        .staged()?
        .revalidate()
        .map_err(|_| history::rejected("D1 original custody changed"))?;
    let low = clock(tx, slot).await?;
    let accepted = runtime::settle(slot, async {
        plan.complete(
            tx,
            &attempt.copied.prepared,
            attempt.copied.staged()?.directory(),
        )
        .await
        .map_err(|_| history::rejected("D1 canonical complete SQL failed"))
    })
    .await?;
    if !accepted {
        return Err(history::rejected(
            "D1 canonical complete rejected original lease",
        ));
    }
    let high = clock(tx, slot).await?;
    let after = rows(
        tx,
        owner,
        slot,
        &original_lease,
        before.request.request_id,
        ReadPhase::After,
    )
    .await?;
    after_matches(
        &before,
        &after,
        attempt,
        (plan.capability(), plan.digest(), plan.spaces()),
        low,
        high,
    )?;
    canonical(tx, owner, slot).await?;
    let final_identity = identity(tx, owner, slot, attempt).await?;
    if original_identity.backend != final_identity.backend
        || original_identity.transaction != final_identity.transaction
    {
        return Err(history::rejected("D1 original runtime transaction changed"));
    }
    let previous = owner.head(slot).await?;
    if previous != attempt.observation {
        return Err(history::rejected("D1 original package prefix changed"));
    }
    let encoded = history::encode(
        &BusinessRecord {
            schema: BusinessSchema::V1,
            version: 1,
            event: BusinessEvent::Known,
            epoch: owner.epoch,
            binding: owner.binding,
            request: attempt.copied.claimed.request(),
            previous,
            issue: attempt.copied.issue.reference,
            package: attempt.observation,
            claim_committed: attempt.copied.claimed.records().1,
            permit_nonce: attempt.copied.issue.nonce,
            before: &before,
            after: &after,
            runtime: &original_identity,
            sql_before: low,
            sql_after: high,
            commit: BusinessKnowledge::Original,
        },
        &owner.budget,
    )?;
    // Reserve headroom before COMMIT for both actual journal appends, all
    // prefix scans, close encoding/hash and metadata. Actual work also charges.
    owner.preflight()?;
    owner.stage(slot, Stage::BeforeBusinessCommit).await?;
    let known = commit(
        owned
            .take()
            .ok_or_else(|| history::rejected("D1 original commit missing"))?,
        owner,
        slot,
    )
    .await?;
    Ok((known, encoded))
}

pub(in super::super) async fn drive(
    owner: Arc<Original>,
    slot: Arc<Slot>,
    job: Uuid,
    request: Uuid,
    runtime_pool: PgPool,
    writer: CustodyWriter,
) -> Result<PublishedComponent> {
    let store = SnapshotStore::new(runtime_pool.clone());
    let attempt = first_write::stage(
        owner.clone(),
        slot.clone(),
        job,
        request,
        runtime_pool,
        writer,
    )
    .await?;
    finish(owner, slot, request, store, attempt).await
}
pub(in crate::snapshot_broker) async fn drive_claimed(
    owner: Arc<Original>,
    slot: Arc<Slot>,
    claimed: crate::snapshot_broker::transition::Claimed,
    runtime_pool: PgPool,
    writer: CustodyWriter,
) -> Result<PublishedComponent> {
    let request = claimed.request();
    let store = SnapshotStore::new(runtime_pool.clone());
    let attempt =
        first_write::stage_claimed(owner.clone(), slot.clone(), claimed, runtime_pool, writer)
            .await?;
    finish(owner, slot, request, store, attempt).await
}
async fn finish(
    owner: Arc<Original>,
    slot: Arc<Slot>,
    request: Uuid,
    store: SnapshotStore,
    mut attempt: StagedAttempt,
) -> Result<PublishedComponent> {
    owner.stage(&slot, Stage::BeforePublish).await?;
    let (known, encoded) = transaction(&owner, &slot, &store, &mut attempt).await?;
    owner.stage(&slot, Stage::BeforeBusinessEvent).await?;
    let DurableBusinessCommit(business) = append_business(known, &owner, &slot, encoded).await?;
    slot.live()?;
    let previous = owner.head(&slot).await?;
    if previous != business {
        return Err(history::rejected("D1 original business prefix changed"));
    }
    let closed = history::encode(
        &CloseRecord {
            schema: CloseSchema::V1,
            version: 1,
            event: CloseEvent::Closed,
            epoch: owner.epoch,
            binding: owner.binding,
            request,
            previous,
            business_commit: business,
            issue: attempt.copied.issue.reference,
            package: attempt.observation,
            permit_nonce: attempt.copied.issue.nonce,
            manifest_sha256: attempt.copied.staged()?.directory().manifest_sha256(),
            outcome: ClosedOutcome::Succeeded,
        },
        &owner.budget,
    )?;
    owner.stage(&slot, Stage::BeforeCloseEvent).await?;
    let closed = owner.append(&slot, closed).await?;
    slot.live()?;
    attempt
        .copied
        .staged()?
        .revalidate()
        .map_err(|_| history::rejected("D1 final original custody changed"))?;
    owner
        .budget
        .charge(64, 0)
        .map_err(|_| ObservationError::Capacity)?;
    let manifest = attempt
        .copied
        .staged()?
        .directory()
        .manifest_sha256()
        .to_owned();
    let issue = attempt.copied.issue.reference;
    let package = attempt.observation;
    let final_lease_expiry = attempt.copied.claimed.lease().lease_expires_at;
    drop(attempt);
    Ok(PublishedComponent {
        _business: business,
        _closed: closed,
        issue,
        package,
        manifest,
        final_lease_expiry,
    })
}

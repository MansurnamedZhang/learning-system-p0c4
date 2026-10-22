//! Private C2+D0 staging and D1 original-operation publication. No admission.
use super::history::{self, Claim, Result, Rows};
use chrono::{DateTime, Utc};
#[cfg(target_os = "linux")]
mod publication;

#[cfg(target_os = "linux")]
pub(super) async fn install_publication_observation(
    pool: &sqlx::PgPool,
) -> std::result::Result<(), learning_core::ContentError> {
    publication::observation::install(pool).await
}
#[cfg(target_os = "linux")]
pub(super) async fn validate_publication_observation(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<()> {
    publication::validate_observation(tx)
        .await
        .map_err(|_| history::rejected("publication observation profile rejected"))
}

pub(super) fn validate_issue_rows(
    original: &Rows,
    claim: &Claim,
    current: &Rows,
    clock: DateTime<Utc>,
) -> Result<()> {
    validate_issue_rows_inner(original, claim, current, clock, false)
}
fn validate_issue_rows_inner(
    original: &Rows,
    claim: &Claim,
    current: &Rows,
    clock: DateTime<Utc>,
    versioned: bool,
) -> Result<()> {
    if original.job != current.job
        || original.outbox != current.outbox
        || current.job.id != claim.job_id
        || current.job.status != "snapshot_running"
        || current.job.attempt_count != claim.attempts
        || if versioned {
            !(1..=3).contains(&claim.attempts)
        } else {
            claim.attempts != 1
        }
        || current.job.lease_token != Some(claim.token)
        || current.job.lease_expires_at != Some(claim.expires_at)
        || current.observed_at < original.observed_at
        || clock < current.observed_at
        || clock >= claim.expires_at
    {
        return Err(history::rejected(
            "C2 original claim binding or database expiry changed",
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
mod first_write {
    use super::super::{
        history::{
            DatabaseIdentity, RecordRef,
            runtime::{self, Original, Slot, Stage},
        },
        transaction,
        transition::{self, Claimed},
    };
    use super::*;
    use crate::{PreparedSnapshotExport, SnapshotStore};
    use learning_assets::{CustodyStaged, CustodyWriter, verify_snapshot};
    use learning_core::{JobInput, SnapshotManifest};
    use serde::Serialize;
    use sqlx::{PgPool, Postgres, Transaction};
    use std::sync::Arc;
    use uuid::Uuid;

    #[derive(Serialize, sqlx::FromRow)]
    pub(super) struct RuntimeIdentity {
        pub(super) database: String,
        pub(super) oid: i64,
        pub(super) postmaster: DateTime<Utc>,
        pub(super) session_user: String,
        pub(super) current_user: String,
        pub(super) original_login: Option<String>,
        pub(super) restricted_login: bool,
        pub(super) server_address: Option<String>,
        pub(super) server_port: Option<i32>,
    }
    pub(super) const RUNTIME_IDENTITY: &str = "SELECT current_database()::text AS database,(SELECT oid::bigint FROM pg_catalog.pg_database WHERE datname=current_database()) AS oid,pg_catalog.pg_postmaster_start_time() AS postmaster,session_user::text AS session_user,current_user::text AS current_user,system_user AS original_login,(r.rolcanlogin AND NOT r.rolsuper AND NOT r.rolcreatedb AND NOT r.rolcreaterole AND NOT r.rolreplication AND NOT r.rolbypassrls AND NOT EXISTS(SELECT FROM pg_catalog.pg_auth_members m WHERE m.member=r.oid) AND NOT EXISTS(SELECT FROM pg_catalog.pg_shdepend d WHERE d.refclassid='pg_catalog.pg_authid'::regclass AND d.refobjid=r.oid AND d.deptype='o')) AS restricted_login,pg_catalog.inet_server_addr()::text AS server_address,pg_catalog.inet_server_port() AS server_port FROM pg_catalog.pg_roles r WHERE r.rolname=session_user";

    pub(super) fn original_database(
        actual: &RuntimeIdentity,
        expected: &DatabaseIdentity,
    ) -> Result<()> {
        if actual.database != expected.database
            || actual.oid != expected.oid
            || actual.postmaster != expected.postmaster
        {
            return Err(history::rejected("C2 source database identity mismatch"));
        }
        Ok(())
    }
    pub(super) fn restricted_runtime(actual: &RuntimeIdentity) -> Result<()> {
        let authenticated = actual
            .original_login
            .as_deref()
            .and_then(|v| v.split_once(':'))
            .is_some_and(|(method, login)| !method.is_empty() && login == "learning_runtime");
        if actual.session_user != "learning_runtime"
            || actual.current_user != "learning_runtime"
            || !actual.restricted_login
            || !authenticated
        {
            return Err(history::rejected(
                "C2 real restricted runtime LOGIN required",
            ));
        }
        Ok(())
    }
    async fn runtime_identity(
        owner: &Original,
        slot: &Slot,
        runtime_pool: &PgPool,
        expected: &DatabaseIdentity,
    ) -> Result<RuntimeIdentity> {
        let broker_options = owner.pool.connect_options();
        let runtime_options = runtime_pool.connect_options();
        if runtime_options.get_username() != "learning_runtime"
            || runtime_options.get_host() != broker_options.get_host()
            || runtime_options.get_port() != broker_options.get_port()
            || runtime_options.get_socket() != broker_options.get_socket()
            || runtime_options.get_database() != broker_options.get_database()
        {
            return Err(history::rejected(
                "C2 runtime endpoint or original login mismatch",
            ));
        }
        owner.observation.charge(1024)?;
        let actual: RuntimeIdentity = runtime::settle(slot, async {
            Ok(sqlx::query_as(RUNTIME_IDENTITY)
                .fetch_one(runtime_pool)
                .await?)
        })
        .await?;
        restricted_runtime(&actual)?;
        original_database(&actual, expected)?;
        Ok(actual)
    }

    // New component schema, deliberately separate from P1 and from any future
    // NoEffect/AlreadyCommitted or business-publication record. Serialize only:
    // reading bytes back never reconstructs any of the private capabilities.
    #[derive(Serialize)]
    enum IssueSchema {
        #[serde(rename = "knowweave.snapshot-first-write.c2d0.issue")]
        V1,
    }
    #[derive(Serialize)]
    enum PackageSchema {
        #[serde(rename = "knowweave.snapshot-first-write.c2d0.staged")]
        V1,
    }
    #[derive(Serialize)]
    enum IssueEvent {
        #[serde(rename = "WRITE_AUTHORITY_ISSUED")]
        Issued,
    }
    #[derive(Serialize)]
    enum PackageEvent {
        #[serde(rename = "PACKAGE_STAGED")]
        Staged,
    }
    #[derive(Serialize)]
    struct IssueRecord<'a> {
        schema: IssueSchema,
        version: u8,
        event: IssueEvent,
        epoch: Uuid,
        binding: Uuid,
        request: Uuid,
        previous: RecordRef,
        claim_intent: RecordRef,
        claim_committed: RecordRef,
        claim: &'a Claim,
        input_sha256: String,
        rows: &'a Rows,
        database_clock: DateTime<Utc>,
        runtime: &'a RuntimeIdentity,
        permit_nonce: Uuid,
    }
    #[derive(Serialize)]
    struct PackageRecord<'a> {
        schema: PackageSchema,
        version: u8,
        event: PackageEvent,
        epoch: Uuid,
        binding: Uuid,
        request: Uuid,
        previous: RecordRef,
        issue: RecordRef,
        claim_committed: RecordRef,
        claim: &'a Claim,
        manifest_sha256: &'a str,
        verified_files: u64,
        verified_bytes: u64,
        permit_nonce: Uuid,
    }
    pub(super) struct IssueReceipt {
        pub(super) reference: RecordRef,
        pub(super) nonce: Uuid,
    }
    struct WritePermit {
        claimed: Claimed,
        issue: IssueReceipt,
        writer: CustodyWriter,
    }
    // OS guard is inseparable from the actual staged package. No permit/capture
    // extraction, public constructor, Clone, deserialization or lease adoption.
    pub(in super::super) struct StagedComponent {
        staged: CustodyStaged,
        issue: RecordRef,
        observation: RecordRef,
    }
    #[cfg(test)]
    impl StagedComponent {
        pub(in super::super) fn directory(&self) -> &learning_assets::SnapshotDirectory {
            self.staged.directory()
        }
        pub(in super::super) fn sequences(&self) -> (u64, u64) {
            (self.issue.sequence(), self.observation.sequence())
        }
    }

    async fn issue(
        owner: &Arc<Original>,
        slot: &Arc<Slot>,
        claimed: Claimed,
        runtime_identity: &RuntimeIdentity,
        writer: CustodyWriter,
    ) -> Result<WritePermit> {
        claimed.check_original(owner, slot)?;
        let mut tx = Some(runtime::settle(slot, transaction::begin_owned(&owner.pool)).await?);
        let result = issue_locked(
            owner,
            slot,
            &claimed,
            runtime_identity,
            tx.as_mut()
                .ok_or_else(|| history::rejected("C2 missing original issue transaction"))?,
        )
        .await;
        // No job/outbox lock survives into the long blocking package copy. A
        // late rollback remains settled by the original owner without adoption.
        if let Some(tx) = tx.take() {
            runtime::settle_started(slot, async {
                tx.rollback().await?;
                Ok(())
            })
            .await?;
        }
        let issue = result?;
        slot.live()?;
        Ok(WritePermit {
            claimed,
            issue,
            writer,
        })
    }
    async fn issue_locked(
        owner: &Arc<Original>,
        slot: &Arc<Slot>,
        claimed: &Claimed,
        runtime_identity: &RuntimeIdentity,
        tx: &mut Transaction<'static, Postgres>,
    ) -> Result<IssueReceipt> {
        let original = claimed.output();
        let current = Rows::from(
            runtime::settle(
                slot,
                transaction::read_locked(tx, &owner.observation, original.claim.job_id),
            )
            .await?
            .ok_or_else(|| history::rejected("C2 current original claim missing"))?,
        );
        owner.observation.charge(1024)?;
        let broker: RuntimeIdentity = runtime::settle(slot, async {
            Ok(sqlx::query_as(RUNTIME_IDENTITY)
                .fetch_one(&mut **tx)
                .await?)
        })
        .await?;
        original_database(&broker, &original.identity)?;
        if broker.session_user != super::super::BROKER_ROLE
            || broker.current_user != super::super::BROKER_ROLE
            || broker.server_address != runtime_identity.server_address
            || broker.server_port != runtime_identity.server_port
        {
            return Err(history::rejected("C2 actual source endpoint changed"));
        }
        let clock: DateTime<Utc> = runtime::settle(slot, async {
            Ok(sqlx::query_scalar("SELECT pg_catalog.clock_timestamp()")
                .fetch_one(&mut **tx)
                .await?)
        })
        .await?;
        if owner.is_installed() {
            validate_issue_rows_inner(&original.after, &original.claim, &current, clock, true)?;
        } else {
            validate_issue_rows(&original.after, &original.claim, &current, clock)?;
        }
        let (intent, committed) = claimed.records();
        let previous = owner.head(slot).await?;
        if previous != claimed.continuation() {
            return Err(history::rejected(
                "C2 original journal continuation changed",
            ));
        }
        let input = history::encode(&original.after.outbox, &owner.budget)?;
        let nonce = Uuid::new_v4();
        let record = IssueRecord {
            schema: IssueSchema::V1,
            version: 1,
            event: IssueEvent::Issued,
            epoch: owner.epoch,
            binding: owner.binding,
            request: claimed.request(),
            previous,
            claim_intent: intent,
            claim_committed: committed,
            claim: &original.claim,
            input_sha256: history::digest(&input, &owner.budget)?,
            rows: &current,
            database_clock: clock,
            runtime: runtime_identity,
            permit_nonce: nonce,
        };
        let encoded = history::encode(&record, &owner.budget)?;
        owner.preflight()?;
        // This gate precedes append start. Any append failure/ambiguity retains
        // issued=true and HOLD forever, including empty directories/cleanup.
        slot.issue_sticky()?;
        Ok(IssueReceipt {
            reference: owner.append(slot, encoded).await?,
            nonce,
        })
    }

    pub(super) struct Copied {
        pub(super) staged: Option<CustodyStaged>,
        pub(super) prepared: PreparedSnapshotExport,
        pub(super) claimed: Claimed,
        pub(super) issue: IssueReceipt,
        pub(super) verified_files: u64,
        pub(super) verified_bytes: u64,
    }
    struct CopyInput {
        lease: crate::JobLease,
        issue: IssueReceipt,
        writer: CustodyWriter,
    }
    struct CopyOutput {
        staged: Option<CustodyStaged>,
        prepared: PreparedSnapshotExport,
        issue: IssueReceipt,
        verified_files: u64,
        verified_bytes: u64,
    }
    pub(super) struct StagedAttempt {
        pub(super) copied: Copied,
        pub(super) observation: RecordRef,
        pub(super) runtime_identity: RuntimeIdentity,
    }
    impl Copied {
        pub(super) fn staged(&self) -> Result<&CustodyStaged> {
            self.staged
                .as_ref()
                .ok_or_else(|| history::rejected("D1 original staged value is in flight"))
        }
    }
    fn stage_copy(
        owner: Arc<Original>,
        slot: Arc<Slot>,
        permit: CopyInput,
        prepared: PreparedSnapshotExport,
    ) -> Result<CopyOutput> {
        slot.live()?;
        let CopyInput {
            lease,
            issue,
            writer,
        } = permit;
        let copy = prepared
            .broker_reading_copy()
            .map_err(|_| history::rejected("D0 prepared nonempty reading copy required"))?;
        // The original cumulative meter also pays for this actual prepared
        // body; there is no fresh meter attached to the writer or staged result.
        let _prepared_bytes = history::encode(copy, &owner.budget)?;
        let staged = writer
            .stage_copy_metered(lease.job_id, lease.token, copy, owner.budget.clone())
            .map_err(|_| history::rejected("D0 actual custody stage failed"))?;
        staged
            .revalidate()
            .map_err(|_| history::rejected("D0 held root identity changed"))?;
        let verified = verify_snapshot(staged.directory())
            .map_err(|_| history::rejected("D0 actual package verification failed"))?;
        staged
            .revalidate()
            .map_err(|_| history::rejected("D0 held root identity changed"))?;
        let SnapshotManifest::ReadingCopyV1(manifest) = &verified.manifest else {
            return Err(history::rejected("D0 exact packages unsupported"));
        };
        if manifest.copy_id != lease.job_id
            || verified.files.len() != 3
            || !verified.files.iter().all(|f| {
                matches!(
                    f.path.as_str(),
                    "reading.md" | "reading.html" | "validation.json"
                )
            })
        {
            return Err(history::rejected("D0 package shape rejected"));
        }
        let bytes = verified
            .files
            .iter()
            .try_fold(0u64, |n, f| n.checked_add(f.size))
            .ok_or(super::super::ObservationError::Capacity)?;
        // The custody-only metered entry prepaid this original second verify
        // (including manifest bytes) before any filesystem mutation. These
        // actual verifier metrics remain observations, not budget payments.
        #[cfg(test)]
        if let Some(hooks) = &owner.hooks
            && hooks.stage == Stage::AfterStageCopy
        {
            hooks.arrived.notify_one();
            let (lock, wake) = &hooks.blocking_release;
            let mut released = lock
                .lock()
                .map_err(|_| history::rejected("D0 fixture gate poisoned"))?;
            while !*released {
                released = wake
                    .wait(released)
                    .map_err(|_| history::rejected("D0 fixture gate poisoned"))?;
            }
        }
        // Both the actual staged package+SH and the same original owner remain
        // owned until this closure and its original JoinHandle have settled.
        Ok(CopyOutput {
            staged: Some(staged),
            prepared,
            issue,
            verified_files: verified.files.len() as u64,
            verified_bytes: bytes,
        })
    }
    pub(in super::super) async fn drive(
        owner: Arc<Original>,
        slot: Arc<Slot>,
        job: Uuid,
        request: Uuid,
        runtime_pool: PgPool,
        writer: CustodyWriter,
    ) -> Result<StagedComponent> {
        let StagedAttempt {
            copied,
            observation,
            ..
        } = stage(owner, slot, job, request, runtime_pool, writer).await?;
        Ok(StagedComponent {
            staged: copied
                .staged
                .ok_or_else(|| history::rejected("D0 original staged value missing"))?,
            issue: copied.issue.reference,
            observation,
        })
    }
    pub(super) async fn stage(
        owner: Arc<Original>,
        slot: Arc<Slot>,
        job: Uuid,
        request: Uuid,
        runtime_pool: PgPool,
        writer: CustodyWriter,
    ) -> Result<StagedAttempt> {
        let claimed = transition::claim(owner.clone(), slot.clone(), job, request).await?;
        stage_claimed(owner, slot, claimed, runtime_pool, writer).await
    }
    pub(super) async fn stage_claimed(
        owner: Arc<Original>,
        slot: Arc<Slot>,
        claimed: Claimed,
        runtime_pool: PgPool,
        writer: CustodyWriter,
    ) -> Result<StagedAttempt> {
        claimed.check_original(&owner, &slot)?;
        let input = JobInput::from_value(claimed.output().after.outbox.payload.clone())
            .map_err(|_| history::rejected("D0 original input rejected"))?;
        if !matches!(input,JobInput::SnapshotExport{request,..} if !request.include_personal) {
            return Err(history::rejected(
                "D0 exact export unsupported before issue",
            ));
        }
        let runtime_identity =
            runtime_identity(&owner, &slot, &runtime_pool, &claimed.output().identity).await?;
        let store = SnapshotStore::new(runtime_pool);
        let lease = claimed.lease();
        let prepared = runtime::settle(&slot, async {
            store
                .prepare_export(&lease)
                .await
                .map_err(|_| history::rejected("D0 original runtime prepare rejected"))
        })
        .await?;
        prepared
            .broker_reading_copy()
            .map_err(|_| history::rejected("D0 prepared reading copy required before issue"))?;
        owner.stage(&slot, Stage::BeforeIssue).await?;
        let permit = issue(&owner, &slot, claimed, &runtime_identity, writer).await?;
        owner.stage(&slot, Stage::AfterIssue).await?;
        owner.stage(&slot, Stage::BeforeStageSpawn).await?;
        slot.live()?;
        // The original async owner retains the sole claim. The blocking task
        // receives only this consumed, already-issued private copy input.
        let WritePermit {
            mut claimed,
            issue,
            writer,
        } = permit;
        let original_lease = claimed.lease();
        let lease_ms = claimed.lease_ms();
        let permit = CopyInput {
            lease: claimed.lease(),
            issue,
            writer,
        };
        let copy_owner = owner.clone();
        let copy_slot = slot.clone();
        let mut task = tokio::task::spawn_blocking(move || {
            stage_copy(copy_owner, copy_slot, permit, prepared)
        });
        let mut package_previous = None;
        // No live check, timeout wrapper or handle drop between spawn and this
        // unconditional original-owner settlement of the real blocking task.
        let copied = runtime::settle_started(&slot, async {
            if !owner.is_installed() {
                return task.await.map_err(|_| history::rejected("D0 original blocking copy panicked"))?;
            }
            let interval = std::time::Duration::from_millis((lease_ms as u64 / 3).clamp(250, 30_000));
            loop {
                tokio::select! {
                    result = &mut task => return result.map_err(|_| history::rejected("D0 original blocking copy panicked"))?,
                    _ = tokio::time::sleep(interval) => {
                        match super::super::attempts::transition(&owner, &slot, &mut claimed, super::super::attempts::Transition::Renew(lease_ms)).await {
                            Ok((reference,true)) => package_previous = Some(reference),
                            Ok((_,false)) => {
                                slot.poison();
                                let _ = (&mut task).await;
                                return Err(history::rejected("original copy renewal was not acquired"));
                            }
                            Err(error) => {
                                slot.poison();
                                // Renewal ambiguity never detaches the already
                                // started writer or returns either held SH.
                                let _ = (&mut task).await;
                                return Err(error);
                            }
                        }
                    }
                }
            }
        })
        .await;
        let mut copied = match copied {
            Ok(copied) => copied,
            Err(error) => {
                slot.poison();
                // The original supervisor's separate SH is still owned even
                // after a consumed OS writer has returned its actual error.
                let _ = owner.stage(&slot, Stage::AfterStageError).await;
                return Err(error);
            }
        };
        if package_previous.is_some() {
            copied
                .prepared
                .broker_apply_known_renewal(&original_lease, claimed.lease())
                .map_err(|_| history::rejected("original prepared renewal differs"))?;
        }
        let copied = Copied {
            staged: copied.staged,
            prepared: copied.prepared,
            claimed,
            issue: copied.issue,
            verified_files: copied.verified_files,
            verified_bytes: copied.verified_bytes,
        };
        slot.live()?;
        copied
            .staged()?
            .revalidate()
            .map_err(|_| history::rejected("D0 original staged root identity changed"))?;
        let previous = owner.head(&slot).await?;
        if previous != package_previous.unwrap_or(copied.issue.reference) {
            return Err(history::rejected("D0 original issue prefix changed"));
        }
        let record = PackageRecord {
            schema: PackageSchema::V1,
            version: 1,
            event: PackageEvent::Staged,
            epoch: owner.epoch,
            binding: owner.binding,
            request: copied.claimed.request(),
            previous,
            issue: copied.issue.reference,
            claim_committed: copied.claimed.records().1,
            claim: &copied.claimed.output().claim,
            manifest_sha256: copied.staged()?.directory().manifest_sha256(),
            verified_files: copied.verified_files,
            verified_bytes: copied.verified_bytes,
            permit_nonce: copied.issue.nonce,
        };
        let encoded = history::encode(&record, &owner.budget)?;
        owner.preflight()?;
        let observation = owner.append(&slot, encoded).await?;
        slot.live()?;
        Ok(StagedAttempt {
            copied,
            observation,
            runtime_identity,
        })
    }
}
#[cfg(target_os = "linux")]
pub(super) use first_write::{StagedComponent, drive};
#[cfg(target_os = "linux")]
pub(super) async fn publish_claimed(
    owner: std::sync::Arc<history::runtime::Original>,
    slot: std::sync::Arc<history::runtime::Slot>,
    claimed: super::transition::Claimed,
    runtime: sqlx::PgPool,
    writer: learning_assets::CustodyWriter,
) -> Result<PublishedComponent> {
    publication::drive_claimed(owner, slot, claimed, runtime, writer).await
}
#[cfg(target_os = "linux")]
pub(super) use publication::{PublishedComponent, drive as publish};

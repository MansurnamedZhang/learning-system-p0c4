//! Private C2+D0 FIRST WRITE COMPONENT. No production admission or publication.
use super::history::{self, Claim, Result, Rows};
use chrono::{DateTime, Utc};

pub(super) fn validate_issue_rows(
    original: &Rows,
    claim: &Claim,
    current: &Rows,
    clock: DateTime<Utc>,
) -> Result<()> {
    if original.job != current.job
        || original.outbox != current.outbox
        || current.job.id != claim.job_id
        || current.job.status != "snapshot_running"
        || current.job.attempt_count != claim.attempts
        || claim.attempts != 1
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
    struct RuntimeIdentity {
        database: String,
        oid: i64,
        postmaster: DateTime<Utc>,
        session_user: String,
        current_user: String,
        original_login: Option<String>,
        restricted_login: bool,
        server_address: Option<String>,
        server_port: Option<i32>,
    }
    const RUNTIME_IDENTITY: &str = "SELECT current_database()::text AS database,(SELECT oid::bigint FROM pg_catalog.pg_database WHERE datname=current_database()) AS oid,pg_catalog.pg_postmaster_start_time() AS postmaster,session_user::text AS session_user,current_user::text AS current_user,system_user AS original_login,(r.rolcanlogin AND NOT r.rolsuper AND NOT r.rolcreatedb AND NOT r.rolcreaterole AND NOT r.rolreplication AND NOT r.rolbypassrls AND NOT EXISTS(SELECT FROM pg_catalog.pg_auth_members m WHERE m.member=r.oid) AND NOT EXISTS(SELECT FROM pg_catalog.pg_shdepend d WHERE d.refclassid='pg_catalog.pg_authid'::regclass AND d.refobjid=r.oid AND d.deptype='o')) AS restricted_login,pg_catalog.inet_server_addr()::text AS server_address,pg_catalog.inet_server_port() AS server_port FROM pg_catalog.pg_roles r WHERE r.rolname=session_user";

    fn original_database(actual: &RuntimeIdentity, expected: &DatabaseIdentity) -> Result<()> {
        if actual.database != expected.database
            || actual.oid != expected.oid
            || actual.postmaster != expected.postmaster
        {
            return Err(history::rejected("C2 source database identity mismatch"));
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
    struct IssueReceipt {
        reference: RecordRef,
        nonce: Uuid,
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
        validate_issue_rows(&original.after, &original.claim, &current, clock)?;
        let (intent, committed) = claimed.records();
        let previous = owner.head(slot).await?;
        if previous != committed {
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

    struct Copied {
        staged: CustodyStaged,
        claimed: Claimed,
        issue: IssueReceipt,
        verified_files: u64,
        verified_bytes: u64,
    }
    fn stage_copy(
        owner: Arc<Original>,
        slot: Arc<Slot>,
        permit: WritePermit,
        prepared: PreparedSnapshotExport,
    ) -> Result<Copied> {
        slot.live()?;
        let WritePermit {
            claimed,
            issue,
            writer,
        } = permit;
        let lease = claimed.lease();
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
        Ok(Copied {
            staged,
            claimed,
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
        let claimed = transition::claim(owner.clone(), slot.clone(), job, request).await?;
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
        let copy_owner = owner.clone();
        let copy_slot = slot.clone();
        let task = tokio::task::spawn_blocking(move || {
            stage_copy(copy_owner, copy_slot, permit, prepared)
        });
        // No live check, timeout wrapper or handle drop between spawn and this
        // unconditional original-owner settlement of the real blocking task.
        let copied = runtime::settle_started(&slot, async {
            task.await
                .map_err(|_| history::rejected("D0 original blocking copy panicked"))?
        })
        .await;
        let copied = match copied {
            Ok(copied) => copied,
            Err(error) => {
                slot.poison();
                // The original supervisor's separate SH is still owned even
                // after a consumed OS writer has returned its actual error.
                let _ = owner.stage(&slot, Stage::AfterStageError).await;
                return Err(error);
            }
        };
        slot.live()?;
        copied
            .staged
            .revalidate()
            .map_err(|_| history::rejected("D0 original staged root identity changed"))?;
        let previous = owner.head(&slot).await?;
        if previous != copied.issue.reference {
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
            manifest_sha256: copied.staged.directory().manifest_sha256(),
            verified_files: copied.verified_files,
            verified_bytes: copied.verified_bytes,
            permit_nonce: copied.issue.nonce,
        };
        let encoded = history::encode(&record, &owner.budget)?;
        owner.preflight()?;
        let observation = owner.append(&slot, encoded).await?;
        slot.live()?;
        Ok(StagedComponent {
            staged: copied.staged,
            issue: copied.issue.reference,
            observation,
        })
    }
}
#[cfg(target_os = "linux")]
pub(super) use first_write::{StagedComponent, drive};

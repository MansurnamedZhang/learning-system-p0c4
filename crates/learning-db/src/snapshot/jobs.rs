//! C3 authorization is checked at planning, publication and every delivery.
use super::{SnapshotPlan, SnapshotStore, walk};
use crate::{
    AssetProcessOutcome, JobFailureClass, JobLease, JobStore, authorization, overlay::model,
    reading, references, request, storage,
};
use learning_assets::root_journal::SharedJournalBudget;
use learning_assets::{
    FsAssetStore, SnapshotDirectory, SnapshotJobDirectory, sanitize_reading_copy, verify_snapshot,
};
use learning_core::*;
use sqlx::{Postgres, Transaction};
use std::{collections::BTreeSet, fs::File, io, path::PathBuf};
use uuid::Uuid;

pub struct SnapshotDelivery {
    pub manifest_sha256: String,
    pub files: Vec<(String, File)>,
}

// Prepared data has no public constructor: neither a client-supplied closure nor
// a stage path can be substituted at the publication boundary.
pub struct PreparedSnapshotExport {
    lease: JobLease,
    actor: Principal,
    space: Uuid,
    request: SnapshotRequest,
    plan: ExportPlan,
}
pub struct StagedSnapshotExport {
    prepared: PreparedSnapshotExport,
    directory: SnapshotDirectory,
}

#[cfg(target_os = "linux")]
impl PreparedSnapshotExport {
    /// Read-only adapter for the private first-write component. The plan still
    /// comes exclusively from the existing runtime collector, never a caller.
    pub(crate) fn broker_reading_copy(&self) -> Result<&ReadingCopy, ContentError> {
        match &self.plan {
            ExportPlan::Copy { copy, .. }
                if !self.request.include_personal && !copy.items.is_empty() =>
            {
                Ok(copy)
            }
            _ => Err(ContentError::Invalid("broker_reading_copy_required".into())),
        }
    }
    pub(crate) fn broker_matches(&self, lease: &JobLease, input: &JobInput) -> bool {
        let Ok(expected) =
            JobInput::snapshot_export(self.actor.actor_id, self.space, self.request.clone())
        else {
            return false;
        };
        self.lease.job_id == lease.job_id
            && self.lease.token == lease.token
            && self.lease.attempt_count == lease.attempt_count
            && self.lease.lease_expires_at == lease.lease_expires_at
            && expected.to_value() == input.to_value()
    }
    pub(crate) async fn broker_recheck(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        budget: &SharedJournalBudget,
    ) -> Result<CheckedPublication, ContentError> {
        checked_publication(tx, self, Some(budget)).await
    }
    pub(crate) async fn broker_initialize_publication(
        &self,
        tx: &mut Transaction<'static, Postgres>,
    ) -> Result<(), ContentError> {
        // Same fixed request transaction settings. The caller already owns the
        // top-level transaction, so even a setup failure has an awaited rollback.
        for sql in [
            "SET TRANSACTION ISOLATION LEVEL READ COMMITTED",
            "SET LOCAL lock_timeout='10s'",
            "SET LOCAL statement_timeout='15s'",
        ] {
            sqlx::query(sql).execute(&mut **tx).await.map_err(storage)?;
        }
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!(
                "learning/content/request/{}:{}",
                self.actor.actor_id, self.lease.job_id
            ))
            .execute(&mut **tx)
            .await
            .map_err(storage)?;
        Ok(())
    }
}

// Fresh collector data, never a caller-supplied SQL/JSON closure or authority.
// The original publisher and the private broker use the same final SQL core.
pub(crate) struct CheckedPublication {
    capability: &'static str,
    digest: String,
    spaces: Vec<Uuid>,
}
impl CheckedPublication {
    #[cfg(target_os = "linux")]
    pub(crate) fn capability(&self) -> &str {
        self.capability
    }
    #[cfg(target_os = "linux")]
    pub(crate) fn digest(&self) -> &str {
        &self.digest
    }
    #[cfg(target_os = "linux")]
    pub(crate) fn spaces(&self) -> &[Uuid] {
        &self.spaces
    }
    pub(crate) fn accepts(&self, manifest: &SnapshotManifest) -> bool {
        matches!(
            (self.capability, manifest),
            ("exact_import_v1", SnapshotManifest::ExactImportV1(_))
                | ("reading_copy_v1", SnapshotManifest::ReadingCopyV1(_))
        )
    }
    pub(crate) async fn complete(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        prepared: &PreparedSnapshotExport,
        directory: &SnapshotDirectory,
    ) -> Result<bool, ContentError> {
        sqlx::query_scalar("SELECT public.p0c3_complete_snapshot_job($1,$2,$3,$4,$5,$6)")
            .bind(prepared.lease.job_id)
            .bind(prepared.lease.token)
            .bind(self.capability)
            .bind(directory.manifest_sha256())
            .bind(&self.digest)
            .bind(&self.spaces)
            .fetch_one(&mut **tx)
            .await
            .map_err(storage)
    }
}
enum ExportPlan {
    Exact {
        plan: SnapshotPlan,
        spaces: Vec<(Uuid, bool)>,
    },
    Copy {
        copy: ReadingCopy,
        spaces: Vec<(Uuid, bool)>,
    },
}
impl ExportPlan {
    fn spaces(&self) -> Vec<Uuid> {
        let spaces = match self {
            Self::Exact { spaces, .. } => spaces,
            Self::Copy { spaces, .. } => spaces,
        };
        spaces
            .iter()
            .map(|(id, _)| *id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
    fn digest(&self) -> String {
        let value = match self {
            Self::Exact { plan: p, .. } => {
                serde_json::json!({"manifest":p.manifest,"rows":p.rows,"assets":p.assets,"spaces":self.spaces()})
            }
            Self::Copy { copy, .. } => serde_json::json!({"copy":copy,"spaces":self.spaces()}),
        };
        canonical_record_hash(&value)
    }
    fn capability(&self) -> &'static str {
        match self {
            Self::Exact { .. } => "exact_import_v1",
            Self::Copy { .. } => "reading_copy_v1",
        }
    }
    fn checked_digest(&self, budget: Option<&SharedJournalBudget>) -> Result<String, ContentError> {
        let Some(budget) = budget else {
            return Ok(self.digest());
        };
        let Self::Copy { copy, .. } = self else {
            return Err(ContentError::NotFound);
        };
        metered_plan_digest(copy, &self.checked_spaces(Some(budget))?, budget)
    }
    fn checked_spaces(
        &self,
        budget: Option<&SharedJournalBudget>,
    ) -> Result<Vec<Uuid>, ContentError> {
        let Some(budget) = budget else {
            return Ok(self.spaces());
        };
        let raw = match self {
            Self::Exact { spaces, .. } | Self::Copy { spaces, .. } => spaces,
        };
        let mut sorted: Vec<Uuid> = Vec::new();
        for (id, _) in raw {
            budget.charge(16, 0).map_err(storage)?;
            let (mut low, mut high) = (0, sorted.len());
            let mut duplicate = false;
            while low < high {
                let mid = low + (high - low) / 2;
                budget.charge(32, 0).map_err(storage)?;
                match sorted[mid].cmp(id) {
                    std::cmp::Ordering::Less => low = mid + 1,
                    std::cmp::Ordering::Greater => high = mid,
                    std::cmp::Ordering::Equal => {
                        duplicate = true;
                        break;
                    }
                }
            }
            if duplicate {
                continue;
            }
            // Prepay a possible old-buffer read/write at reserve and the
            // insertion shift, even when the allocator can grow in place.
            let movement = sorted
                .len()
                .checked_add(sorted.len() - low)
                .and_then(|n| n.checked_mul(32))
                .and_then(|n| n.checked_add(16))
                .ok_or(ContentError::Storage)?;
            budget.charge(movement as u64, 0).map_err(storage)?;
            sorted.try_reserve_exact(1).map_err(storage)?;
            sorted.insert(low, *id);
        }
        Ok(sorted)
    }
    fn checked_spaces_match(
        &self,
        required: &[Uuid],
        budget: Option<&SharedJournalBudget>,
    ) -> Result<bool, ContentError> {
        let actual = self.checked_spaces(budget)?;
        if let Some(budget) = budget {
            let comparison = actual
                .len()
                .min(required.len())
                .checked_mul(32)
                .ok_or(ContentError::Storage)?;
            budget.charge(comparison as u64, 0).map_err(storage)?;
        }
        Ok(actual == required)
    }
}

fn metered_plan_digest(
    copy: &ReadingCopy,
    spaces: &[Uuid],
    budget: &SharedJournalBudget,
) -> Result<String, ContentError> {
    const CAP: usize = 64 * 1024 * 1024;
    struct Count<'a> {
        length: usize,
        budget: &'a SharedJournalBudget,
    }
    impl io::Write for Count<'_> {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.length = self
                .length
                .checked_add(bytes.len())
                .filter(|n| *n <= CAP)
                .ok_or_else(|| io::Error::other("snapshot plan capacity"))?;
            self.budget.charge(bytes.len() as u64, 0)?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count { length: 0, budget };
    write_canonical_copy(&mut count, copy, spaces).map_err(storage)?;
    // Prepay output before allocation. The second writer consumes exactly this
    // paid bound, so neither extend nor allocator growth can copy unpaid bytes.
    budget.charge(count.length as u64, 1).map_err(storage)?;
    struct Bounded {
        bytes: Vec<u8>,
        limit: usize,
    }
    impl io::Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self
                .bytes
                .len()
                .checked_add(bytes.len())
                .is_none_or(|n| n > self.limit)
            {
                return Err(io::Error::other("snapshot plan unpaid growth"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(count.length).map_err(storage)?;
    let mut out = Bounded {
        bytes,
        limit: count.length,
    };
    write_canonical_copy(&mut out, copy, spaces).map_err(storage)?;
    if out.bytes.len() != count.length {
        return Err(ContentError::Storage);
    }
    budget.charge(out.bytes.len() as u64, 0).map_err(storage)?;
    Ok(hex_digest(&out.bytes))
}

// This private D1 path emits only the fixed ReadingCopy schema, in the same
// byte order as canonical_json. All strings stream through serde's escaping;
// it creates no Value, recursive canonical String or leaf-sized temporary.
fn write_canonical_copy(
    out: &mut impl io::Write,
    copy: &ReadingCopy,
    spaces: &[Uuid],
) -> io::Result<()> {
    out.write_all(b"{\"copy\":{\"evidence\":[")?;
    for (index, evidence) in copy.evidence.iter().enumerate() {
        if index != 0 {
            out.write_all(b",")?;
        }
        serde_json::to_writer(&mut *out, evidence)?;
    }
    out.write_all(b"],\"items\":[")?;
    for (index, item) in copy.items.iter().enumerate() {
        if index != 0 {
            out.write_all(b",")?;
        }
        match item {
            CopyItem::Heading { title } => {
                out.write_all(b"{\"title\":")?;
                serde_json::to_writer(&mut *out, title)?;
                out.write_all(b",\"type\":\"heading\"}")?;
            }
            CopyItem::Content {
                intent,
                title,
                display_text,
            } => {
                out.write_all(b"{\"display_text\":")?;
                serde_json::to_writer(&mut *out, display_text)?;
                out.write_all(b",\"intent\":")?;
                serde_json::to_writer(&mut *out, intent)?;
                out.write_all(b",\"title\":")?;
                serde_json::to_writer(&mut *out, title)?;
                out.write_all(b",\"type\":\"content\"}")?;
            }
            CopyItem::Omitted => out.write_all(b"{\"type\":\"omitted\"}")?,
        }
    }
    out.write_all(b"]},\"spaces\":[")?;
    for (index, id) in spaces.iter().enumerate() {
        if index != 0 {
            out.write_all(b",")?;
        }
        serde_json::to_writer(&mut *out, id)?;
    }
    out.write_all(b"]}")
}

async fn checked_publication(
    tx: &mut Transaction<'_, Postgres>,
    prepared: &PreparedSnapshotExport,
    budget: Option<&SharedJournalBudget>,
) -> Result<CheckedPublication, ContentError> {
    let plan = recheck_locked_with_budget(
        tx,
        prepared.actor,
        prepared.space,
        &prepared.request,
        &prepared.plan.checked_digest(budget)?,
        &prepared.plan.checked_spaces(budget)?,
        budget,
    )
    .await?;
    Ok(CheckedPublication {
        capability: plan.capability(),
        digest: plan.checked_digest(budget)?,
        spaces: plan.checked_spaces(budget)?,
    })
}

impl SnapshotStore {
    #[cfg(target_os = "linux")]
    pub(crate) async fn broker_begin_publication(
        &self,
    ) -> Result<Transaction<'static, Postgres>, ContentError> {
        // Transfer the original owned value before any fallible setup SQL.
        self.pool.begin().await.map_err(storage)
    }
    /// Configure a non-public, service-owned 0700 export root, separate from the
    /// original asset and upload roots. Unsupported platforms fail closed on use.
    pub fn with_export_storage(mut self, root: PathBuf, assets: FsAssetStore) -> Self {
        self.export_storage = Some((root, assets));
        self
    }

    pub async fn runnable_export_ids(&self, limit: usize) -> Result<Vec<Uuid>, ContentError> {
        if self.export_storage.is_none() {
            return Ok(vec![]);
        }
        JobStore::new(self.pool.clone())
            .runnable_snapshot_ids(limit)
            .await
    }

    /// An unconfigured worker leaves C3 jobs queued rather than consuming retries.
    pub async fn claim_export(
        &self,
        job_id: Uuid,
        lease: std::time::Duration,
    ) -> Result<Option<JobLease>, ContentError> {
        if self.export_storage.is_none() {
            return Ok(None);
        }
        JobStore::new(self.pool.clone())
            .claim_snapshot(job_id, lease)
            .await
    }

    pub async fn enqueue_export(
        &self,
        actor: Principal,
        request_id: Uuid,
        input: SnapshotRequest,
    ) -> Result<Uuid, ContentError> {
        if request_id.is_nil() {
            return Err(ContentError::Invalid("invalid_request_id".into()));
        }
        let mut tx = request::begin(&self.pool, actor, request_id).await?;
        let layer = model::load_view(&mut tx, actor, input.reading.clone()).await?;
        let job_input = JobInput::snapshot_export(actor.actor_id, layer.space, input.clone())?;
        let digest = canonical_record_hash(&job_input.to_value());
        if request::check(&mut tx, actor, request_id, &digest, "snapshot_export").await? {
            let id = sqlx::query_scalar("SELECT job_id FROM public.snapshot_export_request WHERE actor_id=$1 AND request_id=$2")
                .bind(actor.actor_id).bind(request_id).fetch_one(&mut *tx).await.map_err(storage)?;
            tx.commit().await.map_err(storage)?;
            return Ok(id);
        }
        let plan = collect_export(&mut tx, actor, &input).await?;
        let required = plan
            .spaces()
            .into_iter()
            .map(|id| (id, false))
            .collect::<Vec<_>>();
        authorization::lock_grants(&mut tx, actor, &required).await?;
        // Identical payloads share a business event; distinct options share no key.
        let outbox = Uuid::new_v4();
        sqlx::query("INSERT INTO public.job_outbox(id,business_key,event_type,payload_version,payload,actor_id,processor_version) VALUES($1,$2,'snapshot_export_requested',1,$3,$4,1) ON CONFLICT(business_key) DO NOTHING")
            .bind(outbox).bind(job_input.business_key()).bind(job_input.to_value()).bind(actor.actor_id).execute(&mut *tx).await.map_err(storage)?;
        let id: Uuid = sqlx::query_scalar("SELECT id FROM public.job_outbox WHERE business_key=$1")
            .bind(job_input.business_key())
            .fetch_one(&mut *tx)
            .await
            .map_err(storage)?;
        request::register(&mut tx, actor, request_id, &digest, "snapshot_export").await?;
        sqlx::query("INSERT INTO public.snapshot_export_request(actor_id,request_id,outbox_id,job_id) VALUES($1,$2,$3,$3)")
            .bind(actor.actor_id).bind(request_id).bind(id).execute(&mut *tx).await.map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        Ok(id)
    }

    pub async fn prepare_export(
        &self,
        lease: &JobLease,
    ) -> Result<PreparedSnapshotExport, ContentError> {
        let job = JobStore::new(self.pool.clone())
            .get(lease.job_id)
            .await?
            .ok_or(ContentError::NotFound)?;
        if job.status != JobStatus::Running
            || job.attempt_count != lease.attempt_count
            || job.processor_version != 1
        {
            return Err(ContentError::Invalid("stale_job_lease".into()));
        }
        let JobInput::SnapshotExport {
            actor_id,
            space_id,
            request,
        } = job.input
        else {
            return Err(ContentError::Invalid("wrong_job_kind".into()));
        };
        let actor = Principal { actor_id };
        let mut tx = request::begin_read(&self.pool).await?;
        let layer = model::load_view(&mut tx, actor, request.reading.clone()).await?;
        if layer.space != space_id {
            return Err(ContentError::NotFound);
        }
        let plan = collect_export(&mut tx, actor, &request).await?;
        tx.commit().await.map_err(storage)?;
        Ok(PreparedSnapshotExport {
            lease: lease.clone(),
            actor,
            space: space_id,
            request,
            plan,
        })
    }

    pub fn stage_export(
        &self,
        prepared: PreparedSnapshotExport,
    ) -> Result<StagedSnapshotExport, ContentError> {
        let (root, files) = self
            .export_storage
            .as_ref()
            .ok_or_else(|| ContentError::Invalid("snapshot_storage_unconfigured".into()))?;
        let job_dir = SnapshotJobDirectory::open(root, prepared.lease.job_id).map_err(storage)?;
        let directory = match &prepared.plan {
            ExportPlan::Exact { plan, .. } => job_dir.stage_exact(
                prepared.lease.token,
                &plan.manifest,
                &plan.rows,
                &plan.assets,
                files,
            ),
            ExportPlan::Copy { copy, .. } => job_dir.stage_copy(prepared.lease.token, copy),
        }
        .map_err(storage)?;
        Ok(StagedSnapshotExport {
            prepared,
            directory,
        })
    }

    pub async fn publish_export(
        &self,
        staged: StagedSnapshotExport,
    ) -> Result<AssetProcessOutcome, ContentError> {
        let prepared = &staged.prepared;
        let result = self.publish_in_tx(&staged).await;
        match result {
            Ok(outcome) => Ok(outcome),
            Err(error) => self.export_failure(&prepared.lease, &error).await,
        }
    }
    async fn publish_in_tx(
        &self,
        staged: &StagedSnapshotExport,
    ) -> Result<AssetProcessOutcome, ContentError> {
        let p = &staged.prepared;
        let mut tx = request::begin(&self.pool, p.actor, p.lease.job_id).await?;
        let plan = checked_publication(&mut tx, p, None).await?;
        let verified = verify_snapshot(&staged.directory).map_err(storage)?;
        if !plan.accepts(&verified.manifest) {
            return Err(ContentError::NotFound);
        }
        let accepted = plan.complete(&mut tx, p, &staged.directory).await?;
        if !accepted {
            tx.rollback().await.map_err(storage)?;
            return Ok(AssetProcessOutcome::LeaseLost);
        }
        tx.commit().await.map_err(storage)?;
        Ok(AssetProcessOutcome::Succeeded)
    }

    pub async fn process_snapshot_export(
        &self,
        lease: &JobLease,
    ) -> Result<AssetProcessOutcome, ContentError> {
        let prepared = match self.prepare_export(lease).await {
            Ok(p) => p,
            Err(e) => return self.export_failure(lease, &e).await,
        };
        let store = self.clone();
        // File copying and hashing must not prevent the worker heartbeat renewing.
        let staged = tokio::task::spawn_blocking(move || store.stage_export(prepared))
            .await
            .map_err(storage)?;
        match staged {
            Ok(s) => self.publish_export(s).await,
            Err(e) => self.export_failure(lease, &e).await,
        }
    }
    async fn export_failure(
        &self,
        lease: &JobLease,
        error: &ContentError,
    ) -> Result<AssetProcessOutcome, ContentError> {
        let class = if matches!(error, ContentError::Storage) {
            JobFailureClass::TransientStorage
        } else {
            JobFailureClass::InvalidInput
        };
        if !JobStore::new(self.pool.clone())
            .fail(lease.job_id, lease.token, class)
            .await?
        {
            return Ok(AssetProcessOutcome::LeaseLost);
        }
        Ok(
            if class == JobFailureClass::TransientStorage && lease.attempt_count < 3 {
                AssetProcessOutcome::RetryWait
            } else {
                AssetProcessOutcome::Failed
            },
        )
    }

    pub async fn deliver_export(
        &self,
        actor: Principal,
        job_id: Uuid,
    ) -> Result<SnapshotDelivery, ContentError> {
        let mut tx = request::begin(&self.pool, actor, job_id).await?;
        let saved: Option<(serde_json::Value, Uuid, String, String, Vec<Uuid>, String)> = sqlx::query_as(
            "SELECT e.payload,r.stage_token,r.manifest_sha256,r.plan_sha256,r.required_spaces,r.capability FROM public.snapshot_export_result r JOIN public.job j ON j.id=r.job_id JOIN public.job_outbox e ON e.id=j.outbox_id WHERE j.id=$1 AND j.status='succeeded' AND j.output_digest=r.manifest_sha256 AND e.actor_id=$2")
            .bind(job_id).bind(actor.actor_id).fetch_optional(&mut *tx).await.map_err(storage)?;
        let (payload, token, manifest_sha256, plan_sha256, spaces, capability) =
            saved.ok_or(ContentError::NotFound)?;
        let JobInput::SnapshotExport {
            space_id, request, ..
        } = JobInput::from_value(payload)?
        else {
            return Err(ContentError::NotFound);
        };
        let plan =
            recheck_locked(&mut tx, actor, space_id, &request, &plan_sha256, &spaces).await?;
        if plan.capability() != capability {
            return Err(ContentError::NotFound);
        }
        let (root, _) = self.export_storage.as_ref().ok_or(ContentError::NotFound)?;
        let root = root.clone();
        let hash = manifest_sha256.clone();
        let files = tokio::task::spawn_blocking(move || {
            SnapshotJobDirectory::open(&root, job_id)?
                .reopen(token, &hash)?
                .open_verified_files()
        })
        .await
        .map_err(storage)?
        .map_err(storage)?;
        // Revocation arriving after these locks waits until sealed handles are
        // obtained. An already delivered handle is intentionally not revocable.
        tx.commit().await.map_err(storage)?;
        Ok(SnapshotDelivery {
            manifest_sha256,
            files,
        })
    }
}

async fn collect_export(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    input: &SnapshotRequest,
) -> Result<ExportPlan, ContentError> {
    if input.include_personal {
        // The same collector used by SnapshotStore::plan_exact, in the caller's
        // transaction so publication/delivery can hold all required grant locks.
        let (plan, spaces) = walk::collect_authorized(tx, actor, input).await?;
        return Ok(ExportPlan::Exact { plan, spaces });
    }
    let layer = model::load_view(tx, actor, input.reading.clone()).await?;
    let mut session = references::Session::default();
    let access = model::access_with_session(tx, actor, &layer, &mut session).await?;
    let mut projection = reading::project_for_impact(&layer, &access, input.mode)?;
    let (version, choices) = reading::selection::choices(tx, &layer.view).await?;
    projection.contract_version = version;
    projection.evidence = reading::selection::project(tx, actor, &choices, &mut session).await?;
    let mut spaces = access.spaces;
    spaces.extend(session.spaces());
    let copy = sanitize_reading_copy(&projection, input.mode, false)?;
    Ok(ExportPlan::Copy { copy, spaces })
}

async fn recheck_locked(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    space: Uuid,
    input: &SnapshotRequest,
    expected: &str,
    required: &[Uuid],
) -> Result<ExportPlan, ContentError> {
    recheck_locked_with_budget(tx, actor, space, input, expected, required, None).await
}
async fn recheck_locked_with_budget(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    space: Uuid,
    input: &SnapshotRequest,
    expected: &str,
    required: &[Uuid],
    budget: Option<&SharedJournalBudget>,
) -> Result<ExportPlan, ContentError> {
    let layer = model::load_view(tx, actor, input.reading.clone()).await?;
    if layer.space != space {
        return Err(ContentError::NotFound);
    }
    let fresh = collect_export(tx, actor, input).await?;
    if fresh.checked_digest(budget)? != expected || !fresh.checked_spaces_match(required, budget)? {
        return Err(ContentError::NotFound);
    }
    if let Some(budget) = budget {
        let grant_copy = required
            .len()
            .checked_mul(2 * std::mem::size_of::<(Uuid, bool)>())
            .ok_or(ContentError::Storage)?;
        budget.charge(grant_copy as u64, 0).map_err(storage)?;
    }
    authorization::lock_grants(
        tx,
        actor,
        &required.iter().map(|id| (*id, false)).collect::<Vec<_>>(),
    )
    .await?;
    // READ COMMITTED reread after locks handles revocation winning while planning.
    let locked = collect_export(tx, actor, input).await?;
    if locked.checked_digest(budget)? != expected
        || !locked.checked_spaces_match(required, budget)?
    {
        return Err(ContentError::NotFound);
    }
    Ok(locked)
}

#[cfg(test)]
mod publication_budget_tests {
    use super::*;

    fn plan() -> ExportPlan {
        ExportPlan::Copy {
            copy: ReadingCopy {
                items: vec![CopyItem::Heading {
                    title: "汉<\"".into(),
                }],
                evidence: vec!["actual".into()],
            },
            spaces: vec![(Uuid::from_u128(2), false), (Uuid::from_u128(1), false)],
        }
    }

    #[test]
    fn d1_metered_plan_preserves_canonical_copy_and_sorted_spaces() {
        let budget = SharedJournalBudget::default();
        let expected = canonical_record_hash(&serde_json::json!({
            "copy": {"items": [{"type":"heading", "title":"汉<\""}], "evidence":["actual"]},
            "spaces": ["00000000-0000-0000-0000-000000000001", "00000000-0000-0000-0000-000000000002"]
        }));
        assert_eq!(plan().checked_digest(Some(&budget)).unwrap(), expected);
        assert!(budget.usage().unwrap().charged_bytes > 100);
        assert_eq!(budget.usage().unwrap().charged_entries, 1);
    }

    #[test]
    fn d1_plan_encoding_uses_original_exhausted_budget_without_reset() {
        let budget = SharedJournalBudget::default();
        budget.charge(512 * 1024 * 1024 - 100, 0).unwrap();
        assert!(plan().checked_digest(Some(&budget)).is_err());
        assert!(budget.usage().unwrap().exhausted);
        assert!(plan().checked_digest(Some(&budget.clone())).is_err());
    }

    fn large_plan_and_legacy_wire() -> (ExportPlan, String) {
        let plan = ExportPlan::Copy {
            copy: ReadingCopy {
                items: vec![
                    CopyItem::Heading {
                        title: "汉字 \" \\ /\n\r\t\u{0000}".into(),
                    },
                    CopyItem::Content {
                        intent: "note".into(),
                        title: "UTF8 与控制字符\u{001f}".into(),
                        display_text: "a".repeat(1024 * 1024) + "汉\"\\/\n\r\t\u{0000}\u{001f}",
                    },
                    CopyItem::Omitted,
                ],
                evidence: vec!["evidence 汉\"\\/\n\u{0001}".into()],
            },
            spaces: vec![
                (Uuid::from_u128(2), false),
                (Uuid::from_u128(1), false),
                (Uuid::from_u128(2), true),
            ],
        };
        let ExportPlan::Copy { copy, .. } = &plan else {
            unreachable!()
        };
        let legacy = canonical_json(&serde_json::json!({"copy": copy, "spaces": plan.spaces()}));
        assert!(legacy.len() > 1024 * 1024 && legacy.len() < 1024 * 1024 + 1024);
        (plan, legacy)
    }

    #[test]
    fn d1_large_leaf_streaming_canonical_fits_only_output_and_hash_budget() {
        let (plan, legacy) = large_plan_and_legacy_wire();
        let budget = SharedJournalBudget::default();
        // Count, bounded materialization and hash each emit/read n bytes.
        // UUID inputs [2,1,2] pay 208 bytes: source48, comparisons64,
        // inserted UUIDs32, one old-Vec move32 and one insertion shift32.
        let linear = 3 * legacy.len() as u64 + 208;
        budget.charge(512 * 1024 * 1024 - linear, 0).unwrap();
        let actual = plan.checked_digest(Some(&budget)).expect(
            "D1 linear output plus hash budget must be sufficient without recursive String copies",
        );
        assert_eq!(actual, hex_digest(legacy.as_bytes()));
        assert_eq!(actual, plan.digest());
        assert_eq!(budget.usage().unwrap().charged_bytes, 512 * 1024 * 1024);
        assert_eq!(budget.usage().unwrap().charged_entries, 1);
        assert!(!budget.usage().unwrap().exhausted);
    }

    #[test]
    fn d1_canonical_output_or_hash_one_byte_short_rejects_sticky_shared_budget() {
        let (plan, legacy) = large_plan_and_legacy_wire();
        for (stage, available) in [
            208 + legacy.len() as u64 - 1,
            208 + 2 * legacy.len() as u64 - 1,
            208 + 3 * legacy.len() as u64 - 1,
        ]
        .into_iter()
        .enumerate()
        {
            let budget = SharedJournalBudget::default();
            let initial = 512 * 1024 * 1024 - available;
            budget.charge(initial, 0).unwrap();
            assert!(plan.checked_digest(Some(&budget)).is_err());
            let used = budget.usage().unwrap();
            assert!(used.exhausted);
            assert!(used.charged_bytes <= 512 * 1024 * 1024);
            assert_eq!(used.charged_entries, u64::from(stage == 2));
            if stage != 0 {
                // Output prepayment rejects before reservation; hash rejection
                // retains exactly the already-paid count and output work.
                assert_eq!(
                    used.charged_bytes,
                    initial + 208 + stage as u64 * legacy.len() as u64
                );
            }
            assert!(budget.clone().charge(0, 0).is_err());
            assert!(plan.checked_digest(Some(&budget.clone())).is_err());
            assert_eq!(budget.usage().unwrap().charged_bytes, used.charged_bytes);
        }
    }
}

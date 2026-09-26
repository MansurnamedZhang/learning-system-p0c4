//! C3 authorization is checked at planning, publication and every delivery.
use super::{SnapshotPlan, SnapshotStore, walk};
use crate::{
    AssetProcessOutcome, JobFailureClass, JobLease, JobStore, authorization, overlay::model,
    reading, references, request, storage,
};
use learning_assets::{
    FsAssetStore, SnapshotDirectory, SnapshotJobDirectory, sanitize_reading_copy, verify_snapshot,
};
use learning_core::*;
use sqlx::{Postgres, Transaction};
use std::{collections::BTreeSet, fs::File, path::PathBuf};
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
}

impl SnapshotStore {
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
        let plan = recheck_locked(
            &mut tx,
            p.actor,
            p.space,
            &p.request,
            &p.plan.digest(),
            &p.plan.spaces(),
        )
        .await?;
        let verified = verify_snapshot(&staged.directory).map_err(storage)?;
        if !matches!(
            (&plan, verified.manifest),
            (ExportPlan::Exact { .. }, SnapshotManifest::ExactImportV1(_))
                | (ExportPlan::Copy { .. }, SnapshotManifest::ReadingCopyV1(_))
        ) {
            return Err(ContentError::NotFound);
        }
        let accepted: bool =
            sqlx::query_scalar("SELECT public.p0c3_complete_snapshot_job($1,$2,$3,$4,$5,$6)")
                .bind(p.lease.job_id)
                .bind(p.lease.token)
                .bind(plan.capability())
                .bind(staged.directory.manifest_sha256())
                .bind(plan.digest())
                .bind(plan.spaces())
                .fetch_one(&mut *tx)
                .await
                .map_err(storage)?;
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
    let layer = model::load_view(tx, actor, input.reading.clone()).await?;
    if layer.space != space {
        return Err(ContentError::NotFound);
    }
    let fresh = collect_export(tx, actor, input).await?;
    if fresh.digest() != expected || fresh.spaces() != required {
        return Err(ContentError::NotFound);
    }
    authorization::lock_grants(
        tx,
        actor,
        &required.iter().map(|id| (*id, false)).collect::<Vec<_>>(),
    )
    .await?;
    // READ COMMITTED reread after locks handles revocation winning while planning.
    let locked = collect_export(tx, actor, input).await?;
    if locked.digest() != expected || locked.spaces() != required {
        return Err(ContentError::NotFound);
    }
    Ok(locked)
}

//! Asset integrity and light metadata processor for exact block uses.
//!
//! Result-v1 output digest contract: SHA-256 of `canonical_json` UTF-8 over
//! precisely `version=1`, `kind="asset_integrity"`, `processor_version=1`,
//! `space_id`, `block_id`, `revision_id`, `asset_id`, `sha256`, `byte_size`, and
//! `media_type`. The digest itself is excluded. This is not a complete
//! PDF/PNG/Notebook parser, malware scanner, or preview generator.
use crate::{
    AssetRecord, AssetStore, JobFailureClass, JobLease, JobStore, authorization, references,
};
use learning_core::{
    AssetUseRef, BlockRef, ContentError, ExactRef, JobStatus, Principal, canonical_json, hex_digest,
};
use serde_json::json;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetProcessOutcome {
    Succeeded,
    RetryWait,
    Failed,
    LeaseLost,
}

/// Opaque, exact-input evidence collected before current-authorization
/// publication. It is not itself a permission or a durable result.
pub struct PreparedAssetIntegrity {
    lease: JobLease,
    actor: Principal,
    space_id: Uuid,
    block: BlockRef,
    asset: AssetRecord,
}

#[derive(Clone)]
pub struct AssetIntegrityProcessor {
    assets: AssetStore,
    jobs: JobStore,
}

impl AssetIntegrityProcessor {
    pub fn new(assets: AssetStore, jobs: JobStore) -> Self {
        Self { assets, jobs }
    }

    /// Resolve the persisted version-1 input, verify current use authorization,
    /// and rehash the exact stored bytes through the authorized file API.
    pub async fn prepare(&self, lease: &JobLease) -> Result<PreparedAssetIntegrity, ContentError> {
        let job = self
            .jobs
            .get(lease.job_id)
            .await?
            .ok_or(ContentError::NotFound)?;
        if job.status != JobStatus::Running || job.attempt_count != lease.attempt_count {
            return Err(ContentError::Invalid("stale_job_lease".into()));
        }
        if job.processor_version != 1 {
            return Err(ContentError::Invalid(
                "unknown_asset_processor_version".into(),
            ));
        }
        let actor = Principal {
            actor_id: job.input.actor_id(),
        };
        let space_id = job.input.space_id();
        let block = job
            .input
            .asset_block()
            .ok_or_else(|| ContentError::Invalid("wrong_job_kind".into()))?
            .clone();
        let use_ref = AssetUseRef::Block(block.clone());
        let asset = self
            .assets
            .read_for_use(actor, use_ref.clone())
            .await?
            .ok_or(ContentError::NotFound)?;
        // BlockRef itself has no space. A syntactically valid forged event may
        // name a real, authorized block in a different space.
        if asset.reference.space_id != space_id {
            return Err(ContentError::Invalid("job_input_space_mismatch".into()));
        }
        self.assets
            .open_for_use(actor, use_ref)
            .await?
            .ok_or(ContentError::NotFound)?;
        Ok(PreparedAssetIntegrity {
            lease: lease.clone(),
            actor,
            space_id,
            block,
            asset,
        })
    }

    /// Revalidate the whole necessary reference closure and lock each required
    /// grant before the fenced result write. Revocation taking the lock first
    /// denies publication; revocation arriving second waits for commit.
    pub async fn publish(
        &self,
        prepared: PreparedAssetIntegrity,
    ) -> Result<AssetProcessOutcome, ContentError> {
        let mut tx = self
            .jobs
            .pool()
            .begin()
            .await
            .map_err(|_| ContentError::Storage)?;
        sqlx::query("SET LOCAL lock_timeout='10s'")
            .execute(&mut *tx)
            .await
            .map_err(|_| ContentError::Storage)?;
        sqlx::query("SET LOCAL statement_timeout='15s'")
            .execute(&mut *tx)
            .await
            .map_err(|_| ContentError::Storage)?;
        match self.recheck(&mut tx, &prepared).await {
            Ok(true) => {}
            Ok(false) | Err(ContentError::NotFound) => {
                tx.rollback().await.map_err(|_| ContentError::Storage)?;
                return self
                    .record_failure(&prepared.lease, JobFailureClass::InvalidInput)
                    .await;
            }
            Err(error) => {
                tx.rollback().await.map_err(|_| ContentError::Storage)?;
                return self
                    .record_failure(&prepared.lease, failure_class(&error))
                    .await;
            }
        }
        // The immutable file store is checked again while required grants are
        // locked; this catches damage between prepare and publish.
        if self.assets.verify_record_bytes(&prepared.asset).is_err() {
            tx.rollback().await.map_err(|_| ContentError::Storage)?;
            return self
                .record_failure(&prepared.lease, JobFailureClass::TransientStorage)
                .await;
        }
        let value = json!({
            "version": 1,
            "kind": "asset_integrity",
            "processor_version": 1,
            "space_id": prepared.space_id,
            "block_id": prepared.block.block_id,
            "revision_id": prepared.block.revision_id,
            "asset_id": prepared.asset.reference.asset_id,
            "sha256": &prepared.asset.sha256,
            "byte_size": prepared.asset.byte_size,
            "media_type": &prepared.asset.media.media_type,
        });
        let digest = hex_digest(canonical_json(&value).as_bytes());
        let accepted: bool = sqlx::query_scalar(
            "SELECT public.p0c2_complete_asset_job($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
        )
        .bind(prepared.lease.job_id)
        .bind(prepared.lease.token)
        .bind(prepared.space_id)
        .bind(prepared.block.block_id)
        .bind(prepared.block.revision_id)
        .bind(prepared.asset.reference.asset_id)
        .bind(&prepared.asset.sha256)
        .bind(prepared.asset.byte_size)
        .bind(&prepared.asset.media.media_type)
        .bind(&digest)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| ContentError::Storage)?;
        if !accepted {
            tx.rollback().await.map_err(|_| ContentError::Storage)?;
            return Ok(AssetProcessOutcome::LeaseLost);
        }
        tx.commit().await.map_err(|_| ContentError::Storage)?;
        Ok(AssetProcessOutcome::Succeeded)
    }

    pub async fn process(&self, lease: &JobLease) -> Result<AssetProcessOutcome, ContentError> {
        match self.prepare(lease).await {
            Ok(prepared) => self.publish(prepared).await,
            Err(error) => self.record_failure(lease, failure_class(&error)).await,
        }
    }

    async fn recheck(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        prepared: &PreparedAssetIntegrity,
    ) -> Result<bool, ContentError> {
        let mut session = references::Session::default();
        if !session
            .authorize(tx, prepared.actor, &ExactRef::Block(prepared.block.clone()))
            .await?
        {
            return Ok(false);
        }
        authorization::lock_grants(tx, prepared.actor, &session.spaces()).await?;
        let current: Option<(Uuid, String, i64, String)> = sqlx::query_as(
            "SELECT a.id,a.sha256,a.byte_size,a.media_type \
             FROM public.block_asset_use AS u JOIN public.asset AS a \
               ON (a.space_id,a.id)=(u.space_id,u.asset_id) \
             WHERE u.space_id=$1 AND u.block_id=$2 AND u.revision_id=$3 \
               AND a.status='ready'",
        )
        .bind(prepared.space_id)
        .bind(prepared.block.block_id)
        .bind(prepared.block.revision_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| ContentError::Storage)?;
        Ok(matches!(current, Some((id, sha, size, media))
            if id == prepared.asset.reference.asset_id
                && sha == prepared.asset.sha256
                && size == prepared.asset.byte_size
                && media == prepared.asset.media.media_type))
    }

    async fn record_failure(
        &self,
        lease: &JobLease,
        class: JobFailureClass,
    ) -> Result<AssetProcessOutcome, ContentError> {
        if !self.jobs.fail(lease.job_id, lease.token, class).await? {
            return Ok(AssetProcessOutcome::LeaseLost);
        }
        Ok(match class {
            JobFailureClass::TransientStorage if lease.attempt_count < 3 => {
                AssetProcessOutcome::RetryWait
            }
            _ => AssetProcessOutcome::Failed,
        })
    }
}

fn failure_class(error: &ContentError) -> JobFailureClass {
    match error {
        ContentError::Storage => JobFailureClass::TransientStorage,
        _ => JobFailureClass::InvalidInput,
    }
}

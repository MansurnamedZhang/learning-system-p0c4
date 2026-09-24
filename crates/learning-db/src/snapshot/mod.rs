//! Exact snapshot planning. All database reads share one read-only snapshot.
mod rows;
mod walk;

use crate::{request, storage};
use learning_core::*;
use sqlx::PgPool;

#[derive(Debug, Clone)]
pub struct SnapshotPlan {
    /// Planned object/asset files. The package writer adds its validation report
    /// and revalidates final limits; this is not yet a published package.
    pub manifest: ExactSnapshotManifest,
    pub rows: Vec<SnapshotRow>,
    pub assets: Vec<SnapshotAssetUse>,
}

#[derive(Clone)]
pub struct SnapshotStore {
    pool: PgPool,
}
impl SnapshotStore {
    /// Use the non-owner runtime pool. This API collects no user/space/grant rows.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn plan_exact(
        &self,
        actor: Principal,
        input: &SnapshotRequest,
    ) -> Result<SnapshotPlan, ContentError> {
        if !input.include_personal {
            return Err(ContentError::Invalid("snapshot_not_exact".into()));
        }
        if input
            .resource_versions
            .len()
            .saturating_add(input.source_segments.len())
            > SNAPSHOT_MAX_OBJECTS
        {
            return Err(ContentError::Invalid("snapshot_limit_exceeded".into()));
        }
        let mut tx = request::begin_read(&self.pool).await?;
        let plan = walk::collect(&mut tx, actor, input).await?;
        tx.commit().await.map_err(storage)?;
        Ok(plan)
    }
}

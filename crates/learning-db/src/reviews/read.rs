use super::ReviewStore;
use crate::{references, request, storage};
use learning_core::*;
use std::collections::BTreeSet;

impl ReviewStore {
    pub async fn read(
        &self,
        actor: Principal,
        reference: EpistemicReviewRef,
    ) -> Result<ReviewProjection<EpistemicReview>, ContentError> {
        let mut tx = request::begin_read(&self.pool).await?;
        let root = ExactRef::EpistemicReview(reference);
        let result = if !references::directly_visible(&mut tx, actor, &root).await? {
            ReviewProjection::Unavailable
        } else if let Some(object) = references::project(&mut tx, actor, &root).await? {
            ReviewProjection::Available(object.epistemic_review()?)
        } else {
            ReviewProjection::Incomplete
        };
        tx.commit().await.map_err(storage)?;
        Ok(result)
    }

    /// Preserve first appearance and member order, omitting inaccessible members
    /// with their necessary source runs. Unassigned evidence remains singleton.
    pub async fn group_sources(
        &self,
        actor: Principal,
        evidence: Vec<BlockRef>,
    ) -> Result<Vec<SourceGroup>, ContentError> {
        if evidence.len() > 256 {
            return Err(ContentError::Invalid(
                "epistemic_selection_limit_exceeded".into(),
            ));
        }
        let mut tx = request::begin_read(&self.pool).await?;
        let mut session = references::Session::default();
        let mut seen = BTreeSet::new();
        let mut groups: Vec<SourceGroup> = vec![];
        for r in evidence {
            if !seen.insert(r.clone()) {
                continue;
            }
            let Some(object) = session
                .object(&mut tx, actor, &ExactRef::Block(r.clone()))
                .await?
            else {
                continue;
            };
            let source_run = match object.block()?.draft {
                ContentDraft::V1(_) => None,
                ContentDraft::V2(d) => d.source_run,
                ContentDraft::V3(d) => d.source_run,
            };
            if let Some(group) = groups
                .iter_mut()
                .find(|g| source_run.is_some() && g.source_run == source_run)
            {
                group.evidence.push(r);
            } else {
                groups.push(SourceGroup {
                    source_run,
                    evidence: vec![r],
                });
            }
        }
        tx.commit().await.map_err(storage)?;
        Ok(groups)
    }
}

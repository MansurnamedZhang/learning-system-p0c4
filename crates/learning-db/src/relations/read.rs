use super::RelationStore;
use crate::{references, request, storage};
use learning_core::*;
use std::collections::BTreeSet;

impl RelationStore {
    pub async fn read(
        &self,
        actor: Principal,
        reference: RelationRef,
    ) -> Result<Option<RelationRevision>, ContentError> {
        Ok(self
            .read_selected(actor, vec![reference])
            .await?
            .into_iter()
            .next())
    }
    pub async fn read_selected(
        &self,
        actor: Principal,
        selections: Vec<RelationRef>,
    ) -> Result<Vec<RelationRevision>, ContentError> {
        if selections.len() > 256 {
            return Err(ContentError::Invalid(
                "relation_selection_limit_exceeded".into(),
            ));
        }
        let mut seen = BTreeSet::new();
        if selections.iter().any(|r| !seen.insert(r)) {
            return Err(ContentError::Invalid("duplicate_relation_selection".into()));
        }
        let mut tx = request::begin_read(&self.pool).await?;
        let mut session = references::Session::default();
        let mut result = vec![];
        for reference in selections {
            if let Some(object) = session
                .object(&mut tx, actor, &ExactRef::Relation(reference))
                .await?
            {
                result.push(object.relation()?);
            }
        }
        tx.commit().await.map_err(storage)?;
        Ok(result)
    }
    pub async fn read_review(
        &self,
        actor: Principal,
        reference: RelationReviewRef,
    ) -> Result<ReviewProjection<RelationReview>, ContentError> {
        let mut tx = request::begin_read(&self.pool).await?;
        let root = ExactRef::RelationReview(reference);
        let result = if !references::directly_visible(&mut tx, actor, &root).await? {
            ReviewProjection::Unavailable
        } else if let Some(object) = references::project(&mut tx, actor, &root).await? {
            ReviewProjection::Available(object.relation_review()?)
        } else {
            ReviewProjection::Incomplete
        };
        tx.commit().await.map_err(storage)?;
        Ok(result)
    }
}

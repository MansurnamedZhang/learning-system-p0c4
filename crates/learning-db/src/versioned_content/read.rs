use super::VersionedContentStore;
use crate::{references, request, storage};
use learning_core::*;
use std::collections::BTreeSet;
impl VersionedContentStore {
    pub async fn read(
        &self,
        actor: Principal,
        reference: BlockRef,
    ) -> Result<Option<ContentRevision>, ContentError> {
        Ok(self
            .read_many(actor, vec![reference])
            .await?
            .into_iter()
            .next())
    }
    pub async fn read_many(
        &self,
        actor: Principal,
        roots: Vec<BlockRef>,
    ) -> Result<Vec<ContentRevision>, ContentError> {
        if roots.len() > 200 {
            return Err(ContentError::Invalid("批量读取最多 200 个输入 ID".into()));
        }
        let mut tx = request::begin_read(&self.pool).await?;
        let mut session = references::Session::default();
        let mut seen = BTreeSet::new();
        let mut out = vec![];
        for root in roots {
            if seen.insert(root.clone())
                && let Some(object) = session
                    .object(&mut tx, actor, &ExactRef::Block(root))
                    .await?
            {
                out.push(object.block()?);
            }
        }
        tx.commit().await.map_err(storage)?;
        Ok(out)
    }
    pub async fn preview(
        &self,
        actor: Principal,
        root: BlockRef,
    ) -> Result<Option<ReferencePreview>, ContentError> {
        let mut tx = request::begin_read(&self.pool).await?;
        let closure = match references::load(&mut tx, actor, &[ExactRef::Block(root.clone())]).await
        {
            Ok(c) => c,
            Err(ContentError::NotFound) => return Ok(None),
            Err(e) => return Err(e),
        };
        let result = preview(&closure, &root, &[])?;
        tx.commit().await.map_err(storage)?;
        Ok(Some(result))
    }
}
fn preview(
    closure: &references::AuthorizedClosure,
    root: &BlockRef,
    path: &[BlockRef],
) -> Result<ReferencePreview, ContentError> {
    let revision = closure
        .objects
        .get(&ExactRef::Block(root.clone()))
        .ok_or(ContentError::Storage)?
        .1
        .clone()
        .block()?;
    let mut next = path.to_vec();
    next.push(root.clone());
    let target = match &revision.draft {
        ContentDraft::V2(ContentV2 {
            body: BodyV2::Reference { target },
            ..
        }) => Some(target),
        ContentDraft::V3(ContentV3 {
            body: BodyV3::Reference { target },
            ..
        }) => Some(target),
        _ => None,
    };
    let target = match target {
        Some(target) => {
            if !closure
                .objects
                .contains_key(&ExactRef::Block(target.clone()))
            {
                return Err(ContentError::Storage);
            }
            Some(if next.len() >= 2 || next.contains(target) {
                PreviewTarget::Link(target.clone())
            } else {
                PreviewTarget::Embedded(Box::new(preview(closure, target, &next)?))
            })
        }
        None => None,
    };
    Ok(ReferencePreview { revision, target })
}

impl VersionedContentStore {
    pub async fn list(
        &self,
        actor: Principal,
        space: uuid::Uuid,
        cursor: Option<PageCursor>,
    ) -> Result<ContentRevisionPage, ContentError> {
        crate::ContentStore::new(self.pool.clone())
            .page(actor, space, cursor, false)
            .await
    }
    pub async fn history(
        &self,
        actor: Principal,
        block: uuid::Uuid,
        cursor: Option<PageCursor>,
    ) -> Result<ContentRevisionPage, ContentError> {
        crate::ContentStore::new(self.pool.clone())
            .page(actor, block, cursor, true)
            .await
    }
}

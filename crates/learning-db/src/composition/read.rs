use super::{CompositionStore, closure};
use crate::{request, storage};
use learning_core::*;
impl CompositionStore {
    pub async fn read(
        &self,
        actor: Principal,
        root: CompositionRef,
    ) -> Result<Option<CompositionSnapshot>, ContentError> {
        let mut tx = request::begin_read(&self.pool).await?;
        match closure::load(&mut tx, actor, std::slice::from_ref(&root), None).await {
            Ok(data) => {
                tx.commit().await.map_err(storage)?;
                Ok(Some(data.snapshot(root)))
            }
            Err(ContentError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

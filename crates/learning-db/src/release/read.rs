use super::ReleaseStore;
use crate::{composition::closure, request, storage};
use chrono::{DateTime, Utc};
use learning_core::*;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
#[derive(sqlx::FromRow)]
struct ReleaseRow {
    id: Uuid,
    space_id: Uuid,
    author_id: Uuid,
    reason: String,
    created_at: DateTime<Utc>,
}
pub(super) async fn load(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    id: Uuid,
) -> Result<Release, ContentError> {
    let row=sqlx::query_as::<_,ReleaseRow>("SELECT r.* FROM public.release r JOIN public.space_grant g ON g.space_id=r.space_id WHERE r.id=$1 AND g.actor_id=$2").bind(id).bind(actor.actor_id).fetch_optional(&mut **tx).await.map_err(storage)?.ok_or(ContentError::NotFound)?;
    let refs:Vec<(Uuid,Uuid)>=sqlx::query_as("SELECT composition_id,revision_id FROM public.release_root WHERE release_id=$1 ORDER BY composition_id").bind(id).fetch_all(&mut **tx).await.map_err(storage)?;
    let roots = refs
        .into_iter()
        .map(|(composition_id, revision_id)| CompositionRef {
            composition_id,
            revision_id,
        })
        .collect();
    Ok(Release {
        release_id: row.id,
        space_id: row.space_id,
        roots,
        author_id: row.author_id,
        reason: row.reason,
        created_at: row.created_at,
    })
}
impl ReleaseStore {
    /// Read a root-scoped publication basis without revealing a multi-root release ID.
    pub async fn state(
        &self,
        actor: Principal,
        id: Uuid,
    ) -> Result<Option<PublicationState>, ContentError> {
        let mut tx = request::begin_read(&self.pool).await?;
        let row:Option<(Uuid,Option<Uuid>,Option<Uuid>)>=sqlx::query_as("SELECT c.head_revision_id,c.published_revision_id,c.last_release_id FROM public.composition c JOIN public.space_grant g ON g.space_id=c.space_id WHERE c.id=$1 AND g.actor_id=$2").bind(id).bind(actor.actor_id).fetch_optional(&mut *tx).await.map_err(storage)?;
        let Some((head, published, release)) = row else {
            return Ok(None);
        };
        let working = CompositionRef {
            composition_id: id,
            revision_id: head,
        };
        let current = published.map(|revision_id| CompositionRef {
            composition_id: id,
            revision_id,
        });
        let checked = async {
            closure::load(&mut tx, actor, std::slice::from_ref(&working), None).await?;
            if let Some(published) = &current
                && published != &working
            {
                closure::load(&mut tx, actor, std::slice::from_ref(published), None).await?;
            }
            Ok::<_, ContentError>(())
        }
        .await;
        match checked {
            Ok(()) => {
                tx.commit().await.map_err(storage)?;
                Ok(Some(PublicationState {
                    composition_id: id,
                    head_revision_id: head,
                    published: current,
                    publication_token: super::publication_token(id, release),
                }))
            }
            Err(ContentError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }
    pub async fn read(&self, actor: Principal, id: Uuid) -> Result<Option<Release>, ContentError> {
        let mut tx = request::begin_read(&self.pool).await?;
        let checked = async {
            let r = load(&mut tx, actor, id).await?;
            let version: i32 =
                sqlx::query_scalar("SELECT contract_version FROM release WHERE id=$1")
                    .bind(id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(storage)?;
            if version == 2 {
                super::evidence_read::load(&mut tx, actor, id).await?;
                return Err(ContentError::Invalid("unsupported_content_version".into()));
            }
            closure::load(&mut tx, actor, &r.roots, None).await?;
            Ok::<_, ContentError>(r)
        }
        .await;
        match checked {
            Ok(r) => {
                tx.commit().await.map_err(storage)?;
                Ok(Some(r))
            }
            Err(ContentError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }
    pub async fn active(
        &self,
        actor: Principal,
        id: Uuid,
    ) -> Result<Option<CompositionRef>, ContentError> {
        let mut tx = request::begin_read(&self.pool).await?;
        let found:Option<(Uuid,Option<Uuid>)>=sqlx::query_as("SELECT c.id,c.published_revision_id FROM public.composition c JOIN public.space_grant g ON g.space_id=c.space_id WHERE c.id=$1 AND g.actor_id=$2").bind(id).bind(actor.actor_id).fetch_optional(&mut *tx).await.map_err(storage)?;
        let Some((composition_id, Some(revision_id))) = found else {
            return Ok(None);
        };
        let reference = CompositionRef {
            composition_id,
            revision_id,
        };
        match closure::load(&mut tx, actor, std::slice::from_ref(&reference), None).await {
            Ok(_) => {
                tx.commit().await.map_err(storage)?;
                Ok(Some(reference))
            }
            Err(ContentError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

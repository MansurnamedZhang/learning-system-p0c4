use super::{RelationStore, check_dependencies, enum_text, lock_dependencies, scope};
use crate::{references, request, storage};
use learning_core::*;
use serde_json::json;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct Identity {
    id: Uuid,
    space_id: Uuid,
    overlay_id: Option<Uuid>,
    r#type: String,
    from_block_id: Uuid,
    to_block_id: Uuid,
    head_revision_id: Uuid,
}
impl Identity {
    fn reference(&self) -> ExactRef {
        ExactRef::Relation(RelationRef {
            relation_id: self.id,
            revision_id: self.head_revision_id,
        })
    }
}
async fn identity(
    tx: &mut Transaction<'_, Postgres>,
    c: &SaveRelation,
    lock: bool,
) -> Result<Option<Identity>, ContentError> {
    let columns = "id,space_id,overlay_id,type,from_block_id,to_block_id,head_revision_id";
    let suffix = if lock { " FOR UPDATE" } else { "" };
    let result = if let Some(id) = c.relation_id {
        sqlx::query_as(&format!(
            "SELECT {columns} FROM public.relation WHERE id=$1{suffix}"
        ))
        .bind(id)
        .fetch_optional(&mut **tx)
        .await
    } else {
        let (space, overlay) = scope(&c.scope);
        sqlx::query_as(&format!("SELECT {columns} FROM public.relation WHERE space_id=$1 AND overlay_id IS NOT DISTINCT FROM $2 AND type=$3 AND from_block_id=$4 AND to_block_id=$5{suffix}"))
            .bind(space).bind(overlay).bind(enum_text(json!(c.relation_type))?).bind(c.from.block_id).bind(c.to.block_id).fetch_optional(&mut **tx).await
    };
    result.map_err(storage)
}
impl RelationStore {
    pub async fn save(
        &self,
        actor: Principal,
        mut command: SaveRelation,
    ) -> Result<RelationRevision, ContentError> {
        command.validate()?;
        // Hash the original command: reversing symmetric endpoints is a different
        // request payload even though it addresses the same normalized identity.
        let digest=hex_digest(canonical_json(&json!({"domain":"relation-request-v1","operation":"relation_save","actor_id":actor.actor_id,"command":command})).as_bytes());
        if command.relation_type == RelationType::RelatedTo
            && command.from.block_id > command.to.block_id
        {
            std::mem::swap(&mut command.from, &mut command.to);
        }
        let c = &command;
        let mut tx = request::begin(&self.pool, actor, c.request_id).await?;
        let receipt:Option<(Uuid,Uuid)>=sqlx::query_as("SELECT r.relation_id,r.id FROM public.relation_receipt m JOIN public.relation_revision r ON r.id=m.revision_id WHERE m.actor_id=$1 AND m.request_id=$2")
            .bind(actor.actor_id).bind(c.request_id).fetch_optional(&mut *tx).await.map_err(storage)?;
        let mut roots = vec![
            ExactRef::Block(c.from.clone()),
            ExactRef::Block(c.to.clone()),
        ];
        // expected_revision is a CAS token, not evidence. Authorize the saved
        // result on replay and the current head on a new write. An inaccessible
        // audit predecessor is redacted by projection rather than required.
        if let Some((relation_id, revision_id)) = receipt {
            roots.push(ExactRef::Relation(RelationRef {
                relation_id,
                revision_id,
            }));
        } else if let Some(existing) = identity(&mut tx, c, false).await? {
            roots.push(existing.reference());
        }
        let allowed = lock_dependencies(&mut tx, actor, &c.scope, &roots).await?;
        if request::check(&mut tx, actor, c.request_id, &digest, "relation_save").await? {
            let (relation_id, revision_id) = receipt.ok_or(ContentError::Storage)?;
            let result = references::project(
                &mut tx,
                actor,
                &ExactRef::Relation(RelationRef {
                    relation_id,
                    revision_id,
                }),
            )
            .await?
            .ok_or(ContentError::NotFound)?
            .relation()?;
            tx.commit().await.map_err(storage)?;
            return Ok(result);
        }
        let (space, overlay) = scope(&c.scope);
        let kind = enum_text(json!(c.relation_type))?;
        let key = canonical_json(&json!([
            space,
            overlay,
            kind,
            c.from.block_id,
            c.to.block_id
        ]));
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("learning/relation/identity/{key}"))
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        let existing = identity(&mut tx, c, true).await?;
        let (relation, parent) = match existing {
            Some(existing) => {
                let mut current_roots = roots.clone();
                current_roots.push(existing.reference());
                check_dependencies(&mut tx, actor, &current_roots, &allowed).await?;
                if c.relation_id.is_none() {
                    return Err(ContentError::Invalid("relation_exists".into()));
                }
                if existing.space_id != space
                    || existing.overlay_id != overlay
                    || existing.r#type != kind
                    || existing.from_block_id != c.from.block_id
                    || existing.to_block_id != c.to.block_id
                {
                    return Err(ContentError::Invalid("relation_identity_mismatch".into()));
                }
                if c.expected_revision != Some(existing.head_revision_id) {
                    return Err(ContentError::Conflict {
                        current_revision_id: existing.head_revision_id,
                    });
                }
                (existing.id, Some(existing.head_revision_id))
            }
            None if c.relation_id.is_some() => return Err(ContentError::NotFound),
            None => (Uuid::new_v4(), None),
        };
        let checked = check_dependencies(&mut tx, actor, &roots, &allowed).await?;
        let from_space = checked
            .objects
            .get(&ExactRef::Block(c.from.clone()))
            .ok_or(ContentError::Storage)?
            .0;
        let to_space = checked
            .objects
            .get(&ExactRef::Block(c.to.clone()))
            .ok_or(ContentError::Storage)?
            .0;
        let revision = Uuid::new_v4();
        if parent.is_none() {
            sqlx::query("INSERT INTO public.relation(id,space_id,overlay_id,type,from_block_id,to_block_id,head_revision_id) VALUES($1,$2,$3,$4,$5,$6,$7)")
                .bind(relation).bind(space).bind(overlay).bind(&kind).bind(c.from.block_id).bind(c.to.block_id).bind(revision).execute(&mut *tx).await.map_err(storage)?;
        }
        let content_digest=hex_digest(canonical_json(&json!({"domain":"relation-content-v1","scope":c.scope,"type":c.relation_type,"from":c.from,"to":c.to,"rationale":c.rationale,"conditions":c.conditions})).as_bytes());
        sqlx::query("INSERT INTO public.relation_revision(id,space_id,relation_id,parent_revision_id,from_space_id,from_block_id,from_revision_id,to_space_id,to_block_id,to_revision_id,rationale,conditions,content_sha256,author_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)")
            .bind(revision).bind(space).bind(relation).bind(parent).bind(from_space).bind(c.from.block_id).bind(c.from.revision_id).bind(to_space).bind(c.to.block_id).bind(c.to.revision_id).bind(&c.rationale).bind(&c.conditions).bind(content_digest).bind(actor.actor_id).execute(&mut *tx).await.map_err(storage)?;
        let reference = RelationRef {
            relation_id: relation,
            revision_id: revision,
        };
        references::insert(
            &mut tx,
            &ExactRef::Relation(reference.clone()),
            &[
                Dependency {
                    role: DependencyRole::Target,
                    target: ExactRef::Block(c.from.clone()),
                },
                Dependency {
                    role: DependencyRole::Target,
                    target: ExactRef::Block(c.to.clone()),
                },
            ],
        )
        .await?;
        if parent.is_some() {
            sqlx::query("UPDATE public.relation SET head_revision_id=$1 WHERE id=$2")
                .bind(revision)
                .bind(relation)
                .execute(&mut *tx)
                .await
                .map_err(storage)?;
        }
        request::register(&mut tx, actor, c.request_id, &digest, "relation_save").await?;
        sqlx::query(
            "INSERT INTO public.relation_receipt(actor_id,request_id,revision_id) VALUES($1,$2,$3)",
        )
        .bind(actor.actor_id)
        .bind(c.request_id)
        .bind(revision)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
        let result = references::project(&mut tx, actor, &ExactRef::Relation(reference))
            .await?
            .ok_or(ContentError::NotFound)?
            .relation()?;
        tx.commit().await.map_err(storage)?;
        Ok(result)
    }
}

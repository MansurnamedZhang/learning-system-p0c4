use crate::storage;
use learning_core::*;
use sqlx::{Postgres, Transaction};
use std::collections::BTreeSet;
use uuid::Uuid;

pub(super) struct Fact {
    pub space: Uuid,
    pub bytes: usize,
    pub edges: Vec<(String, ExactRef)>,
}
pub(crate) fn key(r: &ExactRef) -> (&'static str, Uuid, Uuid) {
    match r {
        ExactRef::Block(r) => ("block", r.block_id, r.revision_id),
        ExactRef::Relation(r) => ("relation", r.relation_id, r.revision_id),
        ExactRef::RelationReview(r) => ("relation_review", r.relation.revision_id, r.review_id),
        ExactRef::EpistemicReview(r) => ("epistemic_review", r.stream_id, r.review_id),
    }
}
pub(super) async fn fact(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    r: &ExactRef,
) -> Result<Option<Fact>, ContentError> {
    let (kind, object, revision) = key(r);
    // Size is measured only for a directly authorized real object. Personal
    // scopes additionally require ownership, even when the space is shared.
    let (from, filter, size) = match r {
        ExactRef::Block(_) => (
            "public.block_revision r",
            "r.block_id=$2 AND r.id=$3",
            "octet_length(r.content::text)",
        ),
        ExactRef::Relation(_) => (
            "public.relation_revision r JOIN public.relation s ON s.id=r.relation_id",
            "r.relation_id=$2 AND r.id=$3 AND (s.overlay_id IS NULL OR EXISTS(SELECT 1 FROM public.overlay o WHERE o.id=s.overlay_id AND o.space_id=r.space_id AND o.owner_id=$1))",
            "octet_length(to_jsonb(r)::text)",
        ),
        ExactRef::RelationReview(_) => (
            "public.relation_review r JOIN public.relation s ON s.id=r.relation_id",
            "r.relation_revision_id=$2 AND r.id=$3 AND r.relation_id=$4 AND (s.overlay_id IS NULL OR EXISTS(SELECT 1 FROM public.overlay o WHERE o.id=s.overlay_id AND o.space_id=r.space_id AND o.owner_id=$1))",
            "octet_length(to_jsonb(r)::text)",
        ),
        ExactRef::EpistemicReview(_) => (
            "public.epistemic_review r JOIN public.epistemic_stream s ON s.id=r.stream_id",
            "r.stream_id=$2 AND r.id=$3 AND (s.overlay_id IS NULL OR EXISTS(SELECT 1 FROM public.overlay o WHERE o.id=s.overlay_id AND o.space_id=r.space_id AND o.owner_id=$1))",
            "octet_length(to_jsonb(r)::text)",
        ),
    };
    let sql = format!(
        "SELECT r.space_id,{size} FROM {from} JOIN public.space_grant g ON g.space_id=r.space_id AND g.actor_id=$1 WHERE {filter}"
    );
    let mut query = sqlx::query_as::<_, (Uuid, i32)>(&sql)
        .bind(actor.actor_id)
        .bind(object)
        .bind(revision);
    if let ExactRef::RelationReview(r) = r {
        query = query.bind(r.relation.relation_id);
    }
    let Some((space, bytes)) = query.fetch_optional(&mut **tx).await.map_err(storage)? else {
        return Ok(None);
    };
    let edges: Vec<(String,String,Uuid,Uuid,Option<Uuid>)> = sqlx::query_as("SELECT d.role,d.target_kind,d.target_object_id,d.target_revision_id,rr.relation_id FROM public.reference_dependency d LEFT JOIN public.relation_review rr ON d.target_kind='relation_review' AND rr.relation_revision_id=d.target_object_id AND rr.id=d.target_revision_id WHERE d.source_kind=$1 AND d.source_object_id=$2 AND d.source_revision_id=$3 ORDER BY d.position LIMIT 4097")
        .bind(kind).bind(object).bind(revision).fetch_all(&mut **tx).await.map_err(storage)?;
    let mut unique = BTreeSet::new();
    for (role, kind, object, revision, relation) in edges {
        let target = match kind.as_str() {
            "block" => ExactRef::Block(BlockRef {
                block_id: object,
                revision_id: revision,
            }),
            "relation" => ExactRef::Relation(RelationRef {
                relation_id: object,
                revision_id: revision,
            }),
            "relation_review" => ExactRef::RelationReview(RelationReviewRef {
                relation: RelationRef {
                    relation_id: relation.ok_or(ContentError::Storage)?,
                    revision_id: object,
                },
                review_id: revision,
            }),
            "epistemic_review" => ExactRef::EpistemicReview(EpistemicReviewRef {
                stream_id: object,
                review_id: revision,
            }),
            _ => return Err(ContentError::Storage),
        };
        unique.insert((role, target));
    }
    Ok(Some(Fact {
        space,
        bytes: bytes as usize,
        edges: unique.into_iter().collect(),
    }))
}
pub(crate) async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    source: &ExactRef,
    dependencies: &[Dependency],
) -> Result<(), ContentError> {
    let (kind, object, revision) = key(source);
    for (position, dep) in dependencies.iter().enumerate() {
        let (tk, ti, tr) = key(&dep.target);
        let role = serde_json::to_value(dep.role).map_err(storage)?;
        sqlx::query("INSERT INTO public.reference_dependency(source_kind,source_object_id,source_revision_id,position,role,target_kind,target_object_id,target_revision_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
            .bind(kind).bind(object).bind(revision).bind(position as i32).bind(role.as_str().ok_or(ContentError::Storage)?)
            .bind(tk).bind(ti).bind(tr).execute(&mut **tx).await.map_err(storage)?;
    }
    Ok(())
}

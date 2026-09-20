use serde_json::Value;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
type Tx<'a> = Transaction<'a, Postgres>;

pub async fn dependency(
    tx: &mut Tx<'_>,
    source: (&str, Uuid, Uuid),
    position: i32,
    role: &str,
    target: (&str, Uuid, Uuid),
) {
    sqlx::query("INSERT INTO reference_dependency(source_kind,source_object_id,source_revision_id,position,role,target_kind,target_object_id,target_revision_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
        .bind(source.0).bind(source.1).bind(source.2).bind(position).bind(role)
        .bind(target.0).bind(target.1).bind(target.2).execute(&mut **tx).await.unwrap();
}

pub async fn relation(
    tx: &mut Tx<'_>,
    actor: Uuid,
    space: Uuid,
    overlay: Option<Uuid>,
    from: (Uuid, Uuid),
    to: (Uuid, Uuid),
    indexed: bool,
) -> (Uuid, Uuid) {
    let id = Uuid::new_v4();
    let revision = Uuid::new_v4();
    sqlx::query("INSERT INTO relation(id,space_id,type,from_block_id,to_block_id,head_revision_id,overlay_id) VALUES($1,$2,'supports',$3,$4,$5,$6)")
        .bind(id).bind(space).bind(from.0).bind(to.0).bind(revision).bind(overlay).execute(&mut **tx).await.unwrap();
    sqlx::query("INSERT INTO relation_revision(id,space_id,relation_id,from_space_id,from_block_id,from_revision_id,to_space_id,to_block_id,to_revision_id,rationale,conditions,content_sha256,author_id) VALUES($1,$2,$3,$2,$4,$5,$2,$6,$7,'observation','within scope',$8,$9)")
        .bind(revision).bind(space).bind(id).bind(from.0).bind(from.1).bind(to.0).bind(to.1).bind("a".repeat(64)).bind(actor).execute(&mut **tx).await.unwrap();
    if indexed {
        dependency(
            tx,
            ("relation", id, revision),
            0,
            "target",
            ("block", from.0, from.1),
        )
        .await;
        dependency(
            tx,
            ("relation", id, revision),
            1,
            "target",
            ("block", to.0, to.1),
        )
        .await;
    }
    (id, revision)
}

pub async fn review(
    tx: &mut Tx<'_>,
    actor: Uuid,
    space: Uuid,
    rel: (Uuid, Uuid),
    previous: Option<Uuid>,
    indexed: bool,
) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO relation_review_head(relation_id,relation_revision_id,head_review_id) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
        .bind(rel.0).bind(rel.1).bind(id).execute(&mut **tx).await.unwrap();
    sqlx::query("INSERT INTO relation_review(id,space_id,relation_id,relation_revision_id,previous_review_id,state,explanation,reviewer_id) VALUES($1,$2,$3,$4,$5,'reviewed','checked',$6)")
        .bind(id).bind(space).bind(rel.0).bind(rel.1).bind(previous).bind(actor).execute(&mut **tx).await.unwrap();
    if indexed {
        dependency(
            tx,
            ("relation_review", rel.1, id),
            0,
            "target",
            ("relation", rel.0, rel.1),
        )
        .await;
    }
    id
}

#[allow(clippy::too_many_arguments)]
pub async fn epistemic(
    tx: &mut Tx<'_>,
    actor: Uuid,
    space: Uuid,
    overlay: Option<Uuid>,
    target: (Uuid, Uuid),
    selected: Value,
    evidence: Value,
    indexed: bool,
) -> (Uuid, Uuid) {
    let stream = Uuid::new_v4();
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO epistemic_stream(id,space_id,target_space_id,target_block_id,target_revision_id,actor_id,head_review_id,overlay_id) VALUES($1,$2,$2,$3,$4,$5,$6,$7)")
        .bind(stream).bind(space).bind(target.0).bind(target.1).bind(actor).bind(id).bind(overlay).execute(&mut **tx).await.unwrap();
    sqlx::query("INSERT INTO epistemic_review(id,space_id,stream_id,state,relations,evidence,conditions,explanation,reviewer_id) VALUES($1,$2,$3,'inconclusive',$4,$5,'bounded','pending',$6)")
        .bind(id).bind(space).bind(stream).bind(selected).bind(evidence).bind(actor).execute(&mut **tx).await.unwrap();
    if indexed {
        dependency(
            tx,
            ("epistemic_review", stream, id),
            0,
            "target",
            ("block", target.0, target.1),
        )
        .await;
    }
    (stream, id)
}

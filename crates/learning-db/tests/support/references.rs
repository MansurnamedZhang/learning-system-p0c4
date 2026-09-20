use super::TestRig;
use learning_core::*;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub async fn seed_content(
    tx: &mut Transaction<'_, Postgres>,
    actor: Principal,
    space: Uuid,
    draft: ContentDraft,
    previous: Option<BlockRef>,
) -> BlockRef {
    let reference = BlockRef {
        block_id: previous.as_ref().map_or_else(Uuid::new_v4, |r| r.block_id),
        revision_id: Uuid::new_v4(),
    };
    if previous.is_none() {
        sqlx::query("INSERT INTO block(id,space_id,head_revision_id) VALUES($1,$2,$3)")
            .bind(reference.block_id)
            .bind(space)
            .bind(reference.revision_id)
            .execute(&mut **tx)
            .await
            .unwrap();
    } else {
        sqlx::query("UPDATE block SET head_revision_id=$1 WHERE id=$2")
            .bind(reference.revision_id)
            .bind(reference.block_id)
            .execute(&mut **tx)
            .await
            .unwrap();
    }
    sqlx::query("INSERT INTO block_revision(id,space_id,block_id,parent_revision_id,contract_version,content,content_sha256,author_id,reason) VALUES($1,$2,$3,$4,$5,$6,$7,$8,'fixture')")
        .bind(reference.revision_id).bind(space).bind(reference.block_id).bind(previous.map(|r| r.revision_id))
        .bind(draft.contract_version() as i32).bind(serde_json::to_value(&draft).unwrap()).bind(draft.digest()).bind(actor.actor_id)
        .execute(&mut **tx).await.unwrap();
    for (position, dep) in draft.dependencies().into_iter().enumerate() {
        let (kind, object, revision) = key(&dep.target);
        sqlx::query("INSERT INTO reference_dependency(source_kind,source_object_id,source_revision_id,position,role,target_kind,target_object_id,target_revision_id) VALUES('block',$1,$2,$3,$4,$5,$6,$7)")
            .bind(reference.block_id).bind(reference.revision_id).bind(position as i32)
            .bind(serde_json::to_value(dep.role).unwrap().as_str().unwrap()).bind(kind).bind(object).bind(revision)
            .execute(&mut **tx).await.unwrap();
    }
    reference
}
pub fn key(r: &ExactRef) -> (&'static str, Uuid, Uuid) {
    match r {
        ExactRef::Block(r) => ("block", r.block_id, r.revision_id),
        ExactRef::Relation(r) => ("relation", r.relation_id, r.revision_id),
        ExactRef::RelationReview(r) => ("relation_review", r.relation.revision_id, r.review_id),
        ExactRef::EpistemicReview(r) => ("epistemic_review", r.stream_id, r.review_id),
    }
}
pub fn v2(dependency: BlockRef, target: bool) -> ContentDraft {
    ContentDraft::V2(ContentV2 {
        intent: Intent::Note,
        language: "en".into(),
        title: "conclusion".into(),
        body: if target {
            BodyV2::Reference {
                target: dependency.clone(),
            }
        } else {
            BodyV2::Text(super::command("conclusion body").draft.payload)
        },
        basis_refs: if target {
            vec![]
        } else {
            vec![ExactRef::Block(dependency)]
        },
        requires_context: vec![],
        source_run: None,
    })
}
pub async fn fixture(rig: &TestRig, target: bool) -> (Principal, Uuid, Uuid, BlockRef, BlockRef) {
    let (actor, space) = rig.seed_actor_space(true).await;
    let (_, evidence_space) = rig.seed_actor_space(true).await;
    rig.grant(actor, evidence_space, true).await;
    let (block_id, revision_id) = rig.seed_block(actor, evidence_space).await;
    let evidence = BlockRef {
        block_id,
        revision_id,
    };
    let mut tx = rig.runtime_pool.begin().await.unwrap();
    let conclusion = seed_content(&mut tx, actor, space, v2(evidence.clone(), target), None).await;
    tx.commit().await.unwrap();
    (actor, space, evidence_space, evidence, conclusion)
}

pub async fn seed_composition(
    rig: &TestRig,
    actor: Principal,
    space: Uuid,
    block: &BlockRef,
) -> CompositionRef {
    let root = CompositionRef {
        composition_id: Uuid::new_v4(),
        revision_id: Uuid::new_v4(),
    };
    let mut tx = rig.runtime_pool.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO composition(id,space_id,kind,head_revision_id) VALUES($1,$2,'document',$3)",
    )
    .bind(root.composition_id)
    .bind(space)
    .bind(root.revision_id)
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query("INSERT INTO composition_revision(id,space_id,composition_id,kind,title,content_sha256,author_id,reason) VALUES($1,$2,$3,'document','doc',$4,$5,'fixture')").bind(root.revision_id).bind(space).bind(root.composition_id).bind("a".repeat(64)).bind(actor.actor_id).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO composition_occurrence(composition_revision_id,space_id,composition_id,occurrence_id,position,block_space_id,block_id,block_revision_id) VALUES($1,$2,$3,$4,0,$2,$5,$6)").bind(root.revision_id).bind(space).bind(root.composition_id).bind(Uuid::new_v4()).bind(block.block_id).bind(block.revision_id).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    root
}
pub async fn seed_reading(
    rig: &TestRig,
    actor: Principal,
    space: Uuid,
    root: &CompositionRef,
) -> ReadingRef {
    let layer = Uuid::new_v4();
    let revision = Uuid::new_v4();
    let view = ReadingRef {
        view_id: Uuid::new_v4(),
        revision_id: Uuid::new_v4(),
    };
    let mut tx = rig.runtime_pool.begin().await.unwrap();
    sqlx::query("INSERT INTO overlay(id,space_id,owner_id,root_composition_id,head_revision_id) VALUES($1,$2,$3,$4,$5)").bind(layer).bind(space).bind(actor.actor_id).bind(root.composition_id).bind(revision).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO reading_view(id,overlay_id,head_revision_id) VALUES($1,$2,$3)")
        .bind(view.view_id)
        .bind(layer)
        .bind(view.revision_id)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO overlay_revision(id,overlay_id,space_id,root_composition_id,source_space_id,base_revision_id,title,content_sha256,author_id,reason) VALUES($1,$2,$3,$4,$3,$5,'reading',$6,$7,'fixture')").bind(revision).bind(layer).bind(space).bind(root.composition_id).bind(root.revision_id).bind("a".repeat(64)).bind(actor.actor_id).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO reading_view_revision(id,view_id,overlay_id,overlay_revision_id,author_id) VALUES($1,$2,$3,$4,$5)").bind(view.revision_id).bind(view.view_id).bind(layer).bind(revision).bind(actor.actor_id).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    view
}

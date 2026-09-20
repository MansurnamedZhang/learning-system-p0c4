#![allow(dead_code)]
use super::support::TestRig;
use learning_core::*;
use learning_db::RelationStore;
use uuid::Uuid;

pub fn save(space: Uuid, from: BlockRef, to: BlockRef) -> SaveRelation {
    SaveRelation {
        request_id: Uuid::new_v4(),
        scope: RelationScope::Space { space_id: space },
        relation_id: None,
        expected_revision: None,
        relation_type: RelationType::Supports,
        from,
        to,
        rationale: "observation".into(),
        conditions: "within scope".into(),
    }
}
pub fn edit(r: &RelationRevision) -> SaveRelation {
    SaveRelation {
        request_id: Uuid::new_v4(),
        scope: r.scope.clone(),
        relation_id: Some(r.reference.relation_id),
        expected_revision: Some(r.reference.revision_id),
        relation_type: r.relation_type,
        from: r.from.clone(),
        to: r.to.clone(),
        rationale: r.rationale.clone(),
        conditions: r.conditions.clone(),
    }
}
pub fn review(r: &RelationRevision) -> ReviewRelation {
    ReviewRelation {
        request_id: Uuid::new_v4(),
        relation: r.reference.clone(),
        expected_previous: None,
        state: RelationReviewState::Reviewed,
        explanation: "checked within stated conditions".into(),
    }
}
pub fn exact(r: &Revision) -> BlockRef {
    BlockRef {
        block_id: r.block_id,
        revision_id: r.revision_id,
    }
}
pub async fn fixture() -> (TestRig, RelationStore, Principal, Uuid, Revision, Revision) {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let from = rig
        .store
        .create(actor, space, super::support::command("E@1"))
        .await
        .unwrap();
    let to = rig
        .store
        .create(actor, space, super::support::command("H@1"))
        .await
        .unwrap();
    let store = RelationStore::new(rig.runtime_pool.clone());
    (rig, store, actor, space, from, to)
}
pub async fn counts(rig: &TestRig, space: Uuid) -> (i64, i64, i64, i64, i64) {
    sqlx::query_as("SELECT (SELECT count(*) FROM relation WHERE space_id=$1),(SELECT count(*) FROM relation_revision WHERE space_id=$1),(SELECT count(*) FROM relation_review WHERE space_id=$1),(SELECT count(*) FROM relation_receipt m JOIN relation_revision r ON r.id=m.revision_id WHERE r.space_id=$1),(SELECT count(*) FROM relation_review_receipt m JOIN relation_review r ON r.id=m.review_id WHERE r.space_id=$1)").bind(space).fetch_one(&rig.admin_pool).await.unwrap()
}

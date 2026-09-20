use super::*;
use learning_core::*;
use learning_db::ReadingStore;
pub fn store(r: &TestRig) -> ReadingStore {
    ReadingStore::new(r.runtime_pool.clone())
}
pub fn create(base: CompositionRef) -> CreateReading {
    CreateReading {
        request_id: Uuid::new_v4(),
        base,
        title: "我的学习".into(),
        reason: "start".into(),
    }
}
pub fn edit(saved: &ReadingSaved, edit: ReadingEdit) -> EditReading {
    EditReading {
        request_id: Uuid::new_v4(),
        expected_overlay_revision: saved.overlay.revision_id,
        expected_reading_view_revision: saved.view.revision_id,
        edit,
        reason: "edit".into(),
    }
}
pub fn gap(doc: &CompositionRevision, index: usize) -> GapAnchor {
    GapAnchor {
        base: doc.reference.clone(),
        parent_occurrence_path: vec![],
        left_occurrence_id: index.checked_sub(1).map(|i| doc.nodes[i].occurrence_id),
        right_occurrence_id: doc.nodes.get(index).map(|n| n.occurrence_id),
        affinity: Affinity::AfterLeft,
    }
}
pub fn add(anchor: GapAnchor, text: &str) -> ReadingEdit {
    ReadingEdit::InsertNew {
        drafts: vec![command(text).draft],
        target: InsertTarget::NewGroup { anchor },
    }
}
pub fn text(projection: &ReadingProjection) -> Vec<String> {
    projection
        .items
        .iter()
        .filter_map(|i| match i {
            ReadingItem::Original { revision, .. } => Some(revision.draft.payload.text.clone()),
            ReadingItem::Personal { item } => Some(item.revision.draft.payload.text.clone()),
            _ => None,
        })
        .collect()
}
pub async fn fixture() -> (TestRig, Principal, Uuid, CompositionRevision, ReadingSaved) {
    let r = TestRig::from_env().await;
    let (a, s) = r.seed_actor_space(true).await;
    let k = r.store.create(a, s, command("K")).await.unwrap();
    let m = r.store.create(a, s, command("M")).await.unwrap();
    let doc = r
        .compositions()
        .save(
            a,
            s,
            assembly::doc(vec![assembly::block(&k), assembly::block(&m)]),
        )
        .await
        .unwrap();
    let saved = store(&r)
        .create(a, s, create(doc.reference.clone()))
        .await
        .unwrap();
    (r, a, s, doc, saved)
}
pub async fn counts(r: &TestRig, actor: Principal) -> Vec<i64> {
    let mut out = vec![];
    for q in [
        "SELECT count(*) FROM block WHERE space_id IN (SELECT id FROM space WHERE owner_id=$1)",
        "SELECT count(*) FROM block_revision WHERE author_id=$1",
        "SELECT count(*) FROM overlay WHERE owner_id=$1",
        "SELECT count(*) FROM overlay_revision WHERE author_id=$1",
        "SELECT count(*) FROM overlay_group WHERE overlay_id IN (SELECT id FROM overlay WHERE owner_id=$1)",
        "SELECT count(*) FROM overlay_group_identity WHERE overlay_id IN (SELECT id FROM overlay WHERE owner_id=$1)",
        "SELECT count(*) FROM overlay_placement WHERE overlay_id IN (SELECT id FROM overlay WHERE owner_id=$1)",
        "SELECT count(*) FROM overlay_placement_identity WHERE overlay_id IN (SELECT id FROM overlay WHERE owner_id=$1)",
        "SELECT count(*) FROM reading_view WHERE overlay_id IN (SELECT id FROM overlay WHERE owner_id=$1)",
        "SELECT count(*) FROM reading_view_revision WHERE author_id=$1",
        "SELECT count(*) FROM request_key WHERE actor_id=$1",
        "SELECT count(*) FROM reading_receipt WHERE actor_id=$1",
        "SELECT count(*) FROM placement_migration WHERE author_id=$1",
        "SELECT count(*) FROM placement_migration_decision WHERE author_id=$1",
        "SELECT count(*) FROM placement_migration_mapping WHERE decision_id IN (SELECT id FROM placement_migration_decision WHERE author_id=$1)",
        "SELECT count(*) FROM migration_receipt WHERE actor_id=$1",
        "SELECT count(*) FROM placement_manual_decision WHERE overlay_id IN (SELECT id FROM overlay WHERE owner_id=$1)",
        "SELECT count(*) FROM reading_receipt_block WHERE actor_id=$1",
    ] {
        out.push(
            sqlx::query_scalar(q)
                .bind(actor.actor_id)
                .fetch_one(&r.admin_pool)
                .await
                .unwrap(),
        );
    }
    out
}

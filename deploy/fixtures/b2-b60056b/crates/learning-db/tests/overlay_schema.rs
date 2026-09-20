mod support;
use support::TestRig;
use support::{reading as h, sqlstate};
use uuid::Uuid;
#[tokio::test]
async fn immutable_runtime_writes_and_cross_object_heads_are_rejected() {
    let (r, actor, _s, doc, zero) = h::fixture().await;
    let one = h::store(&r)
        .edit(
            actor,
            zero.overlay.overlay_id,
            h::edit(&zero, h::add(h::gap(&doc, 1), "N")),
        )
        .await
        .unwrap();
    let other = h::store(&r)
        .create(
            actor,
            r_space(&r, actor).await,
            h::create(doc.reference.clone()),
        )
        .await
        .unwrap();
    for table in [
        "overlay_revision",
        "overlay_group",
        "overlay_placement",
        "reading_view_revision",
        "reading_receipt",
    ] {
        let err = sqlx::query(&format!("DELETE FROM {table} WHERE false"))
            .execute(&r.runtime_pool)
            .await
            .unwrap_err();
        assert_eq!(sqlstate(&err).as_deref(), Some("42501"));
    }
    for (table, id, head) in [
        ("overlay", one.overlay.overlay_id, other.overlay.revision_id),
        ("reading_view", one.view.view_id, other.view.revision_id),
    ] {
        let mut tx = r.runtime_pool.begin().await.unwrap();
        sqlx::query(&format!(
            "UPDATE {table} SET head_revision_id=$1 WHERE id=$2"
        ))
        .bind(head)
        .bind(id)
        .execute(&mut *tx)
        .await
        .unwrap();
        let err = tx.commit().await.unwrap_err();
        assert_eq!(sqlstate(&err).as_deref(), Some("23503"));
    }
    for table in ["overlay_group", "overlay_placement"] {
        let mut tx = r.admin_pool.begin().await.unwrap();
        let err = sqlx::query(&format!(
            "UPDATE {table} SET overlay_id=$1 WHERE overlay_revision_id=$2"
        ))
        .bind(other.overlay.overlay_id)
        .bind(one.overlay.revision_id)
        .execute(&mut *tx)
        .await
        .unwrap_err();
        assert_eq!(sqlstate(&err).as_deref(), Some("23503"));
        tx.rollback().await.unwrap();
    }
    let learning_core::NodeTarget::Block(original) = &doc.nodes[0].target else {
        panic!("fixture block")
    };
    let mut tx = r.admin_pool.begin().await.unwrap();
    let err = sqlx::query(
        "UPDATE overlay_placement SET block_revision_id=$1 WHERE overlay_revision_id=$2",
    )
    .bind(original.revision_id)
    .bind(one.overlay.revision_id)
    .execute(&mut *tx)
    .await
    .unwrap_err();
    assert_eq!(sqlstate(&err).as_deref(), Some("23503"));
    tx.rollback().await.unwrap();
}
async fn r_space(r: &TestRig, a: learning_core::Principal) -> Uuid {
    sqlx::query_scalar("SELECT id FROM space WHERE owner_id=$1")
        .bind(a.actor_id)
        .fetch_one(&r.admin_pool)
        .await
        .unwrap()
}
#[tokio::test]
async fn immutable_personal_tables_have_no_runtime_mutation_rights() {
    let r = TestRig::from_env().await;
    for table in [
        "overlay_revision",
        "overlay_group",
        "overlay_placement",
        "reading_view_revision",
        "placement_migration",
        "placement_migration_decision",
        "reading_receipt",
    ] {
        let exists: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
            .bind(format!("public.{table}"))
            .fetch_one(&r.runtime_pool)
            .await
            .unwrap();
        assert!(exists, "missing {table}");
        for privilege in ["UPDATE", "DELETE", "TRUNCATE"] {
            let yes: bool = sqlx::query_scalar("SELECT has_table_privilege(current_user,$1,$2)")
                .bind(format!("public.{table}"))
                .bind(privilege)
                .fetch_one(&r.runtime_pool)
                .await
                .unwrap();
            assert!(!yes, "{table} {privilege}");
        }
    }
}

mod support;
use support::{TestRig, sqlstate};
use uuid::Uuid;

#[tokio::test]
async fn real_foreign_child_revision_and_space_mismatches_fail_database_constraints() {
    let rig = TestRig::from_env().await;
    let (a, s) = rig.seed_actor_space(true).await;
    let (o, d) = rig.seed_actor_space(true).await;
    let c = rig
        .compositions()
        .save(a, s, support::assembly::doc(vec![]))
        .await
        .unwrap();
    let other = rig
        .compositions()
        .save(o, d, support::assembly::doc(vec![]))
        .await
        .unwrap();
    let query = "INSERT INTO composition_occurrence(composition_revision_id,space_id,composition_id,occurrence_id,position,child_space_id,child_composition_id,child_revision_id) VALUES($1,$2,$3,$4,0,$5,$6,$7)";
    for (target_space, target_id) in [
        (d, c.reference.composition_id),
        (s, other.reference.composition_id),
    ] {
        let error = sqlx::query(query)
            .bind(c.reference.revision_id)
            .bind(s)
            .bind(c.reference.composition_id)
            .bind(Uuid::new_v4())
            .bind(target_space)
            .bind(target_id)
            .bind(other.reference.revision_id)
            .execute(&rig.admin_pool)
            .await
            .unwrap_err();
        assert_eq!(sqlstate(&error).as_deref(), Some("23503"));
    }
    let (block, block_rev) = rig.seed_block(a, s).await;
    let error=sqlx::query("INSERT INTO composition_occurrence(composition_revision_id,space_id,composition_id,occurrence_id,position,child_space_id,child_composition_id,child_revision_id,block_space_id,block_id,block_revision_id) VALUES($1,$2,$3,$4,0,$5,$6,$7,$2,$8,$9)").bind(c.reference.revision_id).bind(s).bind(c.reference.composition_id).bind(Uuid::new_v4()).bind(d).bind(other.reference.composition_id).bind(other.reference.revision_id).bind(block).bind(block_rev).execute(&rig.admin_pool).await.unwrap_err();
    assert_eq!(sqlstate(&error).as_deref(), Some("23514"));
}

#[tokio::test]
async fn assembly_tables_exist_with_append_only_runtime_access() {
    let rig = TestRig::from_env().await;
    for table in [
        "request_key",
        "composition",
        "composition_revision",
        "composition_occurrence",
        "composition_receipt",
        "release",
        "release_root",
        "release_receipt",
        "outbox_event",
    ] {
        let exists: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
            .bind(format!("public.{table}"))
            .fetch_one(&rig.runtime_pool)
            .await
            .unwrap();
        assert!(exists, "missing authoritative table {table}");
        for privilege in ["DELETE", "TRUNCATE", "UPDATE"] {
            let allowed: bool =
                sqlx::query_scalar("SELECT has_table_privilege(current_user,$1,$2)")
                    .bind(format!("public.{table}"))
                    .bind(privilege)
                    .fetch_one(&rig.runtime_pool)
                    .await
                    .unwrap();
            assert!(!allowed, "unexpected {table} {privilege}");
        }
    }
}

#[tokio::test]
async fn composition_head_and_targets_require_exact_object_identity() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let id = Uuid::new_v4();
    let rev = Uuid::new_v4();
    let mut tx = rig.admin_pool.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO composition(id,space_id,kind,head_revision_id) VALUES($1,$2,'document',$3)",
    )
    .bind(id)
    .bind(space)
    .bind(rev)
    .execute(&mut *tx)
    .await
    .unwrap();
    let error = tx.commit().await.unwrap_err();
    assert_eq!(sqlstate(&error).as_deref(), Some("23503"));
    let error = sqlx::query(
        "INSERT INTO composition(id,space_id,kind,head_revision_id) VALUES($1,$2,'document',NULL)",
    )
    .bind(id)
    .bind(space)
    .execute(&rig.admin_pool)
    .await
    .unwrap_err();
    assert_eq!(sqlstate(&error).as_deref(), Some("23502"));
    let (block, block_rev) = rig.seed_block(actor, space).await;
    let mut tx = rig.admin_pool.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO composition(id,space_id,kind,head_revision_id) VALUES($1,$2,'document',$3)",
    )
    .bind(id)
    .bind(space)
    .bind(rev)
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query("INSERT INTO composition_revision(id,space_id,composition_id,kind,title,content_sha256,author_id,reason) VALUES($1,$2,$3,'document','fixture',$4,$5,'fixture')").bind(rev).bind(space).bind(id).bind("0".repeat(64)).bind(actor.actor_id).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let (other_block, _) = rig.seed_block(actor, space).await;
    let query = "INSERT INTO composition_occurrence(composition_revision_id,space_id,composition_id,occurrence_id,position,block_space_id,block_id,block_revision_id) VALUES($1,$2,$3,$4,0,$2,$5,$6)";
    let error = sqlx::query(query)
        .bind(rev)
        .bind(space)
        .bind(id)
        .bind(Uuid::new_v4())
        .bind(other_block)
        .bind(block_rev)
        .execute(&rig.admin_pool)
        .await
        .unwrap_err();
    assert_eq!(sqlstate(&error).as_deref(), Some("23503"));
    sqlx::query(query)
        .bind(rev)
        .bind(space)
        .bind(id)
        .bind(Uuid::new_v4())
        .bind(block)
        .bind(block_rev)
        .execute(&rig.admin_pool)
        .await
        .unwrap();
    let error = sqlx::query(query)
        .bind(rev)
        .bind(space)
        .bind(id)
        .bind(Uuid::new_v4())
        .bind(block)
        .bind(block_rev)
        .execute(&rig.admin_pool)
        .await
        .unwrap_err();
    assert_eq!(sqlstate(&error).as_deref(), Some("23505"));
    let error=sqlx::query("INSERT INTO composition_occurrence(composition_revision_id,space_id,composition_id,occurrence_id,position) VALUES($1,$2,$3,$4,1)").bind(rev).bind(space).bind(id).bind(Uuid::new_v4()).execute(&rig.admin_pool).await.unwrap_err();
    assert_eq!(sqlstate(&error).as_deref(), Some("23514"));
    let error = sqlx::query("UPDATE composition SET published_revision_id=$1 WHERE id=$2")
        .bind(rev)
        .bind(id)
        .execute(&rig.admin_pool)
        .await
        .unwrap_err();
    assert_eq!(sqlstate(&error).as_deref(), Some("23514"));
}

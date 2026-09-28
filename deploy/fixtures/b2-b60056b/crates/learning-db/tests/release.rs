mod support;
use learning_core::*;
use support::{assembly::*, *};
use uuid::Uuid;

#[tokio::test]
async fn edit_publish_selected_and_rollback_preserve_old_snapshots() {
    let rig = TestRig::from_env().await;
    let (a, s) = rig.seed_actor_space(true).await;
    let cs = rig.compositions();
    let rs = rig.releases();
    let old = rig.store.create(a, s, command("old")).await.unwrap();
    let x = cs.save(a, s, doc(vec![block(&old)])).await.unwrap();
    let y = cs.save(a, s, doc(vec![block(&old)])).await.unwrap();
    let initial = rs
        .publish(a, s, publish(vec![root(&x, None), root(&y, None)]))
        .await
        .unwrap();
    let new = rig
        .store
        .revise(a, old.block_id, change(&old, "new"))
        .await
        .unwrap();
    let mut c = edit(&x);
    c.nodes[0].target = block(&new);
    let x2 = cs.save(a, s, c).await.unwrap();
    assert_eq!(
        rs.active(a, x.reference.composition_id).await.unwrap(),
        Some(x.reference.clone())
    );
    let selected = rs
        .publish(a, s, publish(vec![root(&x2, Some(initial.release_id))]))
        .await
        .unwrap();
    assert_eq!(
        rs.active(a, x.reference.composition_id).await.unwrap(),
        Some(x2.reference.clone())
    );
    assert_eq!(
        rs.active(a, y.reference.composition_id).await.unwrap(),
        Some(y.reference.clone())
    );
    let mut rollback = root(&x2, Some(selected.release_id));
    rollback.revision_id = x.reference.revision_id;
    let reverted = rs.publish(a, s, publish(vec![rollback])).await.unwrap();
    assert_ne!(reverted.release_id, initial.release_id);
    assert_eq!(
        rs.active(a, x.reference.composition_id).await.unwrap(),
        Some(x.reference.clone())
    );
    assert_eq!(
        rs.read(a, selected.release_id).await.unwrap(),
        Some(selected)
    );
    let head: Uuid = sqlx::query_scalar("SELECT head_revision_id FROM composition WHERE id=$1")
        .bind(x.reference.composition_id)
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap();
    assert_eq!(head, x2.reference.revision_id);
    assert_eq!(
        cs.read(a, x.reference).await.unwrap().unwrap().blocks,
        vec![old]
    );
}
#[tokio::test]
async fn release_replay_precedes_stale_checks_but_changed_commands_conflict() {
    let rig = TestRig::from_env().await;
    let (a, s) = rig.seed_actor_space(true).await;
    let cs = rig.compositions();
    let rs = rig.releases();
    let x = cs.save(a, s, doc(vec![])).await.unwrap();
    let y = cs.save(a, s, doc(vec![])).await.unwrap();
    let cmd = publish(vec![root(&x, None), root(&y, None)]);
    let saved = rs.publish(a, s, cmd.clone()).await.unwrap();
    let x2 = cs.save(a, s, edit(&x)).await.unwrap();
    rs.publish(a, s, publish(vec![root(&x2, Some(saved.release_id))]))
        .await
        .unwrap();
    let mut reversed = cmd.clone();
    reversed.roots.reverse();
    assert_eq!(rs.publish(a, s, reversed).await.unwrap(), saved);
    for which in 0..3 {
        let mut changed = cmd.clone();
        match which {
            0 => changed.reason.push('x'),
            1 => changed.roots[0].expected_head_revision_id = Uuid::new_v4(),
            _ => {
                changed.roots.pop();
            }
        }
        assert!(matches!(
            rs.publish(a, s, changed).await,
            Err(ContentError::IdempotencyConflict)
        ));
    }
    assert!(
        matches!(rs.publish(a,s,publish(vec![root(&x2,None)])).await,Err(ContentError::PublicationConflict{composition_id}) if composition_id==x.reference.composition_id)
    );
    assert!(matches!(
        rs.publish(a, s, publish(vec![root(&x, Some(saved.release_id))]))
            .await,
        Err(ContentError::Conflict { .. })
    ));
}
#[tokio::test]
async fn late_outbox_failure_rolls_back_every_root_and_retries_same_key() {
    let rig = TestRig::from_env().await;
    let (a, s) = rig.seed_actor_space(true).await;
    let cs = rig.compositions();
    let rs = rig.releases();
    let x = cs.save(a, s, doc(vec![])).await.unwrap();
    let y = cs.save(a, s, doc(vec![])).await.unwrap();
    let initial = rs
        .publish(a, s, publish(vec![root(&x, None), root(&y, None)]))
        .await
        .unwrap();
    let x2 = cs.save(a, s, edit(&x)).await.unwrap();
    let y2 = cs.save(a, s, edit(&y)).await.unwrap();
    let cmd = publish(vec![
        root(&x2, Some(initial.release_id)),
        root(&y2, Some(initial.release_id)),
    ]);
    let before = rig.assembly_counts(a).await;
    let structure_before = rig.structure_counts(s).await;
    let suffix = a.actor_id.simple();
    sqlx::raw_sql(&format!("CREATE FUNCTION fail_outbox_{suffix}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF EXISTS(SELECT 1 FROM release WHERE id=NEW.aggregate_id AND author_id='{}'::uuid) THEN RAISE EXCEPTION 'injected_release_failure'; END IF; RETURN NEW; END; $$; CREATE TRIGGER fail_{suffix} BEFORE INSERT ON outbox_event FOR EACH ROW EXECUTE FUNCTION fail_outbox_{suffix}()",a.actor_id)).execute(&rig.admin_pool).await.unwrap();
    let failed = rs.publish(a, s, cmd.clone()).await;
    sqlx::raw_sql(&format!(
        "DROP TRIGGER fail_{suffix} ON outbox_event; DROP FUNCTION fail_outbox_{suffix}()"
    ))
    .execute(&rig.admin_pool)
    .await
    .unwrap();
    assert!(matches!(failed, Err(ContentError::Storage)));
    assert_eq!(rig.assembly_counts(a).await, before);
    assert_eq!(rig.structure_counts(s).await, structure_before);
    assert_eq!(
        rs.active(a, x.reference.composition_id).await.unwrap(),
        Some(x.reference)
    );
    assert_eq!(
        rs.active(a, y.reference.composition_id).await.unwrap(),
        Some(y.reference)
    );
    let retry = rs.publish(a, s, cmd.clone()).await.unwrap();
    assert_eq!(rs.publish(a, s, cmd).await.unwrap(), retry);
    let after = rig.assembly_counts(a).await;
    assert_eq!(after.1, before.1 + 1);
    assert_eq!(after.2, before.2 + 1);
    assert_eq!(after.3, before.3 + 1);
}

mod support;
use learning_core::*;
use support::{assembly::*, *};
use uuid::Uuid;

#[tokio::test]
async fn readonly_dependency_is_usable_but_revocation_hides_all_reference_outputs() {
    let rig = TestRig::from_env().await;
    let (a, s) = rig.seed_actor_space(true).await;
    let (owner, dependency) = rig.seed_actor_space(true).await;
    rig.grant(a, dependency, false).await;
    let k = rig
        .store
        .create(owner, dependency, command("SECRET_BODY"))
        .await
        .unwrap();
    let cs = rig.compositions();
    let rs = rig.releases();
    let cmd = doc(vec![block(&k)]);
    let c = cs.save(a, s, cmd.clone()).await.unwrap();
    let pubcmd = publish(vec![root(&c, None)]);
    let published = rs.publish(a, s, pubcmd.clone()).await.unwrap();
    assert_eq!(
        cs.read(a, c.reference.clone())
            .await
            .unwrap()
            .unwrap()
            .blocks,
        vec![k.clone()]
    );
    rig.revoke(a, dependency).await;
    assert!(cs.read(a, c.reference.clone()).await.unwrap().is_none());
    assert!(rs.read(a, published.release_id).await.unwrap().is_none());
    assert!(
        rs.active(a, c.reference.composition_id)
            .await
            .unwrap()
            .is_none()
    );
    for error in [
        cs.save(a, s, cmd).await.unwrap_err(),
        rs.publish(a, s, pubcmd).await.unwrap_err(),
    ] {
        assert!(matches!(error, ContentError::NotFound));
        let text = error.to_string();
        assert!(!text.contains(&k.block_id.to_string()));
        assert!(!text.contains("SECRET_BODY"));
    }
    let mut hidden = doc(vec![block(&k)]);
    let err = cs.save(a, s, hidden.clone()).await.unwrap_err();
    hidden.nodes[0].target = NodeTarget::Block(BlockRef {
        block_id: Uuid::new_v4(),
        revision_id: Uuid::new_v4(),
    });
    assert_eq!(
        err.to_string(),
        cs.save(a, s, hidden).await.unwrap_err().to_string()
    );
}
#[tokio::test]
async fn release_read_checks_all_roots_but_active_checks_only_its_selected_root() {
    let rig = TestRig::from_env().await;
    let (a, s) = rig.seed_actor_space(true).await;
    let (o, d) = rig.seed_actor_space(true).await;
    rig.grant(a, d, false).await;
    let k = rig.store.create(o, d, command("secret")).await.unwrap();
    let cs = rig.compositions();
    let rs = rig.releases();
    let visible = cs.save(a, s, doc(vec![])).await.unwrap();
    let hidden = cs.save(a, s, doc(vec![block(&k)])).await.unwrap();
    let r = rs
        .publish(
            a,
            s,
            publish(vec![root(&visible, None), root(&hidden, None)]),
        )
        .await
        .unwrap();
    rig.revoke(a, d).await;
    assert!(rs.read(a, r.release_id).await.unwrap().is_none());
    assert_eq!(
        rs.active(a, visible.reference.composition_id)
            .await
            .unwrap(),
        Some(visible.reference)
    );
    assert!(
        rs.active(a, hidden.reference.composition_id)
            .await
            .unwrap()
            .is_none()
    );
}
#[tokio::test]
async fn target_space_must_be_writable_and_cross_operation_keys_are_global() {
    let rig = TestRig::from_env().await;
    let (a, s) = rig.seed_actor_space(true).await;
    let cs = rig.compositions();
    let rs = rig.releases();
    let c = doc(vec![]);
    let saved = cs.save(a, s, c.clone()).await.unwrap();
    let mut blockcmd = command("same key");
    blockcmd.request_id = c.request_id;
    assert!(matches!(
        rig.store.create(a, s, blockcmd).await,
        Err(ContentError::IdempotencyConflict)
    ));
    let content = command("content key");
    rig.store.create(a, s, content.clone()).await.unwrap();
    let mut comp = doc(vec![]);
    comp.request_id = content.request_id;
    assert!(matches!(
        cs.save(a, s, comp).await,
        Err(ContentError::IdempotencyConflict)
    ));
    let mut pubcmd = publish(vec![root(&saved, None)]);
    pubcmd.request_id = c.request_id;
    assert!(matches!(
        rs.publish(a, s, pubcmd).await,
        Err(ContentError::IdempotencyConflict)
    ));
    rig.grant(a, s, false).await;
    assert!(cs.read(a, saved.reference.clone()).await.unwrap().is_some());
    assert!(matches!(
        cs.save(a, s, c).await,
        Err(ContentError::NotFound)
    ));
    assert!(matches!(
        rs.publish(a, s, publish(vec![root(&saved, None)])).await,
        Err(ContentError::NotFound)
    ));
}

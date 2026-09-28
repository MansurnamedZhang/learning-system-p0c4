mod support;
use learning_core::*;
use learning_db::ReleaseStore;
use support::{assembly::*, *};

fn prepare(state: &PublicationState, reference: &CompositionRef) -> PublishCommand {
    publish(vec![PublishRoot {
        composition_id: state.composition_id,
        revision_id: reference.revision_id,
        expected_head_revision_id: state.head_revision_id,
        expected_publication_token: state.publication_token.clone(),
    }])
}

#[tokio::test]
async fn fresh_caller_can_discover_working_head_and_publish_without_a_saved_release_id() {
    let rig = TestRig::from_env().await;
    let (a, s) = rig.seed_actor_space(true).await;
    let cs = rig.compositions();
    let doc = cs.save(a, s, doc(vec![])).await.unwrap();
    let state = rig
        .releases()
        .state(a, doc.reference.composition_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.head_revision_id, doc.reference.revision_id);
    assert_eq!(state.published, None);
    let published = rig
        .releases()
        .publish(a, s, prepare(&state, &doc.reference))
        .await
        .unwrap();
    let updated = cs.save(a, s, edit(&doc)).await.unwrap();
    let fresh = ReleaseStore::new(rig.runtime_pool.clone());
    let state = fresh
        .state(a, doc.reference.composition_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.head_revision_id, updated.reference.revision_id);
    assert_eq!(state.published, Some(doc.reference));
    assert!(
        !serde_json::to_string(&state)
            .unwrap()
            .contains(&published.release_id.to_string())
    );
    let next = fresh
        .publish(a, s, prepare(&state, &updated.reference))
        .await
        .unwrap();
    assert_eq!(next.roots, vec![updated.reference]);
}
#[tokio::test]
async fn stale_publication_token_can_be_refreshed_and_tokens_are_root_scoped() {
    let rig = TestRig::from_env().await;
    let (a, s) = rig.seed_actor_space(true).await;
    let cs = rig.compositions();
    let rs = rig.releases();
    let x = cs.save(a, s, doc(vec![])).await.unwrap();
    let y = cs.save(a, s, doc(vec![])).await.unwrap();
    rs.publish(a, s, publish(vec![root(&x, None), root(&y, None)]))
        .await
        .unwrap();
    let before = rs
        .state(a, x.reference.composition_id)
        .await
        .unwrap()
        .unwrap();
    let sibling = rs
        .state(a, y.reference.composition_id)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(before.publication_token, sibling.publication_token);
    let mut crossed = prepare(&sibling, &y.reference);
    crossed.roots[0].expected_publication_token = before.publication_token.clone();
    assert!(matches!(
        rs.publish(a, s, crossed).await,
        Err(ContentError::PublicationConflict { .. })
    ));
    rs.publish(a, s, prepare(&before, &x.reference))
        .await
        .unwrap();
    assert!(matches!(
        rs.publish(a, s, prepare(&before, &x.reference)).await,
        Err(ContentError::PublicationConflict { .. })
    ));
    let after = rs
        .state(a, x.reference.composition_id)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(after.publication_token, before.publication_token);
    assert_eq!(after.published, before.published);
    assert!(
        rs.publish(a, s, prepare(&after, &x.reference))
            .await
            .is_ok()
    );
}
#[tokio::test]
async fn state_of_readable_root_survives_sibling_revocation_without_exposing_release_identity() {
    let rig = TestRig::from_env().await;
    let (a, s) = rig.seed_actor_space(true).await;
    let (o, d) = rig.seed_actor_space(true).await;
    rig.grant(a, d, false).await;
    let k = rig
        .store
        .create(o, d, command("hidden sibling"))
        .await
        .unwrap();
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
    assert!(
        rs.state(a, hidden.reference.composition_id)
            .await
            .unwrap()
            .is_none()
    );
    let state = rs
        .state(a, visible.reference.composition_id)
        .await
        .unwrap()
        .unwrap();
    let text = serde_json::to_string(&state).unwrap();
    for secret in [
        r.release_id,
        hidden.reference.composition_id,
        k.block_id,
        k.revision_id,
    ] {
        assert!(!text.contains(&secret.to_string()));
    }
    assert!(
        rs.publish(a, s, prepare(&state, &visible.reference))
            .await
            .is_ok()
    );
    rig.revoke(a, s).await;
    assert!(
        rs.state(a, visible.reference.composition_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(rs.state(a, uuid::Uuid::new_v4()).await.unwrap().is_none());
}

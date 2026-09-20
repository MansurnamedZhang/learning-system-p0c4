mod support;
use learning_core::*;
use support::{assembly as a, reading as h, *};
#[tokio::test]
async fn source_revocation_preserves_notes_without_source_identifiers_and_restores() {
    let r = TestRig::from_env().await;
    let (owner, source) = r.seed_actor_space(true).await;
    let (actor, personal) = r.seed_actor_space(true).await;
    let k = r
        .store
        .create(owner, source, command("SECRET_SOURCE"))
        .await
        .unwrap();
    let doc = r
        .compositions()
        .save(owner, source, a::doc(vec![a::block(&k)]))
        .await
        .unwrap();
    r.grant(actor, source, false).await;
    let store = h::store(&r);
    let zero = store
        .create(actor, personal, h::create(doc.reference.clone()))
        .await
        .unwrap();
    let cmd = h::edit(&zero, h::add(h::gap(&doc, 1), "MY_NOTE"));
    let one = store
        .edit(actor, zero.overlay.overlay_id, cmd.clone())
        .await
        .unwrap();
    let before = h::counts(&r, actor).await;
    r.grant(owner, personal, true).await;
    assert!(
        store
            .read(owner, one.view.clone(), ReadingMode::Fused)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .state(owner, one.overlay.overlay_id)
            .await
            .unwrap()
            .is_none()
    );
    r.revoke(actor, source).await;
    for mode in [
        ReadingMode::Original,
        ReadingMode::Fused,
        ReadingMode::Personal,
    ] {
        let p = store
            .read(actor, one.view.clone(), mode)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(p.source, SourceProjection::Unavailable));
        assert!(p.items.is_empty());
        if mode != ReadingMode::Original {
            assert_eq!(p.unplaced.len(), 1);
            assert!(p.unplaced[0].location.is_none());
        }
        let json = serde_json::to_string(&p).unwrap();
        for hidden in [
            doc.reference.composition_id.to_string(),
            doc.reference.revision_id.to_string(),
            doc.nodes[0].occurrence_id.to_string(),
            k.block_id.to_string(),
            "SECRET_SOURCE".into(),
        ] {
            assert!(!json.contains(&hidden), "leaked {hidden}");
        }
    }
    assert!(
        store
            .state(actor, one.overlay.overlay_id)
            .await
            .unwrap()
            .unwrap()
            .editable
            .is_none()
    );
    assert!(matches!(
        store
            .edit(actor, zero.overlay.overlay_id, cmd.clone())
            .await,
        Err(ContentError::NotFound)
    ));
    r.grant(actor, source, false).await;
    assert_eq!(
        store
            .edit(actor, zero.overlay.overlay_id, cmd)
            .await
            .unwrap(),
        one
    );
    assert_eq!(
        h::text(
            &store
                .read(actor, one.view, ReadingMode::Fused)
                .await
                .unwrap()
                .unwrap()
        ),
        ["SECRET_SOURCE", "MY_NOTE"]
    );
    assert_eq!(h::counts(&r, actor).await, before);
}
#[tokio::test]
async fn unreadable_personal_reference_is_omitted_and_prevents_rewrite() {
    let (r, actor, _space, doc, zero) = h::fixture().await;
    let (other, external) = r.seed_actor_space(true).await;
    let b = r
        .store
        .create(other, external, command("PRIVATE_NOTE"))
        .await
        .unwrap();
    r.grant(actor, external, false).await;
    let store = h::store(&r);
    let one = store
        .edit(
            actor,
            zero.overlay.overlay_id,
            h::edit(
                &zero,
                ReadingEdit::InsertExisting {
                    block: BlockRef {
                        block_id: b.block_id,
                        revision_id: b.revision_id,
                    },
                    target: InsertTarget::NewGroup {
                        anchor: h::gap(&doc, 1),
                    },
                },
            ),
        )
        .await
        .unwrap();
    let st = store
        .state(actor, one.overlay.overlay_id)
        .await
        .unwrap()
        .unwrap()
        .editable
        .unwrap();
    let placement = st.groups[0].placements[0].placement_id;
    r.revoke(actor, external).await;
    let p = store
        .read(actor, one.view.clone(), ReadingMode::Fused)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(h::text(&p), ["K", "M"]);
    let json = serde_json::to_string(&p).unwrap();
    for hidden in [
        b.block_id.to_string(),
        b.revision_id.to_string(),
        placement.to_string(),
        "PRIVATE_NOTE".into(),
    ] {
        assert!(!json.contains(&hidden));
    }
    assert!(
        store
            .state(actor, one.overlay.overlay_id)
            .await
            .unwrap()
            .unwrap()
            .editable
            .is_none()
    );
    let before = h::counts(&r, actor).await;
    assert!(matches!(
        store
            .edit(
                actor,
                one.overlay.overlay_id,
                h::edit(&one, h::add(h::gap(&doc, 0), "new"))
            )
            .await,
        Err(ContentError::NotFound)
    ));
    assert_eq!(h::counts(&r, actor).await, before);
}

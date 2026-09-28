mod support;
use learning_core::*;
use support::{assembly as a, reading as h, *};
#[tokio::test]
async fn inserts_between_personal_blocks_and_preserves_old_view() {
    let (r, actor, _s, doc, zero) = h::fixture().await;
    let store = h::store(&r);
    let one = store
        .edit(
            actor,
            zero.overlay.overlay_id,
            h::edit(&zero, h::add(h::gap(&doc, 1), "N")),
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
    let g = &st.groups[0];
    let cmd = h::edit(
        &one,
        ReadingEdit::InsertNew {
            drafts: vec![command("I").draft],
            target: InsertTarget::ExistingGroup {
                group_id: g.group_id,
                order: PlacementOrder {
                    left_placement_id: Some(g.placements[0].placement_id),
                    right_placement_id: None,
                },
            },
        },
    );
    let two = store
        .edit(actor, one.overlay.overlay_id, cmd)
        .await
        .unwrap();
    let st = store
        .state(actor, one.overlay.overlay_id)
        .await
        .unwrap()
        .unwrap()
        .editable
        .unwrap();
    let g = &st.groups[0];
    let three = store
        .edit(
            actor,
            one.overlay.overlay_id,
            h::edit(
                &two,
                ReadingEdit::InsertNew {
                    drafts: vec![command("H").draft],
                    target: InsertTarget::ExistingGroup {
                        group_id: g.group_id,
                        order: PlacementOrder {
                            left_placement_id: Some(g.placements[0].placement_id),
                            right_placement_id: Some(g.placements[1].placement_id),
                        },
                    },
                },
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        h::text(
            &store
                .read(actor, three.view, ReadingMode::Fused)
                .await
                .unwrap()
                .unwrap()
        ),
        ["K", "N", "H", "I", "M"]
    );
    assert_eq!(
        h::text(
            &store
                .read(actor, one.view, ReadingMode::Fused)
                .await
                .unwrap()
                .unwrap()
        ),
        ["K", "N", "M"]
    );
    assert_eq!(
        h::text(
            &store
                .read(actor, zero.view, ReadingMode::Fused)
                .await
                .unwrap()
                .unwrap()
        ),
        ["K", "M"]
    );
}
#[tokio::test]
async fn revisions_upgrade_only_selected_occurrence_and_move_does_not_write_body() {
    let (r, actor, _s, doc, zero) = h::fixture().await;
    let store = h::store(&r);
    let one = store
        .edit(
            actor,
            zero.overlay.overlay_id,
            h::edit(&zero, h::add(h::gap(&doc, 0), "N")),
        )
        .await
        .unwrap();
    let block = one.changed_blocks[0].clone();
    let two = store
        .edit(
            actor,
            one.overlay.overlay_id,
            h::edit(
                &one,
                ReadingEdit::InsertExisting {
                    block: block.clone(),
                    target: InsertTarget::NewGroup {
                        anchor: h::gap(&doc, 2),
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
    let first = st
        .groups
        .iter()
        .find(|g| g.location.anchor().left_occurrence_id.is_none())
        .unwrap()
        .placements[0]
        .placement_id;
    let cmd = h::edit(
        &two,
        ReadingEdit::ReviseSelected {
            changes: vec![TextChange {
                block_id: block.block_id,
                base_revision_id: block.revision_id,
                draft: command("N2").draft,
                selected_placements: vec![first],
            }],
        },
    );
    let three = store
        .edit(actor, one.overlay.overlay_id, cmd.clone())
        .await
        .unwrap();
    assert_eq!(
        h::text(
            &store
                .read(actor, three.view.clone(), ReadingMode::Fused)
                .await
                .unwrap()
                .unwrap()
        ),
        ["N2", "K", "M", "N"]
    );
    let before: r#Vec<i64> = h::counts(&r, actor).await;
    let moved = store
        .edit(
            actor,
            one.overlay.overlay_id,
            h::edit(
                &three,
                ReadingEdit::Move {
                    placement_id: first,
                    target: InsertTarget::NewGroup {
                        anchor: h::gap(&doc, 1),
                    },
                },
            ),
        )
        .await
        .unwrap();
    let after = h::counts(&r, actor).await;
    assert_eq!(&before[..2], &after[..2]);
    assert_eq!(
        h::text(
            &store
                .read(actor, moved.view, ReadingMode::Fused)
                .await
                .unwrap()
                .unwrap()
        ),
        ["K", "N2", "M", "N"]
    );
    assert_eq!(
        store
            .edit(actor, one.overlay.overlay_id, cmd)
            .await
            .unwrap(),
        three
    );
    let newer = r
        .store
        .revise(
            actor,
            block.block_id,
            ReviseCommand {
                request_id: uuid::Uuid::new_v4(),
                base_revision_id: three.changed_blocks[0].revision_id,
                draft: command("N3").draft,
                reason: "standalone".into(),
            },
        )
        .await
        .unwrap();
    assert_ne!(newer.revision_id, block.revision_id);
    assert_eq!(
        h::text(
            &store
                .read(actor, two.view, ReadingMode::Fused)
                .await
                .unwrap()
                .unwrap()
        ),
        ["N", "K", "M", "N"]
    );
}
#[tokio::test]
async fn empty_document_and_repeated_empty_sections_have_distinct_anchors() {
    let r = TestRig::from_env().await;
    let (actor, s) = r.seed_actor_space(true).await;
    let section = r
        .compositions()
        .save(actor, s, a::doc(vec![]))
        .await
        .unwrap();
    let doc = r
        .compositions()
        .save(
            actor,
            s,
            a::doc(vec![a::child(&section), a::child(&section)]),
        )
        .await
        .unwrap();
    let zero = h::store(&r)
        .create(actor, s, h::create(doc.reference.clone()))
        .await
        .unwrap();
    let anchor = GapAnchor {
        base: doc.reference.clone(),
        parent_occurrence_path: vec![doc.nodes[1].occurrence_id],
        left_occurrence_id: None,
        right_occurrence_id: None,
        affinity: Affinity::AfterLeft,
    };
    let one = h::store(&r)
        .edit(
            actor,
            zero.overlay.overlay_id,
            h::edit(&zero, h::add(anchor, "inside second")),
        )
        .await
        .unwrap();
    let p = h::store(&r)
        .read(actor, one.view, ReadingMode::Fused)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(h::text(&p), ["inside second"]);
    assert!(
        matches!(&p.items[2],ReadingItem::SectionStart{path,..} if path==&vec![doc.nodes[1].occurrence_id])
    );
    let empty = h::store(&r)
        .create(actor, s, h::create(section.reference.clone()))
        .await
        .unwrap();
    let saved = h::store(&r)
        .edit(
            actor,
            empty.overlay.overlay_id,
            h::edit(&empty, h::add(h::gap(&section, 0), "empty note")),
        )
        .await
        .unwrap();
    assert_eq!(
        h::text(
            &h::store(&r)
                .read(actor, saved.view, ReadingMode::Fused)
                .await
                .unwrap()
                .unwrap()
        ),
        ["empty note"]
    );
}

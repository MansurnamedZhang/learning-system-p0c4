mod support;
use learning_core::*;
use support::{reading as h, *};
#[tokio::test]
async fn personal_budget_is_exact_deduplicated_and_separate_from_original() {
    let (r, actor, _s, doc, zero) = h::fixture().await;
    let store = h::store(&r);
    let overhead: i32 = sqlx::query_scalar("SELECT octet_length($1::jsonb::text)")
        .bind(sqlx::types::Json(command("").draft))
        .fetch_one(&r.admin_pool)
        .await
        .unwrap();
    let mut remaining = 8 * 1024 * 1024usize;
    let mut drafts = vec![];
    while remaining > 0 {
        let size = (remaining - overhead as usize).min(200000);
        drafts.push(command(&"x".repeat(size)).draft);
        remaining -= size + overhead as usize;
    }
    assert_eq!(drafts.len(), 42);
    let first = store
        .edit(
            actor,
            zero.overlay.overlay_id,
            h::edit(
                &zero,
                ReadingEdit::InsertNew {
                    drafts: drafts[..32].to_vec(),
                    target: InsertTarget::NewGroup {
                        anchor: h::gap(&doc, 1),
                    },
                },
            ),
        )
        .await
        .unwrap();
    let group = store
        .state(actor, first.overlay.overlay_id)
        .await
        .unwrap()
        .unwrap()
        .editable
        .unwrap()
        .groups
        .remove(0);
    let second = store
        .edit(
            actor,
            first.overlay.overlay_id,
            h::edit(
                &first,
                ReadingEdit::InsertNew {
                    drafts: drafts[32..].to_vec(),
                    target: InsertTarget::ExistingGroup {
                        group_id: group.group_id,
                        order: PlacementOrder {
                            left_placement_id: Some(group.placements.last().unwrap().placement_id),
                            right_placement_id: None,
                        },
                    },
                },
            ),
        )
        .await
        .unwrap();
    let projection = store
        .read(actor, second.view.clone(), ReadingMode::Fused)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(projection.items.len(), 44);
    assert_eq!(h::text(&projection)[0], "K");
    let group = store
        .state(actor, second.overlay.overlay_id)
        .await
        .unwrap()
        .unwrap()
        .editable
        .unwrap()
        .groups
        .remove(0);
    let last = group.placements.last().unwrap();
    let third = store
        .edit(
            actor,
            second.overlay.overlay_id,
            h::edit(
                &second,
                ReadingEdit::InsertExisting {
                    block: last.block.clone(),
                    target: InsertTarget::ExistingGroup {
                        group_id: group.group_id,
                        order: PlacementOrder {
                            left_placement_id: Some(last.placement_id),
                            right_placement_id: None,
                        },
                    },
                },
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .read(actor, third.view.clone(), ReadingMode::Personal)
            .await
            .unwrap()
            .unwrap()
            .items
            .len(),
        43
    );
    let mut next = drafts.last().unwrap().clone();
    next.payload.text.push('x');
    let before = h::counts(&r, actor).await;
    let error = store
        .edit(
            actor,
            third.overlay.overlay_id,
            h::edit(
                &third,
                ReadingEdit::ReviseSelected {
                    changes: vec![TextChange {
                        block_id: last.block.block_id,
                        base_revision_id: last.block.revision_id,
                        draft: next,
                        selected_placements: vec![
                            last.placement_id,
                            store
                                .state(actor, third.overlay.overlay_id)
                                .await
                                .unwrap()
                                .unwrap()
                                .editable
                                .unwrap()
                                .groups[0]
                                .placements
                                .last()
                                .unwrap()
                                .placement_id,
                        ],
                    }],
                },
            ),
        )
        .await
        .unwrap_err();
    assert!(matches!(error,ContentError::Invalid(ref s) if s=="personal_body_limit"));
    assert_eq!(h::counts(&r, actor).await, before);
    assert_eq!(
        store
            .state(actor, third.overlay.overlay_id)
            .await
            .unwrap()
            .unwrap()
            .view,
        third.view
    );
}

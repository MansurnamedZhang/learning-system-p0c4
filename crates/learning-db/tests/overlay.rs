mod support;
use learning_core::*;
use support::{reading as h, *};
use uuid::Uuid;
#[tokio::test]
async fn receipt_failure_rolls_back_every_body_and_structure_then_retries() {
    let (r, actor, _space, doc, zero) = h::fixture().await;
    let store = h::store(&r);
    let before = h::counts(&r, actor).await;
    let function = format!("b2_fail_{}", actor.actor_id.simple());
    sqlx::query(&format!("CREATE FUNCTION {function}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.actor_id='{}'::uuid THEN RAISE EXCEPTION 'injected'; END IF; RETURN NEW; END $$",actor.actor_id)).execute(&r.admin_pool).await.unwrap();
    sqlx::query(&format!("CREATE TRIGGER {function} BEFORE INSERT ON reading_receipt FOR EACH ROW EXECUTE FUNCTION {function}()")).execute(&r.admin_pool).await.unwrap();
    let c = h::edit(&zero, h::add(h::gap(&doc, 1), "N"));
    assert!(matches!(
        store.edit(actor, zero.overlay.overlay_id, c.clone()).await,
        Err(ContentError::Storage)
    ));
    assert_eq!(h::counts(&r, actor).await, before);
    assert_eq!(
        store
            .state(actor, zero.overlay.overlay_id)
            .await
            .unwrap()
            .unwrap()
            .view,
        zero.view
    );
    sqlx::query(&format!("DROP TRIGGER {function} ON reading_receipt"))
        .execute(&r.admin_pool)
        .await
        .unwrap();
    sqlx::query(&format!("DROP FUNCTION {function}()"))
        .execute(&r.admin_pool)
        .await
        .unwrap();
    let one = store
        .edit(actor, zero.overlay.overlay_id, c.clone())
        .await
        .unwrap();
    assert_eq!(
        store.edit(actor, zero.overlay.overlay_id, c).await.unwrap(),
        one
    );
}
#[tokio::test]
async fn no_change_batch_failure_and_delete_preserve_bodies() {
    let (r, actor, _space, doc, zero) = h::fixture().await;
    let store = h::store(&r);
    let one = store
        .edit(
            actor,
            zero.overlay.overlay_id,
            h::edit(&zero, h::add(h::gap(&doc, 1), "N")),
        )
        .await
        .unwrap();
    let g = store
        .state(actor, one.overlay.overlay_id)
        .await
        .unwrap()
        .unwrap()
        .editable
        .unwrap()
        .groups
        .remove(0);
    let p = &g.placements[0];
    let before = h::counts(&r, actor).await;
    for edit in [
        ReadingEdit::AdoptExisting {
            block: p.block.clone(),
            selected_placements: vec![p.placement_id],
        },
        ReadingEdit::ReviseSelected {
            changes: vec![TextChange {
                block_id: p.block.block_id,
                base_revision_id: p.block.revision_id,
                draft: command("N").draft,
                selected_placements: vec![p.placement_id],
            }],
        },
    ] {
        assert!(
            matches!(store.edit(actor,one.overlay.overlay_id,h::edit(&one,edit)).await,Err(ContentError::Invalid(ref code)) if code=="no_change")
        );
    }
    let edit = ReadingEdit::ReviseSelected {
        changes: vec![TextChange {
            block_id: p.block.block_id,
            base_revision_id: p.block.revision_id,
            draft: command("N2").draft,
            selected_placements: vec![Uuid::new_v4()],
        }],
    };
    assert!(matches!(
        store
            .edit(actor, one.overlay.overlay_id, h::edit(&one, edit))
            .await,
        Err(ContentError::Invalid(_))
    ));
    assert_eq!(h::counts(&r, actor).await, before);
    let two = store
        .edit(
            actor,
            one.overlay.overlay_id,
            h::edit(
                &one,
                ReadingEdit::Remove {
                    placement_id: p.placement_id,
                },
            ),
        )
        .await
        .unwrap();
    assert_eq!(&h::counts(&r, actor).await[..2], &before[..2]);
    assert_eq!(
        h::text(
            &store
                .read(actor, two.view, ReadingMode::Fused)
                .await
                .unwrap()
                .unwrap()
        ),
        ["K", "M"]
    );
    assert_eq!(
        h::text(
            &store
                .read(actor, one.view, ReadingMode::Personal)
                .await
                .unwrap()
                .unwrap()
        ),
        ["N"]
    );
}

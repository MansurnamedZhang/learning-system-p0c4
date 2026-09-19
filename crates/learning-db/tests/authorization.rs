mod support;
use learning_core::{ContentError, Principal, ReviseCommand};
use support::*;
use uuid::Uuid;

#[tokio::test]
async fn current_grants_control_single_batch_lists_and_history() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let (block, rev) = rig.seed_block(actor, space).await;
    assert_eq!(
        rig.store
            .read(actor, rev)
            .await
            .unwrap()
            .unwrap()
            .draft
            .payload
            .text,
        "保留空格  与换行\n"
    );
    assert_eq!(
        rig.store
            .history(actor, block, None)
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    assert_eq!(
        rig.store
            .list(actor, space, None)
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    rig.revoke(actor, space).await;
    assert!(rig.store.read(actor, rev).await.unwrap().is_none());
    assert!(
        rig.store
            .read(actor, Uuid::new_v4())
            .await
            .unwrap()
            .is_none()
    );
    let batch = rig.store.read_many(actor, &[rev]).await.unwrap();
    assert!(batch.is_empty());
    let history = rig.store.history(actor, block, None).await.unwrap();
    assert!(history.items.is_empty());
    assert!(history.next_cursor.is_none());
    let list = rig.store.list(actor, space, None).await.unwrap();
    assert!(list.items.is_empty());
    assert!(list.next_cursor.is_none());
    assert_eq!(
        list,
        rig.store.list(actor, Uuid::new_v4(), None).await.unwrap()
    );
    assert_eq!(
        history,
        rig.store
            .history(actor, Uuid::new_v4(), None)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn batch_deduplicates_preserves_order_and_omits_hidden_identifiers() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let (_, a) = rig.seed_block(actor, space).await;
    let (_, b) = rig.seed_block(actor, space).await;
    let (stranger, private) = rig.seed_actor_space(true).await;
    let (hidden_block, hidden) = rig.seed_block(stranger, private).await;
    let result = rig
        .store
        .read_many(actor, &[b, hidden, a, b, Uuid::new_v4()])
        .await
        .unwrap();
    assert_eq!(
        result.iter().map(|r| r.revision_id).collect::<Vec<_>>(),
        vec![b, a]
    );
    let serialized = serde_json::to_string(&result).unwrap();
    assert!(!serialized.contains(&hidden.to_string()));
    assert!(!serialized.contains(&hidden_block.to_string()));
    assert!(rig.store.read_many(actor, &[]).await.unwrap().is_empty());
    assert!(matches!(
        rig.store.read_many(actor, &vec![a; 201]).await,
        Err(ContentError::Invalid(_))
    ));
    assert_eq!(
        rig.store
            .read_many(actor, &vec![a; 200])
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn readonly_and_revoked_users_cannot_write_or_replay_receipts() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let cmd = command("私有内容");
    let first = rig.store.create(actor, space, cmd.clone()).await.unwrap();
    let edit = change(&first, "新版");
    let second = rig
        .store
        .revise(actor, first.block_id, edit.clone())
        .await
        .unwrap();
    sqlx::query("UPDATE public.space_grant SET can_write=false WHERE actor_id=$1")
        .bind(actor.actor_id)
        .execute(&rig.admin_pool)
        .await
        .unwrap();
    assert_eq!(
        rig.store.read(actor, first.revision_id).await.unwrap(),
        Some(first.clone())
    );
    for attempt in [cmd.clone(), command("denied")] {
        assert!(matches!(
            rig.store.create(actor, space, attempt).await,
            Err(ContentError::NotFound)
        ));
    }
    assert!(matches!(
        rig.store.revise(actor, first.block_id, edit.clone()).await,
        Err(ContentError::NotFound)
    ));
    rig.revoke(actor, space).await;
    assert!(matches!(
        rig.store.create(actor, space, cmd).await,
        Err(ContentError::NotFound)
    ));
    assert!(matches!(
        rig.store.revise(actor, first.block_id, edit).await,
        Err(ContentError::NotFound)
    ));
    let stranger = Principal {
        actor_id: Uuid::new_v4(),
    };
    for block in [first.block_id, Uuid::new_v4()] {
        let attempt = ReviseCommand {
            request_id: Uuid::new_v4(),
            base_revision_id: second.revision_id,
            draft: command("x").draft,
            reason: "x".into(),
        };
        assert!(matches!(
            rig.store.revise(stranger, block, attempt).await,
            Err(ContentError::NotFound)
        ));
    }
    assert_eq!(rig.counts(actor, space).await, (1, 2, 2));
}

#[tokio::test]
async fn list_cursor_uses_stable_block_order_and_filters_before_limit() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let (other, hidden_space) = rig.seed_actor_space(true).await;
    let (_, hidden) = rig.seed_block(other, hidden_space).await;
    let mut ids = Vec::new();
    for _ in 0..102 {
        ids.push(rig.seed_block(actor, space).await.0);
    }
    sqlx::query("UPDATE public.block SET created_at='2026-01-01T00:00:00Z' WHERE space_id=$1")
        .bind(space)
        .execute(&rig.admin_pool)
        .await
        .unwrap();
    ids.sort();
    ids.reverse();
    let first = rig.store.list(actor, space, None).await.unwrap();
    assert_eq!(
        first.items.iter().map(|r| r.block_id).collect::<Vec<_>>(),
        ids[..100]
    );
    let cursor = first.next_cursor.unwrap();
    assert_eq!(cursor.id, ids[99]);
    // A later head must not change the stable block's pagination key.
    let visible = first.items[0].clone();
    rig.store
        .revise(
            actor,
            visible.block_id,
            change(&visible, "edited between pages"),
        )
        .await
        .unwrap();
    let second = rig
        .store
        .list(actor, space, Some(cursor.clone()))
        .await
        .unwrap();
    assert_eq!(
        second.items.iter().map(|r| r.block_id).collect::<Vec<_>>(),
        ids[100..]
    );
    assert!(second.next_cursor.is_none());
    assert!(
        !serde_json::to_string(&second)
            .unwrap()
            .contains(&hidden.to_string())
    );
    rig.revoke(actor, space).await;
    let after = rig.store.list(actor, space, Some(cursor)).await.unwrap();
    assert!(after.items.is_empty());
    assert!(after.next_cursor.is_none());
}

#[tokio::test]
async fn history_paginates_equal_timestamps_without_duplicates() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let (block, first) = rig.seed_block(actor, space).await;
    let mut ids = vec![first];
    for _ in 0..101 {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO public.block_revision SELECT $1,space_id,block_id,id,content,content_sha256,author_id,reason,created_at FROM public.block_revision WHERE id=$2")
            .bind(id).bind(first).execute(&rig.admin_pool).await.unwrap();
        ids.push(id);
    }
    ids.sort();
    ids.reverse();
    let page = rig.store.history(actor, block, None).await.unwrap();
    assert_eq!(
        page.items.iter().map(|r| r.revision_id).collect::<Vec<_>>(),
        ids[..100]
    );
    let next = rig
        .store
        .history(actor, block, page.next_cursor)
        .await
        .unwrap();
    assert_eq!(
        next.items.iter().map(|r| r.revision_id).collect::<Vec<_>>(),
        ids[100..]
    );
    assert!(next.next_cursor.is_none());
}

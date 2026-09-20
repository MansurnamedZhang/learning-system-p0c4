mod support;
use learning_core::{ContentError, Intent};
use support::*;
use uuid::Uuid;

#[tokio::test]
async fn retries_preserve_ids_audit_and_old_content_even_after_later_edits() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let cmd = command("旧文  \n");
    let first = rig.store.create(actor, space, cmd.clone()).await.unwrap();
    let separate = rig
        .store
        .create(actor, space, command("旧文  \n"))
        .await
        .unwrap();
    assert_ne!(first.block_id, separate.block_id);
    assert_eq!(first.content_sha256, separate.content_sha256);
    assert_eq!(first.author_id, actor.actor_id);
    assert_eq!(first.reason, "initial");
    assert!(first.parent_revision_id.is_none());
    let edit = change(&first, "新版");
    let second = rig
        .store
        .revise(actor, first.block_id, edit.clone())
        .await
        .unwrap();
    assert_eq!(second.parent_revision_id, Some(first.revision_id));
    assert_eq!(second.block_id, first.block_id);
    let third = rig
        .store
        .revise(actor, first.block_id, change(&second, "第三版"))
        .await
        .unwrap();
    assert_eq!(first, rig.store.create(actor, space, cmd).await.unwrap());
    assert_eq!(
        second,
        rig.store.revise(actor, first.block_id, edit).await.unwrap()
    );
    assert_eq!(rig.head(first.block_id).await, third.revision_id);
    assert_eq!(rig.counts(actor, space).await, (2, 4, 4));
    let body: String = sqlx::query_scalar(
        "SELECT content->'payload'->>'text' FROM public.block_revision WHERE id=$1",
    )
    .bind(first.revision_id)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(body, "旧文  \n");
}

#[tokio::test]
async fn request_identity_includes_operation_target_content_base_and_reason() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let (_, other_space) = rig.seed_actor_space(true).await;
    sqlx::query("INSERT INTO public.space_grant VALUES($1,$2,true)")
        .bind(actor.actor_id)
        .bind(other_space)
        .execute(&rig.admin_pool)
        .await
        .unwrap();
    let cmd = command("原文");
    let first = rig.store.create(actor, space, cmd.clone()).await.unwrap();
    let mut altered = cmd.clone();
    altered.reason = "different".into();
    let mut changed = cmd.clone();
    changed.draft.intent = Intent::Idea;
    let mut body = cmd.clone();
    body.draft.payload.text.push('变');
    for attempt in [altered, changed, body] {
        assert!(matches!(
            rig.store.create(actor, space, attempt).await,
            Err(ContentError::IdempotencyConflict)
        ));
    }
    assert!(matches!(
        rig.store.create(actor, other_space, cmd.clone()).await,
        Err(ContentError::IdempotencyConflict)
    ));
    let mut edit = change(&first, "新版");
    edit.request_id = cmd.request_id;
    assert!(matches!(
        rig.store.revise(actor, first.block_id, edit).await,
        Err(ContentError::IdempotencyConflict)
    ));
    let edit = change(&first, "新版");
    let second = rig
        .store
        .revise(actor, first.block_id, edit.clone())
        .await
        .unwrap();
    let mut wrong_base = edit.clone();
    wrong_base.base_revision_id = second.revision_id;
    assert!(matches!(
        rig.store.revise(actor, first.block_id, wrong_base).await,
        Err(ContentError::IdempotencyConflict)
    ));
    let other = rig
        .store
        .create(actor, space, command("另一块"))
        .await
        .unwrap();
    assert!(matches!(
        rig.store.revise(actor, other.block_id, edit).await,
        Err(ContentError::IdempotencyConflict)
    ));
    let (other_actor, own_space) = rig.seed_actor_space(true).await;
    assert!(rig.store.create(other_actor, own_space, cmd).await.is_ok());
    assert_eq!(rig.counts(actor, space).await, (2, 3, 3));
}

#[tokio::test]
async fn stale_or_foreign_base_does_not_overwrite_the_current_head() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let first = rig
        .store
        .create(actor, space, command("旧版"))
        .await
        .unwrap();
    let second = rig
        .store
        .revise(actor, first.block_id, change(&first, "新版"))
        .await
        .unwrap();
    for base in [first.revision_id, Uuid::new_v4()] {
        let mut cmd = change(&first, "冲突");
        cmd.base_revision_id = base;
        assert!(
            matches!(rig.store.revise(actor,first.block_id,cmd).await,Err(ContentError::Conflict { current_revision_id }) if current_revision_id==second.revision_id)
        );
    }
    assert_eq!(rig.head(first.block_id).await, second.revision_id);
    assert_eq!(rig.counts(actor, space).await, (1, 2, 2));
}

#[tokio::test]
async fn invalid_internal_commands_write_nothing() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    for reason in [String::new(), "字".repeat(1001), "bad\0reason".into()] {
        let mut cmd = command("text");
        cmd.reason = reason;
        assert!(matches!(
            rig.store.create(actor, space, cmd).await,
            Err(ContentError::Invalid(_))
        ));
    }
    let mut cmd = command("text");
    cmd.draft.language = "中文".into();
    assert!(matches!(
        rig.store.create(actor, space, cmd).await,
        Err(ContentError::Invalid(_))
    ));
    let (block, revision) = rig.seed_block(actor, space).await;
    let invalid = learning_core::ReviseCommand {
        request_id: Uuid::new_v4(),
        base_revision_id: revision,
        draft: command("x").draft,
        reason: String::new(),
    };
    assert!(matches!(
        rig.store.revise(actor, block, invalid).await,
        Err(ContentError::Invalid(_))
    ));
    assert_eq!(rig.counts(actor, space).await, (1, 1, 0));
}

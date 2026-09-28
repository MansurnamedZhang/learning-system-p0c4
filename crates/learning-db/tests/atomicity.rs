mod support;
use learning_core::{ContentError, Principal};
use support::*;

async fn install(rig: &TestRig, actor: Principal) {
    let suffix = actor.actor_id.simple();
    sqlx::raw_sql(&format!("CREATE FUNCTION public.fail_receipt_{suffix}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected receipt failure'; END; $$; CREATE TRIGGER fail_{suffix} BEFORE INSERT ON public.mutation_receipt FOR EACH ROW WHEN (NEW.actor_id='{}'::uuid) EXECUTE FUNCTION public.fail_receipt_{suffix}()",actor.actor_id)).execute(&rig.admin_pool).await.unwrap();
}
async fn remove(rig: &TestRig, actor: Principal) {
    let suffix = actor.actor_id.simple();
    sqlx::raw_sql(&format!("DROP TRIGGER fail_{suffix} ON public.mutation_receipt; DROP FUNCTION public.fail_receipt_{suffix}()" )).execute(&rig.admin_pool).await.unwrap();
}
#[tokio::test]
async fn receipt_failure_rolls_back_revision_and_head_and_same_request_can_retry() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let first = rig
        .store
        .create(actor, space, command("original"))
        .await
        .unwrap();
    let cmd = change(&first, "must roll back");
    install(&rig, actor).await;
    let result = rig.store.revise(actor, first.block_id, cmd.clone()).await;
    remove(&rig, actor).await;
    assert!(matches!(result, Err(ContentError::Storage)));
    assert_eq!(rig.head(first.block_id).await, first.revision_id);
    assert_eq!(rig.counts(actor, space).await, (1, 1, 1));
    let restored = rig
        .store
        .revise(actor, first.block_id, cmd.clone())
        .await
        .unwrap();
    assert_eq!(
        restored,
        rig.store.revise(actor, first.block_id, cmd).await.unwrap()
    );
    assert_eq!(rig.counts(actor, space).await, (1, 2, 2));
}
#[tokio::test]
async fn create_receipt_failure_leaves_no_orphan_block_and_can_retry() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let cmd = command("new content");
    install(&rig, actor).await;
    let result = rig.store.create(actor, space, cmd.clone()).await;
    remove(&rig, actor).await;
    assert!(matches!(result, Err(ContentError::Storage)));
    assert_eq!(rig.counts(actor, space).await, (0, 0, 0));
    let restored = rig.store.create(actor, space, cmd.clone()).await.unwrap();
    assert_eq!(restored, rig.store.create(actor, space, cmd).await.unwrap());
    assert_eq!(rig.counts(actor, space).await, (1, 1, 1));
}

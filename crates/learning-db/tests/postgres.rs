use learning_core::{ContentError, CreateCommand, Principal, ReviseCommand, TextDraft};
use learning_db::{ContentStore, MIGRATOR};
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

async fn fixture() -> (PgPool, ContentStore, Principal, Uuid) {
    let admin = PgPoolOptions::new().max_connections(3).connect(&std::env::var("TEST_ADMIN_DATABASE_URL").expect("explicit isolated test admin DSN required")).await.unwrap();
    MIGRATOR.run(&admin).await.unwrap();
    let pool = PgPoolOptions::new().max_connections(5).connect(&std::env::var("TEST_DATABASE_URL").expect("non-owner test runtime DSN required")).await.unwrap();
    let actor = Principal { id: Uuid::new_v4() }; let space = Uuid::new_v4();
    sqlx::query("INSERT INTO app_user VALUES ($1)").bind(actor.id).execute(&admin).await.unwrap();
    sqlx::query("INSERT INTO space VALUES ($1,$2)").bind(space).bind(actor.id).execute(&admin).await.unwrap();
    sqlx::query("INSERT INTO space_grant VALUES ($1,$2,true)").bind(actor.id).bind(space).execute(&admin).await.unwrap();
    (admin, ContentStore::new(pool), actor, space)
}

fn command(text: &str) -> CreateCommand {
    CreateCommand { request_id: Uuid::new_v4(), reason: "fixture".into(), draft: serde_json::from_value::<TextDraft>(serde_json::json!({"kind":"text","intent":"note","language":"zh-CN","title":"私有标题","payload":{"format":"markdown","text":text}})).unwrap() }
}

#[tokio::test]
async fn retry_and_revision_preserve_original_content() {
    let (_, store, actor, space) = fixture().await;
    let cmd = command("旧文"); let first = store.create(actor, space, cmd.clone()).await.unwrap();
    assert_eq!(first, store.create(actor, space, cmd).await.unwrap());
    let change = ReviseCommand { request_id: Uuid::new_v4(), base_revision_id: first.revision_id, draft: command("新版").draft, reason: "修订".into() };
    let second = store.revise(actor, first.block_id, change.clone()).await.unwrap();
    assert_eq!(second, store.revise(actor, first.block_id, change).await.unwrap());
    assert_eq!(store.read(actor, first.revision_id).await.unwrap().unwrap().draft.payload.text, "旧文");
    assert_eq!(second.parent_revision_id, Some(first.revision_id));
    assert_eq!(store.history(actor, first.block_id).await.unwrap().len(), 2);
}

#[tokio::test]
async fn changed_request_or_stale_base_cannot_overwrite() {
    let (_, store, actor, space) = fixture().await;
    let cmd = command("原文"); let first = store.create(actor, space, cmd.clone()).await.unwrap();
    let mut changed = cmd; changed.draft.payload.text = "不同请求".into();
    assert!(matches!(store.create(actor, space, changed).await, Err(ContentError::IdempotencyConflict)));
    let change = ReviseCommand { request_id: Uuid::new_v4(), base_revision_id: first.revision_id, draft: command("新版").draft, reason: "修订".into() };
    let second = store.revise(actor, first.block_id, change.clone()).await.unwrap();
    let stale = ReviseCommand { request_id: Uuid::new_v4(), ..change };
    assert!(matches!(store.revise(actor, first.block_id, stale).await, Err(ContentError::Conflict { current_revision_id }) if current_revision_id == second.revision_id));
}

#[tokio::test]
async fn revoked_grant_blocks_bodies_lists_history_and_retry() {
    let (admin, store, actor, space) = fixture().await;
    let cmd = command("不可泄露"); let first = store.create(actor, space, cmd.clone()).await.unwrap();
    sqlx::query("DELETE FROM space_grant WHERE actor_id=$1").bind(actor.id).execute(&admin).await.unwrap();
    assert!(store.read(actor, first.revision_id).await.unwrap().is_none());
    assert!(store.read_many(actor, &[first.revision_id]).await.unwrap().is_empty());
    assert!(store.list(actor, space).await.unwrap().is_empty());
    assert!(store.history(actor, first.block_id).await.unwrap().is_empty());
    assert!(matches!(store.create(actor, space, cmd).await, Err(ContentError::NotFound)));
}

#[tokio::test]
async fn batch_omits_hidden_ids_and_preserves_visible_order() {
    let (_, store, actor, space) = fixture().await;
    let first = store.create(actor, space, command("甲")).await.unwrap();
    let second = store.create(actor, space, command("乙")).await.unwrap();
    let (_, other_store, other, other_space) = fixture().await;
    let hidden = other_store.create(other, other_space, command("隐藏")).await.unwrap();
    let results = store.read_many(actor, &[second.revision_id, hidden.revision_id, first.revision_id, second.revision_id, Uuid::new_v4()]).await.unwrap();
    assert_eq!(results.iter().map(|r| r.revision_id).collect::<Vec<_>>(), vec![second.revision_id, first.revision_id]);
    let body = serde_json::to_string(&results).unwrap();
    assert!(!body.contains(&hidden.revision_id.to_string())); assert!(!body.contains("隐藏"));
}

#[tokio::test]
async fn two_connections_get_one_commit_and_one_conflict() {
    let (_, store, actor, space) = fixture().await;
    let first = store.create(actor, space, command("基础")).await.unwrap();
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let run = |text: &'static str| {
        let s = store.clone(); let b = barrier.clone(); let f = first.clone();
        tokio::spawn(async move { b.wait().await; s.revise(actor, f.block_id, ReviseCommand { request_id: Uuid::new_v4(), base_revision_id: f.revision_id, draft: command(text).draft, reason: "竞争".into() }).await })
    };
    let (a, b) = tokio::join!(run("甲"), run("乙"));
    let results = [a.unwrap(), b.unwrap()];
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|r| matches!(r, Err(ContentError::Conflict { .. }))).count(), 1);
    assert_eq!(store.history(actor, first.block_id).await.unwrap().len(), 2);
}

#[tokio::test]
async fn concurrent_duplicate_request_creates_one_revision() {
    let (_, store, actor, space) = fixture().await;
    let cmd = command("只写一次");
    let (a,b) = tokio::join!(store.create(actor, space, cmd.clone()), store.create(actor, space, cmd));
    assert_eq!(a.unwrap(), b.unwrap());
    assert_eq!(store.list(actor, space).await.unwrap().len(), 1);
}

#[tokio::test]
async fn runtime_cannot_modify_historical_rows_or_cross_block_parents() {
    let (admin, store, actor, space) = fixture().await;
    let flags: (bool,bool) = sqlx::query_as("SELECT rolsuper,rolbypassrls FROM pg_roles WHERE rolname=current_user").fetch_one(&store.pool).await.unwrap();
    assert_eq!(flags, (false,false));
    let owner: bool = sqlx::query_scalar("SELECT tableowner=current_user FROM pg_tables WHERE tablename='block_revision' AND schemaname='public'").fetch_one(&store.pool).await.unwrap();
    assert!(!owner);
    let first = store.create(actor, space, command("历史")).await.unwrap();
    let other = store.create(actor, space, command("另一块")).await.unwrap();
    assert!(sqlx::query("UPDATE block_revision SET reason='changed' WHERE id=$1").bind(first.revision_id).execute(&store.pool).await.is_err());
    assert!(sqlx::query("DELETE FROM block_revision WHERE id=$1").bind(first.revision_id).execute(&store.pool).await.is_err());
    assert!(sqlx::query("INSERT INTO block_revision SELECT $1,space_id,block_id,$2,content,content_sha256,author_id,reason,created_at FROM block_revision WHERE id=$3")
        .bind(Uuid::new_v4()).bind(other.revision_id).bind(first.revision_id).execute(&admin).await.is_err());
}

#[tokio::test]
async fn receipt_failure_rolls_back_content_and_working_head() {
    let (admin, store, actor, space) = fixture().await;
    let first = store.create(actor, space, command("原文")).await.unwrap();
    let function = format!("CREATE FUNCTION fail_receipt_{}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected failure'; END; $$", actor.id.simple());
    sqlx::raw_sql(&function).execute(&admin).await.unwrap();
    let trigger = format!("CREATE TRIGGER fail_{} BEFORE INSERT ON mutation_receipt FOR EACH ROW WHEN (NEW.actor_id='{}'::uuid) EXECUTE FUNCTION fail_receipt_{}()", actor.id.simple(), actor.id, actor.id.simple());
    sqlx::raw_sql(&trigger).execute(&admin).await.unwrap();
    let change = ReviseCommand { request_id: Uuid::new_v4(), base_revision_id: first.revision_id, draft: command("必须回滚").draft, reason: "故障".into() };
    let result = store.revise(actor, first.block_id, change).await;
    sqlx::raw_sql(&format!("DROP TRIGGER fail_{} ON mutation_receipt; DROP FUNCTION fail_receipt_{}()", actor.id.simple(), actor.id.simple())).execute(&admin).await.unwrap();
    assert!(result.is_err());
    assert_eq!(store.history(actor, first.block_id).await.unwrap().len(), 1);
    assert_eq!(store.list(actor, space).await.unwrap()[0].revision_id, first.revision_id);
}

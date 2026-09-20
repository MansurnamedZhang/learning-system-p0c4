mod support;
use support::*;
use uuid::Uuid;

#[tokio::test]
async fn runtime_cannot_mutate_history_receipts_or_permissions() {
    let rig = TestRig::from_env().await;
    let flags: (bool,bool,bool,bool) = sqlx::query_as("SELECT rolsuper,rolbypassrls,rolcreaterole,rolcreatedb FROM pg_roles WHERE rolname=current_user").fetch_one(&rig.runtime_pool).await.unwrap();
    assert_eq!(flags, (false, false, false, false));
    let owned: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_tables WHERE schemaname='public' AND tableowner=current_user",
    )
    .fetch_one(&rig.runtime_pool)
    .await
    .unwrap();
    assert_eq!(owned, 0);
    for statement in [
        "UPDATE public.block_revision SET reason='changed'",
        "DELETE FROM public.block_revision",
        "TRUNCATE public.block_revision CASCADE",
        "UPDATE public.mutation_receipt SET request_sha256='changed'",
        "DELETE FROM public.mutation_receipt",
        "TRUNCATE public.mutation_receipt",
        "UPDATE public.space_grant SET can_write=true",
        "INSERT INTO public.app_user VALUES (gen_random_uuid())",
        "CREATE TABLE public.should_be_denied(id int)",
    ] {
        let error = sqlx::raw_sql(statement)
            .execute(&rig.runtime_pool)
            .await
            .unwrap_err();
        assert_eq!(sqlstate(&error).as_deref(), Some("42501"), "{statement}");
    }
}

#[tokio::test]
async fn heads_are_mandatory_and_must_belong_to_their_block() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let result = sqlx::query("INSERT INTO public.block(id,space_id) VALUES ($1,$2)")
        .bind(Uuid::new_v4())
        .bind(space)
        .execute(&rig.admin_pool)
        .await;
    assert!(
        result.is_err(),
        "a committed block must not have a null head"
    );
    let (first, _) = rig.seed_block(actor, space).await;
    let (_, other) = rig.seed_block(actor, space).await;
    let error = sqlx::query("UPDATE public.block SET head_revision_id=$1 WHERE id=$2")
        .bind(other)
        .bind(first)
        .execute(&rig.runtime_pool)
        .await
        .unwrap_err();
    assert_eq!(sqlstate(&error).as_deref(), Some("23503"));
}

#[tokio::test]
async fn parents_cannot_reference_self_another_block_or_another_space() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let (_, first) = rig.seed_block(actor, space).await;
    let (_, other) = rig.seed_block(actor, space).await;
    let (stranger, hidden_space) = rig.seed_actor_space(true).await;
    let (_, hidden) = rig.seed_block(stranger, hidden_space).await;
    let new_id = Uuid::new_v4();
    for (parent, want) in [(new_id, "23514"), (other, "23503"), (hidden, "23503")] {
        let error = sqlx::query("INSERT INTO public.block_revision SELECT $1,space_id,block_id,$2,content,content_sha256,author_id,reason,created_at FROM public.block_revision WHERE id=$3")
            .bind(new_id).bind(parent).bind(first).execute(&rig.admin_pool).await.unwrap_err();
        assert_eq!(sqlstate(&error).as_deref(), Some(want));
    }
}

#[tokio::test]
async fn restricted_grant_lock_cannot_be_replaced_or_shadowed() {
    let rig = TestRig::from_env().await;
    let exists: bool = sqlx::query_scalar(
        "SELECT to_regprocedure('public.lock_space_grant(uuid,uuid)') IS NOT NULL",
    )
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert!(
        exists,
        "migration must install a restricted grant lock function"
    );
    let (actor, space) = rig.seed_actor_space(false).await;
    let mut tx = rig.runtime_pool.begin().await.unwrap();
    sqlx::raw_sql("CREATE TEMP TABLE space_grant(actor_id uuid,space_id uuid,can_write bool); INSERT INTO space_grant VALUES (gen_random_uuid(),gen_random_uuid(),true)").execute(&mut *tx).await.unwrap();
    let permission: Option<bool> = sqlx::query_scalar("SELECT public.lock_space_grant($1,$2)")
        .bind(actor.actor_id)
        .bind(space)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(permission, Some(false));
    let missing: Option<bool> = sqlx::query_scalar("SELECT public.lock_space_grant($1,$2)")
        .bind(Uuid::new_v4())
        .bind(space)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(missing, None);
    tx.rollback().await.unwrap();
    let error = sqlx::raw_sql("CREATE OR REPLACE FUNCTION public.lock_space_grant(uuid,uuid) RETURNS boolean LANGUAGE sql AS 'SELECT true'").execute(&rig.runtime_pool).await.unwrap_err();
    assert_eq!(sqlstate(&error).as_deref(), Some("42501"));
    let flags: (bool,bool,bool) = sqlx::query_as("SELECT p.prosecdef,r.rolcanlogin,pg_has_role(current_user,r.oid,'MEMBER') FROM pg_proc p JOIN pg_roles r ON r.oid=p.proowner WHERE p.oid='public.lock_space_grant(uuid,uuid)'::regprocedure").fetch_one(&rig.runtime_pool).await.unwrap();
    assert_eq!(flags, (true, false, false));
    let public_execute: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_proc p,LATERAL aclexplode(p.proacl) a WHERE p.oid='public.lock_space_grant(uuid,uuid)'::regprocedure AND a.grantee=0 AND a.privilege_type='EXECUTE')").fetch_one(&rig.admin_pool).await.unwrap();
    assert!(!public_execute);
}

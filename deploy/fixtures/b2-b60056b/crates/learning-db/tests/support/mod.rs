#![allow(dead_code)]
pub mod assembly;
pub mod reading;
use learning_core::{CreateCommand, Principal, ReviseCommand, Revision};
use learning_db::{ContentStore, MIGRATOR};
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

pub struct TestRig {
    pub admin_pool: PgPool,
    pub runtime_pool: PgPool,
    pub store: ContentStore,
}
impl TestRig {
    pub async fn from_env() -> Self {
        let admin_url = std::env::var("TEST_ADMIN_DATABASE_URL")
            .expect("explicit isolated admin DSN required; tests do not skip");
        let runtime_url = std::env::var("TEST_DATABASE_URL")
            .expect("explicit non-owner runtime DSN required; tests do not skip");
        let admin_pool = PgPoolOptions::new()
            .max_connections(4)
            .connect(&admin_url)
            .await
            .expect("admin connection failed");
        MIGRATOR.run(&admin_pool).await.expect("migration failed");
        let runtime_pool = PgPoolOptions::new()
            .max_connections(6)
            .connect(&runtime_url)
            .await
            .expect("runtime connection failed");
        let store = ContentStore::new(runtime_pool.clone());
        Self {
            admin_pool,
            runtime_pool,
            store,
        }
    }
    pub async fn seed_actor_space(&self, can_write: bool) -> (Principal, Uuid) {
        let actor = Principal {
            actor_id: Uuid::new_v4(),
        };
        let space = Uuid::new_v4();
        sqlx::query("INSERT INTO public.app_user VALUES ($1)")
            .bind(actor.actor_id)
            .execute(&self.admin_pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO public.space VALUES ($1,$2)")
            .bind(space)
            .bind(actor.actor_id)
            .execute(&self.admin_pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO public.space_grant VALUES ($1,$2,$3)")
            .bind(actor.actor_id)
            .bind(space)
            .bind(can_write)
            .execute(&self.admin_pool)
            .await
            .unwrap();
        (actor, space)
    }
    pub async fn revoke(&self, actor: Principal, space: Uuid) {
        sqlx::query("DELETE FROM public.space_grant WHERE actor_id=$1 AND space_id=$2")
            .bind(actor.actor_id)
            .bind(space)
            .execute(&self.admin_pool)
            .await
            .unwrap();
    }
    /// Literal fixture independent of the service implementation and content hash code.
    pub async fn seed_block(&self, actor: Principal, space: Uuid) -> (Uuid, Uuid) {
        let block = Uuid::new_v4();
        let revision = Uuid::new_v4();
        let mut tx = self.admin_pool.begin().await.unwrap();
        sqlx::query("INSERT INTO public.block(id,space_id,head_revision_id) VALUES ($1,$2,$3)")
            .bind(block)
            .bind(space)
            .bind(revision)
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("INSERT INTO public.block_revision(id,space_id,block_id,content,content_sha256,author_id,reason) VALUES ($1,$2,$3,$4,$5,$6,'fixture')")
            .bind(revision).bind(space).bind(block).bind(serde_json::json!({"kind":"text","intent":"note","language":"zh-CN","title":"注记","payload":{"format":"markdown","text":"保留空格  与换行\n"}}))
            .bind("35db28db30d4e6d1b4f81c32a365ba9d2b34757bbb6694e09ab308baab1037ec").bind(actor.actor_id).execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        (block, revision)
    }
    pub async fn counts(&self, actor: Principal, space: Uuid) -> (i64, i64, i64) {
        sqlx::query_as("SELECT (SELECT count(*) FROM public.block WHERE space_id=$1),(SELECT count(*) FROM public.block_revision WHERE space_id=$1),(SELECT count(*) FROM public.mutation_receipt WHERE actor_id=$2)")
            .bind(space).bind(actor.actor_id).fetch_one(&self.admin_pool).await.unwrap()
    }
    pub async fn head(&self, block: Uuid) -> Uuid {
        sqlx::query_scalar("SELECT head_revision_id FROM public.block WHERE id=$1")
            .bind(block)
            .fetch_one(&self.admin_pool)
            .await
            .unwrap()
    }
}
pub fn command(text: &str) -> CreateCommand {
    CreateCommand { request_id: Uuid::new_v4(), reason: "initial".into(), draft: serde_json::from_value(serde_json::json!({"kind":"text","intent":"note","language":"zh-CN","title":"私有标题","payload":{"format":"markdown","text":text}})).unwrap() }
}
pub fn change(first: &Revision, text: &str) -> ReviseCommand {
    ReviseCommand {
        request_id: Uuid::new_v4(),
        base_revision_id: first.revision_id,
        draft: command(text).draft,
        reason: "revision".into(),
    }
}
pub fn sqlstate(error: &sqlx::Error) -> Option<String> {
    error
        .as_database_error()
        .and_then(|e| e.code())
        .map(|s| s.into_owned())
}

use crate::{DOCUMENT_TITLE, Fixture, Result, SeedConfig};
use learning_core::{
    Affinity, BlockKind, BlockRef, CompositionKind, CreateCommand, CreateReading, EditReading,
    GapAnchor, InsertTarget, Intent, NodeDraft, NodeTarget, Principal, ReadingEdit, Revision,
    SaveComposition, TextDraft, TextFormat, TextPayload,
};
use learning_db::{CompositionStore, ContentStore, MIGRATOR, ReadingStore};
use uuid::Uuid;

fn draft(title: &str, text: &str, intent: Intent) -> TextDraft {
    TextDraft {
        kind: BlockKind::Text,
        intent,
        language: "zh-CN".into(),
        title: title.into(),
        payload: TextPayload {
            format: TextFormat::Markdown,
            text: text.into(),
        },
    }
}

fn occurrence(revision: &Revision) -> NodeDraft {
    NodeDraft {
        occurrence_id: None,
        target: NodeTarget::Block(BlockRef {
            block_id: revision.block_id,
            revision_id: revision.revision_id,
        }),
    }
}

pub(crate) async fn seed(config: SeedConfig) -> Result<Fixture> {
    let admin = config
        .db
        .connect("learning_admin", &config.admin_password)
        .await?;
    let empty: bool = sqlx::query_scalar(
        "SELECT to_regclass('public._sqlx_migrations') IS NULL
            AND to_regclass('public.app_user') IS NULL",
    )
    .fetch_one(&admin)
    .await?;
    if !empty {
        return Err("preview seeding requires a fresh synthetic database".into());
    }
    MIGRATOR.run(&admin).await?;
    let runtime = config
        .db
        .connect("learning_runtime", &config.runtime_password)
        .await?;
    let source_author = Principal {
        actor_id: Uuid::new_v4(),
    };
    let owner = Principal {
        actor_id: Uuid::new_v4(),
    };
    let visitor = Uuid::new_v4();
    let source_space = Uuid::new_v4();
    let personal_space = Uuid::new_v4();
    let mut tx = admin.begin().await?;
    for actor in [source_author.actor_id, owner.actor_id, visitor] {
        sqlx::query("INSERT INTO public.app_user (id) VALUES ($1)")
            .bind(actor)
            .execute(&mut *tx)
            .await?;
    }
    for (space, actor) in [
        (source_space, source_author.actor_id),
        (personal_space, owner.actor_id),
    ] {
        sqlx::query("INSERT INTO public.space (id, owner_id) VALUES ($1, $2)")
            .bind(space)
            .bind(actor)
            .execute(&mut *tx)
            .await?;
    }
    for (actor, space, write) in [
        (source_author.actor_id, source_space, true),
        (owner.actor_id, personal_space, true),
        (owner.actor_id, source_space, false),
        (visitor, source_space, false),
    ] {
        sqlx::query(
            "INSERT INTO public.space_grant (actor_id, space_id, can_write) VALUES ($1, $2, $3)",
        )
        .bind(actor)
        .bind(space)
        .bind(write)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    let blocks = ContentStore::new(runtime.clone());
    let mut originals = Vec::new();
    for (title, text) in [
        (
            "查询、键与值",
            "用 Q 与 K 的点积衡量相关性。对分数做归一化后，用权重对 V 求和。",
        ),
        (
            "缩放与掩码",
            "把点积分数除以 √dₖ，再将不允许关注的位置在 softmax 前掩掉。\n\nAttention(Q, K, V) = softmax(QKᵀ / √dₖ + M)V",
        ),
        (
            "动手检验",
            "做一个三 token 的例子。检查每行权重之和，并验证因果掩码下未来 token 的权重为零。",
        ),
    ] {
        originals.push(
            blocks
                .create(
                    source_author,
                    source_space,
                    CreateCommand {
                        request_id: Uuid::new_v4(),
                        draft: draft(title, text, Intent::Knowledge),
                        reason: "synthetic preview".into(),
                    },
                )
                .await?,
        );
    }
    let document = CompositionStore::new(runtime.clone())
        .save(
            source_author,
            source_space,
            SaveComposition {
                request_id: Uuid::new_v4(),
                composition_id: None,
                base_revision_id: None,
                kind: CompositionKind::Document,
                title: DOCUMENT_TITLE.into(),
                nodes: [0, 1, 0, 2]
                    .into_iter()
                    .map(|index| occurrence(&originals[index]))
                    .collect(),
                reason: "repeat the same revision at two occurrences".into(),
            },
        )
        .await?;
    let reading = ReadingStore::new(runtime.clone());
    let first = reading
        .create(
            owner,
            personal_space,
            CreateReading {
                request_id: Uuid::new_v4(),
                base: document.reference.clone(),
                title: DOCUMENT_TITLE.into(),
                reason: "synthetic reading".into(),
            },
        )
        .await?;
    let anchor = GapAnchor {
        base: document.reference,
        parent_occurrence_path: vec![],
        left_occurrence_id: Some(document.nodes[1].occurrence_id),
        right_occurrence_id: Some(document.nodes[2].occurrence_id),
        affinity: Affinity::AfterLeft,
    };
    let saved = reading
        .edit(
            owner,
            first.overlay.overlay_id,
            EditReading {
                request_id: Uuid::new_v4(),
                expected_overlay_revision: first.overlay.revision_id,
                expected_reading_view_revision: first.view.revision_id,
                edit: ReadingEdit::InsertNew {
                    drafts: vec![draft(
                        "我的核对",
                        "先检查掩码再看输出，避免把未来信息带入当前 token。\n\n实验字面量 <strong>原样显示</strong>",
                        Intent::Note,
                    )],
                    target: InsertTarget::NewGroup { anchor },
                },
                reason: "synthetic private overlay".into(),
            },
        )
        .await?;
    sqlx::query("GRANT SELECT ON ALL TABLES IN SCHEMA public TO kw_preview_reader")
        .execute(&admin)
        .await?;
    runtime.close().await;
    admin.close().await;
    Ok(Fixture {
        owner: owner.actor_id,
        visitor,
        source_space,
        personal_space,
        reading: saved.view,
    })
}

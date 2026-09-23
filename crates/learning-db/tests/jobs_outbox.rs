mod support;

use learning_assets::{FsAssetStore, UploadDeclaration};
use learning_core::*;
use learning_db::{AssetMedia, AssetStore, LineageStore, ResourceInput, VersionedContentStore};
use serde_json::{Value, json};
use sqlx::Row;
use std::{fs, path::PathBuf};
use support::TestRig;
use uuid::Uuid;

const PNG: &[u8] = &[137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 73, 69, 78, 68];
const NOTEBOOK: &[u8] = br#"{"cells":[{"cell_type":"code","source":["print(42)"]}],"nbformat":4}"#;

struct Originals {
    root: PathBuf,
    files: FsAssetStore,
}

impl Originals {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("p0c2-event-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let files = FsAssetStore::new(root.join("assets"), root.join("staging")).unwrap();
        Self { root, files }
    }

    async fn register(
        &self,
        rig: &TestRig,
        actor: Principal,
        space: Uuid,
        name: &str,
        media_type: &str,
        bytes: &[u8],
    ) -> AssetRef {
        let source = self.root.join(name);
        fs::write(&source, bytes).unwrap();
        let blob = self
            .files
            .put_from_file(
                Uuid::new_v4(),
                &source,
                UploadDeclaration {
                    expected_size_bytes: bytes.len() as u64,
                    max_size_bytes: 1_000_000,
                },
            )
            .unwrap();
        AssetStore::new(rig.runtime_pool.clone(), self.files.clone())
            .register_verified(
                actor,
                space,
                Uuid::new_v4(),
                blob,
                AssetMedia {
                    media_type: media_type.into(),
                    original_file_name: name.into(),
                },
            )
            .await
            .unwrap()
            .reference
    }
}

impl Drop for Originals {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

fn draft(body: BodyV3) -> ContentDraft {
    ContentDraft::V3(ContentV3 {
        intent: Intent::Note,
        language: "en".into(),
        title: "C2 original".into(),
        body,
        basis_refs: vec![],
        requires_context: vec![],
        source_run: None,
    })
}

fn figure(asset: AssetRef) -> ContentDraft {
    draft(BodyV3::Figure {
        asset,
        usage: "lecture_diagram".into(),
        caption: "Attention".into(),
        alt: "Query and key arrows".into(),
        decorative: false,
    })
}

fn attachment(asset: AssetRef) -> ContentDraft {
    draft(BodyV3::Attachment {
        asset,
        display_name: "Analysis notebook".into(),
    })
}

fn create(draft: ContentDraft) -> CreateContent {
    CreateContent {
        request_id: Uuid::new_v4(),
        draft,
        reason: "C2 event".into(),
    }
}

async fn event_count(rig: &TestRig, actor: Principal) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM public.job_outbox WHERE actor_id=$1")
        .bind(actor.actor_id)
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap()
}

async fn assert_exact_event(
    rig: &TestRig,
    actor: Principal,
    space: Uuid,
    block: BlockRef,
    asset: Uuid,
) {
    let key = format!(
        "asset-integrity:v1:{space}:{}:{}",
        block.block_id, block.revision_id
    );
    let row = sqlx::query(
        "SELECT e.event_type,e.payload_version,e.payload,e.actor_id,e.processor_version,\
                e.dispatched_at,u.asset_id \
           FROM public.job_outbox e JOIN public.block_asset_use u \
             ON u.space_id=(e.payload->>'space_id')::uuid \
            AND u.block_id=(e.payload->>'block_id')::uuid \
            AND u.revision_id=(e.payload->>'revision_id')::uuid \
          WHERE e.business_key=$1",
    )
    .bind(&key)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(
        row.get::<String, _>("event_type"),
        "asset_integrity_requested"
    );
    assert_eq!(row.get::<i32, _>("payload_version"), 1);
    assert_eq!(row.get::<i32, _>("processor_version"), 1);
    assert_eq!(row.get::<Uuid, _>("actor_id"), actor.actor_id);
    assert_eq!(row.get::<Uuid, _>("asset_id"), asset);
    assert!(
        row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("dispatched_at")
            .is_none()
    );
    assert_eq!(
        row.get::<Value, _>("payload"),
        json!({"version":1,"kind":"asset_integrity","actor_id":actor.actor_id,
               "space_id":space,"block_id":block.block_id,"revision_id":block.revision_id})
    );
}

#[tokio::test]
async fn figure_attachment_and_revision_emit_only_precise_new_events() {
    let rig = TestRig::from_env().await;
    let files = Originals::new();
    let (actor, space) = rig.seed_actor_space(true).await;
    let png = files
        .register(&rig, actor, space, "figure.png", "image/png", PNG)
        .await;
    let notebook = files
        .register(
            &rig,
            actor,
            space,
            "analysis.ipynb",
            "application/x-ipynb+json",
            NOTEBOOK,
        )
        .await;
    assert_eq!(event_count(&rig, actor).await, 0, "registration is not use");
    let old_release_count: i64 = sqlx::query_scalar("SELECT count(*) FROM public.outbox_event")
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap();
    rig.store
        .create(actor, space, support::command("v1 stays unchanged"))
        .await
        .unwrap();
    let content = VersionedContentStore::new(rig.runtime_pool.clone());
    content
        .create(
            actor,
            space,
            create(draft(BodyV3::Text(TextPayload {
                format: TextFormat::Markdown,
                text: "asset-free v3".into(),
            }))),
        )
        .await
        .unwrap();
    assert_eq!(
        event_count(&rig, actor).await,
        0,
        "asset-free content must not queue"
    );
    let figure_command = create(figure(png.clone()));
    let figure_request_id = figure_command.request_id;
    let first = content
        .create(actor, space, figure_command.clone())
        .await
        .unwrap();
    let first_ref = BlockRef {
        block_id: first.block_id,
        revision_id: first.revision_id,
    };
    assert_exact_event(&rig, actor, space, first_ref.clone(), png.asset_id).await;
    assert_eq!(
        content
            .create(actor, space, figure_command)
            .await
            .unwrap()
            .revision_id,
        first.revision_id
    );
    assert_eq!(
        event_count(&rig, actor).await,
        1,
        "receipt replay must not re-emit"
    );
    let mut conflict = create(attachment(notebook.clone()));
    conflict.request_id = figure_request_id;
    assert!(matches!(
        content.create(actor, space, conflict).await,
        Err(ContentError::IdempotencyConflict)
    ));
    assert_eq!(event_count(&rig, actor).await, 1);

    let attached = content
        .create(actor, space, create(attachment(notebook.clone())))
        .await
        .unwrap();
    assert_exact_event(
        &rig,
        actor,
        space,
        BlockRef {
            block_id: attached.block_id,
            revision_id: attached.revision_id,
        },
        notebook.asset_id,
    )
    .await;
    let revise = ReviseContent {
        request_id: Uuid::new_v4(),
        base_revision_id: first.revision_id,
        draft: figure(notebook.clone()),
        reason: "new exact use".into(),
    };
    let second = content
        .revise(actor, first.block_id, revise.clone())
        .await
        .unwrap();
    assert_ne!(second.revision_id, first.revision_id);
    assert_exact_event(
        &rig,
        actor,
        space,
        BlockRef {
            block_id: first.block_id,
            revision_id: second.revision_id,
        },
        notebook.asset_id,
    )
    .await;
    assert_eq!(
        content
            .revise(actor, first.block_id, revise)
            .await
            .unwrap()
            .revision_id,
        second.revision_id
    );

    AssetStore::new(rig.runtime_pool.clone(), files.files.clone())
        .link_resource_version(
            actor,
            ResourceInput {
                space_id: space,
                resource_id: None,
                display_name: "lecture".into(),
            },
            png,
        )
        .await
        .unwrap();
    assert_eq!(event_count(&rig, actor).await, 3);
    let release_count: i64 = sqlx::query_scalar("SELECT count(*) FROM public.outbox_event")
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap();
    assert_eq!(
        release_count, old_release_count,
        "v3 work must not use release outbox"
    );
}

#[tokio::test]
async fn lineage_split_emits_one_event_per_v3_output_and_replay_emits_none() {
    let rig = TestRig::from_env().await;
    let files = Originals::new();
    let (actor, space) = rig.seed_actor_space(true).await;
    let png = files
        .register(&rig, actor, space, "figure.png", "image/png", PNG)
        .await;
    let notebook = files
        .register(
            &rig,
            actor,
            space,
            "analysis.ipynb",
            "application/x-ipynb+json",
            NOTEBOOK,
        )
        .await;
    let input = rig
        .store
        .create(actor, space, support::command("source"))
        .await
        .unwrap();
    let command = LineageCommand {
        request_id: Uuid::new_v4(),
        operation: LineageOperation::Split,
        inputs: vec![BlockRef {
            block_id: input.block_id,
            revision_id: input.revision_id,
        }],
        outputs: vec![figure(png.clone()), attachment(notebook.clone())],
        reason: "make two originals".into(),
    };
    let store = LineageStore::new(rig.runtime_pool.clone());
    let saved = store.apply(actor, space, command.clone()).await.unwrap();
    assert_eq!(saved.outputs.len(), 2);
    assert_exact_event(&rig, actor, space, saved.outputs[0].clone(), png.asset_id).await;
    assert_exact_event(
        &rig,
        actor,
        space,
        saved.outputs[1].clone(),
        notebook.asset_id,
    )
    .await;
    assert_eq!(store.apply(actor, space, command).await.unwrap(), saved);
    assert_eq!(event_count(&rig, actor).await, 2);
}

#[tokio::test]
async fn invalid_create_and_late_invalid_lineage_output_leave_no_event_or_partial_content() {
    let rig = TestRig::from_env().await;
    let files = Originals::new();
    let (actor, space) = rig.seed_actor_space(true).await;
    let ready = files
        .register(&rig, actor, space, "figure.png", "image/png", PNG)
        .await;
    let source = rig
        .store
        .create(actor, space, support::command("source"))
        .await
        .unwrap();
    let before: (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM block_revision WHERE author_id=$1),\
                (SELECT count(*) FROM block_asset_use WHERE space_id=$2),\
                (SELECT count(*) FROM job_outbox WHERE actor_id=$1),\
                (SELECT count(*) FROM lineage_operation WHERE author_id=$1),\
                (SELECT count(*) FROM request_key WHERE actor_id=$1)",
    )
    .bind(actor.actor_id)
    .bind(space)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    let missing = AssetRef {
        space_id: space,
        asset_id: Uuid::new_v4(),
    };
    assert!(
        VersionedContentStore::new(rig.runtime_pool.clone())
            .create(actor, space, create(figure(missing.clone())))
            .await
            .is_err()
    );
    let foreign = AssetRef {
        space_id: Uuid::new_v4(),
        asset_id: ready.asset_id,
    };
    assert!(
        VersionedContentStore::new(rig.runtime_pool.clone())
            .create(actor, space, create(figure(foreign)))
            .await
            .is_err()
    );
    let command = LineageCommand {
        request_id: Uuid::new_v4(),
        operation: LineageOperation::Split,
        inputs: vec![BlockRef {
            block_id: source.block_id,
            revision_id: source.revision_id,
        }],
        outputs: vec![figure(ready), attachment(missing)],
        reason: "second output invalid".into(),
    };
    assert!(
        LineageStore::new(rig.runtime_pool.clone())
            .apply(actor, space, command)
            .await
            .is_err()
    );
    let after: (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM block_revision WHERE author_id=$1),\
                (SELECT count(*) FROM block_asset_use WHERE space_id=$2),\
                (SELECT count(*) FROM job_outbox WHERE actor_id=$1),\
                (SELECT count(*) FROM lineage_operation WHERE author_id=$1),\
                (SELECT count(*) FROM request_key WHERE actor_id=$1)",
    )
    .bind(actor.actor_id)
    .bind(space)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(after, before);
}

#[tokio::test]
async fn outbox_insert_failure_rolls_back_the_v3_revision_use_head_and_receipt() {
    let rig = TestRig::from_env().await;
    let files = Originals::new();
    let (actor, space) = rig.seed_actor_space(true).await;
    let png = files
        .register(&rig, actor, space, "figure.png", "image/png", PNG)
        .await;
    let name = format!("p0c2_fail_{}", Uuid::new_v4().simple());
    let ddl = format!(
        "CREATE FUNCTION public.{name}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.actor_id='{}'::uuid THEN RAISE EXCEPTION 'injected outbox failure'; END IF; RETURN NEW; END $$",
        actor.actor_id
    );
    sqlx::query(&ddl).execute(&rig.admin_pool).await.unwrap();
    sqlx::query(&format!("CREATE TRIGGER {name} BEFORE INSERT ON public.job_outbox FOR EACH ROW EXECUTE FUNCTION public.{name}()"))
        .execute(&rig.admin_pool).await.unwrap();
    let command = create(figure(png));
    let result = VersionedContentStore::new(rig.runtime_pool.clone())
        .create(actor, space, command.clone())
        .await;
    sqlx::query(&format!("DROP TRIGGER {name} ON public.job_outbox"))
        .execute(&rig.admin_pool)
        .await
        .unwrap();
    sqlx::query(&format!("DROP FUNCTION public.{name}()"))
        .execute(&rig.admin_pool)
        .await
        .unwrap();
    assert!(result.is_err(), "outbox failure must abort content write");
    let counts: (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM block WHERE space_id=$1),\
                (SELECT count(*) FROM block_revision WHERE space_id=$1),\
                (SELECT count(*) FROM block_asset_use WHERE space_id=$1),\
                (SELECT count(*) FROM job_outbox WHERE actor_id=$2),\
                (SELECT count(*) FROM mutation_receipt WHERE actor_id=$2 AND request_id=$3)",
    )
    .bind(space)
    .bind(actor.actor_id)
    .bind(command.request_id)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(counts, (0, 0, 0, 0, 0));
}

#[tokio::test]
async fn later_receipt_failure_rolls_back_new_revision_link_event_and_head() {
    let rig = TestRig::from_env().await;
    let files = Originals::new();
    let (actor, space) = rig.seed_actor_space(true).await;
    let png = files
        .register(&rig, actor, space, "figure.png", "image/png", PNG)
        .await;
    let content = VersionedContentStore::new(rig.runtime_pool.clone());
    let first = content
        .create(actor, space, create(figure(png.clone())))
        .await
        .unwrap();
    assert_exact_event(
        &rig,
        actor,
        space,
        BlockRef {
            block_id: first.block_id,
            revision_id: first.revision_id,
        },
        png.asset_id,
    )
    .await;
    let before: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM block_revision WHERE block_id=$1),\
                (SELECT count(*) FROM block_asset_use WHERE block_id=$1),\
                (SELECT count(*) FROM job_outbox WHERE actor_id=$2)",
    )
    .bind(first.block_id)
    .bind(actor.actor_id)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(before.2, 1, "the baseline event must exist before rollback");
    let name = format!("p0c2_receipt_fail_{}", Uuid::new_v4().simple());
    let ddl = format!(
        "CREATE FUNCTION public.{name}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.actor_id='{}'::uuid THEN RAISE EXCEPTION 'injected receipt failure'; END IF; RETURN NEW; END $$",
        actor.actor_id
    );
    sqlx::query(&ddl).execute(&rig.admin_pool).await.unwrap();
    sqlx::query(&format!("CREATE TRIGGER {name} BEFORE INSERT ON public.mutation_receipt FOR EACH ROW EXECUTE FUNCTION public.{name}()"))
        .execute(&rig.admin_pool).await.unwrap();
    let revise = ReviseContent {
        request_id: Uuid::new_v4(),
        base_revision_id: first.revision_id,
        draft: figure(png),
        reason: "receipt failure".into(),
    };
    let result = content.revise(actor, first.block_id, revise.clone()).await;
    sqlx::query(&format!("DROP TRIGGER {name} ON public.mutation_receipt"))
        .execute(&rig.admin_pool)
        .await
        .unwrap();
    sqlx::query(&format!("DROP FUNCTION public.{name}()"))
        .execute(&rig.admin_pool)
        .await
        .unwrap();
    assert!(result.is_err());
    assert_eq!(rig.head(first.block_id).await, first.revision_id);
    let after: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM block_revision WHERE block_id=$1),\
                (SELECT count(*) FROM block_asset_use WHERE block_id=$1),\
                (SELECT count(*) FROM job_outbox WHERE actor_id=$2)",
    )
    .bind(first.block_id)
    .bind(actor.actor_id)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(after, before);
    let receipt_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM mutation_receipt WHERE actor_id=$1 AND request_id=$2",
    )
    .bind(actor.actor_id)
    .bind(revise.request_id)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(receipt_count, 0);
}

#[tokio::test]
async fn event_insert_observes_the_exact_block_use_inside_its_business_transaction() {
    let rig = TestRig::from_env().await;
    let files = Originals::new();
    let (actor, space) = rig.seed_actor_space(true).await;
    let png = files
        .register(&rig, actor, space, "figure.png", "image/png", PNG)
        .await;
    let name = format!("p0c2_order_{}", Uuid::new_v4().simple());
    let ddl = format!(
        "CREATE FUNCTION public.{name}() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN IF NEW.actor_id='{}'::uuid AND NOT EXISTS \
           (SELECT 1 FROM public.block_asset_use u \
             WHERE u.space_id=(NEW.payload->>'space_id')::uuid \
               AND u.block_id=(NEW.payload->>'block_id')::uuid \
               AND u.revision_id=(NEW.payload->>'revision_id')::uuid) \
         THEN RAISE EXCEPTION 'outbox inserted before exact block use'; \
         END IF; RETURN NEW; END $$",
        actor.actor_id
    );
    sqlx::query(&ddl).execute(&rig.admin_pool).await.unwrap();
    sqlx::query(&format!("CREATE TRIGGER {name} BEFORE INSERT ON public.job_outbox FOR EACH ROW EXECUTE FUNCTION public.{name}()"))
        .execute(&rig.admin_pool).await.unwrap();
    let result = VersionedContentStore::new(rig.runtime_pool.clone())
        .create(actor, space, create(figure(png)))
        .await;
    sqlx::query(&format!("DROP TRIGGER {name} ON public.job_outbox"))
        .execute(&rig.admin_pool)
        .await
        .unwrap();
    sqlx::query(&format!("DROP FUNCTION public.{name}()"))
        .execute(&rig.admin_pool)
        .await
        .unwrap();
    assert!(result.is_ok(), "exact link must precede event insert");
    assert_eq!(event_count(&rig, actor).await, 1);
}

mod support;

use learning_assets::{FsAssetStore, UploadDeclaration, VerifiedBlob};
use learning_core::*;
use learning_db::{AssetMedia, AssetStore, LineageStore, ReviewStore, VersionedContentStore};
use std::{fs, path::PathBuf};
use support::TestRig;
use uuid::Uuid;

const PNG: &[u8] = &[137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 73, 69, 78, 68];

struct Files {
    root: PathBuf,
    store: FsAssetStore,
}

impl Files {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("p0c1-asset-write-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let store = FsAssetStore::new(root.join("assets"), root.join("staging")).unwrap();
        Self { root, store }
    }

    fn finalized(&self, bytes: &[u8]) -> VerifiedBlob {
        let source = self.root.join(format!("source-{}", Uuid::new_v4()));
        fs::write(&source, bytes).unwrap();
        self.store
            .put_from_file(
                Uuid::new_v4(),
                &source,
                UploadDeclaration {
                    expected_size_bytes: bytes.len() as u64,
                    max_size_bytes: 1_000_000,
                },
            )
            .unwrap()
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

fn figure(asset: AssetRef, intent: Intent, source_run: Option<BlockRef>) -> ContentDraft {
    ContentDraft::V3(ContentV3 {
        intent,
        language: "en".into(),
        title: "Attention figure".into(),
        body: BodyV3::Figure {
            asset,
            usage: "lecture_diagram".into(),
            caption: "Attention".into(),
            alt: "Query, key and value arrows".into(),
            decorative: false,
        },
        basis_refs: vec![],
        requires_context: vec![],
        source_run,
    })
}

fn create(draft: ContentDraft) -> CreateContent {
    CreateContent {
        request_id: Uuid::new_v4(),
        draft,
        reason: "figure".into(),
    }
}

async fn ready(rig: &TestRig, files: &Files, actor: Principal, space: Uuid) -> AssetRef {
    AssetStore::new(rig.runtime_pool.clone(), files.store.clone())
        .register_verified(
            actor,
            space,
            Uuid::new_v4(),
            files.finalized(PNG),
            AssetMedia {
                media_type: "image/png".into(),
                original_file_name: "figure.png".into(),
            },
        )
        .await
        .unwrap()
        .reference
}

async fn counts(rig: &TestRig, actor: Principal, space: Uuid) -> (i64, i64, i64, i64, i64) {
    sqlx::query_as("SELECT (SELECT count(*) FROM block WHERE space_id=$1), (SELECT count(*) FROM block_revision WHERE space_id=$1), (SELECT count(*) FROM block_asset_use WHERE space_id=$1), (SELECT count(*) FROM request_key WHERE actor_id=$2), (SELECT count(*) FROM mutation_receipt WHERE actor_id=$2)")
        .bind(space)
        .bind(actor.actor_id)
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn missing_or_wrong_space_asset_rolls_back_every_content_row_and_receipt() {
    let rig = TestRig::from_env().await;
    let files = Files::new();
    let (actor, space) = rig.seed_actor_space(true).await;
    let (_, other_space) = rig.seed_actor_space(true).await;
    rig.grant(actor, other_space, true).await;
    let wrong_space = ready(&rig, &files, actor, other_space).await;
    let store = VersionedContentStore::new(rig.runtime_pool.clone());
    let before = counts(&rig, actor, space).await;
    for reference in [
        AssetRef {
            space_id: space,
            asset_id: Uuid::new_v4(),
        },
        wrong_space,
    ] {
        assert!(matches!(
            store
                .create(actor, space, create(figure(reference, Intent::Note, None)))
                .await,
            Err(ContentError::NotFound)
        ));
        assert_eq!(counts(&rig, actor, space).await, before);
    }
    let blob = files.finalized(b"uncommitted image bytes");
    let hidden_id = Uuid::new_v4();
    let mut pending = rig.admin_pool.begin().await.unwrap();
    sqlx::query("INSERT INTO asset(space_id,id,sha256,byte_size,storage_key,media_type,original_file_name,status) VALUES($1,$2,$3,$4,$5,'image/png','pending.png','ready')")
        .bind(space).bind(hidden_id).bind(blob.sha256()).bind(blob.size_bytes() as i64)
        .bind(blob.storage_key()).execute(&mut *pending).await.unwrap();
    let attempted = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        store.create(
            actor,
            space,
            create(figure(
                AssetRef {
                    space_id: space,
                    asset_id: hidden_id,
                },
                Intent::Note,
                None,
            )),
        ),
    )
    .await
    .expect("uncommitted asset check must not wait for foreign-key publication");
    assert!(matches!(attempted, Err(ContentError::NotFound)));
    pending.rollback().await.unwrap();
    assert_eq!(counts(&rig, actor, space).await, before);
}

#[tokio::test]
async fn ready_asset_is_linked_once_per_exact_revision_and_v3_request_replays() {
    let rig = TestRig::from_env().await;
    let files = Files::new();
    let (actor, space) = rig.seed_actor_space(true).await;
    let asset = ready(&rig, &files, actor, space).await;
    let store = VersionedContentStore::new(rig.runtime_pool.clone());
    let command = create(figure(asset.clone(), Intent::Note, None));
    let first = store.create(actor, space, command.clone()).await.unwrap();
    assert_eq!(first.draft, command.draft);
    assert_eq!(
        store.create(actor, space, command.clone()).await.unwrap(),
        first
    );
    let linked: Uuid = sqlx::query_scalar(
        "SELECT asset_id FROM block_asset_use WHERE space_id=$1 AND block_id=$2 AND revision_id=$3",
    )
    .bind(space)
    .bind(first.block_id)
    .bind(first.revision_id)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(linked, asset.asset_id);
    let operation: String =
        sqlx::query_scalar("SELECT operation FROM request_key WHERE actor_id=$1 AND request_id=$2")
            .bind(actor.actor_id)
            .bind(command.request_id)
            .fetch_one(&rig.admin_pool)
            .await
            .unwrap();
    assert_eq!(operation, "content_v3");
    let second_asset = ready(&rig, &files, actor, space).await;
    let mut changed = command.clone();
    changed.draft = figure(second_asset, Intent::Note, None);
    assert!(matches!(
        store.create(actor, space, changed).await,
        Err(ContentError::IdempotencyConflict)
    ));
    let before_failed_revision = counts(&rig, actor, space).await;
    assert!(matches!(
        store
            .revise(
                actor,
                first.block_id,
                ReviseContent {
                    request_id: Uuid::new_v4(),
                    base_revision_id: first.revision_id,
                    draft: figure(
                        AssetRef {
                            space_id: space,
                            asset_id: Uuid::new_v4()
                        },
                        Intent::Note,
                        None
                    ),
                    reason: "unavailable replacement".into(),
                }
            )
            .await,
        Err(ContentError::NotFound)
    ));
    assert_eq!(rig.head(first.block_id).await, first.revision_id);
    assert_eq!(counts(&rig, actor, space).await, before_failed_revision);
    let mut revised_draft = figure(asset.clone(), Intent::Note, None);
    if let ContentDraft::V3(content) = &mut revised_draft
        && let BodyV3::Figure { caption, .. } = &mut content.body
    {
        *caption = "Updated attention caption".into();
    }
    let revised = store
        .revise(
            actor,
            first.block_id,
            ReviseContent {
                request_id: Uuid::new_v4(),
                base_revision_id: first.revision_id,
                draft: revised_draft,
                reason: "replace caption".into(),
            },
        )
        .await
        .unwrap();
    assert_ne!(revised.revision_id, first.revision_id);
    let linked_revisions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM block_asset_use WHERE space_id=$1 AND block_id=$2 AND asset_id=$3",
    )
    .bind(space)
    .bind(first.block_id)
    .bind(asset.asset_id)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(linked_revisions, 2);
}

#[tokio::test]
async fn later_raw_use_deletion_or_replacement_cannot_corrupt_a_committed_figure() {
    let rig = TestRig::from_env().await;
    let files = Files::new();
    let (actor, space) = rig.seed_actor_space(true).await;
    let asset = ready(&rig, &files, actor, space).await;
    let replacement = ready(&rig, &files, actor, space).await;
    let saved = VersionedContentStore::new(rig.runtime_pool.clone())
        .create(
            actor,
            space,
            create(figure(asset.clone(), Intent::Note, None)),
        )
        .await
        .unwrap();
    let mut deleting = rig.admin_pool.begin().await.unwrap();
    sqlx::query("DELETE FROM block_asset_use WHERE space_id=$1 AND block_id=$2 AND revision_id=$3")
        .bind(space)
        .bind(saved.block_id)
        .bind(saved.revision_id)
        .execute(&mut *deleting)
        .await
        .unwrap();
    assert_eq!(
        support::sqlstate(&deleting.commit().await.unwrap_err()).as_deref(),
        Some("23514")
    );
    let mut replacing = rig.admin_pool.begin().await.unwrap();
    sqlx::query("UPDATE block_asset_use SET asset_id=$1 WHERE space_id=$2 AND block_id=$3 AND revision_id=$4")
        .bind(replacement.asset_id).bind(space).bind(saved.block_id).bind(saved.revision_id)
        .execute(&mut *replacing).await.unwrap();
    assert_eq!(
        support::sqlstate(&replacing.commit().await.unwrap_err()).as_deref(),
        Some("23514")
    );
    let linked: Uuid = sqlx::query_scalar(
        "SELECT asset_id FROM block_asset_use WHERE space_id=$1 AND block_id=$2 AND revision_id=$3",
    )
    .bind(space)
    .bind(saved.block_id)
    .bind(saved.revision_id)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(linked, asset.asset_id);
}

#[tokio::test]
async fn derive_and_split_share_the_central_guard_and_rollback_all_outputs() {
    let rig = TestRig::from_env().await;
    let files = Files::new();
    let (actor, space) = rig.seed_actor_space(true).await;
    let source = rig
        .store
        .create(actor, space, support::command("source"))
        .await
        .unwrap();
    let source = BlockRef {
        block_id: source.block_id,
        revision_id: source.revision_id,
    };
    let asset = ready(&rig, &files, actor, space).await;
    let store = LineageStore::new(rig.runtime_pool.clone());
    let before = counts(&rig, actor, space).await;
    for operation in [LineageOperation::Derive, LineageOperation::Split] {
        let mut outputs = vec![figure(asset.clone(), Intent::Note, None)];
        if operation == LineageOperation::Split {
            outputs.push(figure(
                AssetRef {
                    space_id: space,
                    asset_id: Uuid::new_v4(),
                },
                Intent::Note,
                None,
            ));
        } else {
            outputs[0] = figure(
                AssetRef {
                    space_id: space,
                    asset_id: Uuid::new_v4(),
                },
                Intent::Note,
                None,
            );
        }
        let command = LineageCommand {
            request_id: Uuid::new_v4(),
            operation,
            inputs: vec![source.clone()],
            outputs,
            reason: "transform".into(),
        };
        assert!(matches!(
            store.apply(actor, space, command).await,
            Err(ContentError::NotFound)
        ));
        assert_eq!(counts(&rig, actor, space).await, before);
    }
    let saved = store
        .apply(
            actor,
            space,
            LineageCommand {
                request_id: Uuid::new_v4(),
                operation: LineageOperation::Derive,
                inputs: vec![source],
                outputs: vec![figure(asset.clone(), Intent::Note, None)],
                reason: "derive figure".into(),
            },
        )
        .await
        .unwrap();
    let output = &saved.outputs[0];
    let linked: Uuid = sqlx::query_scalar(
        "SELECT asset_id FROM block_asset_use WHERE space_id=$1 AND block_id=$2 AND revision_id=$3",
    )
    .bind(space)
    .bind(output.block_id)
    .bind(output.revision_id)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(linked, asset.asset_id);
}

#[tokio::test]
async fn failed_database_registration_keeps_finalized_bytes_without_visible_asset() {
    let rig = TestRig::from_env().await;
    let files = Files::new();
    let (actor, space) = rig.seed_actor_space(false).await;
    let blob = files.finalized(PNG);
    let request_id = Uuid::new_v4();
    let store = AssetStore::new(rig.runtime_pool.clone(), files.store.clone());
    assert!(matches!(
        store
            .register_verified(
                actor,
                space,
                request_id,
                blob.clone(),
                AssetMedia {
                    media_type: "image/png".into(),
                    original_file_name: "failed.png".into(),
                }
            )
            .await,
        Err(ContentError::NotFound)
    ));
    files.store.verify(&blob).unwrap();
    let visible: i64 = sqlx::query_scalar("SELECT count(*) FROM asset WHERE space_id=$1")
        .bind(space)
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap();
    let receipts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM request_key WHERE actor_id=$1 AND request_id=$2")
            .bind(actor.actor_id)
            .bind(request_id)
            .fetch_one(&rig.admin_pool)
            .await
            .unwrap();
    assert_eq!((visible, receipts), (0, 0));
}

#[tokio::test]
async fn database_rejects_v3_json_without_its_exact_asset_use_at_commit() {
    let rig = TestRig::from_env().await;
    let files = Files::new();
    let (actor, space) = rig.seed_actor_space(true).await;
    let asset = ready(&rig, &files, actor, space).await;
    let draft = figure(asset, Intent::Note, None);
    let value = serde_json::to_value(&draft).unwrap();
    let dependencies: serde_json::Value =
        sqlx::query_scalar("SELECT public.b3_content_dependencies(3,$1)")
            .bind(&value)
            .fetch_one(&rig.admin_pool)
            .await
            .unwrap();
    assert_eq!(dependencies, serde_json::json!([]));
    let block = Uuid::new_v4();
    let revision = Uuid::new_v4();
    let mut tx = rig.admin_pool.begin().await.unwrap();
    sqlx::query("INSERT INTO block(id,space_id,head_revision_id) VALUES($1,$2,$3)")
        .bind(block)
        .bind(space)
        .bind(revision)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO block_revision(id,space_id,block_id,content,content_sha256,author_id,reason,contract_version) VALUES($1,$2,$3,$4,$5,$6,'raw',3)")
        .bind(revision).bind(space).bind(block).bind(value).bind(draft.digest()).bind(actor.actor_id)
        .execute(&mut *tx).await.unwrap();
    let error = tx.commit().await.unwrap_err();
    assert_eq!(support::sqlstate(&error).as_deref(), Some("23514"));
    let persisted: i64 = sqlx::query_scalar("SELECT count(*) FROM block WHERE id=$1")
        .bind(block)
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap();
    assert_eq!(persisted, 0);
}

#[tokio::test]
async fn reading_places_an_existing_v3_figure_while_insert_new_stays_v1() {
    use support::{assembly as a, reading as h};
    let rig = TestRig::from_env().await;
    let files = Files::new();
    let (actor, space) = rig.seed_actor_space(true).await;
    let asset = ready(&rig, &files, actor, space).await;
    let figure = VersionedContentStore::new(rig.runtime_pool.clone())
        .create(
            actor,
            space,
            create(figure(asset.clone(), Intent::Note, None)),
        )
        .await
        .unwrap();
    let reference = BlockRef {
        block_id: figure.block_id,
        revision_id: figure.revision_id,
    };
    let original = rig
        .store
        .create(actor, space, support::command("original"))
        .await
        .unwrap();
    let doc = rig
        .compositions()
        .save(actor, space, a::doc(vec![a::block(&original)]))
        .await
        .unwrap();
    let reading = h::store(&rig);
    let saved = reading
        .create(actor, space, h::create(doc.reference.clone()))
        .await
        .unwrap();
    let saved = reading
        .edit(
            actor,
            saved.overlay.overlay_id,
            h::edit(
                &saved,
                ReadingEdit::InsertExisting {
                    block: reference.clone(),
                    target: InsertTarget::NewGroup {
                        anchor: h::gap(&doc, 1),
                    },
                },
            ),
        )
        .await
        .unwrap();
    let state = reading
        .state(actor, saved.overlay.overlay_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        state
            .editable
            .unwrap()
            .groups
            .iter()
            .flat_map(|group| &group.placements)
            .any(|placement| placement.block == reference)
    );
    let projected = reading
        .read_versioned(actor, saved.view.clone(), ReadingMode::Fused)
        .await
        .unwrap()
        .unwrap();
    assert!(projected.items.iter().any(|item| matches!(item,
        ReadingItemData::Personal { item } if item.revision.block_id == reference.block_id
            && item.revision.revision_id == reference.revision_id
            && matches!(&item.revision.draft, ContentDraft::V3(_))
    )));
    let placed_view = saved.view.revision_id;
    let saved = reading
        .edit(
            actor,
            saved.overlay.overlay_id,
            h::edit(&saved, h::add(h::gap(&doc, 0), "personal text")),
        )
        .await
        .unwrap();
    assert_eq!(saved.changed_blocks.len(), 1);
    let mixed = reading
        .read_versioned(actor, saved.view.clone(), ReadingMode::Fused)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        mixed
            .items
            .iter()
            .filter(|item| matches!(item,
                ReadingItemData::Personal { item } if item.revision.block_id == reference.block_id
                    && item.revision.revision_id == reference.revision_id
                    && matches!(&item.revision.draft, ContentDraft::V3(_))
            ))
            .count(),
        1
    );
    assert_eq!(
        mixed
            .items
            .iter()
            .filter(|item| matches!(item,
                ReadingItemData::Personal { item } if item.revision.block_id == saved.changed_blocks[0].block_id
                    && item.revision.revision_id == saved.changed_blocks[0].revision_id
                    && matches!(&item.revision.draft, ContentDraft::V1(_))
            ))
            .count(),
        1
    );
    let link_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM block_asset_use WHERE space_id=$1 AND asset_id=$2",
    )
    .bind(space)
    .bind(asset.asset_id)
    .fetch_one(&rig.admin_pool)
    .await
    .unwrap();
    assert_eq!(
        link_count, 1,
        "placement and v1 text must not create another asset use"
    );
    assert_ne!(saved.view.revision_id, placed_view);
}

#[tokio::test]
async fn v3_figure_projects_through_reviews_and_evidence_release_only() {
    use support::assembly as a;
    let rig = TestRig::from_env().await;
    let files = Files::new();
    let (actor, space) = rig.seed_actor_space(true).await;
    let asset = ready(&rig, &files, actor, space).await;
    let source = rig
        .store
        .create(actor, space, support::command("run"))
        .await
        .unwrap();
    let run = BlockRef {
        block_id: source.block_id,
        revision_id: source.revision_id,
    };
    let store = VersionedContentStore::new(rig.runtime_pool.clone());
    let evidence = store
        .create(
            actor,
            space,
            create(figure(asset.clone(), Intent::Evidence, Some(run.clone()))),
        )
        .await
        .unwrap();
    let target = store
        .create(
            actor,
            space,
            create(figure(asset, Intent::Conjecture, None)),
        )
        .await
        .unwrap();
    let evidence_ref = BlockRef {
        block_id: evidence.block_id,
        revision_id: evidence.revision_id,
    };
    let target_ref = BlockRef {
        block_id: target.block_id,
        revision_id: target.revision_id,
    };
    let reviews = ReviewStore::new(rig.runtime_pool.clone());
    let groups = reviews
        .group_sources(actor, vec![evidence_ref.clone()])
        .await
        .unwrap();
    assert_eq!(groups[0].source_run, Some(run));
    let review = reviews
        .append(
            actor,
            AppendEpistemicReview {
                request_id: Uuid::new_v4(),
                scope: RelationScope::Space { space_id: space },
                target: target_ref.clone(),
                expected_previous: None,
                state: EpistemicState::Untested,
                relations: vec![],
                evidence: vec![],
                conditions: String::new(),
                explanation: String::new(),
            },
        )
        .await
        .unwrap();
    assert_eq!(review.target, target_ref);
    let doc = rig
        .compositions()
        .save(actor, space, a::doc(vec![NodeTarget::Block(evidence_ref)]))
        .await
        .unwrap();
    let root = a::root(&doc, None);
    assert!(
        matches!(rig.releases().publish(actor, space, a::publish(vec![root.clone()])).await,
        Err(ContentError::Invalid(code)) if code == "unsupported_content_version")
    );
    let published = rig
        .releases()
        .publish_evidence(
            actor,
            space,
            PublishEvidence {
                request_id: Uuid::new_v4(),
                roots: vec![root],
                readings: vec![],
                reason: "evidence figure".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(published.roots, vec![doc.reference]);
}

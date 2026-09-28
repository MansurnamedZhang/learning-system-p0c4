mod support;

use learning_assets::{FsAssetStore, UploadDeclaration};
use learning_core::*;
use learning_db::{AssetMedia, AssetStore, ResourceInput, VersionedContentStore};
use std::{fs, io::Read, path::PathBuf};
use support::{TestRig, assembly as a, reading as h};
use uuid::Uuid;

const PNG: &[u8] = &[137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 73, 69, 78, 68];

struct Files {
    root: PathBuf,
    store: FsAssetStore,
}

impl Files {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("p0c1-asset-read-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let store = FsAssetStore::new(root.join("assets"), root.join("staging")).unwrap();
        Self { root, store }
    }

    fn finalized(&self, bytes: &[u8]) -> learning_assets::VerifiedBlob {
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

async fn ready(
    rig: &TestRig,
    files: &Files,
    actor: Principal,
    space: Uuid,
    bytes: &[u8],
) -> learning_db::AssetRecord {
    AssetStore::new(rig.runtime_pool.clone(), files.store.clone())
        .register_verified(
            actor,
            space,
            Uuid::new_v4(),
            files.finalized(bytes),
            AssetMedia {
                media_type: "image/png".into(),
                original_file_name: "figure.png".into(),
            },
        )
        .await
        .unwrap()
}

fn figure_draft(asset: AssetRef) -> ContentDraft {
    ContentDraft::V3(ContentV3 {
        intent: Intent::Note,
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
        source_run: None,
    })
}

async fn figure(rig: &TestRig, actor: Principal, space: Uuid, asset: AssetRef) -> BlockRef {
    let saved = VersionedContentStore::new(rig.runtime_pool.clone())
        .create(
            actor,
            space,
            CreateContent {
                request_id: Uuid::new_v4(),
                draft: figure_draft(asset),
                reason: "figure".into(),
            },
        )
        .await
        .unwrap();
    BlockRef {
        block_id: saved.block_id,
        revision_id: saved.revision_id,
    }
}

#[tokio::test]
async fn exact_block_and_resource_version_reads_hide_absent_and_revoked_uses() {
    let rig = TestRig::from_env().await;
    let files = Files::new();
    let (owner, space) = rig.seed_actor_space(true).await;
    let (outsider, _) = rig.seed_actor_space(true).await;
    let store = AssetStore::new(rig.runtime_pool.clone(), files.store.clone());
    let old_asset = ready(&rig, &files, owner, space, PNG).await;
    let new_asset = ready(
        &rig,
        &files,
        owner,
        space,
        &[137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 0, 73, 69, 78, 69],
    )
    .await;
    let block = figure(&rig, owner, space, old_asset.reference.clone()).await;
    let new_block_revision = VersionedContentStore::new(rig.runtime_pool.clone())
        .revise(
            owner,
            block.block_id,
            ReviseContent {
                request_id: Uuid::new_v4(),
                base_revision_id: block.revision_id,
                draft: figure_draft(new_asset.reference.clone()),
                reason: "replace original".into(),
            },
        )
        .await
        .unwrap();
    let first = store
        .link_resource_version(
            owner,
            ResourceInput {
                space_id: space,
                resource_id: None,
                display_name: "lecture".into(),
            },
            old_asset.reference.clone(),
        )
        .await
        .unwrap();
    let second = store
        .link_resource_version(
            owner,
            ResourceInput {
                space_id: space,
                resource_id: Some(first.resource_id),
                display_name: "lecture".into(),
            },
            new_asset.reference.clone(),
        )
        .await
        .unwrap();
    let block_use = AssetUseRef::Block(block.clone());
    let new_block_use = AssetUseRef::Block(BlockRef {
        block_id: block.block_id,
        revision_id: new_block_revision.revision_id,
    });
    let old_use = AssetUseRef::Resource(first.clone());
    let new_use = AssetUseRef::Resource(second);
    let absent_block = AssetUseRef::Block(BlockRef {
        block_id: block.block_id,
        revision_id: Uuid::new_v4(),
    });
    let absent_resource = AssetUseRef::Resource(ResourceVersionRef {
        space_id: space,
        resource_id: first.resource_id,
        version_id: Uuid::new_v4(),
    });

    assert_eq!(
        store.read_for_use(owner, block_use.clone()).await.unwrap(),
        Some(old_asset.clone())
    );
    assert_eq!(
        store.read_for_use(owner, new_block_use).await.unwrap(),
        Some(new_asset.clone())
    );
    assert_eq!(
        store.read_for_use(owner, old_use.clone()).await.unwrap(),
        Some(old_asset.clone())
    );
    assert_eq!(
        store.read_for_use(owner, new_use.clone()).await.unwrap(),
        Some(new_asset)
    );
    assert_eq!(
        store
            .read_for_use(owner, absent_block.clone())
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        store
            .read_for_use(owner, absent_resource.clone())
            .await
            .unwrap(),
        None
    );
    // A hidden use and a nonexistent use share the same external result, including
    // the absence of a storage key or digest in the response.
    for use_ref in [&block_use, &old_use, &absent_block, &absent_resource] {
        assert_eq!(
            store
                .read_for_use(outsider, (*use_ref).clone())
                .await
                .unwrap(),
            None
        );
    }
    rig.grant(outsider, space, false).await;
    assert_eq!(
        store.read_for_use(outsider, old_use.clone()).await.unwrap(),
        Some(old_asset.clone())
    );
    rig.revoke(outsider, space).await;
    assert_eq!(
        store.read_for_use(outsider, old_use.clone()).await.unwrap(),
        None
    );
    rig.revoke(owner, space).await;
    assert_eq!(
        store.read_for_use(owner, block_use.clone()).await.unwrap(),
        None
    );
    rig.grant(owner, space, true).await;
    assert_eq!(
        store.read_for_use(owner, block_use).await.unwrap(),
        Some(old_asset)
    );
}

#[tokio::test]
async fn byte_access_checks_the_exact_use_before_opening_and_rejects_missing_or_corrupt_bytes() {
    let rig = TestRig::from_env().await;
    let files = Files::new();
    let (owner, space) = rig.seed_actor_space(true).await;
    let (outsider, _) = rig.seed_actor_space(true).await;
    let asset = ready(&rig, &files, owner, space, PNG).await;
    let block = figure(&rig, owner, space, asset.reference.clone()).await;
    let use_ref = AssetUseRef::Block(block);
    let store = AssetStore::new(rig.runtime_pool.clone(), files.store.clone());
    let mut original = Vec::new();
    store
        .open_for_use(owner, use_ref.clone())
        .await
        .unwrap()
        .unwrap()
        .read_to_end(&mut original)
        .unwrap();
    assert_eq!(original, PNG);
    assert!(
        store
            .open_for_use(outsider, use_ref.clone())
            .await
            .unwrap()
            .is_none()
    );
    let path = files.root.join("assets").join(&asset.storage_key);
    fs::write(&path, b"tampered").unwrap();
    assert!(
        store
            .open_for_use(outsider, use_ref.clone())
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        store.open_for_use(owner, use_ref.clone()).await,
        Err(ContentError::Storage)
    ));
    fs::remove_file(&path).unwrap();
    assert!(matches!(
        store.open_for_use(owner, use_ref).await,
        Err(ContentError::Storage)
    ));
}

#[tokio::test]
async fn block_asset_read_obeys_the_whole_reference_closure_after_dependency_revoke() {
    let rig = TestRig::from_env().await;
    let files = Files::new();
    let (actor, space) = rig.seed_actor_space(true).await;
    let (_, dependency_space) = rig.seed_actor_space(true).await;
    rig.grant(actor, dependency_space, true).await;
    let dependency = rig
        .store
        .create(
            actor,
            dependency_space,
            support::command("private prerequisite"),
        )
        .await
        .unwrap();
    let dependency = BlockRef {
        block_id: dependency.block_id,
        revision_id: dependency.revision_id,
    };
    let asset = ready(&rig, &files, actor, space, PNG).await;
    let mut draft = figure_draft(asset.reference.clone());
    if let ContentDraft::V3(ref mut content) = draft {
        content.basis_refs.push(ExactRef::Block(dependency));
    }
    let saved = VersionedContentStore::new(rig.runtime_pool.clone())
        .create(
            actor,
            space,
            CreateContent {
                request_id: Uuid::new_v4(),
                draft,
                reason: "depends on private prerequisite".into(),
            },
        )
        .await
        .unwrap();
    let use_ref = AssetUseRef::Block(BlockRef {
        block_id: saved.block_id,
        revision_id: saved.revision_id,
    });
    let store = AssetStore::new(rig.runtime_pool.clone(), files.store.clone());
    assert_eq!(
        store.read_for_use(actor, use_ref.clone()).await.unwrap(),
        Some(asset.clone())
    );
    rig.revoke(actor, dependency_space).await;
    assert_eq!(
        store.read_for_use(actor, use_ref.clone()).await.unwrap(),
        None
    );
    assert!(
        store
            .open_for_use(actor, use_ref.clone())
            .await
            .unwrap()
            .is_none()
    );
    rig.grant(actor, dependency_space, false).await;
    assert_eq!(
        store.read_for_use(actor, use_ref).await.unwrap(),
        Some(asset)
    );
}

fn item_labels(projection: &VersionedReadingProjection) -> Vec<String> {
    projection
        .items
        .iter()
        .filter_map(|item| {
            let draft = match item {
                ReadingItemData::Original { revision, .. } => &revision.draft,
                ReadingItemData::Personal { item } => &item.revision.draft,
                _ => return None,
            };
            Some(match draft {
                ContentDraft::V1(text) => text.payload.text.clone(),
                ContentDraft::V3(content) if matches!(&content.body, BodyV3::Figure { .. }) => {
                    "FIG".into()
                }
                _ => panic!("unexpected content in fixed reading"),
            })
        })
        .collect()
}

#[tokio::test]
async fn moving_one_figure_placement_keeps_old_views_and_one_original_byte_object() {
    let rig = TestRig::from_env().await;
    let files = Files::new();
    let (actor, space) = rig.seed_actor_space(true).await;
    let asset = ready(&rig, &files, actor, space, PNG).await;
    let figure = figure(&rig, actor, space, asset.reference.clone()).await;
    let shared = rig
        .store
        .create(actor, space, support::command("H"))
        .await
        .unwrap();
    let tail = rig
        .store
        .create(actor, space, support::command("K"))
        .await
        .unwrap();
    let doc_a = rig
        .compositions()
        .save(
            actor,
            space,
            a::doc(vec![a::block(&shared), a::block(&tail)]),
        )
        .await
        .unwrap();
    let doc_b = rig
        .compositions()
        .save(actor, space, a::doc(vec![a::block(&shared)]))
        .await
        .unwrap();
    let reading = h::store(&rig);
    let initial_a = reading
        .create(actor, space, h::create(doc_a.reference.clone()))
        .await
        .unwrap();
    let initial_b = reading
        .create(actor, space, h::create(doc_b.reference.clone()))
        .await
        .unwrap();
    let with_note = reading
        .edit(
            actor,
            initial_a.overlay.overlay_id,
            h::edit(&initial_a, h::add(h::gap(&doc_a, 1), "N")),
        )
        .await
        .unwrap();
    let first_state = reading
        .state(actor, with_note.overlay.overlay_id)
        .await
        .unwrap()
        .unwrap()
        .editable
        .unwrap();
    let first_group = &first_state.groups[0];
    let with_figure = reading
        .edit(
            actor,
            with_note.overlay.overlay_id,
            h::edit(
                &with_note,
                ReadingEdit::InsertExisting {
                    block: figure.clone(),
                    target: InsertTarget::ExistingGroup {
                        group_id: first_group.group_id,
                        order: PlacementOrder {
                            left_placement_id: Some(first_group.placements[0].placement_id),
                            right_placement_id: None,
                        },
                    },
                },
            ),
        )
        .await
        .unwrap();
    let figure_state = reading
        .state(actor, with_figure.overlay.overlay_id)
        .await
        .unwrap()
        .unwrap()
        .editable
        .unwrap();
    let group = &figure_state.groups[0];
    let figure_placement = group
        .placements
        .iter()
        .find(|p| p.block == figure)
        .unwrap()
        .placement_id;
    let with_two_notes = reading
        .edit(
            actor,
            with_figure.overlay.overlay_id,
            h::edit(
                &with_figure,
                ReadingEdit::InsertNew {
                    drafts: vec![support::command("I").draft],
                    target: InsertTarget::ExistingGroup {
                        group_id: group.group_id,
                        order: PlacementOrder {
                            left_placement_id: Some(figure_placement),
                            right_placement_id: None,
                        },
                    },
                },
            ),
        )
        .await
        .unwrap();
    let before_move = reading
        .read_versioned(actor, with_two_notes.view.clone(), ReadingMode::Fused)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(item_labels(&before_move), ["H", "N", "FIG", "I", "K"]);
    let moved = reading
        .edit(
            actor,
            with_two_notes.overlay.overlay_id,
            h::edit(
                &with_two_notes,
                ReadingEdit::Move {
                    placement_id: figure_placement,
                    target: InsertTarget::NewGroup {
                        anchor: h::gap(&doc_a, 0),
                    },
                },
            ),
        )
        .await
        .unwrap();
    assert!(moved.changed_blocks.is_empty());
    assert_eq!(
        item_labels(
            &reading
                .read_versioned(actor, moved.view, ReadingMode::Fused)
                .await
                .unwrap()
                .unwrap()
        ),
        ["FIG", "H", "N", "I", "K"]
    );
    assert_eq!(
        item_labels(
            &reading
                .read_versioned(actor, with_two_notes.view, ReadingMode::Fused)
                .await
                .unwrap()
                .unwrap()
        ),
        ["H", "N", "FIG", "I", "K"]
    );
    assert_eq!(
        item_labels(
            &reading
                .read_versioned(actor, initial_a.view, ReadingMode::Fused)
                .await
                .unwrap()
                .unwrap()
        ),
        ["H", "K"]
    );
    assert_eq!(
        item_labels(
            &reading
                .read_versioned(actor, initial_b.view, ReadingMode::Fused)
                .await
                .unwrap()
                .unwrap()
        ),
        ["H"]
    );
    let (assets, uses): (i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM asset WHERE space_id=$1),(SELECT count(*) FROM block_asset_use WHERE space_id=$1 AND asset_id=$2)")
        .bind(space).bind(asset.reference.asset_id).fetch_one(&rig.admin_pool).await.unwrap();
    assert_eq!((assets, uses), (1, 1));
    assert_eq!(
        fs::read(files.root.join("assets").join(&asset.storage_key)).unwrap(),
        PNG
    );
    assert_eq!(
        fs::read_dir(files.root.join("assets/sha256").join(&asset.sha256[..2]))
            .unwrap()
            .count(),
        1
    );
}

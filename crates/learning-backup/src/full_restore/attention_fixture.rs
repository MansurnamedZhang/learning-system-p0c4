//! Task7 fixed Attention inputs and real domain fixture. Test-only, no restore authority.
#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]
use crate::BackupError;
use serde::{Deserialize, Serialize};

pub(crate) const PDF: &[u8] = include_bytes!("../../../learning-worker/examples/c3/attention.pdf");
pub(crate) const PNG: &[u8] = include_bytes!("../../../learning-worker/examples/c3/attention.png");
pub(crate) const PERSONAL_PNG: &[u8] =
    &[137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 0, 73, 69, 78, 68];
pub(crate) const NOTEBOOK: &[u8] =
    br#"{"cells":[{"cell_type":"code","source":["print(42)"]}],"nbformat":4}"#;
pub(crate) const UNLINKED: &[u8] = b"C4 Task7 unlinked ready original\n";
pub(crate) fn originals() -> [(&'static str, &'static [u8]); 5] {
    [
        ("attention.pdf", PDF),
        ("figure.png", PNG),
        ("analysis.ipynb", NOTEBOOK),
        ("personal.png", PERSONAL_PNG),
        ("unlinked.bin", UNLINKED),
    ]
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct HypothesisHistory {
    pub(crate) historical: u64,
    pub(crate) current: u64,
}
impl HypothesisHistory {
    pub(crate) fn require_complete(self) -> Result<(), BackupError> {
        if self.historical == 0 || self.current == 0 {
            Err(BackupError::Invalid(
                "attention historical-current fixture is incomplete",
            ))
        } else {
            Ok(())
        }
    }
}
#[path = "../../../learning-db/tests/support/relation_store.rs"]
mod relation_helpers;
/// Fixed source/target catalog difference. Source capture closes CONNECT,
/// while its existing restricted runtime TEMP grant remains in force.
/// The recovered target remains the original admin-only database contract.
pub(crate) fn closed_source_catalog(
    target: &serde_json::Value,
) -> Result<serde_json::Value, BackupError> {
    use serde_json::json;
    if target["database"]
        != json!({"owner":"learning_admin","acl":"{learning_admin=CTc/learning_admin}"})
    {
        return Err(BackupError::Invalid(
            "attention expected target database ACL",
        ));
    }
    let mut source = target.clone();
    source["database"] = json!({"owner":"learning_admin","acl":"{learning_admin=CTc/learning_admin,learning_runtime=T/learning_admin}"});
    let roles = source["roles"]
        .as_array_mut()
        .ok_or(BackupError::Invalid("attention catalog roles"))?;
    let mut count = 0;
    for role in roles {
        if role["name"] == "learning_snapshot_claim_broker" {
            if role["login"] != json!(false) {
                return Err(BackupError::Invalid(
                    "attention expected target broker login",
                ));
            }
            role["login"] = json!(true);
            count += 1;
        }
    }
    if count != 1 {
        return Err(BackupError::Invalid("attention exact broker role roster"));
    }
    Ok(source)
}
pub(crate) fn validate_typed_roster(roster: &serde_json::Value) -> Result<(), BackupError> {
    use learning_core::{AssetRef, BlockRef, ReadingRef};
    use uuid::Uuid;
    let bad = || BackupError::Invalid("attention typed identity roster");
    let h1: BlockRef =
        serde_json::from_value(roster["hypothesis_historical"].clone()).map_err(|_| bad())?;
    let h2: BlockRef =
        serde_json::from_value(roster["hypothesis_current"].clone()).map_err(|_| bad())?;
    let historical: ReadingRef =
        serde_json::from_value(roster["historical_reading"].clone()).map_err(|_| bad())?;
    let current: ReadingRef =
        serde_json::from_value(roster["current_reading"].clone()).map_err(|_| bad())?;
    let reuse: ReadingRef =
        serde_json::from_value(roster["reuse_reading"].clone()).map_err(|_| bad())?;
    let id = |key: &str| -> Result<Uuid, BackupError> {
        roster[key]
            .as_str()
            .and_then(|s| Uuid::parse_str(s).ok())
            .filter(|id| !id.is_nil())
            .ok_or_else(bad)
    };
    if id("actor")? == id("peer")?
        || id("space")? == id("source_space")?
        || h1.block_id != h2.block_id
        || h1.revision_id == h2.revision_id
        || historical.view_id != current.view_id
        || historical.revision_id == current.revision_id
        || reuse.view_id == historical.view_id
        || roster["expected_h1"] != "H@1: toy attention score improves"
        || roster["expected_h2"] != "H@2: narrower claim"
        || !roster["assets"].as_array().is_some_and(|a| a.len() == 6)
        || roster["shared_digest"] != crate::digest(PDF)
    {
        return Err(bad());
    }
    let assets: Vec<AssetRef> =
        serde_json::from_value(roster["assets"].clone()).map_err(|_| bad())?;
    let spaces = std::collections::BTreeSet::from([id("space")?, id("source_space")?]);
    let mut seen = std::collections::BTreeSet::new();
    for asset in assets {
        if asset.asset_id.is_nil()
            || !spaces.contains(&asset.space_id)
            || !seen.insert((asset.space_id, asset.asset_id))
        {
            return Err(bad());
        }
    }
    if [
        h1.block_id,
        h1.revision_id,
        h2.revision_id,
        historical.view_id,
        historical.revision_id,
        current.revision_id,
        reuse.view_id,
        reuse.revision_id,
    ]
    .contains(&Uuid::nil())
    {
        return Err(bad());
    }
    Ok(())
}
#[path = "../../../learning-db/tests/support/mod.rs"]
mod support;
pub(crate) mod live {
    use super::*;
    use super::{
        relation_helpers,
        support::{self, TestRig, assembly as a, reading as h},
    };
    use learning_assets::backup_fs::BackupDir;
    use learning_assets::{FsAssetStore, UploadDeclaration};
    use learning_core::*;
    use learning_db::{
        AssetIntegrityProcessor, AssetMedia, AssetProcessOutcome, AssetStore, JobFailureClass,
        JobStore, LineageStore, MigrationStore, RelationStore, ReviewStore, SnapshotImportStore,
        SnapshotStore, VersionedContentStore,
    };
    use serde_json::{Value, json};
    use sqlx::PgPool;
    use std::{
        collections::BTreeSet,
        io::{Read, Write},
        path::{Path, PathBuf},
        time::Duration,
    };
    use uuid::Uuid;
    pub(crate) fn require(ok: bool, why: &'static str) -> Result<(), BackupError> {
        if ok {
            Ok(())
        } else {
            Err(BackupError::Invalid(why))
        }
    }
    struct Originals {
        root: PathBuf,
        files: FsAssetStore,
    }
    impl Originals {
        async fn register(
            &self,
            rig: &TestRig,
            actor: Principal,
            space: Uuid,
            name: &str,
            media_type: &str,
            bytes: &[u8],
        ) -> learning_db::AssetRecord {
            let root = BackupDir::open_trusted_private_root(&self.root).unwrap();
            let mut file = root.create_file(name).unwrap();
            file.write_all(bytes).unwrap();
            file.sync_all().unwrap();
            root.sync().unwrap();
            let blob = self
                .files
                .put_from_file(
                    Uuid::new_v4(),
                    &self.root.join(name),
                    UploadDeclaration {
                        expected_size_bytes: bytes.len() as u64,
                        max_size_bytes: bytes.len() as u64,
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
        }
    }
    fn draft(
        intent: Intent,
        text: &str,
        basis_refs: Vec<ExactRef>,
        run: Option<BlockRef>,
    ) -> ContentDraft {
        ContentDraft::V2(ContentV2 {
            intent,
            language: "en".into(),
            title: "Synthetic Attention".into(),
            body: BodyV2::Text(TextPayload {
                format: TextFormat::Markdown,
                text: text.into(),
            }),
            basis_refs,
            requires_context: vec![],
            source_run: run,
        })
    }

    fn create(draft: ContentDraft) -> CreateContent {
        CreateContent {
            request_id: Uuid::new_v4(),
            draft,
            reason: "C1 Attention acceptance".into(),
        }
    }

    fn exact(revision: &ContentRevision) -> BlockRef {
        BlockRef {
            block_id: revision.block_id,
            revision_id: revision.revision_id,
        }
    }

    fn v3(body: BodyV3) -> ContentDraft {
        ContentDraft::V3(ContentV3 {
            intent: Intent::Note,
            language: "en".into(),
            title: "Attention original".into(),
            body,
            basis_refs: vec![],
            requires_context: vec![],
            source_run: None,
        })
    }

    fn labels(projection: &VersionedReadingProjection) -> Vec<String> {
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
                    ContentDraft::V2(content) => match &content.body {
                        BodyV2::Text(text) => text.text.clone(),
                        _ => panic!("unexpected v2 body"),
                    },
                    ContentDraft::V3(content) => match &content.body {
                        BodyV3::Figure { .. } => "FIG".into(),
                        BodyV3::Attachment { .. } => "ATT".into(),
                        _ => panic!("unexpected v3 body"),
                    },
                })
            })
            .collect()
    }

    async fn read_bytes(store: &AssetStore, actor: Principal, use_ref: AssetUseRef) -> Vec<u8> {
        let mut bytes = Vec::new();
        store
            .open_for_use(actor, use_ref)
            .await
            .unwrap()
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        bytes
    }

    #[derive(Clone)]
    pub(crate) struct SourceJob {
        pub(crate) id: Uuid,
        pub(crate) snapshot: bool,
        pub(crate) tokens: Vec<Uuid>,
    }
    pub(crate) struct AttentionFixture {
        pub(crate) actor: Principal,
        pub(crate) historical: ReadingRef,
        pub(crate) current: ReadingRef,
        pub(crate) reuse: ReadingRef,
        pub(crate) roster: Value,
        pub(crate) jobs: Vec<SourceJob>,
    }
    pub(crate) async fn seed_attention_source(
        admin: &PgPool,
        files: &FsAssetStore,
        proof_root: &Path,
        runtime: &PgPool,
    ) -> Result<AttentionFixture, BackupError> {
        let rig = TestRig {
            admin_pool: admin.clone(),
            runtime_pool: runtime.clone(),
            store: learning_db::ContentStore::new(runtime.clone()),
        };
        let input_root = BackupDir::open_trusted_private_root(proof_root)?
            .create_dir("attention-fixed-inputs")?;
        let _held_input = input_root;
        let originals = Originals {
            root: proof_root.join("attention-fixed-inputs"),
            files: files.clone(),
        };
        let (actor, space) = rig.seed_actor_space(true).await;
        let (peer, source_space) = rig.seed_actor_space(true).await;
        rig.grant(actor, source_space, false).await;
        rig.grant(peer, space, false).await;
        let assets = AssetStore::new(rig.runtime_pool.clone(), originals.files.clone());
        let content = VersionedContentStore::new(rig.runtime_pool.clone());
        let reading = h::store(&rig);

        let pdf = originals
            .register(&rig, actor, space, "attention.pdf", "application/pdf", PDF)
            .await;
        let png = originals
            .register(&rig, actor, space, "figure.png", "image/png", PNG)
            .await;
        let notebook = originals
            .register(
                &rig,
                actor,
                space,
                "analysis.ipynb",
                "application/x-ipynb+json",
                NOTEBOOK,
            )
            .await;
        let personal_png = originals
            .register(
                &rig,
                actor,
                space,
                "personal.png",
                "image/png",
                PERSONAL_PNG,
            )
            .await;
        let unlinked = originals
            .register(
                &rig,
                actor,
                space,
                "unlinked.bin",
                "application/octet-stream",
                UNLINKED,
            )
            .await;
        let shared = originals
            .register(
                &rig,
                peer,
                source_space,
                "shared-attention.pdf",
                "application/pdf",
                PDF,
            )
            .await;
        let mut resource_versions = Vec::new();
        let mut source_segments = Vec::new();
        for (asset, bytes, selector) in [
            (
                &pdf,
                PDF,
                json!({"page": 7, "region": [0.1, 0.2, 0.3, 0.4]}),
            ),
            (&png, PNG, json!({"region": [10, 20, 30, 40]})),
            (&notebook, NOTEBOOK, json!({"cell_id": "analysis-2"})),
        ] {
            let version = assets
                .link_resource_version(
                    actor,
                    learning_db::ResourceInput {
                        space_id: space,
                        resource_id: None,
                        display_name: asset.media.original_file_name.clone(),
                    },
                    asset.reference.clone(),
                )
                .await
                .unwrap();
            let segment = assets
                .add_source_segment(actor, version.clone(), selector.clone())
                .await
                .unwrap();
            resource_versions.push(version.clone());
            source_segments.push(segment.clone());
            let saved_selector: Value =
                sqlx::query_scalar("SELECT selector FROM source_segment WHERE id=$1")
                    .bind(segment.segment_id)
                    .fetch_one(&rig.admin_pool)
                    .await
                    .unwrap();
            assert_eq!(saved_selector, selector);
            assert_eq!(
                read_bytes(&assets, actor, AssetUseRef::Resource(version)).await,
                bytes
            );
        }

        let shared_version = assets
            .link_resource_version(
                peer,
                learning_db::ResourceInput {
                    space_id: source_space,
                    resource_id: None,
                    display_name: "Shared Attention PDF".into(),
                },
                shared.reference.clone(),
            )
            .await
            .unwrap();
        let shared_segment = assets
            .add_source_segment(peer, shared_version.clone(), json!({"page":1}))
            .await
            .unwrap();
        assert_eq!(
            read_bytes(
                &assets,
                actor,
                AssetUseRef::Resource(shared_version.clone())
            )
            .await,
            PDF
        );
        resource_versions.push(shared_version);
        source_segments.push(shared_segment);
        let unlinked_uses:i64=sqlx::query_scalar("SELECT (SELECT count(*) FROM public.resource_version WHERE asset_id=$1)+(SELECT count(*) FROM public.block_asset_use WHERE asset_id=$1)").bind(unlinked.reference.asset_id).fetch_one(admin).await?;
        require(
            unlinked_uses == 0,
            "attention unlinked ready asset unexpectedly linked",
        )?;
        let original = rig
            .store
            .create(actor, space, support::command("Attention lecture"))
            .await
            .unwrap();
        let original_ref = relation_helpers::exact(&original);
        let h1 = content
            .create(
                actor,
                space,
                create(draft(
                    Intent::Conjecture,
                    "H@1: toy attention score improves",
                    vec![ExactRef::Block(original_ref.clone())],
                    None,
                )),
            )
            .await
            .unwrap();
        let h1_ref = exact(&h1);
        let document_original = content
            .create(
                actor,
                space,
                create(v3(BodyV3::Attachment {
                    asset: pdf.reference.clone(),
                    display_name: "Attention original PDF".into(),
                })),
            )
            .await
            .unwrap();
        let document_original_ref = exact(&document_original);
        let figure = content
            .create(
                actor,
                space,
                create(v3(BodyV3::Figure {
                    asset: png.reference.clone(),
                    usage: "lecture_diagram".into(),
                    caption: "Attention mechanism".into(),
                    alt: "Query, key and value arrows".into(),
                    decorative: false,
                })),
            )
            .await
            .unwrap();
        let figure_ref = exact(&figure);
        let attachment = content
            .create(
                actor,
                space,
                create(v3(BodyV3::Attachment {
                    asset: notebook.reference.clone(),
                    display_name: "Analysis notebook".into(),
                })),
            )
            .await
            .unwrap();
        let attachment_ref = exact(&attachment);
        for (reference, asset_id, bytes) in [
            (&figure_ref, png.reference.asset_id, PNG),
            (&attachment_ref, notebook.reference.asset_id, NOTEBOOK),
        ] {
            let linked: Uuid = sqlx::query_scalar(
            "SELECT asset_id FROM block_asset_use WHERE space_id=$1 AND block_id=$2 AND revision_id=$3",
        )
        .bind(space)
        .bind(reference.block_id)
        .bind(reference.revision_id)
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap();
            assert_eq!(linked, asset_id);
            assert_eq!(
                read_bytes(&assets, actor, AssetUseRef::Block(reference.clone())).await,
                bytes
            );
        }
        rig.revoke(actor, space).await;
        assert_eq!(
            assets
                .read_for_use(actor, AssetUseRef::Block(attachment_ref.clone()))
                .await
                .unwrap(),
            None
        );
        assert!(
            assets
                .open_for_use(actor, AssetUseRef::Block(attachment_ref.clone()))
                .await
                .unwrap()
                .is_none()
        );
        rig.grant(actor, space, true).await;

        let doc_a = rig
            .compositions()
            .save(
                actor,
                space,
                a::doc(vec![
                    NodeTarget::Block(original_ref),
                    NodeTarget::Block(h1_ref.clone()),
                    NodeTarget::Block(attachment_ref.clone()),
                    NodeTarget::Block(figure_ref.clone()),
                    NodeTarget::Block(document_original_ref.clone()),
                ]),
            )
            .await
            .unwrap();
        let doc_b = rig
            .compositions()
            .save(
                actor,
                space,
                a::doc(vec![NodeTarget::Block(h1_ref.clone())]),
            )
            .await
            .unwrap();
        assert_ne!(doc_a.reference, doc_b.reference);
        assert_eq!(doc_a.nodes[1].target, doc_b.nodes[0].target);
        let zero = reading
            .create(actor, space, h::create(doc_a.reference.clone()))
            .await
            .unwrap();
        let second_reading = reading
            .create(actor, space, h::create(doc_b.reference.clone()))
            .await
            .unwrap();
        let with_note = reading
            .edit(
                actor,
                zero.overlay.overlay_id,
                h::edit(&zero, h::add(h::gap(&doc_a, 1), "N: personal note")),
            )
            .await
            .unwrap();
        let note_group = reading
            .state(actor, with_note.overlay.overlay_id)
            .await
            .unwrap()
            .unwrap()
            .editable
            .unwrap()
            .groups
            .into_iter()
            .next()
            .unwrap();

        let run = content
            .create(
                peer,
                source_space,
                create(draft(Intent::Observation, "R: toy run", vec![], None)),
            )
            .await
            .unwrap();
        let e1 = content
            .create(
                peer,
                source_space,
                create(draft(
                    Intent::Evidence,
                    "E1: score 0.7",
                    vec![],
                    Some(exact(&run)),
                )),
            )
            .await
            .unwrap();
        let e2 = content
            .create(
                peer,
                source_space,
                create(draft(
                    Intent::Evidence,
                    "E2: score 0.8",
                    vec![],
                    Some(exact(&run)),
                )),
            )
            .await
            .unwrap();
        let x = content
            .create(
                actor,
                space,
                create(draft(Intent::Evidence, "X: score 0.2", vec![], None)),
            )
            .await
            .unwrap();
        let groups = ReviewStore::new(rig.runtime_pool.clone())
            .group_sources(actor, vec![exact(&e1), exact(&e2), exact(&x)])
            .await
            .unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].source_run, Some(exact(&run)));
        let relations = RelationStore::new(rig.runtime_pool.clone());
        let scope = RelationScope::PersonalOverlay {
            space_id: space,
            overlay_id: zero.overlay.overlay_id,
        };
        let mut selections = Vec::new();
        for (evidence, relation_type) in [
            (&e1, RelationType::Supports),
            (&e2, RelationType::Supports),
            (&x, RelationType::Opposes),
        ] {
            let mut command = relation_helpers::save(space, exact(evidence), h1_ref.clone());
            command.scope = scope.clone();
            command.relation_type = relation_type;
            let saved = relations.save(actor, command).await.unwrap();
            let checked = relations
                .review(actor, relation_helpers::review(&saved))
                .await
                .unwrap();
            let loaded = relations
                .read(actor, saved.reference.clone())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(loaded.from, exact(evidence));
            assert_eq!(loaded.to, h1_ref);
            assert_eq!(loaded.relation_type, relation_type);
            selections.push(RelationSelection {
                relation: saved.reference,
                review: Some(checked.reference),
            });
        }
        let reviews = ReviewStore::new(rig.runtime_pool.clone());
        let judgment = reviews
            .append(
                actor,
                AppendEpistemicReview {
                    request_id: Uuid::new_v4(),
                    scope,
                    target: h1_ref.clone(),
                    expected_previous: None,
                    state: EpistemicState::Inconclusive,
                    relations: selections.clone(),
                    evidence: vec![exact(&e1), exact(&e2), exact(&x)],
                    conditions: "toy protocol P".into(),
                    explanation: "two reports share one run; X is counterevidence".into(),
                },
            )
            .await
            .unwrap();
        let ReviewProjection::Available(loaded_judgment) = reviews
            .read(actor, judgment.reference.clone())
            .await
            .unwrap()
        else {
            panic!("epistemic review must be readable");
        };
        assert_eq!(loaded_judgment.target, h1_ref);
        assert_eq!(loaded_judgment.state, EpistemicState::Inconclusive);
        assert_eq!(loaded_judgment.relations, selections);
        assert_eq!(
            loaded_judgment.evidence,
            vec![exact(&e1), exact(&e2), exact(&x)]
        );
        let conclusion = content
            .create(
                actor,
                space,
                create(draft(
                    Intent::Conclusion,
                    "C@1: inconclusive",
                    vec![
                        ExactRef::Block(h1_ref.clone()),
                        ExactRef::EpistemicReview(judgment.reference.clone()),
                    ],
                    None,
                )),
            )
            .await
            .unwrap();
        let conclusion_ref = exact(&conclusion);
        let with_conclusion = reading
            .edit(
                actor,
                zero.overlay.overlay_id,
                h::edit(
                    &with_note,
                    ReadingEdit::InsertExisting {
                        block: conclusion_ref.clone(),
                        target: InsertTarget::ExistingGroup {
                            group_id: note_group.group_id,
                            order: PlacementOrder {
                                left_placement_id: Some(note_group.placements[0].placement_id),
                                right_placement_id: None,
                            },
                        },
                    },
                ),
            )
            .await
            .unwrap();
        let personal_figure = content
            .create(
                actor,
                space,
                create(v3(BodyV3::Figure {
                    asset: personal_png.reference.clone(),
                    usage: "personal_diagram".into(),
                    caption: "Personal Attention sketch".into(),
                    alt: "Personal arrow".into(),
                    decorative: false,
                })),
            )
            .await
            .unwrap();
        let personal_figure_ref = exact(&personal_figure);
        let with_figure = reading
            .edit(
                actor,
                zero.overlay.overlay_id,
                h::edit(
                    &with_conclusion,
                    ReadingEdit::InsertExisting {
                        block: personal_figure_ref.clone(),
                        target: InsertTarget::NewGroup {
                            anchor: h::gap(&doc_a, 0),
                        },
                    },
                ),
            )
            .await
            .unwrap();
        let figure_group = reading
            .state(actor, with_figure.overlay.overlay_id)
            .await
            .unwrap()
            .unwrap()
            .editable
            .unwrap()
            .groups
            .into_iter()
            .find(|group| {
                group
                    .placements
                    .iter()
                    .any(|p| p.block == personal_figure_ref)
            })
            .unwrap();
        assert_eq!(
            figure_group.location,
            GroupLocation::Placed {
                anchor: h::gap(&doc_a, 0)
            }
        );
        let figure_placement = figure_group
            .placements
            .iter()
            .find(|placement| placement.block == personal_figure_ref)
            .unwrap()
            .placement_id;
        let moved = reading
            .edit(
                actor,
                zero.overlay.overlay_id,
                h::edit(
                    &with_figure,
                    ReadingEdit::Move {
                        placement_id: figure_placement,
                        target: InsertTarget::NewGroup {
                            anchor: h::gap(&doc_a, 2),
                        },
                    },
                ),
            )
            .await
            .unwrap();
        assert!(moved.changed_blocks.is_empty());
        let before_move = reading
            .read_versioned(actor, with_figure.view.clone(), ReadingMode::Fused)
            .await
            .unwrap()
            .unwrap();
        let after_move = reading
            .read_versioned(actor, moved.view.clone(), ReadingMode::Fused)
            .await
            .unwrap()
            .unwrap();
        let moved_figure_group = reading
            .state(actor, moved.overlay.overlay_id)
            .await
            .unwrap()
            .unwrap()
            .editable
            .unwrap()
            .groups
            .into_iter()
            .find(|group| {
                group
                    .placements
                    .iter()
                    .any(|p| p.placement_id == figure_placement)
            })
            .unwrap();
        assert_eq!(
            moved_figure_group.location,
            GroupLocation::Placed {
                anchor: h::gap(&doc_a, 2)
            }
        );
        assert_eq!(
            moved_figure_group
                .placements
                .iter()
                .find(|p| p.placement_id == figure_placement)
                .unwrap()
                .block,
            personal_figure_ref
        );
        assert_eq!(
            labels(&before_move),
            [
                "FIG",
                "Attention lecture",
                "N: personal note",
                "C@1: inconclusive",
                "H@1: toy attention score improves",
                "ATT",
                "FIG",
                "ATT"
            ]
        );
        assert_eq!(
            labels(&after_move),
            [
                "Attention lecture",
                "N: personal note",
                "C@1: inconclusive",
                "H@1: toy attention score improves",
                "FIG",
                "ATT",
                "FIG",
                "ATT"
            ]
        );
        assert_eq!(
            labels(
                &reading
                    .read_versioned(actor, second_reading.view.clone(), ReadingMode::Fused)
                    .await
                    .unwrap()
                    .unwrap()
            ),
            ["H@1: toy attention score improves"]
        );
        let selected = reading
            .select_relations(
                actor,
                zero.overlay.overlay_id,
                SelectRelations {
                    request_id: Uuid::new_v4(),
                    expected_overlay_revision: moved.overlay.revision_id,
                    expected_reading_view_revision: moved.view.revision_id,
                    selections: selections.clone(),
                    epistemic_reviews: vec![judgment.reference.clone()],
                    reason: "fixed human evidence".into(),
                },
            )
            .await
            .unwrap();
        let fixed = reading
            .read_versioned(actor, selected.view.clone(), ReadingMode::Fused)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fixed.evidence.selections.len(), 3);
        assert_eq!(fixed.evidence.epistemic_reviews.len(), 1);
        assert!(labels(&fixed).contains(&"FIG".into()));
        assert!(labels(&fixed).contains(&"ATT".into()));

        let release = rig
            .releases()
            .publish_evidence(
                actor,
                space,
                PublishEvidence {
                    request_id: Uuid::new_v4(),
                    roots: vec![a::root(&doc_a, None), a::root(&doc_b, None)],
                    readings: vec![selected.view.clone()],
                    reason: "fixed C1 Attention".into(),
                },
            )
            .await
            .unwrap();
        let published = rig
            .releases()
            .read_evidence(actor, release.release_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(published.readings, vec![selected.view.clone()]);
        let fixed_reading_count: i64 = sqlx::query_scalar("SELECT count(*) FROM release_reading WHERE release_id=$1 AND view_id=$2 AND view_revision_id=$3")
        .bind(release.release_id)
        .bind(selected.view.view_id)
        .bind(selected.view.revision_id)
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap();
        assert_eq!(fixed_reading_count, 1);
        // Same dependency revocation/restoration boundary as the B3 Attention
        // scenario: no hidden source ID or body may escape partial projections.
        rig.revoke(actor, source_space).await;
        for reference in [exact(&e1), exact(&e2), conclusion_ref.clone()] {
            assert_eq!(content.read(actor, reference).await.unwrap(), None);
        }
        assert_eq!(
            reviews
                .read(actor, judgment.reference.clone())
                .await
                .unwrap(),
            ReviewProjection::Incomplete
        );
        let visible_relations = relations
            .read_selected(
                actor,
                selections.iter().map(|r| r.relation.clone()).collect(),
            )
            .await
            .unwrap();
        assert_eq!(visible_relations.len(), 1);
        assert_eq!(visible_relations[0].from, exact(&x));
        assert_eq!(visible_relations[0].to, h1_ref);
        let hidden = reading
            .read_versioned(actor, selected.view.clone(), ReadingMode::Fused)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(hidden.evidence.selections.len(), 1);
        assert_eq!(
            hidden.evidence.epistemic_reviews,
            vec![ReviewProjection::Incomplete]
        );
        let hidden_wire = serde_json::to_string(&hidden).unwrap();
        for secret in [
            e1.block_id.to_string(),
            e1.revision_id.to_string(),
            e2.block_id.to_string(),
            conclusion.block_id.to_string(),
            judgment.reference.review_id.to_string(),
            selections[0].relation.relation_id.to_string(),
            source_space.to_string(),
            "E1: score 0.7".into(),
            "E2: score 0.8".into(),
            "C@1: inconclusive".into(),
        ] {
            assert!(!hidden_wire.contains(&secret));
        }
        assert_eq!(
            rig.releases()
                .read_evidence(actor, release.release_id)
                .await
                .unwrap(),
            None
        );
        rig.grant(actor, source_space, false).await;
        let restored_reading = reading
            .read_versioned(actor, selected.view.clone(), ReadingMode::Fused)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            learning_core::canonical_json(&serde_json::to_value(restored_reading).unwrap()),
            learning_core::canonical_json(&serde_json::to_value(&fixed).unwrap())
        );
        assert_eq!(
            rig.releases()
                .read_evidence(actor, release.release_id)
                .await
                .unwrap()
                .unwrap(),
            published
        );
        let h2 = content
            .revise(
                actor,
                h1_ref.block_id,
                ReviseContent {
                    request_id: Uuid::new_v4(),
                    base_revision_id: h1_ref.revision_id,
                    draft: draft(
                        Intent::Conjecture,
                        "H@2: narrower claim",
                        vec![ExactRef::Block(relation_helpers::exact(&original))],
                        None,
                    ),
                    reason: "new claim".into(),
                },
            )
            .await
            .unwrap();
        assert_ne!(h2.revision_id, h1_ref.revision_id);
        assert_eq!(
            labels(
                &reading
                    .read_versioned(actor, selected.view.clone(), ReadingMode::Fused)
                    .await
                    .unwrap()
                    .unwrap()
            ),
            labels(&fixed)
        );
        for (reference, expected) in [
            (&h1_ref, 1_i64),
            (&exact(&h2), 0),
            (&figure_ref, 1),
            (&attachment_ref, 1),
        ] {
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM release_manifest_object WHERE release_id=$1 AND kind='block' AND object_id=$2 AND revision_id=$3")
            .bind(release.release_id)
            .bind(reference.block_id)
            .bind(reference.revision_id)
            .fetch_one(&rig.admin_pool)
            .await
            .unwrap();
            assert_eq!(count, expected);
        }
        assert_eq!(
            read_bytes(&assets, actor, AssetUseRef::Block(attachment_ref.clone())).await,
            NOTEBOOK
        );

        let split = LineageStore::new(rig.runtime_pool.clone())
            .apply(
                actor,
                space,
                LineageCommand {
                    request_id: Uuid::new_v4(),
                    operation: LineageOperation::Split,
                    inputs: vec![conclusion_ref.clone()],
                    outputs: vec![
                        draft(Intent::Note, "C part A", vec![], None),
                        draft(Intent::Note, "C part B", vec![], None),
                    ],
                    reason: "human split".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(split.outputs.len(), 2);
        assert_eq!(split.system_relations().len(), 2);
        for (index, output) in split.outputs.iter().enumerate() {
            let loaded = content.read(actor, output.clone()).await.unwrap().unwrap();
            assert_eq!(
                loaded.draft,
                draft(
                    Intent::Note,
                    if index == 0 { "C part A" } else { "C part B" },
                    vec![],
                    None
                )
            );
        }
        for (index, relation) in split.system_relations().iter().enumerate() {
            assert_eq!(relation.from, split.outputs[index]);
            assert_eq!(relation.to, conclusion_ref);
            assert_eq!(
                serde_json::to_value(relation.relation_type).unwrap(),
                json!("split_from")
            );
        }
        assert!(labels(&fixed).contains(&"C@1: inconclusive".into()));

        // A later document revision removes the old anchors. Keep both personal
        // groups explicitly unplaced; the historical released view stays fixed.
        let mut revised_doc = a::edit(&doc_a);
        revised_doc.nodes = vec![NodeDraft {
            occurrence_id: None,
            target: NodeTarget::Block(exact(&h2)),
        }];
        let new_doc = rig
            .compositions()
            .save(actor, space, revised_doc)
            .await
            .unwrap();
        let migration = MigrationStore::new(rig.runtime_pool.clone());
        let proposal = migration
            .propose(
                actor,
                selected.overlay.overlay_id,
                ProposeMigration {
                    request_id: Uuid::new_v4(),
                    expected_overlay_revision: selected.overlay.revision_id,
                    expected_reading_view_revision: selected.view.revision_id,
                    target: new_doc.reference,
                    reason: "new reading base".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(proposal.groups.len(), 2);
        let adopted = migration
            .decide(
                actor,
                selected.overlay.overlay_id,
                DecideMigration {
                    request_id: Uuid::new_v4(),
                    proposal_id: proposal.proposal_id,
                    expected_overlay_revision: selected.overlay.revision_id,
                    expected_reading_view_revision: selected.view.revision_id,
                    action: MigrationAction::Adopt {
                        groups: proposal
                            .groups
                            .iter()
                            .map(|group| GroupDecision::KeepUnplaced {
                                group_id: group.group_id,
                            })
                            .collect(),
                        merges: vec![],
                    },
                    reason: "retain personal material".into(),
                },
            )
            .await
            .unwrap()
            .adopted
            .unwrap();
        let unplaced = reading
            .read_versioned(actor, adopted.view.clone(), ReadingMode::Fused)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(labels(&unplaced), vec!["H@2: narrower claim"]);
        assert_eq!(unplaced.unplaced.len(), 3);
        assert!(unplaced.unplaced.iter().all(|item| item.location.is_none()));
        let unplaced_refs: BTreeSet<BlockRef> = unplaced
            .unplaced
            .iter()
            .map(|item| exact(&item.revision))
            .collect();
        assert_eq!(
            unplaced_refs,
            BTreeSet::from([
                note_group.placements[0].block.clone(),
                conclusion_ref.clone(),
                personal_figure_ref.clone()
            ])
        );
        assert_eq!(
            labels(
                &reading
                    .read_versioned(actor, selected.view.clone(), ReadingMode::Fused)
                    .await
                    .unwrap()
                    .unwrap()
            ),
            labels(&fixed)
        );

        // All seeded blocks are either in the two fixed readings, the source-run
        // closure, lineage or this explicit supported-job edge reading.
        let mut edge_blocks = Vec::new();
        for label in ["queued", "failed", "cancelled"] {
            let revision = content
                .create(
                    actor,
                    space,
                    create(v3(BodyV3::Figure {
                        asset: png.reference.clone(),
                        usage: format!("job_{label}"),
                        caption: format!("Attention {label} job edge"),
                        alt: label.into(),
                        decorative: false,
                    })),
                )
                .await
                .unwrap();
            edge_blocks.push(exact(&revision));
        }
        let edge_doc = rig
            .compositions()
            .save(
                actor,
                space,
                a::doc(edge_blocks.iter().cloned().map(NodeTarget::Block).collect()),
            )
            .await
            .unwrap();
        let edge_reading = reading
            .create(actor, space, h::create(edge_doc.reference.clone()))
            .await
            .unwrap();
        for (reference, expected) in [
            (&h1_ref, "H@1: toy attention score improves"),
            (&exact(&h2), "H@2: narrower claim"),
        ] {
            let loaded = content
                .read(actor, reference.clone())
                .await
                .unwrap()
                .unwrap();
            require(
                loaded.draft
                    == draft(
                        Intent::Conjecture,
                        expected,
                        vec![ExactRef::Block(relation_helpers::exact(&original))],
                        None,
                    ),
                "attention fixed hypothesis body",
            )?;
        }
        // A read grant on the dependency space must not authorize a write there.
        require(
            matches!(
                content
                    .create(
                        actor,
                        source_space,
                        create(draft(
                            Intent::Note,
                            "forbidden dependency-space write",
                            vec![],
                            None
                        ))
                    )
                    .await,
                Err(ContentError::NotFound)
            ),
            "attention read grant allowed write",
        )?;
        for reference in [
            &selected.view,
            &adopted.view,
            &second_reading.view,
            &edge_reading.view,
        ] {
            for mode in [
                ReadingMode::Original,
                ReadingMode::Personal,
                ReadingMode::Fused,
            ] {
                require(
                    reading
                        .read_versioned(actor, reference.clone(), mode)
                        .await
                        .unwrap()
                        .is_some(),
                    "attention owner reading missing",
                )?;
                require(
                    reading
                        .read_versioned(peer, reference.clone(), mode)
                        .await
                        .unwrap()
                        .is_none(),
                    "attention granted nonowner reading leaked",
                )?;
            }
        }
        // An honest exact C3 package of existing rows generates a genuine import
        // receipt. It is not the full inventory and cannot authorize full restore.
        let exact_request = SnapshotRequest {
            reading: adopted.view.clone(),
            mode: ReadingMode::Fused,
            include_personal: true,
            include_originals: true,
            resource_versions: resource_versions.clone(),
            source_segments: source_segments.clone(),
        };
        let exact_plan = SnapshotStore::new(runtime.clone())
            .plan_exact(actor, &exact_request)
            .await
            .map_err(|_| BackupError::Invalid("attention exact import plan"))?;
        let import_root = BackupDir::open_trusted_private_root(proof_root)?
            .create_dir("attention-exact-import")?;
        let _held_import = import_root;
        let package = learning_assets::stage_snapshot(
            &proof_root.join("attention-exact-import"),
            Uuid::new_v4(),
            &exact_plan.manifest,
            &exact_plan.rows,
            &exact_plan.assets,
            files,
        )
        .map_err(|_| BackupError::Invalid("attention exact import package"))?;
        let importer = SnapshotImportStore::new(runtime.clone(), files.clone());
        let prepared = importer
            .validate_exact(actor, &package)
            .await
            .map_err(|_| BackupError::Invalid("attention exact import preflight"))?;
        let import_request = Uuid::new_v4();
        let import_receipt = importer
            .import_exact(actor, import_request, prepared)
            .await
            .map_err(|_| BackupError::Invalid("attention exact import receipt"))?;
        require(
            !import_receipt.reused,
            "attention first exact import was already seen",
        )?;
        let prepared = importer
            .validate_exact(actor, &package)
            .await
            .map_err(|_| BackupError::Invalid("attention exact replay preflight"))?;
        require(
            importer
                .import_exact(actor, import_request, prepared)
                .await
                .map_err(|_| BackupError::Invalid("attention exact replay"))?
                .reused,
            "attention exact import replay not reused",
        )?;
        let mut jobs = Vec::new();
        let job_store = JobStore::new(runtime.clone());
        require(
            job_store
                .dispatch_pending(256)
                .await
                .map_err(|_| BackupError::Invalid("attention C2 dispatch"))?
                == 7,
            "attention exact C2 roster",
        )?;
        let blocks = [
            document_original_ref.clone(),
            figure_ref.clone(),
            attachment_ref.clone(),
            personal_figure_ref.clone(),
            edge_blocks[0].clone(),
            edge_blocks[1].clone(),
            edge_blocks[2].clone(),
        ];
        for (index, block) in blocks.into_iter().enumerate() {
            let key = JobInput::asset_integrity(actor.actor_id, space, block)
                .map_err(|_| BackupError::Invalid("attention C2 input"))?
                .business_key();
            let id: Uuid = sqlx::query_scalar("SELECT id FROM public.job WHERE idempotency_key=$1")
                .bind(key)
                .fetch_one(admin)
                .await?;
            let mut tokens = Vec::new();
            if !matches!(index, 4 | 6) {
                let attempts = if index == 2 { 3 } else { 1 };
                for attempt in 1..=attempts {
                    let lease = job_store
                        .claim(id, Duration::from_secs(30))
                        .await
                        .map_err(|_| BackupError::Invalid("attention C2 claim"))?
                        .ok_or(BackupError::Invalid("attention C2 claim absent"))?;
                    tokens.push(lease.token);
                    if attempt < attempts {
                        sqlx::query("UPDATE public.job SET lease_expires_at=clock_timestamp()-interval '1 second' WHERE id=$1").bind(id).execute(admin).await?;
                    } else {
                        match index {
                            0 => {
                                require(
                                    AssetIntegrityProcessor::new(assets.clone(), job_store.clone())
                                        .process(&lease)
                                        .await
                                        .map_err(|_| {
                                            BackupError::Invalid(
                                                "attention real integrity processor",
                                            )
                                        })?
                                        == AssetProcessOutcome::Succeeded,
                                    "attention result-v1 absent",
                                )?;
                            }
                            2 => {
                                sqlx::query("UPDATE public.job SET lease_expires_at=clock_timestamp()-interval '1 second' WHERE id=$1").bind(id).execute(admin).await?;
                            }
                            3 | 5 => {
                                require(
                                    job_store
                                        .fail(
                                            id,
                                            lease.token,
                                            if index == 3 {
                                                JobFailureClass::TransientStorage
                                            } else {
                                                JobFailureClass::InvalidInput
                                            },
                                        )
                                        .await
                                        .map_err(|_| {
                                            BackupError::Invalid("attention C2 failure edge")
                                        })?,
                                    "attention C2 failure not applied",
                                )?;
                            }
                            _ => {}
                        }
                    }
                }
            } else if index == 6 {
                require(
                    job_store
                        .cancel(id)
                        .await
                        .map_err(|_| BackupError::Invalid("attention C2 cancel"))?,
                    "attention C2 cancellation absent",
                )?;
            }
            jobs.push(SourceJob {
                id,
                snapshot: false,
                tokens,
            });
        }
        let mut fixed_drafts = vec![
            (
                relation_helpers::exact(&original),
                ContentDraft::V1(support::command("Attention lecture").draft),
            ),
            (
                h1_ref.clone(),
                draft(
                    Intent::Conjecture,
                    "H@1: toy attention score improves",
                    vec![ExactRef::Block(relation_helpers::exact(&original))],
                    None,
                ),
            ),
            (
                exact(&h2),
                draft(
                    Intent::Conjecture,
                    "H@2: narrower claim",
                    vec![ExactRef::Block(relation_helpers::exact(&original))],
                    None,
                ),
            ),
            (
                exact(&run),
                draft(Intent::Observation, "R: toy run", vec![], None),
            ),
            (
                exact(&e1),
                draft(Intent::Evidence, "E1: score 0.7", vec![], Some(exact(&run))),
            ),
            (
                exact(&e2),
                draft(Intent::Evidence, "E2: score 0.8", vec![], Some(exact(&run))),
            ),
            (
                exact(&x),
                draft(Intent::Evidence, "X: score 0.2", vec![], None),
            ),
            (
                conclusion_ref.clone(),
                draft(
                    Intent::Conclusion,
                    "C@1: inconclusive",
                    vec![
                        ExactRef::Block(h1_ref.clone()),
                        ExactRef::EpistemicReview(judgment.reference.clone()),
                    ],
                    None,
                ),
            ),
            (
                note_group.placements[0].block.clone(),
                ContentDraft::V1(support::command("N: personal note").draft),
            ),
            (
                figure_ref.clone(),
                v3(BodyV3::Figure {
                    asset: png.reference.clone(),
                    usage: "lecture_diagram".into(),
                    caption: "Attention mechanism".into(),
                    alt: "Query, key and value arrows".into(),
                    decorative: false,
                }),
            ),
            (
                personal_figure_ref.clone(),
                v3(BodyV3::Figure {
                    asset: personal_png.reference.clone(),
                    usage: "personal_diagram".into(),
                    caption: "Personal Attention sketch".into(),
                    alt: "Personal arrow".into(),
                    decorative: false,
                }),
            ),
            (
                attachment_ref.clone(),
                v3(BodyV3::Attachment {
                    asset: notebook.reference.clone(),
                    display_name: "Analysis notebook".into(),
                }),
            ),
            (
                document_original_ref.clone(),
                v3(BodyV3::Attachment {
                    asset: pdf.reference.clone(),
                    display_name: "Attention original PDF".into(),
                }),
            ),
        ];
        for (i, output) in split.outputs.iter().enumerate() {
            fixed_drafts.push((
                output.clone(),
                draft(
                    Intent::Note,
                    if i == 0 { "C part A" } else { "C part B" },
                    vec![],
                    None,
                ),
            ));
        }
        for (reference, label) in edge_blocks.iter().zip(["queued", "failed", "cancelled"]) {
            fixed_drafts.push((
                reference.clone(),
                v3(BodyV3::Figure {
                    asset: png.reference.clone(),
                    usage: format!("job_{label}"),
                    caption: format!("Attention {label} job edge"),
                    alt: label.into(),
                    decorative: false,
                }),
            ));
        }
        for (reference, expected) in &fixed_drafts {
            let actual = content
                .read(actor, reference.clone())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(&actual.draft, expected);
        }
        let roster = json!({"actor":actor.actor_id,"peer":peer.actor_id,"space":space,"source_space":source_space,
      "hypothesis_historical":h1_ref,"hypothesis_current":exact(&h2),"historical_reading":selected.view,"current_reading":adopted.view,"reuse_reading":second_reading.view,
      "compositions":[doc_a.reference,doc_b.reference],"original_figure":figure_ref,"personal_figure":personal_figure_ref,"attachment":attachment_ref,
      "source_run":exact(&run),"evidence":[exact(&e1),exact(&e2),exact(&x)],"relations":selections,"epistemic_review":judgment.reference,
      "conclusion":conclusion_ref,"release_id":release.release_id,"lineage_operation":split.operation_id,"lineage_outputs":split.outputs,
      "migration_proposal":proposal.proposal_id,"unplaced":unplaced_refs,"resources":resource_versions,"segments":source_segments,
      "assets":[pdf.reference,png.reference,notebook.reference,personal_png.reference,unlinked.reference,shared.reference],
      "unlinked_ready":unlinked.reference,"shared_digest":pdf.sha256,"import_request":import_request,"import_manifest_sha256":import_receipt.manifest_sha256,
      "job_edge_reading":edge_reading.view,"expected_h1":"H@1: toy attention score improves","expected_h2":"H@2: narrower claim",
      "fixed_drafts":fixed_drafts,"dependency_revocation":{"content_denied":3,"review_incomplete":1,"visible_relation_assertions":1,"release_denied":true,"hidden_wire_checked":true,"restored_exact_reading_and_release":true},"reachability":"EVERY_ID_FIXED_TYPED_API_PROBED_OR_INTENTIONALLY_DENIED","functions_and_acl":"SEPARATE_CLOSED_SOURCE_INVENTORY"});
        super::validate_typed_roster(&roster)?;
        Ok(AttentionFixture {
            actor,
            historical: selected.view,
            current: adopted.view,
            reuse: second_reading.view,
            roster,
            jobs,
        })
    }
}
#[cfg(target_os = "linux")]
pub(crate) fn validate_preseed(files: &learning_assets::FsAssetStore) -> Result<(), BackupError> {
    use std::{collections::BTreeMap, io::Read};
    let (root, stage) = files
        .backup_root_handles()
        .map_err(|_| BackupError::Invalid("attention original roots"))?;
    if root.list_bounded(2)? != vec!["sha256"] || !stage.list_bounded(1)?.is_empty() {
        return Err(BackupError::Invalid("attention fixed original roots"));
    }
    let sha = root.open_dir("sha256")?;
    let mut expected = BTreeMap::new();
    for (_, bytes) in originals() {
        let hash = crate::digest(bytes);
        expected.insert(hash[..2].to_owned(), hash);
    }
    let mut names = sha.list_bounded(6)?;
    names.sort();
    if names != expected.keys().cloned().collect::<Vec<_>>() {
        return Err(BackupError::Invalid("attention fixed original shards"));
    }
    for (_, bytes) in originals() {
        let hash = crate::digest(bytes);
        let shard = sha.open_dir(&hash[..2])?;
        if shard.list_bounded(2)? != vec![hash.clone()] {
            return Err(BackupError::Invalid("attention fixed original files"));
        }
        let mut read = files
            .open_record(
                &format!("sha256/{}/{hash}", &hash[..2]),
                &hash,
                bytes.len() as i64,
            )
            .map_err(|_| BackupError::Invalid("attention fixed original hash"))?;
        let mut actual = Vec::new();
        read.read_to_end(&mut actual)?;
        if actual != bytes {
            return Err(BackupError::Invalid("attention fixed original bytes"));
        }
    }
    Ok(())
}
#[cfg(target_os = "linux")]
pub(crate) async fn observe_closed_source(
    conn: &mut sqlx::PgConnection,
    roles: &[u8],
    config: &crate::SourceBackupConfig,
) -> Result<(), BackupError> {
    crate::restore_preflight::target_binding::controlled_import::full::attention_tests::observe_closed_source(conn,roles,config).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn attention_history_requires_both_actual_revisions() {
        for (historical, current) in [(0, 0), (1, 0), (0, 1)] {
            assert!(
                HypothesisHistory {
                    historical,
                    current
                }
                .require_complete()
                .is_err()
            );
        }
        assert!(
            HypothesisHistory {
                historical: 1,
                current: 1
            }
            .require_complete()
            .is_ok()
        );
    }
    fn roster() -> serde_json::Value {
        use serde_json::json;
        let a = uuid::Uuid::from_u128(1);
        let b = uuid::Uuid::from_u128(2);
        let c = uuid::Uuid::from_u128(3);
        let d = uuid::Uuid::from_u128(4);
        json!({"actor":a,"peer":b,"space":c,"source_space":d,"hypothesis_historical":{"block_id":a,"revision_id":b},"hypothesis_current":{"block_id":a,"revision_id":c},"historical_reading":{"view_id":b,"revision_id":c},"current_reading":{"view_id":b,"revision_id":d},"reuse_reading":{"view_id":a,"revision_id":d},"assets":[{"space_id":c,"asset_id":a},{"space_id":c,"asset_id":b},{"space_id":c,"asset_id":c},{"space_id":c,"asset_id":d},{"space_id":d,"asset_id":a},{"space_id":d,"asset_id":b}],"expected_h1":"H@1: toy attention score improves","expected_h2":"H@2: narrower claim","shared_digest":crate::digest(PDF)})
    }
    #[test]
    fn typed_roster_refuses_history_alias_and_missing_identity() {
        let valid = roster();
        validate_typed_roster(&valid).unwrap();
        for key in ["hypothesis_current", "current_reading", "reuse_reading"] {
            let mut bad = valid.clone();
            bad[key] = valid[if key == "hypothesis_current" {
                "hypothesis_historical"
            } else {
                "historical_reading"
            }]
            .clone();
            assert!(validate_typed_roster(&bad).is_err());
        }
        let mut bad = valid;
        bad.as_object_mut().unwrap().remove("actor");
        assert!(validate_typed_roster(&bad).is_err());
    }
    #[test]
    fn typed_roster_refuses_permission_space_and_asset_drift() {
        for key in [
            "peer",
            "source_space",
            "expected_h1",
            "shared_digest",
            "assets",
        ] {
            let mut bad = roster();
            bad[key] = serde_json::json!(null);
            assert!(validate_typed_roster(&bad).is_err());
        }
    }
    #[test]
    fn closed_source_catalog_preserves_exact_temp_without_connect() {
        use serde_json::json;
        let target = json!({"database":{"owner":"learning_admin","acl":"{learning_admin=CTc/learning_admin}"},"roles":[{"name":"learning_snapshot_claim_broker","login":false}],"other_guard":{"untouched":true}});
        let source = closed_source_catalog(&target).unwrap();
        assert_eq!(
            source["database"],
            json!({"owner":"learning_admin","acl":"{learning_admin=CTc/learning_admin,learning_runtime=T/learning_admin}"})
        );
        assert_eq!(source["roles"][0]["login"], true);
        assert_eq!(source["other_guard"], target["other_guard"]);
        for acl in [
            "{learning_admin=CTc/learning_admin,learning_runtime=Tc/learning_admin}",
            "{learning_admin=CTc/learning_admin,=T/learning_admin}",
            "{learning_admin=CTc/postgres}",
        ] {
            let mut bad = target.clone();
            bad["database"]["acl"] = json!(acl);
            assert!(closed_source_catalog(&bad).is_err());
        }
        let mut bad = target;
        bad["roles"][0]["login"] = json!(true);
        assert!(closed_source_catalog(&bad).is_err());
    }
    #[test]
    fn closed_source_acl_matches_frozen_native_template_with_only_connect_removed() {
        let native: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../tests/fixtures/c4-full-schema-c0-d1-v1/acl.json"
        ))
        .unwrap();
        let source = &native["catalog"]["database"];
        assert_eq!(source["owner"], "learning_admin");
        assert_eq!(
            source["acl"],
            "{learning_admin=CTc/learning_admin,learning_runtime=Tc/learning_admin,learning_snapshot_claim_broker=c/learning_admin}"
        );
        let mut target = native["catalog"].clone();
        target["database"] = serde_json::json!({"owner":"learning_admin","acl":"{learning_admin=CTc/learning_admin}"});
        for r in target["roles"].as_array_mut().unwrap() {
            if r["name"] == "learning_snapshot_claim_broker" {
                r["login"] = serde_json::json!(false);
            }
        }
        assert_eq!(
            closed_source_catalog(&target).unwrap()["database"]["acl"],
            "{learning_admin=CTc/learning_admin,learning_runtime=T/learning_admin}"
        );
    }
    #[test]
    fn fixed_originals_have_exact_physical_and_logical_budget() {
        assert_eq!(originals().len(), 5);
        assert_eq!(originals().iter().map(|(_, b)| b.len()).sum::<usize>(), 601);
        assert_eq!(
            crate::digest(PDF),
            "a2b4945b6565b105cbf8ab4dc1d86bfc925d1e3eaa91cfb6ea770d292c333d28"
        );
    }
}

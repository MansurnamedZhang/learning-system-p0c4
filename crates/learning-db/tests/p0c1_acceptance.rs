//! One C1 Attention fixture joins original bytes, exact uses, personal
//! placement, human evidence, fixed history and a later migration decision.
#[path = "support/relation_store.rs"]
mod relation_helpers;
mod support;

use learning_assets::{FsAssetStore, UploadDeclaration};
use learning_core::*;
use learning_db::{
    AssetMedia, AssetStore, LineageStore, MigrationStore, RelationStore, ReviewStore,
    VersionedContentStore,
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, io::Read, path::PathBuf};
use support::{TestRig, assembly as a, reading as h};
use uuid::Uuid;

const PDF: &[u8] = b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\n%%EOF\n";
const PNG: &[u8] = &[137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 0, 73, 69, 78, 68];
const NOTEBOOK: &[u8] = br#"{"cells":[{"cell_type":"code","source":["print(42)"]}],"nbformat":4}"#;

struct Originals {
    root: PathBuf,
    files: FsAssetStore,
}

impl Originals {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("p0c1-attention-{}", Uuid::new_v4()));
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
    ) -> learning_db::AssetRecord {
        let path = self.root.join(name);
        fs::write(&path, bytes).unwrap();
        let blob = self
            .files
            .put_from_file(
                Uuid::new_v4(),
                &path,
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
    }
}

impl Drop for Originals {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
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

#[tokio::test]
async fn attention_originals_figure_attachment_evidence_and_fixed_history_share_one_fixture() {
    let rig = TestRig::from_env().await;
    let originals = Originals::new();
    let (actor, space) = rig.seed_actor_space(true).await;
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
            actor,
            space,
            create(draft(Intent::Observation, "R: toy run", vec![], None)),
        )
        .await
        .unwrap();
    let e1 = content
        .create(
            actor,
            space,
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
            actor,
            space,
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
    let with_figure = reading
        .edit(
            actor,
            zero.overlay.overlay_id,
            h::edit(
                &with_conclusion,
                ReadingEdit::InsertExisting {
                    block: figure_ref.clone(),
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
        .find(|group| group.placements.iter().any(|p| p.block == figure_ref))
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
        .find(|placement| placement.block == figure_ref)
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
        figure_ref
    );
    assert_eq!(
        labels(&before_move),
        [
            "FIG",
            "Attention lecture",
            "N: personal note",
            "C@1: inconclusive",
            "H@1: toy attention score improves",
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
            "ATT"
        ]
    );
    assert_eq!(
        labels(
            &reading
                .read_versioned(actor, second_reading.view, ReadingMode::Fused)
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
        read_bytes(&assets, actor, AssetUseRef::Block(attachment_ref)).await,
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
    assert!(labels(&fixed).contains(&"C@1: inconclusive".into()));

    // A later document revision removes the old anchors. Keep both personal
    // groups explicitly unplaced; the historical released view stays fixed.
    let mut revised_doc = a::edit(&doc_a);
    revised_doc.nodes.clear();
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
        .read_versioned(actor, adopted.view, ReadingMode::Fused)
        .await
        .unwrap()
        .unwrap();
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
            conclusion_ref,
            figure_ref
        ])
    );
    assert_eq!(
        labels(
            &reading
                .read_versioned(actor, selected.view, ReadingMode::Fused)
                .await
                .unwrap()
                .unwrap()
        ),
        labels(&fixed)
    );
}

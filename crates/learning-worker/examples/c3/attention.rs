#![allow(dead_code)]
use super::{relations, support};
use learning_core::*;
use learning_db::{
    MigrationStore, RelationStore, ReviewStore, SnapshotPlan, SnapshotStore, VersionedContentStore,
};
use serde_json::{Value, json};
use support::{TestRig, assembly as a, reading as h};
use uuid::Uuid;
pub fn request(view: ReadingRef) -> SnapshotRequest {
    SnapshotRequest {
        reading: view,
        mode: ReadingMode::Fused,
        include_personal: true,
        include_originals: true,
        resource_versions: vec![],
        source_segments: vec![],
    }
}
fn rows(plan: &SnapshotPlan, table: SnapshotTable) -> Vec<&Value> {
    plan.rows
        .iter()
        .filter(|r| r.table == table)
        .map(|r| &r.immutable_values)
        .collect()
}
fn includes(plan: &SnapshotPlan, table: SnapshotTable, id: Uuid) -> bool {
    rows(plan, table).iter().any(|r| r["id"] == json!(id))
}
fn exact(r: &ContentRevision) -> BlockRef {
    BlockRef {
        block_id: r.block_id,
        revision_id: r.revision_id,
    }
}
fn v2(basis: Vec<BlockRef>) -> ContentDraft {
    ContentDraft::V2(ContentV2 {
        intent: Intent::Note,
        language: "en".into(),
        title: "derived".into(),
        body: BodyV2::Text(support::command("body").draft.payload),
        basis_refs: basis.into_iter().map(ExactRef::Block).collect(),
        requires_context: vec![],
        source_run: None,
    })
}
fn conjecture_v2() -> ContentDraft {
    let mut draft = v2(vec![]);
    if let ContentDraft::V2(content) = &mut draft {
        content.intent = Intent::Conjecture;
    }
    draft
}
async fn content(
    r: &TestRig,
    actor: Principal,
    space: Uuid,
    draft: ContentDraft,
) -> ContentRevision {
    VersionedContentStore::new(r.runtime_pool.clone())
        .create(
            actor,
            space,
            CreateContent {
                request_id: Uuid::new_v4(),
                reason: "snapshot fixture".into(),
                draft,
            },
        )
        .await
        .unwrap()
}
async fn asset(r: &TestRig, actor: Principal, space: Uuid, pdf: bool) -> AssetRef {
    let files = super::files();
    let bytes: &[u8] = if pdf {
        include_bytes!("attention.pdf")
    } else {
        include_bytes!("attention.png")
    };
    let path = super::control().join(if pdf { "input.pdf" } else { "input.png" });
    std::fs::write(&path, bytes).unwrap();
    let blob = files
        .put_from_file(
            Uuid::new_v4(),
            &path,
            learning_assets::UploadDeclaration {
                expected_size_bytes: bytes.len() as u64,
                max_size_bytes: 1_000_000,
            },
        )
        .unwrap();
    let record = learning_db::AssetStore::new(r.runtime_pool.clone(), files)
        .register_verified(
            actor,
            space,
            Uuid::new_v4(),
            blob,
            learning_db::AssetMedia {
                media_type: if pdf { "application/pdf" } else { "image/png" }.into(),
                original_file_name: if pdf {
                    "attention.pdf"
                } else {
                    "attention.png"
                }
                .into(),
            },
        )
        .await
        .unwrap();
    record.reference
}
async fn resource(r: &TestRig, asset: &AssetRef) -> (ResourceVersionRef, SourceSegmentRef) {
    let resource = Uuid::new_v4();
    let version = Uuid::new_v4();
    let segment = Uuid::new_v4();
    sqlx::query("INSERT INTO resource(space_id,id,display_name) VALUES($1,$2,'lecture')")
        .bind(asset.space_id)
        .bind(resource)
        .execute(&r.admin_pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO resource_version(space_id,resource_id,id,asset_id,version_no) VALUES($1,$2,$3,$4,1)")
        .bind(asset.space_id).bind(resource).bind(version).bind(asset.asset_id).execute(&r.admin_pool).await.unwrap();
    sqlx::query("INSERT INTO source_segment(space_id,resource_id,resource_version_id,id,selector) VALUES($1,$2,$3,$4,$5)")
        .bind(asset.space_id).bind(resource).bind(version).bind(segment).bind(json!({"page": 1, "region": [1,2,3,4]})).execute(&r.admin_pool).await.unwrap();
    (
        ResourceVersionRef {
            space_id: asset.space_id,
            resource_id: resource,
            version_id: version,
        },
        SourceSegmentRef {
            space_id: asset.space_id,
            resource_id: resource,
            version_id: version,
            segment_id: segment,
        },
    )
}

pub async fn attention() -> (TestRig, Principal, Uuid, SnapshotPlan, SnapshotRequest) {
    let (r, actor, space, doc, zero) = h::fixture().await;
    let one = h::store(&r)
        .edit(
            actor,
            zero.overlay.overlay_id,
            h::edit(
                &zero,
                ReadingEdit::InsertNew {
                    drafts: vec![
                        support::command("Attention note one").draft,
                        support::command("Attention note two").draft,
                    ],
                    target: InsertTarget::NewGroup {
                        anchor: h::gap(&doc, 1),
                    },
                },
            ),
        )
        .await
        .unwrap();
    let basis = one.changed_blocks[0].clone();
    let first = content(&r, actor, space, v2(vec![basis.clone()])).await;
    let revised = VersionedContentStore::new(r.runtime_pool.clone())
        .revise(
            actor,
            first.block_id,
            ReviseContent {
                request_id: Uuid::new_v4(),
                base_revision_id: first.revision_id,
                draft: conjecture_v2(),
                reason: "new revision".into(),
            },
        )
        .await
        .unwrap();
    let bytes = asset(&r, actor, space, false).await;
    let mut uses = Vec::new();
    for body in [
        BodyV3::Figure {
            asset: bytes.clone(),
            usage: "diagram".into(),
            caption: "figure".into(),
            alt: "diagram".into(),
            decorative: false,
        },
        BodyV3::Attachment {
            asset: bytes.clone(),
            display_name: "source".into(),
        },
    ] {
        let value = content(
            &r,
            actor,
            space,
            ContentDraft::V3(ContentV3 {
                intent: Intent::Note,
                language: "en".into(),
                title: "asset".into(),
                body,
                basis_refs: vec![ExactRef::Block(exact(&revised))],
                requires_context: vec![],
                source_run: None,
            }),
        )
        .await;
        uses.push(exact(&value));
    }
    let mut changed = a::edit(&doc);
    changed.nodes = uses
        .iter()
        .map(|b| NodeDraft {
            occurrence_id: None,
            target: NodeTarget::Block(b.clone()),
        })
        .collect();
    let next = r.compositions().save(actor, space, changed).await.unwrap();
    let migration = MigrationStore::new(r.runtime_pool.clone());
    let proposal = migration
        .propose(
            actor,
            one.overlay.overlay_id,
            ProposeMigration {
                request_id: Uuid::new_v4(),
                expected_overlay_revision: one.overlay.revision_id,
                expected_reading_view_revision: one.view.revision_id,
                target: next.reference.clone(),
                reason: "changed source".into(),
            },
        )
        .await
        .unwrap();
    let two = migration
        .decide(
            actor,
            one.overlay.overlay_id,
            DecideMigration {
                request_id: Uuid::new_v4(),
                expected_overlay_revision: one.overlay.revision_id,
                expected_reading_view_revision: one.view.revision_id,
                proposal_id: proposal.proposal_id,
                action: MigrationAction::Adopt {
                    groups: proposal
                        .groups
                        .iter()
                        .map(|g| GroupDecision::KeepUnplaced {
                            group_id: g.group_id,
                        })
                        .collect(),
                    merges: vec![],
                },
                reason: "preserve old gap".into(),
            },
        )
        .await
        .unwrap()
        .adopted
        .unwrap();
    let two = h::store(&r)
        .edit(
            actor,
            two.overlay.overlay_id,
            h::edit(
                &two,
                ReadingEdit::PlaceUnplaced {
                    group_id: proposal.groups[0].group_id,
                    anchor: h::gap(&next, 0),
                    merge_into: None,
                    merged_order: vec![],
                },
            ),
        )
        .await
        .unwrap();
    let before = h::store(&r)
        .state(actor, two.overlay.overlay_id)
        .await
        .unwrap()
        .unwrap()
        .editable
        .unwrap();
    let mut source_edit = a::edit(&next);
    source_edit.nodes.reverse();
    let latest = r
        .compositions()
        .save(actor, space, source_edit)
        .await
        .unwrap();
    let unplaced = migration
        .propose(
            actor,
            two.overlay.overlay_id,
            ProposeMigration {
                request_id: Uuid::new_v4(),
                expected_overlay_revision: two.overlay.revision_id,
                expected_reading_view_revision: two.view.revision_id,
                target: latest.reference.clone(),
                reason: "retain original gap".into(),
            },
        )
        .await
        .unwrap();
    let two = migration
        .decide(
            actor,
            two.overlay.overlay_id,
            DecideMigration {
                request_id: Uuid::new_v4(),
                expected_overlay_revision: two.overlay.revision_id,
                expected_reading_view_revision: two.view.revision_id,
                proposal_id: unplaced.proposal_id,
                action: MigrationAction::Adopt {
                    groups: unplaced
                        .groups
                        .iter()
                        .map(|g| GroupDecision::KeepUnplaced {
                            group_id: g.group_id,
                        })
                        .collect(),
                    merges: vec![],
                },
                reason: "keep currently unplaced".into(),
            },
        )
        .await
        .unwrap()
        .adopted
        .unwrap();
    let after = h::store(&r)
        .state(actor, two.overlay.overlay_id)
        .await
        .unwrap()
        .unwrap()
        .editable
        .unwrap();
    assert_eq!(before.groups.len(), 1);
    assert_eq!(after.groups.len(), 1);
    assert_eq!(before.groups[0].group_id, after.groups[0].group_id);
    assert_eq!(
        before.groups[0].location.anchor(),
        after.groups[0].location.anchor()
    );
    assert_eq!(before.groups[0].placements.len(), 2);
    assert_eq!(before.groups[0].placements, after.groups[0].placements);
    assert!(!after.groups[0].location.placed());
    let relation_store = RelationStore::new(r.runtime_pool.clone());
    let first_relation = relation_store
        .save(
            actor,
            relations::save(space, basis.clone(), exact(&revised)),
        )
        .await
        .unwrap();
    let mut relation_revision = relations::edit(&first_relation);
    relation_revision.rationale = "revised rationale".into();
    let rel = relation_store.save(actor, relation_revision).await.unwrap();
    let old_review = relation_store
        .review(actor, relations::review(&rel))
        .await
        .unwrap();
    let mut review = relations::review(&rel);
    review.expected_previous = Some(old_review.reference.review_id);
    review.explanation = "checked again".into();
    let reviewed = relation_store.review(actor, review).await.unwrap();
    let epistemic = ReviewStore::new(r.runtime_pool.clone())
        .append(
            actor,
            AppendEpistemicReview {
                request_id: Uuid::new_v4(),
                scope: RelationScope::Space { space_id: space },
                target: exact(&revised),
                expected_previous: None,
                state: EpistemicState::Testing,
                relations: vec![RelationSelection {
                    relation: rel.reference.clone(),
                    review: Some(reviewed.reference.clone()),
                }],
                evidence: vec![basis.clone()],
                conditions: "scope".into(),
                explanation: "judgment".into(),
            },
        )
        .await
        .unwrap();
    let epistemic = ReviewStore::new(r.runtime_pool.clone())
        .append(
            actor,
            AppendEpistemicReview {
                request_id: Uuid::new_v4(),
                scope: RelationScope::Space { space_id: space },
                target: exact(&revised),
                expected_previous: Some(epistemic.reference),
                state: EpistemicState::SupportedWithinScope,
                relations: vec![RelationSelection {
                    relation: rel.reference.clone(),
                    review: Some(reviewed.reference.clone()),
                }],
                evidence: vec![basis],
                conditions: "synthetic Attention example only".into(),
                explanation: "reviewed support history".into(),
            },
        )
        .await
        .unwrap();
    let three = h::store(&r)
        .select_relations(
            actor,
            two.overlay.overlay_id,
            SelectRelations {
                request_id: Uuid::new_v4(),
                expected_overlay_revision: two.overlay.revision_id,
                expected_reading_view_revision: two.view.revision_id,
                selections: vec![RelationSelection {
                    relation: rel.reference.clone(),
                    review: Some(reviewed.reference.clone()),
                }],
                epistemic_reviews: vec![epistemic.reference.clone()],
                reason: "select evidence".into(),
            },
        )
        .await
        .unwrap();
    // Real excluded source history exists; the target must not receive it.
    r.releases()
        .publish(actor, space, a::publish(vec![a::root(&latest, None)]))
        .await
        .unwrap();
    learning_db::LineageStore::new(r.runtime_pool.clone())
        .apply(
            actor,
            space,
            LineageCommand {
                request_id: Uuid::new_v4(),
                operation: LineageOperation::Derive,
                inputs: vec![exact(&revised)],
                outputs: vec![ContentDraft::V1(
                    support::command("excluded lineage output").draft,
                )],
                reason: "explicit source provenance".into(),
            },
        )
        .await
        .unwrap();
    for table in [
        "release",
        "lineage_operation",
        "mutation_receipt",
        "job_outbox",
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM public.{table}"))
            .fetch_one(&r.admin_pool)
            .await
            .unwrap();
        assert!(count > 0, "source exclusion fixture must contain {table}");
        eprintln!("c3_source_excluded table={table} count={count}");
    }
    let pdf = asset(&r, actor, space, true).await;
    let (version, segment) = resource(&r, &pdf).await;
    let mut input = request(three.view.clone());
    input.resource_versions.push(version);
    input.source_segments.push(segment);
    let plan = SnapshotStore::new(r.runtime_pool.clone())
        .plan_exact(actor, &input)
        .await
        .unwrap();
    assert_eq!(
        plan.rows
            .iter()
            .map(|r| r.table)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        29
    );
    assert_eq!(
        plan.rows
            .iter()
            .filter(|r| r.table == SnapshotTable::BlockAssetUse)
            .count(),
        2
    );
    assert!(
        plan.rows.iter().any(
            |r| r.table == SnapshotTable::OverlayGroup && r.immutable_values["placed"] == false
        )
    );
    let projection = h::store(&r)
        .read_versioned(actor, three.view.clone(), ReadingMode::Fused)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(projection.unplaced.len(), 2);
    assert_eq!(
        projection.unplaced[0].placement_id,
        before.groups[0].placements[0].placement_id
    );
    assert_eq!(
        projection
            .unplaced
            .iter()
            .map(|p| p.placement_id)
            .collect::<Vec<_>>(),
        before.groups[0]
            .placements
            .iter()
            .map(|p| p.placement_id)
            .collect::<Vec<_>>()
    );
    (r, actor, space, plan, input)
}

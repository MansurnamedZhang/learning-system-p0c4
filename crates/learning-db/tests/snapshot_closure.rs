#[path = "support/relation_store.rs"]
mod relations;
mod support;

use learning_core::*;
use learning_db::{
    MigrationStore, RelationStore, ReviewStore, SnapshotPlan, SnapshotStore, VersionedContentStore,
};
use serde_json::{Value, json};
use support::{TestRig, assembly as a, reading as h};
use uuid::Uuid;

fn request(view: ReadingRef) -> SnapshotRequest {
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
async fn asset(r: &TestRig, space: Uuid) -> AssetRef {
    asset_with_size(r, space, 17).await
}
async fn asset_with_size(r: &TestRig, space: Uuid, byte_size: i64) -> AssetRef {
    let id = Uuid::new_v4();
    let sha = hex_digest(b"snapshot original");
    sqlx::query("INSERT INTO asset(space_id,id,sha256,byte_size,storage_key,media_type,original_file_name,status) VALUES($1,$2,$3,$5,$4,'application/octet-stream','original.bin','ready')")
        .bind(space).bind(id).bind(&sha).bind(format!("sha256/{}/{}", &sha[..2], sha)).bind(byte_size).execute(&r.admin_pool).await.unwrap();
    AssetRef {
        space_id: space,
        asset_id: id,
    }
}

// Planning reads authorized immutable metadata, not original streams. Staging
// and destination preflight retain their separate real-byte hash/size checks.
#[tokio::test]
async fn metadata_only_plan_allows_large_original_but_included_plan_rejects_it() {
    let (r, actor, space, _, saved) = h::fixture().await;
    let size = SNAPSHOT_MAX_ASSET_FILE_BYTES as u64 + 1;
    let original = asset_with_size(&r, space, size as i64).await;
    let (version, _) = resource(&r, &original).await;
    let mut input = request(saved.view);
    input.resource_versions.push(version);
    input.include_originals = false;
    let store = SnapshotStore::new(r.runtime_pool.clone());
    let before = h::counts(&r, actor).await;

    let plan = store.plan_exact(actor, &input).await.unwrap();
    assert!(plan.manifest.requires_destination_assets);
    assert_eq!(plan.assets.len(), 1);
    assert_eq!(plan.assets[0].asset, original);
    assert_eq!(plan.assets[0].byte_size, size);
    let assets = rows(&plan, SnapshotTable::Asset);
    assert_eq!(assets.len(), 1);
    assert_eq!(assets[0]["byte_size"], json!(size));
    assert!(
        plan.manifest
            .files
            .iter()
            .all(|f| !f.path.starts_with("assets/"))
    );

    // Identical source and selected resource; only package inclusion changes.
    input.include_originals = true;
    assert!(matches!(
        store.plan_exact(actor, &input).await,
        Err(ContentError::Invalid(code)) if code == "snapshot_limit_exceeded"
    ));
    assert_eq!(h::counts(&r, actor).await, before);
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
        .bind(asset.space_id).bind(resource).bind(version).bind(segment).bind(json!({"page": 3, "region": [1,2,3,4]})).execute(&r.admin_pool).await.unwrap();
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

// A planner that uses a reading projection instead of immutable rows loses old
// anchors, selection identities, predecessor evidence and registry/index rows.
#[tokio::test]
async fn exact_rows_preserve_ancestors_unplaced_anchors_evidence_and_two_asset_uses() {
    let (r, actor, space, doc, zero) = h::fixture().await;
    let one = h::store(&r)
        .edit(
            actor,
            zero.overlay.overlay_id,
            h::edit(&zero, h::add(h::gap(&doc, 1), "my note")),
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
    let bytes = asset(&r, space).await;
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
                evidence: vec![basis],
                conditions: "scope".into(),
                explanation: "judgment".into(),
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
    let plan = SnapshotStore::new(r.runtime_pool.clone())
        .plan_exact(actor, &request(three.view.clone()))
        .await
        .unwrap();
    for (table, id) in [
        (SnapshotTable::ReadingViewRevision, zero.view.revision_id),
        (SnapshotTable::ReadingViewRevision, three.view.revision_id),
        (SnapshotTable::OverlayRevision, one.overlay.revision_id),
        (SnapshotTable::BlockRevision, first.revision_id),
        (SnapshotTable::BlockRevision, revised.revision_id),
        (
            SnapshotTable::RelationRevision,
            first_relation.reference.revision_id,
        ),
        (SnapshotTable::RelationRevision, rel.reference.revision_id),
        (
            SnapshotTable::CompositionRevision,
            doc.reference.revision_id,
        ),
        (
            SnapshotTable::RelationReview,
            old_review.reference.review_id,
        ),
        (
            SnapshotTable::EpistemicReview,
            epistemic.reference.review_id,
        ),
    ] {
        assert!(includes(&plan, table, id), "missing {table:?}");
    }
    let unplaced = rows(&plan, SnapshotTable::OverlayGroup)
        .into_iter()
        .find(|g| g["placed"] == false)
        .unwrap();
    assert_eq!(
        unplaced["base_revision_id"],
        json!(doc.reference.revision_id)
    );
    assert_eq!(unplaced["left_id"], json!(doc.nodes[0].occurrence_id));
    for table in [
        SnapshotTable::OverlayGroupIdentity,
        SnapshotTable::OverlayPlacementIdentity,
        SnapshotTable::OverlayPlacement,
        SnapshotTable::ReferenceObject,
        SnapshotTable::ReferenceDependency,
        SnapshotTable::ReadingRelationSelection,
        SnapshotTable::ReadingEpistemicSelection,
        SnapshotTable::RelationReviewHead,
    ] {
        assert!(!rows(&plan, table).is_empty(), "missing {table:?}");
    }
    assert_eq!(rows(&plan, SnapshotTable::Asset).len(), 1);
    assert_eq!(rows(&plan, SnapshotTable::BlockAssetUse).len(), 2);
    assert_eq!(plan.assets.len(), 2);
    assert_eq!(plan.manifest.root, three.view);
    for row in &plan.rows {
        assert_eq!(row.sha256, canonical_record_hash(&row.immutable_values));
        assert!(row.immutable_values.get("head_revision_id").is_none());
        assert!(row.immutable_values.get("head_review_id").is_none());
        assert!(row.immutable_values.get("last_release_id").is_none());
        if let Some(timestamp) = row.immutable_values.get("created_at") {
            let value = timestamp.as_str().unwrap();
            assert!(value.ends_with('Z'));
            assert_eq!(value.split('.').nth(1).unwrap().len(), 7);
        }
    }
    let again = SnapshotStore::new(r.runtime_pool.clone())
        .plan_exact(actor, &request(plan.manifest.root.clone()))
        .await
        .unwrap();
    assert_eq!(plan.rows, again.rows);
    assert_eq!(plan.manifest, again.manifest);
}

#[tokio::test]
async fn hidden_ancestor_basis_rejects_even_when_current_reading_is_visible() {
    let (r, actor, space, _, _) = h::fixture().await;
    let (other, foreign) = r.seed_actor_space(true).await;
    r.grant(actor, foreign, false).await;
    let secret = r
        .store
        .create(other, foreign, support::command("secret basis"))
        .await
        .unwrap();
    let old = content(&r, actor, space, v2(vec![relations::exact(&secret)])).await;
    let current = VersionedContentStore::new(r.runtime_pool.clone())
        .revise(
            actor,
            old.block_id,
            ReviseContent {
                request_id: Uuid::new_v4(),
                base_revision_id: old.revision_id,
                draft: v2(vec![]),
                reason: "no current basis".into(),
            },
        )
        .await
        .unwrap();
    let doc = r
        .compositions()
        .save(
            actor,
            space,
            a::doc(vec![NodeTarget::Block(exact(&current))]),
        )
        .await
        .unwrap();
    let saved = h::store(&r)
        .create(actor, space, h::create(doc.reference))
        .await
        .unwrap();
    let store = SnapshotStore::new(r.runtime_pool.clone());
    store
        .plan_exact(actor, &request(saved.view.clone()))
        .await
        .unwrap();
    r.revoke(actor, foreign).await;
    assert!(
        h::store(&r)
            .read_versioned(actor, saved.view.clone(), ReadingMode::Fused)
            .await
            .unwrap()
            .is_some()
    );
    assert!(matches!(
        store.plan_exact(actor, &request(saved.view)).await,
        Err(ContentError::NotFound)
    ));
}

#[tokio::test]
async fn resource_and_segment_roots_are_explicit_authorized_and_never_reverse_selected_by_digest() {
    let (r, actor, _, _, saved) = h::fixture().await;
    let (_, foreign) = r.seed_actor_space(true).await;
    r.grant(actor, foreign, false).await;
    let bytes = asset(&r, foreign).await;
    let (selected, segment) = resource(&r, &bytes).await;
    let (unselected, _) = resource(&r, &bytes).await;
    let store = SnapshotStore::new(r.runtime_pool.clone());
    let mut command = request(saved.view);
    assert!(
        rows(
            &store.plan_exact(actor, &command).await.unwrap(),
            SnapshotTable::Asset
        )
        .is_empty()
    );
    command.source_segments.push(segment.clone());
    let plan = store.plan_exact(actor, &command).await.unwrap();
    assert!(includes(
        &plan,
        SnapshotTable::ResourceVersion,
        selected.version_id
    ));
    assert!(includes(
        &plan,
        SnapshotTable::SourceSegment,
        segment.segment_id
    ));
    assert!(!includes(
        &plan,
        SnapshotTable::Resource,
        unselected.resource_id
    ));
    assert_eq!(plan.assets.len(), 1);
    command.resource_versions.push(selected);
    assert_eq!(
        store.plan_exact(actor, &command).await.unwrap().rows,
        plan.rows
    );
    r.revoke(actor, foreign).await;
    assert!(matches!(
        store.plan_exact(actor, &command).await,
        Err(ContentError::NotFound)
    ));
}

#[tokio::test]
async fn original_display_mode_does_not_silently_remove_personal_exact_data() {
    let (r, actor, _, doc, saved) = h::fixture().await;
    let saved = h::store(&r)
        .edit(
            actor,
            saved.overlay.overlay_id,
            h::edit(&saved, h::add(h::gap(&doc, 0), "private note")),
        )
        .await
        .unwrap();
    let store = SnapshotStore::new(r.runtime_pool.clone());
    let mut command = request(saved.view);
    command.mode = ReadingMode::Original;
    let plan = store.plan_exact(actor, &command).await.unwrap();
    assert!(!rows(&plan, SnapshotTable::OverlayPlacement).is_empty());
    command.include_personal = false;
    assert!(
        matches!(store.plan_exact(actor, &command).await, Err(ContentError::Invalid(ref code)) if code == "snapshot_not_exact")
    );
}

#[tokio::test]
async fn ancestor_and_current_bodies_share_one_package_budget() {
    let (r, actor, space, _, _) = h::fixture().await;
    let text = "x".repeat(190_000);
    let mut tx = r.runtime_pool.begin().await.unwrap();
    let mut roots = vec![];
    for _ in 0..24 {
        roots.push(
            support::references::seed_content(
                &mut tx,
                actor,
                space,
                ContentDraft::V1(support::command(&text).draft),
                None,
            )
            .await,
        );
    }
    tx.commit().await.unwrap();
    let initial = r
        .compositions()
        .save(
            actor,
            space,
            a::doc(roots.into_iter().map(NodeTarget::Block).collect()),
        )
        .await
        .unwrap();
    let mut tx = r.runtime_pool.begin().await.unwrap();
    let mut roots = vec![];
    for _ in 0..24 {
        roots.push(
            support::references::seed_content(
                &mut tx,
                actor,
                space,
                ContentDraft::V1(support::command(&text).draft),
                None,
            )
            .await,
        );
    }
    tx.commit().await.unwrap();
    let mut changed = a::edit(&initial);
    changed.nodes = roots
        .into_iter()
        .map(|r| NodeDraft {
            occurrence_id: None,
            target: NodeTarget::Block(r),
        })
        .collect();
    let current = r.compositions().save(actor, space, changed).await.unwrap();
    let saved = h::store(&r)
        .create(actor, space, h::create(current.reference))
        .await
        .unwrap();
    assert!(
        matches!(SnapshotStore::new(r.runtime_pool.clone()).plan_exact(actor, &request(saved.view)).await,
        Err(ContentError::Invalid(ref code)) if code == "snapshot_limit_exceeded" || code == "reference_budget_exceeded")
    );
}

#[tokio::test]
async fn personal_scope_without_a_fixed_overlay_revision_is_not_exportable() {
    let (r, actor, space, doc, saved) = h::fixture().await;
    let unrelated = h::store(&r)
        .create(actor, space, h::create(doc.reference.clone()))
        .await
        .unwrap();
    let NodeTarget::Block(from) = doc.nodes[0].target.clone() else {
        panic!()
    };
    let NodeTarget::Block(to) = doc.nodes[1].target.clone() else {
        panic!()
    };
    let mut command = relations::save(space, from, to);
    command.scope = RelationScope::PersonalOverlay {
        space_id: space,
        overlay_id: unrelated.overlay.overlay_id,
    };
    let relation = RelationStore::new(r.runtime_pool.clone())
        .save(actor, command)
        .await
        .unwrap();
    let dependent = content(
        &r,
        actor,
        space,
        ContentDraft::V2(ContentV2 {
            intent: Intent::Note,
            language: "en".into(),
            title: "personal evidence".into(),
            body: BodyV2::RelationView {
                selections: vec![RelationSelection {
                    relation: relation.reference,
                    review: None,
                }],
            },
            basis_refs: vec![],
            requires_context: vec![],
            source_run: None,
        }),
    )
    .await;
    let saved = h::store(&r)
        .edit(
            actor,
            saved.overlay.overlay_id,
            h::edit(
                &saved,
                ReadingEdit::InsertExisting {
                    block: exact(&dependent),
                    target: InsertTarget::NewGroup {
                        anchor: h::gap(&doc, 0),
                    },
                },
            ),
        )
        .await
        .unwrap();
    assert!(matches!(
        SnapshotStore::new(r.runtime_pool.clone())
            .plan_exact(actor, &request(saved.view))
            .await,
        Err(ContentError::NotFound)
    ));
}

#[tokio::test]
async fn hidden_epistemic_predecessor_and_selected_relation_are_rejected() {
    let (r, actor, space, doc, saved) = h::fixture().await;
    let (other, foreign) = r.seed_actor_space(true).await;
    // can_write=false grants read access to the evidence until revoke below.
    r.grant(actor, foreign, false).await;
    let secret = r
        .store
        .create(other, foreign, support::command("old private evidence"))
        .await
        .unwrap();
    let target = content(&r, actor, space, conjecture_v2()).await;
    let reviews = ReviewStore::new(r.runtime_pool.clone());
    let mut command = AppendEpistemicReview {
        request_id: Uuid::new_v4(),
        scope: RelationScope::Space { space_id: space },
        target: exact(&target),
        expected_previous: None,
        state: EpistemicState::Testing,
        relations: vec![],
        evidence: vec![relations::exact(&secret)],
        conditions: "".into(),
        explanation: "old".into(),
    };
    let old = reviews.append(actor, command.clone()).await.unwrap();
    command.request_id = Uuid::new_v4();
    command.expected_previous = Some(old.reference);
    command.evidence.clear();
    command.explanation = "current".into();
    let current = reviews.append(actor, command).await.unwrap();
    let selected = h::store(&r)
        .select_relations(
            actor,
            saved.overlay.overlay_id,
            SelectRelations {
                request_id: Uuid::new_v4(),
                expected_overlay_revision: saved.overlay.revision_id,
                expected_reading_view_revision: saved.view.revision_id,
                selections: vec![],
                epistemic_reviews: vec![current.reference.clone()],
                reason: "fixed review".into(),
            },
        )
        .await
        .unwrap();
    let store = SnapshotStore::new(r.runtime_pool.clone());
    store
        .plan_exact(actor, &request(selected.view.clone()))
        .await
        .unwrap();
    r.revoke(actor, foreign).await;
    let visible = h::store(&r)
        .read_versioned(actor, selected.view.clone(), ReadingMode::Fused)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(visible.source, SourceProjection::Available { .. }));
    assert!(matches!(visible.evidence.epistemic_reviews.as_slice(),
        [ReviewProjection::Available(review)]
        if review.reference == current.reference && review.previous.is_none() && review.evidence.is_empty()));
    assert!(matches!(
        store.plan_exact(actor, &request(selected.view)).await,
        Err(ContentError::NotFound)
    ));
    // A currently selected relation also fails uniformly after its own grant is revoked.
    r.grant(actor, foreign, false).await;
    let another = r
        .store
        .create(other, foreign, support::command("another"))
        .await
        .unwrap();
    let relation = RelationStore::new(r.runtime_pool.clone())
        .save(
            other,
            relations::save(
                foreign,
                relations::exact(&secret),
                relations::exact(&another),
            ),
        )
        .await
        .unwrap();
    let reading = h::store(&r)
        .create(actor, space, h::create(doc.reference))
        .await
        .unwrap();
    let selected = h::store(&r)
        .select_relations(
            actor,
            reading.overlay.overlay_id,
            SelectRelations {
                request_id: Uuid::new_v4(),
                expected_overlay_revision: reading.overlay.revision_id,
                expected_reading_view_revision: reading.view.revision_id,
                selections: vec![RelationSelection {
                    relation: relation.reference,
                    review: None,
                }],
                epistemic_reviews: vec![],
                reason: "selected".into(),
            },
        )
        .await
        .unwrap();
    r.revoke(actor, foreign).await;
    assert!(matches!(
        store.plan_exact(actor, &request(selected.view)).await,
        Err(ContentError::NotFound)
    ));
}

#[tokio::test]
async fn later_heads_and_review_heads_do_not_enter_a_fixed_plan() {
    let (r, actor, space, doc, saved) = h::fixture().await;
    let NodeTarget::Block(from) = doc.nodes[0].target.clone() else {
        panic!()
    };
    let NodeTarget::Block(to) = doc.nodes[1].target.clone() else {
        panic!()
    };
    let relations = RelationStore::new(r.runtime_pool.clone());
    let original = relations
        .save(actor, relations::save(space, from, to))
        .await
        .unwrap();
    let review = relations
        .review(actor, relations::review(&original))
        .await
        .unwrap();
    let selected = h::store(&r)
        .select_relations(
            actor,
            saved.overlay.overlay_id,
            SelectRelations {
                request_id: Uuid::new_v4(),
                expected_overlay_revision: saved.overlay.revision_id,
                expected_reading_view_revision: saved.view.revision_id,
                selections: vec![RelationSelection {
                    relation: original.reference.clone(),
                    review: Some(review.reference.clone()),
                }],
                epistemic_reviews: vec![],
                reason: "selected".into(),
            },
        )
        .await
        .unwrap();
    let store = SnapshotStore::new(r.runtime_pool.clone());
    let before = store
        .plan_exact(actor, &request(selected.view.clone()))
        .await
        .unwrap();
    let mut command = relations::edit(&original);
    command.rationale = "new head".into();
    let later = relations.save(actor, command).await.unwrap();
    let mut command = relations::review(&original);
    command.expected_previous = Some(review.reference.review_id);
    command.explanation = "new review head".into();
    let later_review = relations.review(actor, command).await.unwrap();
    let after = store
        .plan_exact(actor, &request(selected.view))
        .await
        .unwrap();
    assert_eq!(before.rows, after.rows);
    assert!(!includes(
        &after,
        SnapshotTable::RelationRevision,
        later.reference.revision_id
    ));
    assert!(!includes(
        &after,
        SnapshotTable::RelationReview,
        later_review.reference.review_id
    ));
}

#[tokio::test]
async fn planner_completes_with_one_connection_and_preserves_database_state() {
    let (r, actor, _, _, saved) = h::fixture().await;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(1))
        .connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let before = h::counts(&r, actor).await;
    let mut input = request(saved.view.clone());
    input.include_originals = false;
    let plan = SnapshotStore::new(pool.clone())
        .plan_exact(actor, &input)
        .await
        .unwrap();
    assert_eq!(plan.manifest.root, saved.view);
    assert!(plan.manifest.requires_destination_assets);
    assert_eq!(before, h::counts(&r, actor).await);
    pool.close().await;
}

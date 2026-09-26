mod support;
use learning_assets::{FsAssetStore, SnapshotDirectory, stage_snapshot};
use learning_core::*;
use learning_db::{SnapshotImportStore, SnapshotPlan, SnapshotStore};
use serde_json::json;
use std::{fs, path::PathBuf};
use uuid::Uuid;
#[path = "support/relation_store.rs"]
mod relations;

// Global row-count assertions must not race other fixtures in this target.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Files {
    root: PathBuf,
    store: FsAssetStore,
}
impl Files {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("snapshot-import-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let store = FsAssetStore::new(root.join("assets"), root.join("uploads")).unwrap();
        Self { root, store }
    }
    fn stage(&self, plan: &SnapshotPlan) -> SnapshotDirectory {
        stage_snapshot(
            &self.root,
            Uuid::new_v4(),
            &plan.manifest,
            &plan.rows,
            &plan.assets,
            &self.store,
        )
        .unwrap()
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn request(root: ReadingRef) -> SnapshotRequest {
    SnapshotRequest {
        reading: root,
        mode: ReadingMode::Fused,
        include_personal: true,
        include_originals: true,
        resource_versions: vec![],
        source_segments: vec![],
    }
}
fn rehash(plan: &mut SnapshotPlan) {
    let mut files = vec![];
    for row in &mut plan.rows {
        row.sha256 = canonical_record_hash(&row.immutable_values);
        let bytes = canonical_json(&serde_json::to_value(&row).unwrap());
        files.push(SnapshotFile {
            path: snapshot_object_path(row.table, &row.identity).unwrap(),
            size: bytes.len() as u64,
            sha256: hex_digest(bytes.as_bytes()),
        });
    }
    files.extend(
        plan.manifest
            .files
            .iter()
            .filter(|f| f.path.starts_with("assets/"))
            .cloned(),
    );
    files.sort_by(|a, b| a.path.cmp(&b.path));
    plan.manifest.files = files;
}
async fn counts(rig: &support::TestRig) -> Vec<i64> {
    // All authoritative tables touched by the proposed import, plus identities,
    // grants, and operational tables which preflight must never write.
    let mut result = vec![];
    for table in [
        "app_user",
        "space",
        "space_grant",
        "asset",
        "resource",
        "resource_version",
        "source_segment",
        "block",
        "block_revision",
        "block_asset_use",
        "composition",
        "composition_revision",
        "composition_occurrence",
        "overlay",
        "overlay_revision",
        "overlay_group_identity",
        "overlay_placement_identity",
        "overlay_group",
        "overlay_placement",
        "placement_manual_decision",
        "reference_object",
        "reference_dependency",
        "relation",
        "relation_revision",
        "relation_review_head",
        "relation_review",
        "epistemic_stream",
        "epistemic_review",
        "reading_view",
        "reading_view_revision",
        "reading_relation_selection",
        "reading_epistemic_selection",
        "job",
        "job_outbox",
        "request_key",
    ] {
        result.push(
            sqlx::query_scalar(&format!("SELECT count(*) FROM public.{table}"))
                .fetch_one(&rig.admin_pool)
                .await
                .unwrap(),
        );
    }
    result
}
#[test]
fn unknown_version_kind_and_bad_paths_are_not_import_contracts() {
    let manifest = json!({"capability":"exact_import_v1","format_version":1,"root":{"view_id":Uuid::new_v4(),"revision_id":Uuid::new_v4()},"files":[],"requires_destination_assets":false});
    for (field, value) in [
        ("format_version", json!(2)),
        ("capability", json!("unknown")),
    ] {
        let mut bad = manifest.clone();
        bad[field] = value;
        assert!(serde_json::from_value::<SnapshotManifest>(bad).is_err());
    }
    for path in [
        "../block.json",
        "/manifest.json",
        "objects/unknown/u-00000000-0000-0000-0000-000000000001.json",
        "objects/block/U-00000000-0000-0000-0000-000000000001.json",
    ] {
        assert!(parse_snapshot_object_path(path).is_err());
    }
    let error = ContentError::IdentityConflict.to_string();
    assert!(!error.contains("00000000"));
}
#[tokio::test]
async fn preflight_reuses_exact_rows_read_only_and_rejects_identity_metadata_changes() {
    let _serial = SERIAL.lock().await;
    let (rig, actor, _, _, saved) = support::reading::fixture().await;
    let plan = SnapshotStore::new(rig.runtime_pool.clone())
        .plan_exact(actor, &request(saved.view))
        .await
        .unwrap();
    let files = Files::new();
    let store = SnapshotImportStore::new(rig.runtime_pool.clone(), files.store.clone());
    let before = counts(&rig).await;
    let prepared = store
        .validate_exact(actor, &files.stage(&plan))
        .await
        .unwrap();
    assert_eq!(prepared.rows().len(), plan.rows.len());
    assert_eq!(counts(&rig).await, before);
    let mut different_parent = plan.clone();
    let parent_id = Uuid::new_v4();
    let mut ancestor = different_parent
        .rows
        .iter()
        .find(|r| r.table == SnapshotTable::BlockRevision)
        .unwrap()
        .clone();
    let revision_id = ancestor.immutable_values["id"].clone();
    let block_id = ancestor.immutable_values["block_id"].clone();
    ancestor.immutable_values["id"] = json!(parent_id);
    ancestor.identity = vec![SnapshotIdentityPart::Uuid(parent_id)];
    different_parent
        .rows
        .iter_mut()
        .find(|r| {
            r.table == SnapshotTable::BlockRevision && r.immutable_values["id"] == revision_id
        })
        .unwrap()
        .immutable_values["parent_revision_id"] = json!(parent_id);
    let mut registry = different_parent
        .rows
        .iter()
        .find(|r| {
            r.table == SnapshotTable::ReferenceObject && r.immutable_values["object_id"] == block_id
        })
        .unwrap()
        .clone();
    registry.immutable_values["revision_id"] = json!(parent_id);
    registry.identity[2] = SnapshotIdentityPart::Uuid(parent_id);
    different_parent.rows.extend([ancestor, registry]);
    rehash(&mut different_parent);
    let before = counts(&rig).await;
    assert!(matches!(
        store
            .validate_exact(actor, &files.stage(&different_parent))
            .await,
        Err(ContentError::IdentityConflict)
    ));
    assert_eq!(counts(&rig).await, before);
    let (other, _) = rig.seed_actor_space(false).await;
    for (key, value) in [
        ("author_id", json!(other.actor_id)),
        ("reason", json!("different reason")),
        ("created_at", json!("2020-01-01T00:00:00.000000Z")),
    ] {
        let mut bad = plan.clone();
        bad.rows
            .iter_mut()
            .find(|r| r.table == SnapshotTable::BlockRevision)
            .unwrap()
            .immutable_values[key] = value;
        rehash(&mut bad);
        let before = counts(&rig).await;
        assert!(matches!(
            store.validate_exact(actor, &files.stage(&bad)).await,
            Err(ContentError::IdentityConflict)
        ));
        assert_eq!(counts(&rig).await, before);
    }
}
#[tokio::test]
async fn malformed_closure_authors_parent_and_body_leave_all_counts_unchanged() {
    let _serial = SERIAL.lock().await;
    let (rig, actor, _, _, saved) = support::reading::fixture().await;
    let plan = SnapshotStore::new(rig.runtime_pool.clone())
        .plan_exact(actor, &request(saved.view))
        .await
        .unwrap();
    let files = Files::new();
    let store = SnapshotImportStore::new(rig.runtime_pool.clone(), files.store.clone());
    let mut bads = vec![];
    for table in [
        SnapshotTable::Block,
        SnapshotTable::ReferenceObject,
        SnapshotTable::CompositionOccurrence,
    ] {
        let mut bad = plan.clone();
        let i = bad.rows.iter().position(|r| r.table == table).unwrap();
        bad.rows.remove(i);
        bads.push(bad);
    }
    for (key, value) in [
        ("author_id", json!(Uuid::new_v4())),
        ("parent_revision_id", json!(Uuid::new_v4())),
    ] {
        let mut bad = plan.clone();
        bad.rows
            .iter_mut()
            .find(|r| r.table == SnapshotTable::BlockRevision)
            .unwrap()
            .immutable_values[key] = value;
        bads.push(bad);
    }
    let mut bad = plan.clone();
    bad.rows
        .iter_mut()
        .find(|r| r.table == SnapshotTable::BlockRevision)
        .unwrap()
        .immutable_values["content"]["payload"]["text"] =
        json!("tampered while retaining business hash");
    bads.push(bad);
    for mut bad in bads {
        rehash(&mut bad);
        let before = counts(&rig).await;
        assert!(
            store
                .validate_exact(actor, &files.stage(&bad))
                .await
                .is_err()
        );
        assert_eq!(counts(&rig).await, before);
    }
    let before = counts(&rig).await;
    assert!(matches!(
        store
            .validate_exact(
                Principal {
                    actor_id: Uuid::new_v4()
                },
                &files.stage(&plan)
            )
            .await,
        Err(ContentError::NotFound)
    ));
    assert_eq!(counts(&rig).await, before);
}
#[tokio::test]
async fn revoked_and_read_only_root_grants_cannot_preflight_and_copy_is_rejected() {
    let _serial = SERIAL.lock().await;
    let (rig, actor, space, _, saved) = support::reading::fixture().await;
    let plan = SnapshotStore::new(rig.runtime_pool.clone())
        .plan_exact(actor, &request(saved.view))
        .await
        .unwrap();
    let files = Files::new();
    let store = SnapshotImportStore::new(rig.runtime_pool.clone(), files.store.clone());
    let staged = files.stage(&plan);
    sqlx::query("UPDATE space_grant SET can_write=false WHERE actor_id=$1 AND space_id=$2")
        .bind(actor.actor_id)
        .bind(space)
        .execute(&rig.admin_pool)
        .await
        .unwrap();
    let before = counts(&rig).await;
    assert!(matches!(
        store.validate_exact(actor, &staged).await,
        Err(ContentError::NotFound)
    ));
    assert_eq!(counts(&rig).await, before);
    rig.revoke(actor, space).await;
    let before = counts(&rig).await;
    assert!(matches!(
        store.validate_exact(actor, &staged).await,
        Err(ContentError::NotFound)
    ));
    assert_eq!(counts(&rig).await, before);
    let id = Uuid::new_v4();
    let copy = learning_assets::stage_reading_copy(
        &files.root,
        id,
        &ReadingCopyManifest {
            format_version: 1,
            copy_id: id,
            files: vec![],
        },
        &ReadingCopy {
            items: vec![],
            evidence: vec![],
        },
    )
    .unwrap();
    assert!(matches!(
        store.validate_exact(actor, &copy).await,
        Err(ContentError::Invalid(_))
    ));
    assert_eq!(counts(&rig).await, before);
}
#[tokio::test]
async fn cross_space_existing_reuse_needs_read_but_missing_rows_need_write() {
    let _serial = SERIAL.lock().await;
    let (rig, actor, _, doc, saved) = support::reading::fixture().await;
    let (other, space) = rig.seed_actor_space(true).await;
    let remote = rig
        .store
        .create(other, space, support::command("cross space"))
        .await
        .unwrap();
    sqlx::query("INSERT INTO space_grant(actor_id,space_id,can_write) VALUES($1,$2,false)")
        .bind(actor.actor_id)
        .bind(space)
        .execute(&rig.admin_pool)
        .await
        .unwrap();
    let rootspace: Uuid = sqlx::query_scalar("SELECT space_id FROM composition WHERE id=$1")
        .bind(doc.reference.composition_id)
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap();
    let newdoc = rig
        .compositions()
        .save(
            actor,
            rootspace,
            support::assembly::doc(vec![support::assembly::block(&remote)]),
        )
        .await
        .unwrap();
    let reading = support::reading::store(&rig)
        .create(actor, rootspace, support::reading::create(newdoc.reference))
        .await
        .unwrap();
    let _ = saved;
    let plan = SnapshotStore::new(rig.runtime_pool.clone())
        .plan_exact(actor, &request(reading.view))
        .await
        .unwrap();
    let files = Files::new();
    let store = SnapshotImportStore::new(rig.runtime_pool.clone(), files.store.clone());
    let before = counts(&rig).await;
    store
        .validate_exact(actor, &files.stage(&plan))
        .await
        .unwrap();
    assert_eq!(counts(&rig).await, before);
    // Replace only the remote block's identity with a new, absent identity,
    // consistently through the package and its composition business digest.
    let mut missing = plan.clone();
    let new_id = Uuid::new_v4();
    let new_revision = Uuid::new_v4();
    let old_id = serde_json::to_string(&remote.block_id).unwrap();
    let old_revision = serde_json::to_string(&remote.revision_id).unwrap();
    for row in &mut missing.rows {
        let encoded = serde_json::to_string(row)
            .unwrap()
            .replace(&old_id, &serde_json::to_string(&new_id).unwrap())
            .replace(
                &old_revision,
                &serde_json::to_string(&new_revision).unwrap(),
            );
        let mut value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        value["sha256"] = json!(canonical_record_hash(&value["immutable_values"]));
        *row = serde_json::from_value(value).unwrap();
    }
    for row in missing
        .rows
        .iter_mut()
        .filter(|r| r.table == SnapshotTable::CompositionRevision)
    {
        row.immutable_values["content_sha256"] = json!(composition_digest(
            CompositionKind::Document,
            row.immutable_values["title"].as_str().unwrap(),
            &[Occurrence {
                occurrence_id: plan
                    .rows
                    .iter()
                    .find(|r| r.table == SnapshotTable::CompositionOccurrence)
                    .unwrap()
                    .immutable_values["occurrence_id"]
                    .as_str()
                    .unwrap()
                    .parse()
                    .unwrap(),
                target: NodeTarget::Block(BlockRef {
                    block_id: new_id,
                    revision_id: new_revision
                })
            }]
        ));
    }
    rehash(&mut missing);
    let before = counts(&rig).await;
    assert!(matches!(
        store.validate_exact(actor, &files.stage(&missing)).await,
        Err(ContentError::NotFound)
    ));
    assert_eq!(counts(&rig).await, before);
    rig.revoke(actor, space).await;
    let before = counts(&rig).await;
    assert!(matches!(
        store.validate_exact(actor, &files.stage(&plan)).await,
        Err(ContentError::NotFound)
    ));
    assert_eq!(counts(&rig).await, before);
}

#[tokio::test]
async fn missing_review_head_dependency_reviewer_and_space_are_read_only_errors() {
    let _serial = SERIAL.lock().await;
    let (rig, actor, space, _, saved) = support::reading::fixture().await;
    let from = rig
        .store
        .create(actor, space, support::command("from"))
        .await
        .unwrap();
    let to = rig
        .store
        .create(actor, space, support::command("to"))
        .await
        .unwrap();
    let relations = learning_db::RelationStore::new(rig.runtime_pool.clone());
    let relation = relations
        .save(
            actor,
            relations::save(space, relations::exact(&from), relations::exact(&to)),
        )
        .await
        .unwrap();
    let review = relations
        .review(actor, relations::review(&relation))
        .await
        .unwrap();
    let reading = support::reading::store(&rig)
        .select_relations(
            actor,
            saved.overlay.overlay_id,
            SelectRelations {
                request_id: Uuid::new_v4(),
                expected_overlay_revision: saved.overlay.revision_id,
                expected_reading_view_revision: saved.view.revision_id,
                selections: vec![RelationSelection {
                    relation: relation.reference,
                    review: Some(review.reference),
                }],
                epistemic_reviews: vec![],
                reason: "preflight".into(),
            },
        )
        .await
        .unwrap();
    let plan = SnapshotStore::new(rig.runtime_pool.clone())
        .plan_exact(actor, &request(reading.view))
        .await
        .unwrap();
    let files = Files::new();
    let store = SnapshotImportStore::new(rig.runtime_pool.clone(), files.store.clone());
    let before = counts(&rig).await;
    store
        .validate_exact(actor, &files.stage(&plan))
        .await
        .unwrap();
    assert_eq!(counts(&rig).await, before);
    let mut bads = vec![];
    for table in [
        SnapshotTable::ReferenceDependency,
        SnapshotTable::RelationReviewHead,
    ] {
        let mut bad = plan.clone();
        let pos = bad.rows.iter().position(|r| r.table == table).unwrap();
        bad.rows.remove(pos);
        bads.push(bad);
    }
    let mut bad = plan.clone();
    bad.rows
        .iter_mut()
        .find(|r| r.table == SnapshotTable::RelationReview)
        .unwrap()
        .immutable_values["reviewer_id"] = json!(Uuid::new_v4());
    bads.push(bad);
    // Remap the complete package scope consistently to an unprovisioned space.
    let mut bad = plan.clone();
    let missing_space = Uuid::new_v4();
    for row in &mut bad.rows {
        for (_, value) in row.immutable_values.as_object_mut().unwrap() {
            if *value == json!(space) {
                *value = json!(missing_space);
            }
        }
    }
    // This mutation can fail business-scope validation before target lookup;
    // both boundaries must remain read-only and return no hidden identifiers.
    bads.push(bad);
    for mut bad in bads {
        rehash(&mut bad);
        let before = counts(&rig).await;
        assert!(
            store
                .validate_exact(actor, &files.stage(&bad))
                .await
                .is_err()
        );
        assert_eq!(counts(&rig).await, before);
    }
}

#[tokio::test]
async fn no_originals_requires_same_space_and_asset_id_and_real_bytes() {
    let _serial = SERIAL.lock().await;
    use learning_assets::UploadDeclaration;
    use learning_db::{AssetMedia, AssetStore};
    let (rig, actor, space, _, saved) = support::reading::fixture().await;
    let mut plan = SnapshotStore::new(rig.runtime_pool.clone())
        .plan_exact(actor, &request(saved.view))
        .await
        .unwrap();
    let files = Files::new();
    let bytes = b"preflight original bytes";
    let source = files.root.join("source.txt");
    fs::write(&source, bytes).unwrap();
    let blob = files
        .store
        .put_from_file(
            Uuid::new_v4(),
            &source,
            UploadDeclaration {
                expected_size_bytes: bytes.len() as u64,
                max_size_bytes: 1000,
            },
        )
        .unwrap();
    let record = AssetStore::new(rig.runtime_pool.clone(), files.store.clone())
        .register_verified(
            actor,
            space,
            Uuid::new_v4(),
            blob,
            AssetMedia {
                media_type: "application/octet-stream".into(),
                original_file_name: "source.txt".into(),
            },
        )
        .await
        .unwrap();
    let mut value: serde_json::Value =
        sqlx::query_scalar("SELECT to_jsonb(a) FROM asset a WHERE space_id=$1 AND id=$2")
            .bind(space)
            .bind(record.reference.asset_id)
            .fetch_one(&rig.runtime_pool)
            .await
            .unwrap();
    let t = chrono::DateTime::parse_from_rfc3339(value["created_at"].as_str().unwrap()).unwrap();
    value["created_at"] = json!(canonical_snapshot_timestamp(t.with_timezone(&chrono::Utc)));
    plan.rows.push(SnapshotRow {
        table: SnapshotTable::Asset,
        identity: vec![
            SnapshotIdentityPart::Uuid(space),
            SnapshotIdentityPart::Uuid(record.reference.asset_id),
        ],
        sha256: canonical_record_hash(&value),
        immutable_values: value,
    });
    plan.manifest.requires_destination_assets = true;
    rehash(&mut plan);
    let store = SnapshotImportStore::new(rig.runtime_pool.clone(), files.store.clone());
    let before = counts(&rig).await;
    let prepared = store
        .validate_exact(actor, &files.stage(&plan))
        .await
        .unwrap();
    assert_eq!(prepared.assets().count(), 1);
    assert_eq!(counts(&rig).await, before);
    let resource = AssetStore::new(rig.runtime_pool.clone(), files.store.clone())
        .link_resource_version(
            actor,
            learning_db::ResourceInput {
                space_id: space,
                resource_id: None,
                display_name: "original resource".into(),
            },
            record.reference.clone(),
        )
        .await
        .unwrap();
    let mut included_request = request(plan.manifest.root.clone());
    included_request.resource_versions.push(resource);
    let included = SnapshotStore::new(rig.runtime_pool.clone())
        .plan_exact(actor, &included_request)
        .await
        .unwrap();
    let before = counts(&rig).await;
    let retained = store
        .validate_exact(actor, &files.stage(&included))
        .await
        .unwrap();
    assert_eq!(retained.assets().count(), 1);
    assert_eq!(counts(&rig).await, before);
    let (other, target_space) = rig.seed_actor_space(true).await;
    sqlx::query("INSERT INTO space_grant(actor_id,space_id,can_write) VALUES($1,$2,true)")
        .bind(actor.actor_id)
        .bind(target_space)
        .execute(&rig.admin_pool)
        .await
        .unwrap();
    let _ = other;
    for (asset_space, asset_id) in [
        (space, Uuid::new_v4()),
        (target_space, record.reference.asset_id),
    ] {
        let mut bad = plan.clone();
        let row = bad
            .rows
            .iter_mut()
            .find(|r| r.table == SnapshotTable::Asset)
            .unwrap();
        row.immutable_values["space_id"] = json!(asset_space);
        row.immutable_values["id"] = json!(asset_id);
        row.identity = vec![
            SnapshotIdentityPart::Uuid(asset_space),
            SnapshotIdentityPart::Uuid(asset_id),
        ];
        rehash(&mut bad);
        let before = counts(&rig).await;
        assert!(matches!(
            store.validate_exact(actor, &files.stage(&bad)).await,
            Err(ContentError::NotFound)
        ));
        assert_eq!(counts(&rig).await, before);
    }
    let mut missing_original = plan.clone();
    missing_original.manifest.requires_destination_assets = false;
    let before = counts(&rig).await;
    assert!(
        store
            .validate_exact(actor, &files.stage(&missing_original))
            .await
            .is_err()
    );
    assert_eq!(counts(&rig).await, before);
    let staged = files.stage(&plan);
    let missing_store = FsAssetStore::new(
        files.root.join("empty-assets"),
        files.root.join("empty-uploads"),
    )
    .unwrap();
    let missing = SnapshotImportStore::new(rig.runtime_pool.clone(), missing_store);
    let before = counts(&rig).await;
    assert!(matches!(
        missing.validate_exact(actor, &staged).await,
        Err(ContentError::NotFound)
    ));
    assert_eq!(counts(&rig).await, before);
}

#[tokio::test]
async fn existing_third_party_epistemic_stream_reuses_read_access_but_cannot_be_created() {
    let _serial = SERIAL.lock().await;
    let (rig, actor, _, _, saved) = support::reading::fixture().await;
    let (reviewer, space) = rig.seed_actor_space(true).await;
    let mut command = support::command("third-party target");
    command.draft.intent = Intent::Conjecture;
    let target = rig.store.create(reviewer, space, command).await.unwrap();
    let review = learning_db::ReviewStore::new(rig.runtime_pool.clone())
        .append(
            reviewer,
            AppendEpistemicReview {
                request_id: Uuid::new_v4(),
                scope: RelationScope::Space { space_id: space },
                target: relations::exact(&target),
                expected_previous: None,
                state: EpistemicState::Testing,
                relations: vec![],
                evidence: vec![],
                conditions: String::new(),
                explanation: "reviewer judgment".into(),
            },
        )
        .await
        .unwrap();
    sqlx::query("INSERT INTO space_grant(actor_id,space_id,can_write) VALUES($1,$2,false)")
        .bind(actor.actor_id)
        .bind(space)
        .execute(&rig.admin_pool)
        .await
        .unwrap();
    let reading = support::reading::store(&rig)
        .select_relations(
            actor,
            saved.overlay.overlay_id,
            SelectRelations {
                request_id: Uuid::new_v4(),
                expected_overlay_revision: saved.overlay.revision_id,
                expected_reading_view_revision: saved.view.revision_id,
                selections: vec![],
                epistemic_reviews: vec![review.reference.clone()],
                reason: "third-party reuse".into(),
            },
        )
        .await
        .unwrap();
    let plan = SnapshotStore::new(rig.runtime_pool.clone())
        .plan_exact(actor, &request(reading.view))
        .await
        .unwrap();
    let files = Files::new();
    let store = SnapshotImportStore::new(rig.runtime_pool.clone(), files.store.clone());
    let before = counts(&rig).await;
    store
        .validate_exact(actor, &files.stage(&plan))
        .await
        .unwrap();
    assert_eq!(counts(&rig).await, before);
    sqlx::query("UPDATE space_grant SET can_write=true WHERE actor_id=$1 AND space_id=$2")
        .bind(actor.actor_id)
        .bind(space)
        .execute(&rig.admin_pool)
        .await
        .unwrap();
    let mut missing = plan.clone();
    let old_stream = serde_json::to_string(&review.reference.stream_id).unwrap();
    let old_review = serde_json::to_string(&review.reference.review_id).unwrap();
    let new_stream = serde_json::to_string(&Uuid::new_v4()).unwrap();
    let new_review = serde_json::to_string(&Uuid::new_v4()).unwrap();
    for row in &mut missing.rows {
        let encoded = serde_json::to_string(row)
            .unwrap()
            .replace(&old_stream, &new_stream)
            .replace(&old_review, &new_review);
        let mut value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        value["sha256"] = json!(canonical_record_hash(&value["immutable_values"]));
        *row = serde_json::from_value(value).unwrap();
    }
    rehash(&mut missing);
    let before = counts(&rig).await;
    assert!(matches!(
        store.validate_exact(actor, &files.stage(&missing)).await,
        Err(ContentError::NotFound)
    ));
    assert_eq!(counts(&rig).await, before);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn symlink_replacement_after_staging_is_rejected_without_database_changes() {
    let _serial = SERIAL.lock().await;
    let (rig, actor, _, _, saved) = support::reading::fixture().await;
    let plan = SnapshotStore::new(rig.runtime_pool.clone())
        .plan_exact(actor, &request(saved.view))
        .await
        .unwrap();
    let files = Files::new();
    let job = Uuid::new_v4();
    let staged = stage_snapshot(
        &files.root,
        job,
        &plan.manifest,
        &plan.rows,
        &plan.assets,
        &files.store,
    )
    .unwrap();
    let row = &plan.rows[0];
    let path = files
        .root
        .join(format!("{job}.ready"))
        .join(snapshot_object_path(row.table, &row.identity).unwrap());
    let outside = files.root.join("outside.json");
    fs::copy(&path, &outside).unwrap();
    // Staging seals table directories to 0500. As their owner, deliberately
    // make only this fixture parent writable before replacing its child.
    // Restore the seal so the rejection exercises the symlink itself.
    use std::os::unix::fs::PermissionsExt;
    let parent = path.parent().unwrap();
    let sealed = fs::metadata(parent).unwrap().permissions();
    assert_eq!(sealed.mode() & 0o777, 0o500);
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).unwrap();
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&outside, &path).unwrap();
    fs::set_permissions(parent, sealed).unwrap();
    assert!(
        fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let store = SnapshotImportStore::new(rig.runtime_pool.clone(), files.store.clone());
    let before = counts(&rig).await;
    assert!(store.validate_exact(actor, &staged).await.is_err());
    assert_eq!(counts(&rig).await, before);
}

#[tokio::test]
async fn private_overlay_revision_id_never_reports_an_identity_collision() {
    let _serial = SERIAL.lock().await;
    let (rig, actor, space, doc, saved) = support::reading::fixture().await;
    let (other, _) = rig.seed_actor_space(true).await;
    sqlx::query("INSERT INTO space_grant(actor_id,space_id,can_write) VALUES($1,$2,true)")
        .bind(other.actor_id)
        .bind(space)
        .execute(&rig.admin_pool)
        .await
        .unwrap();
    let private = support::reading::store(&rig)
        .create(other, space, support::reading::create(doc.reference))
        .await
        .unwrap();
    let plan = SnapshotStore::new(rig.runtime_pool.clone())
        .plan_exact(actor, &request(saved.view.clone()))
        .await
        .unwrap();
    let files = Files::new();
    let store = SnapshotImportStore::new(rig.runtime_pool.clone(), files.store.clone());
    for candidate in [Uuid::new_v4(), private.overlay.revision_id] {
        let mut probe = plan.clone();
        let new_view_revision = Uuid::new_v4();
        for row in &mut probe.rows {
            if row.table == SnapshotTable::OverlayRevision {
                row.immutable_values["id"] = json!(candidate);
                row.identity = vec![SnapshotIdentityPart::Uuid(candidate)];
            }
            if row.table == SnapshotTable::ReadingViewRevision {
                row.immutable_values["id"] = json!(new_view_revision);
                row.immutable_values["overlay_revision_id"] = json!(candidate);
                row.identity = vec![SnapshotIdentityPart::Uuid(new_view_revision)];
            }
        }
        probe.manifest.root.revision_id = new_view_revision;
        rehash(&mut probe);
        let before = counts(&rig).await;
        let result = store.validate_exact(actor, &files.stage(&probe)).await;
        if candidate == private.overlay.revision_id {
            // The supplied overlay belongs to the importer, but authorization
            // must traverse the existing revision's actual private overlay.
            assert!(matches!(result, Err(ContentError::NotFound)));
        } else {
            // Fresh immutable revisions remain valid import candidates.
            assert!(result.is_ok());
        }
        assert_eq!(counts(&rig).await, before);
    }
}

#[tokio::test]
async fn metadata_only_five_max_size_assets_verify_all_destination_bytes() {
    use learning_assets::UploadDeclaration;
    use learning_db::{AssetMedia, AssetStore, ResourceInput};
    use std::io::Write;
    let _serial = SERIAL.lock().await;
    let (rig, actor, space, _, saved) = support::reading::fixture().await;
    let files = Files::new();
    let asset_store = AssetStore::new(rig.runtime_pool.clone(), files.store.clone());
    let mut export = request(saved.view);
    export.include_originals = false;
    let size = SNAPSHOT_MAX_ASSET_FILE_BYTES as u64;
    let mut last_asset = None;
    for n in 0..5u8 {
        // Sparse input plus the store's streaming copy avoids a 128 MiB Vec.
        // Distinct first bytes produce five unique 128 MiB originals.
        let source = files.root.join(format!("large-{n}.bin"));
        let mut input = fs::File::create(&source).unwrap();
        input.set_len(size).unwrap();
        input.write_all(&[n]).unwrap();
        drop(input);
        let blob = files
            .store
            .put_from_file(
                Uuid::new_v4(),
                &source,
                UploadDeclaration {
                    expected_size_bytes: size,
                    max_size_bytes: size,
                },
            )
            .unwrap();
        let asset = asset_store
            .register_verified(
                actor,
                space,
                Uuid::new_v4(),
                blob,
                AssetMedia {
                    media_type: "application/octet-stream".into(),
                    original_file_name: format!("large-{n}.bin"),
                },
            )
            .await
            .unwrap();
        let resource = asset_store
            .link_resource_version(
                actor,
                ResourceInput {
                    space_id: space,
                    resource_id: None,
                    display_name: format!("large {n}"),
                },
                asset.reference.clone(),
            )
            .await
            .unwrap();
        export.resource_versions.push(resource);
        last_asset = Some(asset.reference.asset_id);
        fs::remove_file(source).unwrap();
    }
    let plan = SnapshotStore::new(rig.runtime_pool.clone())
        .plan_exact(actor, &export)
        .await
        .unwrap();
    assert!(plan.manifest.requires_destination_assets);
    assert!(
        plan.manifest
            .files
            .iter()
            .all(|f| !f.path.starts_with("assets/"))
    );
    let staged = files.stage(&plan);
    let store = SnapshotImportStore::new(rig.runtime_pool.clone(), files.store.clone());
    let before = counts(&rig).await;
    let prepared = store.validate_exact(actor, &staged).await.unwrap();
    assert_eq!(prepared.assets().count(), 5);
    assert_eq!(
        prepared.assets().map(|(_, _, bytes)| bytes).sum::<u64>(),
        5 * size
    );
    assert_eq!(counts(&rig).await, before);
    let key: String =
        sqlx::query_scalar("SELECT storage_key FROM asset WHERE space_id=$1 AND id=$2")
            .bind(space)
            .bind(last_asset.unwrap())
            .fetch_one(&rig.runtime_pool)
            .await
            .unwrap();
    let path = files.root.join("assets").join(key);
    let mut corrupt = fs::OpenOptions::new().write(true).open(path).unwrap();
    corrupt.write_all(&[255]).unwrap();
    drop(corrupt);
    assert!(matches!(
        store.validate_exact(actor, &staged).await,
        Err(ContentError::NotFound)
    ));
    assert_eq!(counts(&rig).await, before);
}

//! Every business object is produced by the frozen five B2 stores.
use learning_core::*;
use learning_db::{CompositionStore, ContentStore, MigrationStore, ReadingStore, ReleaseStore};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

macro_rules! capture {
    ($manifest:expr, $key:expr, $value:expr) => {{
        let bytes = serde_json::to_string(&$value).unwrap();
        $manifest["wire"][$key] = json!({"json":serde_json::from_str::<Value>(&bytes).unwrap(),"bytes":bytes,"sha256":hex_digest(bytes.as_bytes())});
    }};
}
fn exact(r: &Revision) -> BlockRef {
    BlockRef {
        block_id: r.block_id,
        revision_id: r.revision_id,
    }
}

pub async fn populate(
    admin: &PgPool,
    runtime: PgPool,
    actor: Principal,
    space: Uuid,
    k_cmd: &CreateCommand,
    k: &Revision,
    manifest: &mut Value,
) {
    let content = ContentStore::new(runtime.clone());
    let compositions = CompositionStore::new(runtime.clone());
    let readings = ReadingStore::new(runtime.clone());
    let migrations = MigrationStore::new(runtime.clone());
    let releases = ReleaseStore::new(runtime);
    manifest["wire"] = json!({});
    manifest["commands"] = json!({"k_create":k_cmd});
    capture!(manifest, "k1", k);
    let m_cmd = super::command("M");
    let m = content.create(actor, space, m_cmd.clone()).await.unwrap();
    manifest["commands"]["m_create"] = json!(m_cmd);
    capture!(manifest, "m1", m);
    let d1_cmd = SaveComposition {
        request_id: Uuid::new_v4(),
        composition_id: None,
        base_revision_id: None,
        kind: CompositionKind::Document,
        title: "Attention".into(),
        nodes: vec![
            NodeDraft {
                occurrence_id: None,
                target: NodeTarget::Block(exact(k)),
            },
            NodeDraft {
                occurrence_id: None,
                target: NodeTarget::Block(exact(&m)),
            },
        ],
        reason: "B2 document".into(),
    };
    let d1 = compositions
        .save(actor, space, d1_cmd.clone())
        .await
        .unwrap();
    manifest["commands"]["d1_save"] = json!(d1_cmd);
    capture!(manifest, "d1", d1);
    let r0_cmd = CreateReading {
        request_id: Uuid::new_v4(),
        base: d1.reference.clone(),
        title: "B2 personal".into(),
        reason: "start".into(),
    };
    let r0 = readings.create(actor, space, r0_cmd.clone()).await.unwrap();
    manifest["commands"]["r0_create"] = json!(r0_cmd);
    capture!(manifest, "r0", r0);
    let anchor = GapAnchor {
        base: d1.reference.clone(),
        parent_occurrence_path: vec![],
        left_occurrence_id: Some(d1.nodes[0].occurrence_id),
        right_occurrence_id: Some(d1.nodes[1].occurrence_id),
        affinity: Affinity::AfterLeft,
    };
    let r1_cmd = EditReading {
        request_id: Uuid::new_v4(),
        expected_overlay_revision: r0.overlay.revision_id,
        expected_reading_view_revision: r0.view.revision_id,
        edit: ReadingEdit::InsertNew {
            drafts: vec![super::command("N").draft],
            target: InsertTarget::NewGroup { anchor },
        },
        reason: "insert N".into(),
    };
    let r1 = readings
        .edit(actor, r0.overlay.overlay_id, r1_cmd.clone())
        .await
        .unwrap();
    manifest["commands"]["r1_edit"] = json!(r1_cmd);
    capture!(manifest, "r1", r1);
    let n = content
        .read(actor, r1.changed_blocks[0].revision_id)
        .await
        .unwrap()
        .unwrap();
    capture!(manifest, "n1", n);
    let state = readings
        .state(actor, r0.overlay.overlay_id)
        .await
        .unwrap()
        .unwrap();
    let group = state.editable.as_ref().unwrap().groups[0].group_id;
    let k2_cmd = ReviseCommand {
        request_id: Uuid::new_v4(),
        base_revision_id: k.revision_id,
        draft: super::command("K2").draft,
        reason: "revise K".into(),
    };
    let k2 = content
        .revise(actor, k.block_id, k2_cmd.clone())
        .await
        .unwrap();
    manifest["commands"]["k2_revise"] = json!(k2_cmd);
    capture!(manifest, "k2", k2);
    let d2_cmd = SaveComposition {
        request_id: Uuid::new_v4(),
        composition_id: Some(d1.reference.composition_id),
        base_revision_id: Some(d1.reference.revision_id),
        kind: CompositionKind::Document,
        title: "Attention revised".into(),
        nodes: vec![
            NodeDraft {
                occurrence_id: Some(d1.nodes[0].occurrence_id),
                target: NodeTarget::Block(exact(&k2)),
            },
            NodeDraft {
                occurrence_id: Some(d1.nodes[1].occurrence_id),
                target: NodeTarget::Block(exact(&m)),
            },
        ],
        reason: "adopt K2".into(),
    };
    let d2 = compositions
        .save(actor, space, d2_cmd.clone())
        .await
        .unwrap();
    manifest["commands"]["d2_save"] = json!(d2_cmd);
    capture!(manifest, "d2", d2);
    let propose_cmd = ProposeMigration {
        request_id: Uuid::new_v4(),
        expected_overlay_revision: r1.overlay.revision_id,
        expected_reading_view_revision: r1.view.revision_id,
        target: d2.reference.clone(),
        reason: "migrate".into(),
    };
    let proposal = migrations
        .propose(actor, r0.overlay.overlay_id, propose_cmd.clone())
        .await
        .unwrap();
    manifest["commands"]["propose"] = json!(propose_cmd);
    capture!(manifest, "proposal_initial", proposal);
    let decide_cmd = DecideMigration {
        request_id: Uuid::new_v4(),
        proposal_id: proposal.proposal_id,
        expected_overlay_revision: r1.overlay.revision_id,
        expected_reading_view_revision: r1.view.revision_id,
        action: MigrationAction::Adopt {
            groups: vec![GroupDecision::Exact { group_id: group }],
            merges: vec![],
        },
        reason: "accept exact neighbors".into(),
    };
    let decision = migrations
        .decide(actor, r0.overlay.overlay_id, decide_cmd.clone())
        .await
        .unwrap();
    manifest["commands"]["decide"] = json!(decide_cmd);
    capture!(manifest, "decision", decision);
    let r2 = decision.adopted.as_ref().unwrap();
    capture!(manifest, "r2", r2);
    capture!(
        manifest,
        "proposal_final",
        migrations.read(actor, proposal.proposal_id).await.unwrap()
    );
    capture!(
        manifest,
        "proposal_replay",
        migrations
            .propose(actor, r0.overlay.overlay_id, propose_cmd)
            .await
            .unwrap()
    );
    capture!(
        manifest,
        "state",
        readings.state(actor, r0.overlay.overlay_id).await.unwrap()
    );
    for (name, saved) in [("r0", &r0), ("r1", &r1), ("r2", r2)] {
        for (mode_name, mode) in [
            ("original", ReadingMode::Original),
            ("fused", ReadingMode::Fused),
            ("personal", ReadingMode::Personal),
        ] {
            capture!(
                manifest,
                &format!("{name}_{mode_name}"),
                readings
                    .read(actor, saved.view.clone(), mode)
                    .await
                    .unwrap()
            );
        }
    }
    for (name, doc) in [("d1_snapshot", &d1), ("d2_snapshot", &d2)] {
        capture!(
            manifest,
            name,
            compositions
                .read(actor, doc.reference.clone())
                .await
                .unwrap()
        );
    }
    let basis = releases
        .state(actor, d2.reference.composition_id)
        .await
        .unwrap()
        .unwrap();
    let publish_cmd = PublishCommand {
        request_id: Uuid::new_v4(),
        roots: vec![PublishRoot {
            composition_id: d2.reference.composition_id,
            revision_id: d2.reference.revision_id,
            expected_head_revision_id: d2.reference.revision_id,
            expected_publication_token: basis.publication_token,
        }],
        reason: "publish B2".into(),
    };
    let release = releases
        .publish(actor, space, publish_cmd.clone())
        .await
        .unwrap();
    manifest["commands"]["publish"] = json!(publish_cmd);
    capture!(manifest, "release", release);
    capture!(
        manifest,
        "release_read",
        releases.read(actor, release.release_id).await.unwrap()
    );
    capture!(
        manifest,
        "active",
        releases
            .active(actor, d2.reference.composition_id)
            .await
            .unwrap()
    );
    capture!(
        manifest,
        "publication_state",
        releases
            .state(actor, d2.reference.composition_id)
            .await
            .unwrap()
    );
    manifest["old_database"] = super::legacy_snapshot::snapshot(admin, None).await;
}

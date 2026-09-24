#[path = "../../../deploy/fixture-runner/src/legacy_snapshot.rs"]
mod legacy_snapshot;
mod support;
use learning_core::*;
use learning_db::{
    CompositionStore, ContentStore, MIGRATOR, MigrationStore, ReadingStore, ReleaseStore,
};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

macro_rules! command {
    ($f:expr, $key:expr) => {
        serde_json::from_value($f["commands"][$key].clone()).unwrap()
    };
}
macro_rules! wire {
    ($f:expr, $key:expr, $actual:expr) => {{
        let actual = $actual;
        let bytes = serde_json::to_string(&actual).unwrap();
        assert_eq!(
            bytes,
            $f["wire"][$key]["bytes"].as_str().unwrap(),
            "old DTO bytes: {}",
            $key
        );
        assert_eq!(
            hex_digest(bytes.as_bytes()),
            $f["wire"][$key]["sha256"],
            "old DTO digest: {}",
            $key
        );
        assert_eq!(
            serde_json::to_value(&actual).unwrap(),
            $f["wire"][$key]["json"],
            "old DTO JSON: {}",
            $key
        );
        actual
    }};
}
fn exact(r: &Revision) -> BlockRef {
    BlockRef {
        block_id: r.block_id,
        revision_id: r.revision_id,
    }
}

#[tokio::test]
async fn genuine_five_store_b2_history_and_replays_survive_b3_upgrade() {
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect(
            &std::env::var("TEST_B2_UPGRADE_ADMIN_DATABASE_URL")
                .expect("fresh full B2 upgrade admin database required"),
        )
        .await
        .unwrap();
    let runtime = PgPoolOptions::new()
        .max_connections(2)
        .connect(
            &std::env::var("TEST_B2_UPGRADE_DATABASE_URL")
                .expect("fresh full B2 upgrade runtime database required"),
        )
        .await
        .unwrap();
    let f = support::frozen_fixture::load("b2", &admin).await;
    let actor = Principal {
        actor_id: serde_json::from_value(f["actor_id"].clone()).unwrap(),
    };
    let space: Uuid = serde_json::from_value(f["space"].clone()).unwrap();
    // Before migrations, bind every old business row, receipt, digest and exact ID
    // to the independently produced fixture, not just its first block.
    let old = &f["old_database"];
    assert_eq!(legacy_snapshot::snapshot(&admin, Some(old)).await, *old);
    assert_eq!(old["app_user"]["count"], 1);
    assert_eq!(old["space"]["count"], 1);
    assert_eq!(old["block"]["count"], 3);
    assert_eq!(old["block_revision"]["count"], 4);
    assert_eq!(old["composition_revision"]["count"], 2);
    assert_eq!(old["overlay_revision"]["count"], 3);
    assert_eq!(old["reading_view_revision"]["count"], 3);
    assert_eq!(old["reading_receipt_block"]["count"], 1);
    assert_eq!(old["placement_migration"]["count"], 1);
    assert_eq!(old["placement_migration_decision"]["count"], 1);
    assert_eq!(old["release"]["count"], 1);
    assert_eq!(old["outbox_event"]["count"], 1);
    for entry in f["wire"].as_object().unwrap().values() {
        let bytes = entry["bytes"].as_str().unwrap();
        assert_eq!(hex_digest(bytes.as_bytes()), entry["sha256"]);
        assert_eq!(serde_json::from_str::<Value>(bytes).unwrap(), entry["json"]);
    }
    MIGRATOR.run(&admin).await.unwrap();
    let sums: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations WHERE version<=3 ORDER BY version",
    )
    .fetch_all(&admin)
    .await
    .unwrap();
    assert_eq!(json!(sums), f["checksums"]);
    let versions: Vec<i64> =
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&admin)
            .await
            .unwrap();
    assert_eq!(versions, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    assert_eq!(legacy_snapshot::snapshot(&admin, Some(old)).await, *old);
    let content = ContentStore::new(runtime.clone());
    let compositions = CompositionStore::new(runtime.clone());
    let readings = ReadingStore::new(runtime.clone());
    let migrations = MigrationStore::new(runtime.clone());
    let releases = ReleaseStore::new(runtime);
    let k = wire!(
        f,
        "k1",
        content
            .create(actor, space, command!(f, "k_create"))
            .await
            .unwrap()
    );
    let m = wire!(
        f,
        "m1",
        content
            .create(actor, space, command!(f, "m_create"))
            .await
            .unwrap()
    );
    let d1 = wire!(
        f,
        "d1",
        compositions
            .save(actor, space, command!(f, "d1_save"))
            .await
            .unwrap()
    );
    let r0 = wire!(
        f,
        "r0",
        readings
            .create(actor, space, command!(f, "r0_create"))
            .await
            .unwrap()
    );
    let r1 = wire!(
        f,
        "r1",
        readings
            .edit(actor, r0.overlay.overlay_id, command!(f, "r1_edit"))
            .await
            .unwrap()
    );
    let k2 = wire!(
        f,
        "k2",
        content
            .revise(actor, k.block_id, command!(f, "k2_revise"))
            .await
            .unwrap()
    );
    let d2 = wire!(
        f,
        "d2",
        compositions
            .save(actor, space, command!(f, "d2_save"))
            .await
            .unwrap()
    );
    let proposal = wire!(
        f,
        "proposal_replay",
        migrations
            .propose(actor, r0.overlay.overlay_id, command!(f, "propose"))
            .await
            .unwrap()
    );
    let decision = wire!(
        f,
        "decision",
        migrations
            .decide(actor, r0.overlay.overlay_id, command!(f, "decide"))
            .await
            .unwrap()
    );
    let r2 = wire!(f, "r2", decision.adopted.unwrap());
    wire!(
        f,
        "proposal_final",
        migrations.read(actor, proposal.proposal_id).await.unwrap()
    );
    let state = wire!(
        f,
        "state",
        readings.state(actor, r0.overlay.overlay_id).await.unwrap()
    )
    .unwrap();
    let n = wire!(
        f,
        "n1",
        content
            .read(actor, r1.changed_blocks[0].revision_id)
            .await
            .unwrap()
            .unwrap()
    );
    for (label, rev, text) in [
        ("k1", &k, "K"),
        ("m1", &m, "M"),
        ("n1", &n, "N"),
        ("k2", &k2, "K2"),
    ] {
        wire!(
            f,
            label,
            content.read(actor, rev.revision_id).await.unwrap().unwrap()
        );
        assert_eq!(rev.draft.payload.text, text);
        // Fixed old draft, independent of the stored digest or upgraded writer.
        let expected = json!({"contract_version":1,"kind":"text","intent":"note","language":"zh-CN","title":"私有标题","payload":{"format":"markdown","text":text}});
        assert_eq!(
            rev.content_sha256,
            hex_digest(canonical_json(&expected).as_bytes())
        );
    }
    assert_eq!(k2.block_id, k.block_id);
    let all_v1: bool =
        sqlx::query_scalar("SELECT bool_and(contract_version=1) FROM block_revision")
            .fetch_one(&admin)
            .await
            .unwrap();
    assert!(all_v1);
    assert_eq!(k2.parent_revision_id, Some(k.revision_id));
    assert_ne!(k2.revision_id, k.revision_id);
    assert_eq!(d1.nodes.len(), 2);
    assert_eq!(d2.nodes.len(), 2);
    assert_eq!(d1.nodes[0].target, NodeTarget::Block(exact(&k)));
    assert_eq!(d2.nodes[0].target, NodeTarget::Block(exact(&k2)));
    assert_eq!(d1.nodes[1].target, NodeTarget::Block(exact(&m)));
    assert_eq!(d2.nodes[1].target, NodeTarget::Block(exact(&m)));
    let ok = d1.nodes[0].occurrence_id;
    let om = d1.nodes[1].occurrence_id;
    assert_eq!(
        [d2.nodes[0].occurrence_id, d2.nodes[1].occurrence_id],
        [ok, om]
    );
    let editable = state.editable.as_ref().unwrap();
    assert_eq!(editable.groups.len(), 1);
    assert_eq!(editable.groups[0].placements.len(), 1);
    let placement = &editable.groups[0].placements[0];
    assert_eq!(placement.block, exact(&n));
    assert_eq!(proposal.groups.len(), 1);
    assert_eq!(
        proposal.groups[0].classification,
        Some(MigrationClass::Exact)
    );
    assert_eq!(
        proposal.groups[0].reason_code.as_deref(),
        Some("neighbors_unchanged")
    );
    assert_eq!(proposal.groups[0].group_id, editable.groups[0].group_id);
    assert_eq!(proposal.groups[0].placement_ids, [placement.placement_id]);
    assert_eq!(proposal.decisions.len(), 1);
    assert_eq!(proposal.decisions[0].decision_id, decision.decision_id);
    assert!(proposal.decisions[0].adopted);
    assert_eq!(
        f["wire"]["proposal_initial"]["json"]["decisions"],
        json!([])
    );
    for (name, saved, doc, k_rev) in [
        ("r0", &r0, &d1, &k),
        ("r1", &r1, &d1, &k),
        ("r2", &r2, &d2, &k2),
    ] {
        assert_eq!(saved.overlay.overlay_id, r0.overlay.overlay_id);
        assert_eq!(saved.view.view_id, r0.view.view_id);
        let expected_anchor = GapAnchor {
            base: doc.reference.clone(),
            parent_occurrence_path: vec![],
            left_occurrence_id: Some(ok),
            right_occurrence_id: Some(om),
            affinity: Affinity::AfterLeft,
        };
        for (mode_name, mode) in [
            ("original", ReadingMode::Original),
            ("fused", ReadingMode::Fused),
            ("personal", ReadingMode::Personal),
        ] {
            let p = wire!(
                f,
                &format!("{name}_{mode_name}"),
                readings
                    .read(actor, saved.view.clone(), mode)
                    .await
                    .unwrap()
            )
            .unwrap();
            assert_eq!(p.overlay, saved.overlay);
            assert_eq!(p.view, saved.view);
            assert!(
                matches!(p.source,SourceProjection::Available{ref base} if *base==doc.reference)
            );
            assert!(p.unplaced.is_empty());
            let original_k = json!({"type":"original","path":[ok],"revision":k_rev});
            let original_m = json!({"type":"original","path":[om],"revision":m});
            let personal = json!({"type":"personal","item":{"placement_id":placement.placement_id,"revision":n,"location":expected_anchor}});
            let expected = match (name, mode) {
                (_, ReadingMode::Original) | ("r0", ReadingMode::Fused) => {
                    vec![original_k, original_m]
                }
                ("r0", ReadingMode::Personal) => vec![],
                (_, ReadingMode::Personal) => vec![personal],
                (_, ReadingMode::Fused) => vec![original_k, personal, original_m],
            };
            assert_eq!(json!(p.items), json!(expected));
            let wire = json!(p);
            assert!(wire.get("evidence").is_none());
            assert!(wire.get("contract_version").is_none());
        }
    }
    assert_eq!(
        [
            r0.overlay.revision_id,
            r1.overlay.revision_id,
            r2.overlay.revision_id
        ]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len(),
        3
    );
    assert_eq!(
        [
            r0.view.revision_id,
            r1.view.revision_id,
            r2.view.revision_id
        ]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len(),
        3
    );
    for (name, doc) in [("d1_snapshot", &d1), ("d2_snapshot", &d2)] {
        wire!(
            f,
            name,
            compositions
                .read(actor, doc.reference.clone())
                .await
                .unwrap()
        );
    }
    let release = wire!(
        f,
        "release",
        releases
            .publish(actor, space, command!(f, "publish"))
            .await
            .unwrap()
    );
    assert_eq!(release.roots, std::slice::from_ref(&d2.reference));
    wire!(
        f,
        "release_read",
        releases.read(actor, release.release_id).await.unwrap()
    );
    wire!(
        f,
        "active",
        releases
            .active(actor, d2.reference.composition_id)
            .await
            .unwrap()
    );
    wire!(
        f,
        "publication_state",
        releases
            .state(actor, d2.reference.composition_id)
            .await
            .unwrap()
    );
    // This includes exact request_key/receipt bytes and release/outbox cardinality.
    // Only newly added registry/dependency/index columns and tables are excluded.
    assert_eq!(legacy_snapshot::snapshot(&admin, Some(old)).await, *old);
}

mod support;
use learning_core::*;
use learning_db::MigrationStore;
use support::{assembly as a, reading as h, *};
use uuid::Uuid;

#[tokio::test]
async fn hidden_current_source_suppresses_visible_historical_anchor_and_filters_migration_sides() {
    let (r, actor, space, doc, zero) = h::fixture().await;
    let store = h::store(&r);
    let one = store
        .edit(
            actor,
            zero.overlay.overlay_id,
            h::edit(&zero, h::add(h::gap(&doc, 1), "N")),
        )
        .await
        .unwrap();
    let (other, external) = r.seed_actor_space(true).await;
    let b = r
        .store
        .create(other, external, command("TARGET_SECRET"))
        .await
        .unwrap();
    r.grant(actor, external, false).await;
    let mut dc = a::edit(&doc);
    dc.nodes = vec![NodeDraft {
        occurrence_id: None,
        target: a::block(&b),
    }];
    let next = r.compositions().save(actor, space, dc).await.unwrap();
    let m = MigrationStore::new(r.runtime_pool.clone());
    let p = m
        .propose(
            actor,
            one.overlay.overlay_id,
            propose(&one, next.reference.clone()),
        )
        .await
        .unwrap();
    let g = p.groups[0].group_id;
    let c = decide(
        &one,
        p.proposal_id,
        MigrationAction::Adopt {
            groups: vec![GroupDecision::KeepUnplaced { group_id: g }],
            merges: vec![],
        },
    );
    let two = m
        .decide(actor, one.overlay.overlay_id, c.clone())
        .await
        .unwrap()
        .adopted
        .unwrap();
    r.revoke(actor, external).await;
    let reading = store
        .read(actor, two.view, ReadingMode::Fused)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(reading.source, SourceProjection::Unavailable));
    assert_eq!(reading.unplaced.len(), 1);
    assert!(
        reading.unplaced[0].location.is_none(),
        "source unavailable must suppress even independently readable prior anchors"
    );
    let projection = m.read(actor, p.proposal_id).await.unwrap().unwrap();
    assert!(projection.target.is_none());
    assert!(projection.groups[0].classification.is_none());
    assert!(projection.groups[0].reason_code.is_none());
    assert!(projection.groups[0].suggested_anchor.is_none());
    assert!(projection.groups[0].old_anchor.is_some());
    assert_eq!(projection.decisions.len(), 1);
    let json = serde_json::to_string(&projection).unwrap();
    assert!(!json.contains(&next.reference.revision_id.to_string()));
    assert!(!json.contains("TARGET_SECRET"));
    assert!(matches!(
        m.decide(actor, one.overlay.overlay_id, c).await,
        Err(ContentError::NotFound)
    ));
}

#[tokio::test]
async fn collision_requires_explicit_stable_merge_and_receipt_failure_rolls_back() {
    let (r, actor, space, doc, zero) = h::fixture().await;
    let store = h::store(&r);
    let one = store
        .edit(
            actor,
            zero.overlay.overlay_id,
            h::edit(&zero, h::add(h::gap(&doc, 0), "N")),
        )
        .await
        .unwrap();
    let two = store
        .edit(
            actor,
            zero.overlay.overlay_id,
            h::edit(&one, h::add(h::gap(&doc, 1), "I")),
        )
        .await
        .unwrap();
    let mut cmd = a::edit(&doc);
    cmd.title = "next".into();
    let next = r.compositions().save(actor, space, cmd).await.unwrap();
    let m = MigrationStore::new(r.runtime_pool.clone());
    let p = m
        .propose(
            actor,
            two.overlay.overlay_id,
            propose(&two, next.reference.clone()),
        )
        .await
        .unwrap();
    let st = store
        .state(actor, two.overlay.overlay_id)
        .await
        .unwrap()
        .unwrap()
        .editable
        .unwrap();
    let groups = p
        .groups
        .iter()
        .map(|g| GroupDecision::Manual {
            group_id: g.group_id,
            anchor: h::gap(&next, 1),
        })
        .collect::<Vec<_>>();
    let before = h::counts(&r, actor).await;
    assert!(
        matches!(m.decide(actor,two.overlay.overlay_id,decide(&two,p.proposal_id,MigrationAction::Adopt{groups:groups.clone(),merges:vec![]})).await,Err(ContentError::Invalid(ref s)) if s=="merge_required")
    );
    assert_eq!(h::counts(&r, actor).await, before);
    let ids = st.groups.iter().map(|g| g.group_id).collect();
    let placement_ids = st
        .groups
        .iter()
        .flat_map(|g| g.placements.iter().map(|p| p.placement_id))
        .collect();
    let command = decide(
        &two,
        p.proposal_id,
        MigrationAction::Adopt {
            groups,
            merges: vec![MergeOrder {
                source_group_ids: ids,
                placement_ids,
            }],
        },
    );
    let f = format!("migration_fail_{}", actor.actor_id.simple());
    sqlx::query(&format!("CREATE FUNCTION {f}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.actor_id='{}'::uuid THEN RAISE EXCEPTION 'injected'; END IF; RETURN NEW; END $$",actor.actor_id)).execute(&r.admin_pool).await.unwrap();
    sqlx::query(&format!(
        "CREATE TRIGGER {f} BEFORE INSERT ON migration_receipt FOR EACH ROW EXECUTE FUNCTION {f}()"
    ))
    .execute(&r.admin_pool)
    .await
    .unwrap();
    assert!(matches!(
        m.decide(actor, two.overlay.overlay_id, command.clone())
            .await,
        Err(ContentError::Storage)
    ));
    assert_eq!(h::counts(&r, actor).await, before);
    assert_eq!(
        store
            .state(actor, two.overlay.overlay_id)
            .await
            .unwrap()
            .unwrap()
            .view,
        two.view
    );
    sqlx::query(&format!("DROP TRIGGER {f} ON migration_receipt"))
        .execute(&r.admin_pool)
        .await
        .unwrap();
    sqlx::query(&format!("DROP FUNCTION {f}()"))
        .execute(&r.admin_pool)
        .await
        .unwrap();
    let d = m
        .decide(actor, two.overlay.overlay_id, command)
        .await
        .unwrap();
    let saved = d.adopted.unwrap();
    let new = store
        .state(actor, saved.overlay.overlay_id)
        .await
        .unwrap()
        .unwrap()
        .editable
        .unwrap();
    assert_eq!(new.groups.len(), 1);
    assert_eq!(new.groups[0].placements.len(), 2);
    assert!(
        !st.groups
            .iter()
            .any(|g| g.group_id == new.groups[0].group_id)
    );
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM placement_migration_mapping WHERE decision_id=$1")
            .bind(d.decision_id)
            .fetch_one(&r.admin_pool)
            .await
            .unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn unplaced_keeps_actual_origin_across_upgrades_and_can_be_placed() {
    let (r, actor, space, doc, zero) = h::fixture().await;
    let store = h::store(&r);
    let one = store
        .edit(
            actor,
            zero.overlay.overlay_id,
            h::edit(&zero, h::add(h::gap(&doc, 1), "N")),
        )
        .await
        .unwrap();
    let m = MigrationStore::new(r.runtime_pool.clone());
    let mut dc = a::edit(&doc);
    dc.nodes.clear();
    let next = r.compositions().save(actor, space, dc).await.unwrap();
    let p = m
        .propose(
            actor,
            one.overlay.overlay_id,
            propose(&one, next.reference.clone()),
        )
        .await
        .unwrap();
    assert_eq!(p.groups[0].classification, Some(MigrationClass::Unresolved));
    let gid = p.groups[0].group_id;
    let two = m
        .decide(
            actor,
            one.overlay.overlay_id,
            decide(
                &one,
                p.proposal_id,
                MigrationAction::Adopt {
                    groups: vec![GroupDecision::KeepUnplaced { group_id: gid }],
                    merges: vec![],
                },
            ),
        )
        .await
        .unwrap()
        .adopted
        .unwrap();
    let mut dc = a::edit(&next);
    dc.title = "third".into();
    let third = r.compositions().save(actor, space, dc).await.unwrap();
    let p = m
        .propose(
            actor,
            two.overlay.overlay_id,
            propose(&two, third.reference.clone()),
        )
        .await
        .unwrap();
    assert_eq!(p.groups[0].old_anchor.as_ref().unwrap().base, doc.reference);
    assert_eq!(p.groups[0].classification, Some(MigrationClass::Unresolved));
    let three = m
        .decide(
            actor,
            two.overlay.overlay_id,
            decide(
                &two,
                p.proposal_id,
                MigrationAction::Adopt {
                    groups: vec![GroupDecision::KeepUnplaced { group_id: gid }],
                    merges: vec![],
                },
            ),
        )
        .await
        .unwrap()
        .adopted
        .unwrap();
    let state = store
        .state(actor, three.overlay.overlay_id)
        .await
        .unwrap()
        .unwrap()
        .editable
        .unwrap();
    assert_eq!(state.groups[0].location.anchor().base, doc.reference);
    let before = h::counts(&r, actor).await;
    let four = store
        .edit(
            actor,
            three.overlay.overlay_id,
            h::edit(
                &three,
                ReadingEdit::PlaceUnplaced {
                    group_id: gid,
                    anchor: h::gap(&third, 0),
                    merge_into: None,
                    merged_order: vec![],
                },
            ),
        )
        .await
        .unwrap();
    assert_eq!(&h::counts(&r, actor).await[..2], &before[..2]);
    assert_eq!(
        h::text(
            &store
                .read(actor, four.view, ReadingMode::Fused)
                .await
                .unwrap()
                .unwrap()
        ),
        ["N"]
    );
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM placement_manual_decision WHERE overlay_id=$1")
            .bind(three.overlay.overlay_id)
            .fetch_one(&r.admin_pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
}
fn propose(s: &ReadingSaved, target: CompositionRef) -> ProposeMigration {
    ProposeMigration {
        request_id: Uuid::new_v4(),
        expected_overlay_revision: s.overlay.revision_id,
        expected_reading_view_revision: s.view.revision_id,
        target,
        reason: "upgrade".into(),
    }
}
fn decide(s: &ReadingSaved, p: Uuid, action: MigrationAction) -> DecideMigration {
    DecideMigration {
        request_id: Uuid::new_v4(),
        proposal_id: p,
        expected_overlay_revision: s.overlay.revision_id,
        expected_reading_view_revision: s.view.revision_id,
        action,
        reason: "reviewed".into(),
    }
}
#[tokio::test]
async fn candidate_requires_explicit_adoption_and_keeps_old_view() {
    let (r, actor, space, doc, zero) = h::fixture().await;
    let store = h::store(&r);
    let one = store
        .edit(
            actor,
            zero.overlay.overlay_id,
            h::edit(&zero, h::add(h::gap(&doc, 1), "N")),
        )
        .await
        .unwrap();
    let x = r.store.create(actor, space, command("X")).await.unwrap();
    let mut cmd = a::edit(&doc);
    cmd.nodes.insert(
        1,
        NodeDraft {
            occurrence_id: None,
            target: a::block(&x),
        },
    );
    let next = r.compositions().save(actor, space, cmd).await.unwrap();
    let m = MigrationStore::new(r.runtime_pool.clone());
    let c = propose(&one, next.reference.clone());
    let p = m
        .propose(actor, one.overlay.overlay_id, c.clone())
        .await
        .unwrap();
    assert_eq!(p.groups.len(), 1);
    assert_eq!(p.groups[0].classification, Some(MigrationClass::Candidate));
    assert_eq!(
        p.groups[0]
            .suggested_anchor
            .as_ref()
            .unwrap()
            .right_occurrence_id,
        Some(next.nodes[1].occurrence_id)
    );
    assert_eq!(
        store
            .state(actor, one.overlay.overlay_id)
            .await
            .unwrap()
            .unwrap()
            .view,
        one.view
    );
    let dc = decide(
        &one,
        p.proposal_id,
        MigrationAction::Adopt {
            groups: vec![GroupDecision::AcceptCandidate {
                group_id: p.groups[0].group_id,
            }],
            merges: vec![],
        },
    );
    let d = m
        .decide(actor, one.overlay.overlay_id, dc.clone())
        .await
        .unwrap();
    let saved = d.adopted.clone().unwrap();
    assert_eq!(
        h::text(
            &store
                .read(actor, saved.view, ReadingMode::Fused)
                .await
                .unwrap()
                .unwrap()
        ),
        ["K", "N", "X", "M"]
    );
    assert_eq!(
        h::text(
            &store
                .read(actor, one.view, ReadingMode::Fused)
                .await
                .unwrap()
                .unwrap()
        ),
        ["K", "N", "M"]
    );
    assert_eq!(
        m.decide(actor, one.overlay.overlay_id, dc).await.unwrap(),
        d
    );
    assert_eq!(
        m.propose(actor, one.overlay.overlay_id, c)
            .await
            .unwrap()
            .proposal_id,
        p.proposal_id
    );
}
#[tokio::test]
async fn reject_is_immutable_and_does_not_advance_heads() {
    let (r, actor, space, doc, zero) = h::fixture().await;
    let mut edit = a::edit(&doc);
    edit.title = "v2".into();
    let next = r.compositions().save(actor, space, edit).await.unwrap();
    let m = MigrationStore::new(r.runtime_pool.clone());
    let p = m
        .propose(
            actor,
            zero.overlay.overlay_id,
            propose(&zero, next.reference),
        )
        .await
        .unwrap();
    let c = decide(&zero, p.proposal_id, MigrationAction::Reject);
    let d = m
        .decide(actor, zero.overlay.overlay_id, c.clone())
        .await
        .unwrap();
    assert!(d.adopted.is_none());
    assert_eq!(
        m.decide(actor, zero.overlay.overlay_id, c).await.unwrap(),
        d
    );
    assert_eq!(
        h::store(&r)
            .state(actor, zero.overlay.overlay_id)
            .await
            .unwrap()
            .unwrap()
            .view,
        zero.view
    );
    let e = m
        .decide(
            actor,
            zero.overlay.overlay_id,
            decide(&zero, p.proposal_id, MigrationAction::Reject),
        )
        .await
        .unwrap_err();
    assert!(matches!(e,ContentError::Invalid(ref s) if s=="proposal_decided"));
    assert_eq!(
        m.read(actor, p.proposal_id)
            .await
            .unwrap()
            .unwrap()
            .decisions
            .len(),
        1
    );
}

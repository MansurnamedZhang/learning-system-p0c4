#[path = "support/relation_store.rs"]
mod relations;
mod support;
use learning_core::*;
use learning_db::{MigrationStore, RelationStore, VersionedContentStore};
use support::{assembly as a, reading as h};
use uuid::Uuid;

#[tokio::test]
async fn native_v1_release_rejects_nested_v2_bodies_and_preserves_all_v1_control() {
    let r = support::TestRig::from_env().await;
    let (actor, space) = r.seed_actor_space(true).await;
    let old = r
        .store
        .create(actor, space, support::command("v1"))
        .await
        .unwrap();
    let new = VersionedContentStore::new(r.runtime_pool.clone())
        .create(
            actor,
            space,
            CreateContent {
                request_id: Uuid::new_v4(),
                reason: "v2".into(),
                draft: ContentDraft::V2(ContentV2 {
                    intent: Intent::Note,
                    language: "en".into(),
                    title: "v2".into(),
                    body: BodyV2::Text(support::command("body").draft.payload),
                    basis_refs: vec![],
                    requires_context: vec![],
                    source_run: None,
                }),
            },
        )
        .await
        .unwrap();
    for (version, block) in [
        (1, relations::exact(&old)),
        (
            2,
            BlockRef {
                block_id: new.block_id,
                revision_id: new.revision_id,
            },
        ),
    ] {
        let mut child = a::doc(vec![NodeTarget::Block(block)]);
        child.kind = CompositionKind::Section;
        let child = r.compositions().save(actor, space, child).await.unwrap();
        let root = r
            .compositions()
            .save(actor, space, a::doc(vec![a::child(&child)]))
            .await
            .unwrap();
        let id = Uuid::new_v4();
        let mut tx = r.runtime_pool.begin().await.unwrap();
        sqlx::query(
            "INSERT INTO release(id,space_id,author_id,reason) VALUES($1,$2,$3,'native v1')",
        )
        .bind(id)
        .bind(space)
        .bind(actor.actor_id)
        .execute(&mut *tx)
        .await
        .unwrap();
        sqlx::query("INSERT INTO release_root(release_id,space_id,composition_id,revision_id) VALUES($1,$2,$3,$4)").bind(id).bind(space).bind(root.reference.composition_id).bind(root.reference.revision_id).execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO outbox_event(id,event_type,aggregate_id,payload_version) VALUES($1,'composition_released',$2,1)").bind(Uuid::new_v4()).bind(id).execute(&mut *tx).await.unwrap();
        let result = tx.commit().await;
        if version == 1 {
            result.unwrap();
            assert_eq!(
                r.releases().read(actor, id).await.unwrap().unwrap().roots,
                [root.reference]
            );
        } else {
            assert_eq!(
                support::sqlstate(&result.unwrap_err()).as_deref(),
                Some("23514")
            );
            assert!(
                !sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM release WHERE id=$1)")
                    .bind(id)
                    .fetch_one(&r.admin_pool)
                    .await
                    .unwrap()
            );
        }
    }
}

async fn selected_fixture() -> (
    support::TestRig,
    Principal,
    Uuid,
    Uuid,
    CompositionRevision,
    ReadingSaved,
) {
    let (r, actor, space, doc, zero) = h::fixture().await;
    let (owner, foreign) = r.seed_actor_space(true).await;
    r.grant(actor, foreign, false).await;
    let endpoint = r
        .store
        .create(
            owner,
            foreign,
            support::command("foreign necessary endpoint"),
        )
        .await
        .unwrap();
    let target = r
        .store
        .create(actor, space, support::command("local target"))
        .await
        .unwrap();
    let relation = RelationStore::new(r.runtime_pool.clone())
        .save(
            actor,
            relations::save(
                space,
                relations::exact(&endpoint),
                relations::exact(&target),
            ),
        )
        .await
        .unwrap();
    let selected = h::store(&r)
        .select_relations(
            actor,
            zero.overlay.overlay_id,
            SelectRelations {
                request_id: Uuid::new_v4(),
                expected_overlay_revision: zero.overlay.revision_id,
                expected_reading_view_revision: zero.view.revision_id,
                selections: vec![RelationSelection {
                    relation: relation.reference,
                    review: None,
                }],
                epistemic_reviews: vec![],
                reason: "fixed evidence".into(),
            },
        )
        .await
        .unwrap();
    (r, actor, space, foreign, doc, selected)
}
async fn clear_current(r: &support::TestRig, actor: Principal, saved: &ReadingSaved) {
    h::store(r)
        .select_relations(
            actor,
            saved.overlay.overlay_id,
            SelectRelations {
                request_id: Uuid::new_v4(),
                expected_overlay_revision: saved.overlay.revision_id,
                expected_reading_view_revision: saved.view.revision_id,
                selections: vec![],
                epistemic_reviews: vec![],
                reason: "head has no evidence; historical receipt still does".into(),
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn edit_receipt_replay_authorizes_its_exact_carried_evidence_after_revocation() {
    let (r, actor, _, foreign, doc, selected) = selected_fixture().await;
    let store = h::store(&r);
    let command = h::edit(&selected, h::add(h::gap(&doc, 1), "personal edit"));
    let saved = store
        .edit(actor, selected.overlay.overlay_id, command.clone())
        .await
        .unwrap();
    clear_current(&r, actor, &saved).await;
    let before = h::counts(&r, actor).await;
    assert_eq!(
        store
            .edit(actor, selected.overlay.overlay_id, command.clone())
            .await
            .unwrap(),
        saved
    );
    r.revoke(actor, foreign).await;
    assert!(matches!(
        store
            .edit(actor, selected.overlay.overlay_id, command.clone())
            .await,
        Err(ContentError::NotFound)
    ));
    assert_eq!(h::counts(&r, actor).await, before);
    r.grant(actor, foreign, false).await;
    assert_eq!(
        store
            .edit(actor, selected.overlay.overlay_id, command)
            .await
            .unwrap(),
        saved
    );
    assert_eq!(h::counts(&r, actor).await, before);
}

#[tokio::test]
async fn adopted_migration_receipt_replay_authorizes_exact_result_evidence_after_revocation() {
    let (r, actor, space, foreign, doc, selected) = selected_fixture().await;
    let one = h::store(&r)
        .edit(
            actor,
            selected.overlay.overlay_id,
            h::edit(&selected, h::add(h::gap(&doc, 1), "personal note")),
        )
        .await
        .unwrap();
    let mut revision = a::edit(&doc);
    revision.title = "new source".into();
    let next = r.compositions().save(actor, space, revision).await.unwrap();
    let store = MigrationStore::new(r.runtime_pool.clone());
    let proposal = store
        .propose(
            actor,
            one.overlay.overlay_id,
            ProposeMigration {
                request_id: Uuid::new_v4(),
                expected_overlay_revision: one.overlay.revision_id,
                expected_reading_view_revision: one.view.revision_id,
                target: next.reference,
                reason: "propose".into(),
            },
        )
        .await
        .unwrap();
    let command = DecideMigration {
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
        reason: "adopt".into(),
    };
    let saved = store
        .decide(actor, one.overlay.overlay_id, command.clone())
        .await
        .unwrap()
        .adopted
        .unwrap();
    clear_current(&r, actor, &saved).await;
    let before = h::counts(&r, actor).await;
    assert_eq!(
        store
            .decide(actor, one.overlay.overlay_id, command.clone())
            .await
            .unwrap()
            .adopted,
        Some(saved.clone())
    );
    r.revoke(actor, foreign).await;
    assert!(matches!(
        store
            .decide(actor, one.overlay.overlay_id, command.clone())
            .await,
        Err(ContentError::NotFound)
    ));
    assert_eq!(h::counts(&r, actor).await, before);
    r.grant(actor, foreign, false).await;
    assert_eq!(
        store
            .decide(actor, one.overlay.overlay_id, command)
            .await
            .unwrap()
            .adopted,
        Some(saved)
    );
    assert_eq!(h::counts(&r, actor).await, before);
}

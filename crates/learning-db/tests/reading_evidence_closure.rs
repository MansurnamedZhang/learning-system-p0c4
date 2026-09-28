#[path = "support/relation_store.rs"]
mod relations;
mod support;
use learning_core::*;
use learning_db::{
    MigrationStore, ReadingStore, RelationStore, ReleaseStore, VersionedContentStore,
};
use support::{assembly as a, reading as h};
use uuid::Uuid;

#[tokio::test]
async fn v2_release_includes_old_origins_personal_blocks_and_transitive_source_dependencies() {
    let r = support::TestRig::from_env().await;
    let (actor, space) = r.seed_actor_space(true).await;
    let (other, foreign) = r.seed_actor_space(true).await;
    r.grant(actor, foreign, false).await;
    let old = r
        .store
        .create(other, foreign, support::command("old source"))
        .await
        .unwrap();
    let run = r
        .store
        .create(other, foreign, support::command("source run"))
        .await
        .unwrap();
    let doc = r
        .compositions()
        .save(actor, space, a::doc(vec![a::block(&old)]))
        .await
        .unwrap();
    let zero = h::store(&r)
        .create(actor, space, h::create(doc.reference.clone()))
        .await
        .unwrap();
    let one = h::store(&r)
        .edit(
            actor,
            zero.overlay.overlay_id,
            h::edit(&zero, h::add(h::gap(&doc, 1), "personal note")),
        )
        .await
        .unwrap();
    let personal = one.changed_blocks[0].clone();
    let v2 = VersionedContentStore::new(r.runtime_pool.clone())
        .create(
            actor,
            space,
            CreateContent {
                request_id: Uuid::new_v4(),
                reason: "new source".into(),
                draft: ContentDraft::V2(ContentV2 {
                    intent: Intent::Note,
                    language: "en".into(),
                    title: "new body".into(),
                    body: BodyV2::Text(support::command("derived observation").draft.payload),
                    basis_refs: vec![],
                    requires_context: vec![],
                    source_run: Some(relations::exact(&run)),
                }),
            },
        )
        .await
        .unwrap();
    let mut revised = a::edit(&doc);
    revised.nodes = vec![NodeDraft {
        occurrence_id: None,
        target: NodeTarget::Block(BlockRef {
            block_id: v2.block_id,
            revision_id: v2.revision_id,
        }),
    }];
    let next = r.compositions().save(actor, space, revised).await.unwrap();
    let migrations = MigrationStore::new(r.runtime_pool.clone());
    let proposal = migrations
        .propose(
            actor,
            one.overlay.overlay_id,
            ProposeMigration {
                request_id: Uuid::new_v4(),
                expected_overlay_revision: one.overlay.revision_id,
                expected_reading_view_revision: one.view.revision_id,
                target: next.reference.clone(),
                reason: "new source".into(),
            },
        )
        .await
        .unwrap();
    let two = migrations
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
                reason: "keep historical note".into(),
            },
        )
        .await
        .unwrap()
        .adopted
        .unwrap();
    assert!(
        matches!(r.releases().publish(actor,space,a::publish(vec![a::root(&next,None)])).await,Err(ContentError::Invalid(s)) if s=="unsupported_content_version")
    );
    let cmd = PublishEvidence {
        request_id: Uuid::new_v4(),
        roots: vec![a::root(&next, None)],
        readings: vec![two.view.clone()],
        reason: "complete evidence".into(),
    };
    let release = r
        .releases()
        .publish_evidence(actor, space, cmd.clone())
        .await
        .unwrap();
    let objects: Vec<(String, Uuid, Uuid)> = sqlx::query_as(
        "SELECT kind,object_id,revision_id FROM release_manifest_object WHERE release_id=$1",
    )
    .bind(release.release_id)
    .fetch_all(&r.admin_pool)
    .await
    .unwrap();
    for block in [
        relations::exact(&old),
        relations::exact(&run),
        personal,
        BlockRef {
            block_id: v2.block_id,
            revision_id: v2.revision_id,
        },
    ] {
        assert!(
            objects.contains(&("block".into(), block.block_id, block.revision_id)),
            "missing {block:?}"
        );
    }
    let compositions: Vec<Uuid> = sqlx::query_scalar(
        "SELECT revision_id FROM release_manifest_composition WHERE release_id=$1",
    )
    .bind(release.release_id)
    .fetch_all(&r.admin_pool)
    .await
    .unwrap();
    assert!(compositions.contains(&doc.reference.revision_id));
    assert!(compositions.contains(&next.reference.revision_id));
    r.revoke(actor, foreign).await;
    let reading = h::store(&r)
        .read_versioned(actor, two.view, ReadingMode::Personal)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(reading.source, SourceProjection::Unavailable));
    assert_eq!(reading.unplaced.len(), 1);
    assert!(
        r.releases()
            .read_evidence(actor, release.release_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        r.releases()
            .read(actor, release.release_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        r.releases().publish_evidence(actor, space, cmd).await,
        Err(ContentError::NotFound)
    ));
}

#[tokio::test]
async fn immutable_selection_children_require_exact_payload_and_matching_view_membership() {
    let (r, actor, space, doc, zero) = h::fixture().await;
    let sibling = h::store(&r)
        .create(actor, space, h::create(doc.reference))
        .await
        .unwrap();
    let left = r
        .store
        .create(actor, space, support::command("left"))
        .await
        .unwrap();
    let right = r
        .store
        .create(actor, space, support::command("right"))
        .await
        .unwrap();
    let relation = RelationStore::new(r.runtime_pool.clone())
        .save(
            actor,
            relations::save(space, relations::exact(&left), relations::exact(&right)),
        )
        .await
        .unwrap();
    let payload = serde_json::json!({"selections":[{"relation":relation.reference,"review":null}],"epistemic_reviews":[]});
    for case in ["valid", "missing", "wrong_view", "old_v1_append"] {
        let mut tx = r.runtime_pool.begin().await.unwrap();
        let id = if case == "old_v1_append" {
            zero.view.revision_id
        } else {
            Uuid::new_v4()
        };
        if case != "old_v1_append" {
            sqlx::query("INSERT INTO reading_view_revision(id,view_id,overlay_id,overlay_revision_id,parent_revision_id,author_id,contract_version,evidence) VALUES($1,$2,$3,$4,$5,$6,2,$7)")
                .bind(id).bind(zero.view.view_id).bind(zero.overlay.overlay_id).bind(zero.overlay.revision_id).bind(zero.view.revision_id).bind(actor.actor_id).bind(&payload).execute(&mut *tx).await.unwrap();
        }
        if case != "missing" {
            let result=sqlx::query("INSERT INTO reading_relation_selection(view_id,view_revision_id,position,relation_id,relation_revision_id) VALUES($1,$2,0,$3,$4)")
                .bind(if case=="wrong_view"{sibling.view.view_id}else{zero.view.view_id}).bind(id).bind(relation.reference.relation_id).bind(relation.reference.revision_id).execute(&mut *tx).await;
            if case == "wrong_view" {
                assert_eq!(
                    support::sqlstate(&result.unwrap_err()).as_deref(),
                    Some("23503")
                );
                tx.rollback().await.unwrap();
                continue;
            }
            result.unwrap();
        }
        let result = tx.commit().await;
        if case == "valid" {
            result.unwrap();
        } else {
            assert_eq!(
                support::sqlstate(&result.unwrap_err()).as_deref(),
                Some("23514")
            );
        }
    }
}

#[tokio::test]
async fn revocation_committing_while_selection_or_publication_waits_for_grants_rolls_back() {
    let (r, actor, space, doc, zero) = h::fixture().await;
    let (other, foreign) = r.seed_actor_space(true).await;
    r.grant(actor, foreign, false).await;
    let left = r
        .store
        .create(other, foreign, support::command("left"))
        .await
        .unwrap();
    let right = r
        .store
        .create(other, foreign, support::command("right"))
        .await
        .unwrap();
    let relation = RelationStore::new(r.runtime_pool.clone())
        .save(
            other,
            relations::save(foreign, relations::exact(&left), relations::exact(&right)),
        )
        .await
        .unwrap();
    let cmd = SelectRelations {
        request_id: Uuid::new_v4(),
        expected_overlay_revision: zero.overlay.revision_id,
        expected_reading_view_revision: zero.view.revision_id,
        selections: vec![RelationSelection {
            relation: relation.reference.clone(),
            review: None,
        }],
        epistemic_reviews: vec![],
        reason: "foreign relation".into(),
    };
    let selected = h::store(&r)
        .select_relations(actor, zero.overlay.overlay_id, cmd)
        .await
        .unwrap();
    let checked = RelationStore::new(r.runtime_pool.clone())
        .review(other, relations::review(&relation))
        .await
        .unwrap();
    for publish in [false, true] {
        r.grant(actor, foreign, false).await;
        let mut blocker = r.admin_pool.begin().await.unwrap();
        let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *blocker)
            .await
            .unwrap();
        sqlx::query(
            "SELECT can_write FROM space_grant WHERE actor_id=$1 AND space_id=$2 FOR UPDATE",
        )
        .bind(actor.actor_id)
        .bind(foreign)
        .execute(&mut *blocker)
        .await
        .unwrap();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&std::env::var("TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
        let waiter: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&pool)
            .await
            .unwrap();
        let before = h::counts(&r, actor).await;
        let release_before = r.assembly_counts(actor).await;
        let view = selected.view.clone();
        let overlay = selected.overlay.clone();
        let root = a::root(&doc, None);
        let pool_task = pool.clone();
        let selection = RelationSelection {
            relation: relation.reference.clone(),
            review: Some(checked.reference.clone()),
        };
        let task = tokio::spawn(async move {
            if publish {
                ReleaseStore::new(pool_task)
                    .publish_evidence(
                        actor,
                        space,
                        PublishEvidence {
                            request_id: Uuid::new_v4(),
                            roots: vec![root],
                            readings: vec![view],
                            reason: "race".into(),
                        },
                    )
                    .await
                    .map(|_| ())
            } else {
                ReadingStore::new(pool_task)
                    .select_relations(
                        actor,
                        overlay.overlay_id,
                        SelectRelations {
                            request_id: Uuid::new_v4(),
                            expected_overlay_revision: overlay.revision_id,
                            expected_reading_view_revision: view.revision_id,
                            selections: vec![selection],
                            epistemic_reviews: vec![],
                            reason: "race".into(),
                        },
                    )
                    .await
                    .map(|_| ())
            }
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if sqlx::query_scalar::<_, bool>("SELECT $1=ANY(pg_blocking_pids($2))")
                    .bind(blocker_pid)
                    .bind(waiter)
                    .fetch_one(&r.admin_pool)
                    .await
                    .unwrap()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("writer must reach the held grant lock");
        sqlx::query("DELETE FROM space_grant WHERE actor_id=$1 AND space_id=$2")
            .bind(actor.actor_id)
            .bind(foreign)
            .execute(&mut *blocker)
            .await
            .unwrap();
        blocker.commit().await.unwrap();
        assert!(matches!(task.await.unwrap(), Err(ContentError::NotFound)));
        assert_eq!(h::counts(&r, actor).await, before);
        assert_eq!(r.assembly_counts(actor).await, release_before);
        pool.close().await;
    }
}

#[tokio::test]
async fn new_release_reader_authorizes_legacy_closure_before_reporting_version() {
    let r = support::TestRig::from_env().await;
    let (actor, space) = r.seed_actor_space(true).await;
    let (other, foreign) = r.seed_actor_space(true).await;
    r.grant(actor, foreign, false).await;
    let hidden = r
        .store
        .create(other, foreign, support::command("hidden"))
        .await
        .unwrap();
    let doc = r
        .compositions()
        .save(actor, space, a::doc(vec![a::block(&hidden)]))
        .await
        .unwrap();
    let release = r
        .releases()
        .publish(actor, space, a::publish(vec![a::root(&doc, None)]))
        .await
        .unwrap();
    r.revoke(actor, foreign).await;
    assert!(
        r.releases()
            .read_evidence(actor, release.release_id)
            .await
            .unwrap()
            .is_none()
    );
}

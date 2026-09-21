#[path = "support/relation_store.rs"]
mod relations;
mod support;
use learning_core::*;
use learning_db::{RelationStore, ReviewStore};
use support::{assembly as a, reading as h};
use uuid::Uuid;

fn select(saved: &ReadingSaved, relation: RelationRef) -> SelectRelations {
    SelectRelations {
        request_id: Uuid::new_v4(),
        expected_overlay_revision: saved.overlay.revision_id,
        expected_reading_view_revision: saved.view.revision_id,
        selections: vec![RelationSelection {
            relation,
            review: None,
        }],
        epistemic_reviews: vec![],
        reason: "select precise evidence".into(),
    }
}
async fn relation(r: &support::TestRig, actor: Principal, space: Uuid) -> RelationRevision {
    let e = r
        .store
        .create(actor, space, support::command("historical E"))
        .await
        .unwrap();
    let h = r
        .store
        .create(actor, space, support::command("historical H"))
        .await
        .unwrap();
    RelationStore::new(r.runtime_pool.clone())
        .save(
            actor,
            relations::save(space, relations::exact(&e), relations::exact(&h)),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn scope_rejection_and_receipt_failures_leave_no_partial_selection_or_release() {
    let (r, actor, space, doc, zero) = h::fixture().await;
    let sibling = h::store(&r)
        .create(actor, space, h::create(doc.reference.clone()))
        .await
        .unwrap();
    let rel = relation(&r, actor, space).await;
    let mut personal = relations::save(space, rel.from.clone(), rel.to.clone());
    personal.scope = RelationScope::PersonalOverlay {
        space_id: space,
        overlay_id: sibling.overlay.overlay_id,
    };
    let personal = RelationStore::new(r.runtime_pool.clone())
        .save(actor, personal)
        .await
        .unwrap();
    let store = h::store(&r);
    let before = h::counts(&r, actor).await;
    assert!(matches!(
        store
            .select_relations(
                actor,
                zero.overlay.overlay_id,
                select(&zero, personal.reference)
            )
            .await,
        Err(ContentError::NotFound)
    ));
    assert_eq!(h::counts(&r, actor).await, before);
    for receipt in ["reading_receipt", "release_receipt"] {
        let name = format!("fail_evidence_{}", Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE FUNCTION {name}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.actor_id='{}'::uuid THEN RAISE EXCEPTION 'injected'; END IF; RETURN NEW; END $$",actor.actor_id)).execute(&r.admin_pool).await.unwrap();
        sqlx::query(&format!("CREATE TRIGGER {name} BEFORE INSERT ON {receipt} FOR EACH ROW EXECUTE FUNCTION {name}()" )).execute(&r.admin_pool).await.unwrap();
        let before = h::counts(&r, actor).await;
        let releases_before = r.assembly_counts(actor).await;
        let result = if receipt == "reading_receipt" {
            store
                .select_relations(
                    actor,
                    zero.overlay.overlay_id,
                    select(&zero, rel.reference.clone()),
                )
                .await
                .map(|_| ())
        } else {
            r.releases()
                .publish_evidence(
                    actor,
                    space,
                    PublishEvidence {
                        request_id: Uuid::new_v4(),
                        roots: vec![a::root(&doc, None)],
                        readings: vec![zero.view.clone()],
                        reason: "fault".into(),
                    },
                )
                .await
                .map(|_| ())
        };
        sqlx::query(&format!("DROP TRIGGER {name} ON {receipt}"))
            .execute(&r.admin_pool)
            .await
            .unwrap();
        sqlx::query(&format!("DROP FUNCTION {name}()"))
            .execute(&r.admin_pool)
            .await
            .unwrap();
        assert!(matches!(result, Err(ContentError::Storage)));
        assert_eq!(h::counts(&r, actor).await, before);
        assert_eq!(r.assembly_counts(actor).await, releases_before);
        assert_eq!(
            store
                .state(actor, zero.overlay.overlay_id)
                .await
                .unwrap()
                .unwrap()
                .view,
            zero.view
        );
    }
}

#[tokio::test]
async fn publication_token_race_is_atomic_and_same_request_has_one_release() {
    let (r, actor, space, doc, zero) = h::fixture().await;
    let releases = r.releases();
    let cmd = PublishEvidence {
        request_id: Uuid::new_v4(),
        roots: vec![a::root(&doc, None)],
        readings: vec![zero.view],
        reason: "competing publication".into(),
    };
    let mut other = cmd.clone();
    other.request_id = Uuid::new_v4();
    let before = r.assembly_counts(actor).await;
    let (left, right) = tokio::join!(
        releases.publish_evidence(actor, space, cmd.clone()),
        releases.publish_evidence(actor, space, other.clone())
    );
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
    let (success, replay, failure) = match (left, right) {
        (Ok(s), e) => (s, cmd, e),
        (e, Ok(s)) => (s, other, e),
        _ => unreachable!(),
    };
    assert!(matches!(
        failure,
        Err(ContentError::PublicationConflict { .. })
    ));
    assert_eq!(
        releases
            .publish_evidence(actor, space, replay)
            .await
            .unwrap(),
        success
    );
    let after = r.assembly_counts(actor).await;
    assert_eq!(after.1, before.1 + 1);
    assert_eq!(after.2, before.2 + 1);
    assert_eq!(after.3, before.3 + 1);
}

#[tokio::test]
async fn incomplete_judgment_is_identity_free_but_cannot_be_published() {
    let (r, actor, space, doc, zero) = h::fixture().await;
    let (other, foreign) = r.seed_actor_space(true).await;
    r.grant(actor, foreign, false).await;
    let hidden = r
        .store
        .create(other, foreign, support::command("private evidence"))
        .await
        .unwrap();
    let mut target = support::command("hypothesis");
    target.draft.intent = Intent::Conjecture;
    let target = r.store.create(actor, space, target).await.unwrap();
    let judgment = ReviewStore::new(r.runtime_pool.clone())
        .append(
            actor,
            AppendEpistemicReview {
                request_id: Uuid::new_v4(),
                scope: RelationScope::Space { space_id: space },
                target: relations::exact(&target),
                expected_previous: None,
                state: EpistemicState::Untested,
                relations: vec![],
                evidence: vec![relations::exact(&hidden)],
                conditions: "".into(),
                explanation: "".into(),
            },
        )
        .await
        .unwrap();
    let rel = relation(&r, actor, space).await;
    let reviewed = RelationStore::new(r.runtime_pool.clone())
        .review(actor, relations::review(&rel))
        .await
        .unwrap();
    let mut command = select(&zero, rel.reference.clone());
    command.selections[0].review = Some(reviewed.reference.clone());
    command.epistemic_reviews.push(judgment.reference.clone());
    let selected = h::store(&r)
        .select_relations(actor, zero.overlay.overlay_id, command.clone())
        .await
        .unwrap();
    r.revoke(actor, foreign).await;
    let projected = h::store(&r)
        .read_versioned(actor, selected.view.clone(), ReadingMode::Personal)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        projected.evidence.epistemic_reviews,
        [ReviewProjection::Incomplete]
    );
    assert!(
        matches!(&projected.evidence.selections[0].review,Some(ReviewProjection::Available(r)) if r.reference==reviewed.reference)
    );
    let wire = serde_json::to_value(&projected.evidence).unwrap();
    assert_eq!(
        wire["epistemic_reviews"],
        serde_json::json!([{"type":"incomplete"}])
    );
    let before = r.assembly_counts(actor).await;
    assert!(matches!(
        r.releases()
            .publish_evidence(
                actor,
                space,
                PublishEvidence {
                    request_id: Uuid::new_v4(),
                    roots: vec![a::root(&doc, None)],
                    readings: vec![selected.view],
                    reason: "must include all fixed evidence".into()
                }
            )
            .await,
        Err(ContentError::NotFound)
    ));
    assert_eq!(r.assembly_counts(actor).await, before);
    assert!(matches!(
        h::store(&r)
            .select_relations(actor, zero.overlay.overlay_id, command)
            .await,
        Err(ContentError::NotFound)
    ));
}

#[tokio::test]
async fn merged_selection_budget_fails_atomically_even_when_each_relation_is_small_enough() {
    let (r, actor, space, doc, zero) = h::fixture().await;
    let target = r
        .store
        .create(actor, space, support::command("shared target"))
        .await
        .unwrap();
    let store = RelationStore::new(r.runtime_pool.clone());
    let mut selections = vec![];
    for _ in 0..43 {
        let body = r
            .store
            .create(actor, space, support::command(&"x".repeat(200_000)))
            .await
            .unwrap();
        let relation = store
            .save(
                actor,
                relations::save(space, relations::exact(&body), relations::exact(&target)),
            )
            .await
            .unwrap();
        selections.push(RelationSelection {
            relation: relation.reference,
            review: None,
        });
    }
    let before = h::counts(&r, actor).await;
    let cmd = SelectRelations {
        request_id: Uuid::new_v4(),
        expected_overlay_revision: zero.overlay.revision_id,
        expected_reading_view_revision: zero.view.revision_id,
        selections: selections.clone(),
        epistemic_reviews: vec![],
        reason: "merged budget".into(),
    };
    assert!(
        matches!(h::store(&r).select_relations(actor,zero.overlay.overlay_id,cmd).await,Err(ContentError::Invalid(s)) if s=="reference_budget_exceeded")
    );
    assert_eq!(h::counts(&r, actor).await, before);
    let sibling = h::store(&r)
        .create(actor, space, h::create(doc.reference.clone()))
        .await
        .unwrap();
    let mut views = vec![];
    for (saved, half) in [(&zero, &selections[..22]), (&sibling, &selections[22..])] {
        let selected = h::store(&r)
            .select_relations(
                actor,
                saved.overlay.overlay_id,
                SelectRelations {
                    request_id: Uuid::new_v4(),
                    expected_overlay_revision: saved.overlay.revision_id,
                    expected_reading_view_revision: saved.view.revision_id,
                    selections: half.to_vec(),
                    epistemic_reviews: vec![],
                    reason: "individual budget fits".into(),
                },
            )
            .await
            .unwrap();
        views.push(selected.view);
    }
    let before = r.assembly_counts(actor).await;
    let result = r
        .releases()
        .publish_evidence(
            actor,
            space,
            PublishEvidence {
                request_id: Uuid::new_v4(),
                roots: vec![a::root(&doc, None)],
                readings: views,
                reason: "merged publication budget".into(),
            },
        )
        .await;
    assert!(matches!(result,Err(ContentError::Invalid(s)) if s=="reference_budget_exceeded"));
    assert_eq!(r.assembly_counts(actor).await, before);
}

#[tokio::test]
async fn manifest_is_exact_sorted_and_database_rejects_missing_extra_and_forged_entries() {
    let (r, actor, space, doc, zero) = h::fixture().await;
    let rel = relation(&r, actor, space).await;
    let selected = h::store(&r)
        .select_relations(actor, zero.overlay.overlay_id, select(&zero, rel.reference))
        .await
        .unwrap();
    let release = r
        .releases()
        .publish_evidence(
            actor,
            space,
            PublishEvidence {
                request_id: Uuid::new_v4(),
                roots: vec![a::root(&doc, None)],
                readings: vec![selected.view],
                reason: "manifest".into(),
            },
        )
        .await
        .unwrap();
    // SQL independently reconstructs the complete set from composition, reading,
    // and dependency rows. Compare its canonical bytes to the public hash.
    let canonical: String = sqlx::query_scalar("SELECT b3_canonical_json(b3_release_manifest($1))")
        .bind(release.release_id)
        .fetch_one(&r.admin_pool)
        .await
        .unwrap();
    assert_eq!(hex_digest(canonical.as_bytes()), release.manifest_sha256);
    let extra = r
        .store
        .create(actor, space, support::command("not in publication"))
        .await
        .unwrap();
    for case in ["valid", "missing", "extra", "forged_hash"] {
        let id = Uuid::new_v4();
        let mut tx = r.runtime_pool.begin().await.unwrap();
        sqlx::query("INSERT INTO release(id,space_id,author_id,reason,contract_version,manifest_sha256) SELECT $1,space_id,author_id,reason,2,CASE WHEN $3 THEN repeat('0',64) ELSE manifest_sha256 END FROM release WHERE id=$2")
            .bind(id).bind(release.release_id).bind(case=="forged_hash").execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO release_root SELECT $1,space_id,composition_id,revision_id FROM release_root WHERE release_id=$2").bind(id).bind(release.release_id).execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO release_reading SELECT $1,view_id,view_revision_id,overlay_id,overlay_revision_id FROM release_reading WHERE release_id=$2").bind(id).bind(release.release_id).execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO release_manifest_composition SELECT $1,space_id,composition_id,revision_id FROM release_manifest_composition WHERE release_id=$2").bind(id).bind(release.release_id).execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO release_manifest_object SELECT $1,kind,object_id,revision_id FROM release_manifest_object WHERE release_id=$2 ORDER BY kind,object_id,revision_id OFFSET $3")
            .bind(id).bind(release.release_id).bind(i64::from(case=="missing")).execute(&mut *tx).await.unwrap();
        if case == "extra" {
            sqlx::query("INSERT INTO release_manifest_object VALUES($1,'block',$2,$3)")
                .bind(id)
                .bind(extra.block_id)
                .bind(extra.revision_id)
                .execute(&mut *tx)
                .await
                .unwrap();
        }
        sqlx::query("INSERT INTO outbox_event(id,event_type,aggregate_id,payload_version) VALUES($1,'composition_released',$2,2)").bind(Uuid::new_v4()).bind(id).execute(&mut *tx).await.unwrap();
        let result = tx.commit().await;
        if case == "valid" {
            result.unwrap();
        } else {
            assert_eq!(
                support::sqlstate(&result.unwrap_err()).as_deref(),
                Some("23514"),
                "{case}"
            );
        }
    }
}

#[tokio::test]
async fn release_freezes_reading_and_full_dependency_manifest_and_revocation_hides_everything() {
    let (r, actor, space, doc, zero) = h::fixture().await;
    let rel = relation(&r, actor, space).await;
    let saved = h::store(&r)
        .select_relations(
            actor,
            zero.overlay.overlay_id,
            select(&zero, rel.reference.clone()),
        )
        .await
        .unwrap();
    let cmd = PublishEvidence {
        request_id: Uuid::new_v4(),
        roots: vec![a::root(&doc, None)],
        readings: vec![saved.view.clone()],
        reason: "release with evidence".into(),
    };
    let releases = r.releases();
    let released = releases
        .publish_evidence(actor, space, cmd.clone())
        .await
        .unwrap();
    assert_eq!(released.readings, [saved.view]);
    assert_eq!(released.manifest_sha256.len(), 64);
    assert_eq!(
        releases.publish_evidence(actor, space, cmd).await.unwrap(),
        released
    );
    assert_eq!(
        releases
            .read_evidence(actor, released.release_id)
            .await
            .unwrap(),
        Some(released.clone())
    );
    assert!(
        matches!(releases.read(actor,released.release_id).await,Err(ContentError::Invalid(s)) if s=="unsupported_content_version")
    );
    let event: i32 =
        sqlx::query_scalar("SELECT payload_version FROM outbox_event WHERE aggregate_id=$1")
            .bind(released.release_id)
            .fetch_one(&r.admin_pool)
            .await
            .unwrap();
    assert_eq!(event, 2);
    let refs:Vec<(String,Uuid,Uuid)>=sqlx::query_as("SELECT kind,object_id,revision_id FROM release_manifest_object WHERE release_id=$1 ORDER BY kind,object_id,revision_id").bind(released.release_id).fetch_all(&r.admin_pool).await.unwrap();
    assert!(refs.contains(&(
        "relation".into(),
        rel.reference.relation_id,
        rel.reference.revision_id
    )));
    assert!(refs.contains(&("block".into(), rel.from.block_id, rel.from.revision_id)));
    assert!(refs.contains(&("block".into(), rel.to.block_id, rel.to.revision_id)));
    r.revoke(actor, space).await;
    assert!(
        releases
            .read_evidence(actor, released.release_id)
            .await
            .unwrap()
            .is_none()
    );
}

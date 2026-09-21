#[path = "support/relation_store.rs"]
mod h;
mod support;
use h::*;
use learning_core::*;
use learning_db::{RelationStore, ReviewStore, VersionedContentStore};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::time::Duration;
use tokio::sync::Barrier;
use uuid::Uuid;

async fn fixture() -> (
    support::TestRig,
    RelationStore,
    Principal,
    Uuid,
    Revision,
    Revision,
) {
    let rig = support::TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let e = rig
        .store
        .create(actor, space, support::command("E"))
        .await
        .unwrap();
    let mut c = support::command("H@1");
    c.draft.intent = Intent::Conjecture;
    let h = rig.store.create(actor, space, c).await.unwrap();
    let relations = RelationStore::new(rig.runtime_pool.clone());
    (rig, relations, actor, space, e, h)
}

fn judgment(r: &RelationRevision, checked: &RelationReview) -> AppendEpistemicReview {
    AppendEpistemicReview {
        request_id: Uuid::new_v4(),
        scope: r.scope.clone(),
        target: r.to.clone(),
        expected_previous: None,
        state: EpistemicState::SupportedWithinScope,
        relations: vec![RelationSelection {
            relation: r.reference.clone(),
            review: Some(checked.reference.clone()),
        }],
        evidence: vec![r.from.clone()],
        conditions: " within these conditions \n".into(),
        explanation: "manual judgment".into(),
    }
}
async fn setup() -> (
    support::TestRig,
    RelationStore,
    ReviewStore,
    Principal,
    Uuid,
    RelationRevision,
    RelationReview,
) {
    let (rig, relations, actor, space, e, h) = fixture().await;
    let r = relations
        .save(actor, save(space, exact(&e), exact(&h)))
        .await
        .unwrap();
    let checked = relations.review(actor, review(&r)).await.unwrap();
    let reviews = ReviewStore::new(rig.runtime_pool.clone());
    (rig, relations, reviews, actor, space, r, checked)
}

#[tokio::test]
async fn fixed_judgments_replay_after_withdrawal_and_authors_have_separate_cas_streams() {
    let (rig, relations, reviews, actor, space, r, checked) = setup().await;
    let c = judgment(&r, &checked);
    let first = reviews.append(actor, c.clone()).await.unwrap();
    assert_eq!(first.reviewer_id, actor.actor_id);
    assert_eq!(first.relations, c.relations);
    assert_eq!(first.conditions, c.conditions);
    let (other, _) = rig.seed_actor_space(true).await;
    rig.grant(other, space, true).await;
    let independent = reviews.append(other, c.clone()).await.unwrap();
    assert_ne!(first.reference.stream_id, independent.reference.stream_id);
    let mut next = c.clone();
    next.request_id = Uuid::new_v4();
    next.expected_previous = Some(first.reference.clone());
    next.state = EpistemicState::Inconclusive;
    let second = reviews.append(actor, next).await.unwrap();
    assert_eq!(second.reference.stream_id, first.reference.stream_id);
    assert_eq!(second.previous, Some(first.reference.clone()));
    let mut stale = c.clone();
    stale.request_id = Uuid::new_v4();
    assert!(
        matches!(reviews.append(actor, stale).await, Err(ContentError::Conflict { current_revision_id }) if current_revision_id == second.reference.review_id)
    );
    let mut withdraw = review(&r);
    withdraw.expected_previous = Some(checked.reference.review_id);
    withdraw.state = RelationReviewState::Withdrawn;
    relations.review(actor, withdraw).await.unwrap();
    assert_eq!(reviews.append(actor, c.clone()).await.unwrap(), first);
    assert_eq!(
        reviews.read(actor, first.reference.clone()).await.unwrap(),
        ReviewProjection::Available(first)
    );
    let mut changed = c.clone();
    changed.explanation.push('!');
    assert!(matches!(
        reviews.append(actor, changed).await,
        Err(ContentError::IdempotencyConflict)
    ));
    let mut legacy = support::command("collision");
    legacy.request_id = c.request_id;
    assert!(matches!(
        rig.store.create(actor, space, legacy).await,
        Err(ContentError::IdempotencyConflict)
    ));
    let mut fresh = c;
    fresh.request_id = Uuid::new_v4();
    fresh.expected_previous = Some(second.reference);
    assert!(matches!(
        reviews.append(actor, fresh).await,
        Err(ContentError::Invalid(_))
    ));
}

#[tokio::test]
async fn exact_target_scope_evidence_direction_and_review_state_are_required() {
    let (rig, relations, reviews, actor, space, r, checked) = setup().await;
    let c = judgment(&r, &checked);
    let mut h2_draft = support::command("H@2").draft;
    h2_draft.intent = Intent::Conjecture;
    let h2 = VersionedContentStore::new(rig.runtime_pool.clone())
        .revise(
            actor,
            r.to.block_id,
            ReviseContent {
                request_id: Uuid::new_v4(),
                base_revision_id: r.to.revision_id,
                draft: ContentDraft::V1(h2_draft),
                reason: "next".into(),
            },
        )
        .await
        .unwrap();
    let (_, other_space) = rig.seed_actor_space(true).await;
    rig.grant(actor, other_space, true).await;
    for variant in 0..7 {
        let mut bad = c.clone();
        bad.request_id = Uuid::new_v4();
        match variant {
            0 => {
                bad.target = BlockRef {
                    block_id: h2.block_id,
                    revision_id: h2.revision_id,
                }
            }
            1 => bad.evidence = vec![r.to.clone()],
            2 => bad.evidence.clear(),
            3 => bad.state = EpistemicState::RefutedWithinScope,
            4 => bad.relations[0].review = None,
            5 => bad.conditions = " \t".into(),
            _ => bad.explanation.clear(),
        }
        assert!(
            matches!(
                reviews.append(actor, bad).await,
                Err(ContentError::Invalid(_))
            ),
            "variant {variant}"
        );
    }
    let mut reverse = save(space, r.to.clone(), r.from.clone());
    reverse.relation_type = RelationType::Opposes;
    let reversed = relations.save(actor, reverse).await.unwrap();
    let rr = relations.review(actor, review(&reversed)).await.unwrap();
    let mut bad = judgment(&reversed, &rr);
    bad.target = r.to.clone();
    bad.evidence = vec![r.from.clone()];
    bad.state = EpistemicState::RefutedWithinScope;
    assert!(matches!(
        reviews.append(actor, bad).await,
        Err(ContentError::Invalid(_))
    ));
    let mut opposing = save(space, r.from.clone(), r.to.clone());
    opposing.relation_type = RelationType::Opposes;
    let opposite = relations.save(actor, opposing).await.unwrap();
    let opposite_review = relations.review(actor, review(&opposite)).await.unwrap();
    let mut valid = judgment(&opposite, &opposite_review);
    valid.state = EpistemicState::RefutedWithinScope;
    reviews.append(actor, valid).await.unwrap();
    for state in [
        RelationReviewState::Unreviewed,
        RelationReviewState::NeedsRecheck,
        RelationReviewState::Withdrawn,
    ] {
        let mut revision = edit(&r);
        revision.to = BlockRef {
            block_id: h2.block_id,
            revision_id: h2.revision_id,
        };
        // Advance from the actual current relation head in each loop.
        revision.expected_revision = Some(
            sqlx::query_scalar("SELECT head_revision_id FROM relation WHERE id=$1")
                .bind(r.reference.relation_id)
                .fetch_one(&rig.admin_pool)
                .await
                .unwrap(),
        );
        let fresh = relations.save(actor, revision).await.unwrap();
        let good = relations.review(actor, review(&fresh)).await.unwrap();
        let mut change = review(&fresh);
        change.expected_previous = Some(good.reference.review_id);
        change.state = state;
        let bad_review = relations.review(actor, change).await.unwrap();
        for selected in [&good, &bad_review] {
            assert!(matches!(
                reviews.append(actor, judgment(&fresh, selected)).await,
                Err(ContentError::Invalid(_))
            ));
        }
    }
}

#[tokio::test]
async fn judgments_accept_readable_cross_scope_relations_and_only_conjecture_or_conclusion_targets()
{
    let (rig, relations, reviews, actor, space, r, checked) = setup().await;
    let (_, output) = rig.seed_actor_space(true).await;
    rig.grant(actor, output, true).await;
    let mut c = judgment(&r, &checked);
    c.scope = RelationScope::Space { space_id: output };
    let saved = reviews.append(actor, c).await.unwrap();
    rig.revoke(actor, space).await;
    assert_eq!(
        reviews.read(actor, saved.reference).await.unwrap(),
        ReviewProjection::Incomplete
    );
    rig.grant(actor, space, true).await;
    let content = VersionedContentStore::new(rig.runtime_pool.clone());
    for v2 in [false, true] {
        for intent in [
            Intent::Note,
            Intent::Knowledge,
            Intent::Question,
            Intent::Idea,
            Intent::Observation,
            Intent::Evidence,
            Intent::Conjecture,
            Intent::Conclusion,
        ] {
            let mut d = support::command("target").draft;
            d.intent = intent;
            let draft = if v2 {
                ContentDraft::V2(ContentV2 {
                    intent,
                    language: "en".into(),
                    title: "target".into(),
                    body: BodyV2::Text(d.payload),
                    basis_refs: vec![],
                    requires_context: vec![],
                    source_run: None,
                })
            } else {
                ContentDraft::V1(d)
            };
            let target = content
                .create(
                    actor,
                    space,
                    CreateContent {
                        request_id: Uuid::new_v4(),
                        draft,
                        reason: "fixture".into(),
                    },
                )
                .await
                .unwrap();
            let mut c = judgment(&r, &checked);
            c.target = BlockRef {
                block_id: target.block_id,
                revision_id: target.revision_id,
            };
            c.state = EpistemicState::Untested;
            c.relations.clear();
            c.evidence.clear();
            let result = reviews.append(actor, c).await;
            if matches!(intent, Intent::Conjecture | Intent::Conclusion) {
                assert_eq!(result.unwrap().state, EpistemicState::Untested);
            } else {
                assert!(matches!(result, Err(ContentError::Invalid(_))));
            }
        }
    }
    let mut non_evidence = save(space, r.to.clone(), r.from.clone());
    non_evidence.relation_type = RelationType::RelatedTo;
    let related = relations.save(actor, non_evidence).await.unwrap();
    let mut c = judgment(&r, &checked);
    c.state = EpistemicState::Testing;
    c.relations = vec![RelationSelection {
        relation: related.reference,
        review: None,
    }];
    c.evidence.clear();
    assert_eq!(
        reviews.append(actor, c).await.unwrap().state,
        EpistemicState::Testing
    );
}

#[tokio::test]
async fn current_permissions_hide_fixed_evidence_and_conclusions_but_not_optional_predecessors() {
    let (rig, relations, actor, space, _, h) = fixture().await;
    let (_, secret) = rig.seed_actor_space(true).await;
    rig.grant(actor, secret, true).await;
    let e = rig
        .store
        .create(actor, secret, support::command("E"))
        .await
        .unwrap();
    let r = relations
        .save(actor, save(space, exact(&e), exact(&h)))
        .await
        .unwrap();
    let checked = relations.review(actor, review(&r)).await.unwrap();
    let reviews = ReviewStore::new(rig.runtime_pool.clone());
    let c = judgment(&r, &checked);
    let first = reviews.append(actor, c.clone()).await.unwrap();
    let mut next = c.clone();
    next.request_id = Uuid::new_v4();
    next.expected_previous = Some(first.reference.clone());
    next.state = EpistemicState::Testing;
    next.relations.clear();
    next.evidence.clear();
    let second = reviews.append(actor, next.clone()).await.unwrap();
    let content = VersionedContentStore::new(rig.runtime_pool.clone());
    let mut draft = support::references::v2(exact(&h), false);
    if let ContentDraft::V2(ref mut d) = draft {
        d.basis_refs.extend([
            ExactRef::EpistemicReview(first.reference.clone()),
            ExactRef::Block(exact(&e)),
        ]);
    }
    let conclusion = content
        .create(
            actor,
            space,
            CreateContent {
                request_id: Uuid::new_v4(),
                draft,
                reason: "conclusion".into(),
            },
        )
        .await
        .unwrap();
    rig.revoke(actor, secret).await;
    assert_eq!(
        reviews.read(actor, first.reference.clone()).await.unwrap(),
        ReviewProjection::Incomplete
    );
    assert!(matches!(
        reviews.append(actor, c.clone()).await,
        Err(ContentError::NotFound)
    ));
    assert!(
        content
            .read(
                actor,
                BlockRef {
                    block_id: conclusion.block_id,
                    revision_id: conclusion.revision_id
                }
            )
            .await
            .unwrap()
            .is_none()
    );
    let mut redacted = second.clone();
    redacted.previous = None;
    assert_eq!(
        reviews.read(actor, second.reference.clone()).await.unwrap(),
        ReviewProjection::Available(redacted.clone())
    );
    assert_eq!(reviews.append(actor, next).await.unwrap(), redacted);
    rig.grant(actor, secret, true).await;
    assert_eq!(
        reviews.read(actor, first.reference.clone()).await.unwrap(),
        ReviewProjection::Available(first)
    );
    rig.revoke(actor, space).await;
    assert_eq!(
        reviews.read(actor, second.reference).await.unwrap(),
        ReviewProjection::Unavailable
    );
}

#[tokio::test]
async fn source_groups_preserve_visible_order_exact_runs_and_unassigned_singletons() {
    let (rig, _, actor, space, x, _) = fixture().await;
    let (_, secret) = rig.seed_actor_space(true).await;
    rig.grant(actor, secret, true).await;
    let run = rig
        .store
        .create(actor, space, support::command("run"))
        .await
        .unwrap();
    let private_run = rig
        .store
        .create(actor, secret, support::command("private run"))
        .await
        .unwrap();
    let content = VersionedContentStore::new(rig.runtime_pool.clone());
    let reviews = ReviewStore::new(rig.runtime_pool.clone());
    let mut evidence = vec![];
    for source in [exact(&run), exact(&run), exact(&private_run)] {
        let mut draft = support::references::v2(exact(&x), false);
        if let ContentDraft::V2(ref mut d) = draft {
            d.basis_refs.clear();
            d.source_run = Some(source);
        }
        let e = content
            .create(
                actor,
                space,
                CreateContent {
                    request_id: Uuid::new_v4(),
                    draft,
                    reason: "observation".into(),
                },
            )
            .await
            .unwrap();
        evidence.push(BlockRef {
            block_id: e.block_id,
            revision_id: e.revision_id,
        });
    }
    let y = rig
        .store
        .create(actor, space, support::command("Y"))
        .await
        .unwrap();
    let input = vec![
        evidence[1].clone(),
        exact(&x),
        evidence[0].clone(),
        exact(&y),
        evidence[1].clone(),
        evidence[2].clone(),
    ];
    rig.revoke(actor, secret).await;
    let groups = reviews.group_sources(actor, input).await.unwrap();
    assert_eq!(
        groups,
        vec![
            SourceGroup {
                source_run: Some(exact(&run)),
                evidence: vec![evidence[1].clone(), evidence[0].clone()]
            },
            SourceGroup {
                source_run: None,
                evidence: vec![exact(&x)]
            },
            SourceGroup {
                source_run: None,
                evidence: vec![exact(&y)]
            }
        ]
    );
    let json = serde_json::to_value(groups).unwrap();
    assert!(!json.to_string().contains("confidence"));
    assert!(!json.to_string().contains("independent"));
    assert!(matches!(
        reviews.group_sources(actor, vec![exact(&x); 257]).await,
        Err(ContentError::Invalid(_))
    ));
}

async fn connection() -> (PgPool, i32) {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let pid = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&pool)
        .await
        .unwrap();
    (pool, pid)
}

#[tokio::test]
async fn personal_judgments_require_owner_and_late_failure_rolls_back_every_write() {
    let (rig, relations, reviews, actor, space, r, checked) = setup().await;
    let root = support::references::seed_composition(&rig, actor, space, &r.from).await;
    let view = support::references::seed_reading(&rig, actor, space, &root).await;
    let overlay = sqlx::query_scalar("SELECT overlay_id FROM reading_view WHERE id=$1")
        .bind(view.view_id)
        .fetch_one(&rig.admin_pool)
        .await
        .unwrap();
    let mut rc = save(space, r.from.clone(), r.to.clone());
    rc.scope = RelationScope::PersonalOverlay {
        space_id: space,
        overlay_id: overlay,
    };
    let private = relations.save(actor, rc).await.unwrap();
    let pc = relations.review(actor, review(&private)).await.unwrap();
    let command = judgment(&private, &pc);
    let saved = reviews.append(actor, command.clone()).await.unwrap();
    let (other, _) = rig.seed_actor_space(true).await;
    rig.grant(other, space, true).await;
    assert_eq!(
        reviews.read(other, saved.reference).await.unwrap(),
        ReviewProjection::Unavailable
    );
    assert!(matches!(
        reviews.append(other, command).await,
        Err(ContentError::NotFound)
    ));
    let c = judgment(&r, &checked);
    let function = format!("task5_fail_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE FUNCTION {function}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.actor_id='{}'::uuid THEN RAISE EXCEPTION 'injected failure'; END IF; RETURN NEW; END $$", actor.actor_id)).execute(&rig.admin_pool).await.unwrap();
    sqlx::query(&format!("CREATE TRIGGER {function} BEFORE INSERT ON epistemic_review_receipt FOR EACH ROW EXECUTE FUNCTION {function}()" )).execute(&rig.admin_pool).await.unwrap();
    assert!(matches!(
        reviews.append(actor, c.clone()).await,
        Err(ContentError::Storage)
    ));
    let totals: (i64,i64,i64,i64,i64) = sqlx::query_as("SELECT (SELECT count(*) FROM epistemic_stream WHERE space_id=$1),(SELECT count(*) FROM epistemic_review WHERE space_id=$1),(SELECT count(*) FROM reference_object WHERE space_id=$1 AND kind='epistemic_review'),(SELECT count(*) FROM request_key WHERE actor_id=$2 AND request_id=$3),(SELECT count(*) FROM reference_dependency d JOIN reference_object o ON (o.kind,o.object_id,o.revision_id)=(d.source_kind,d.source_object_id,d.source_revision_id) WHERE o.space_id=$1 AND o.kind='epistemic_review')").bind(space).bind(actor.actor_id).bind(c.request_id).fetch_one(&rig.admin_pool).await.unwrap();
    assert_eq!(totals, (1, 1, 1, 0, 4));
    sqlx::query(&format!(
        "DROP TRIGGER {function} ON epistemic_review_receipt"
    ))
    .execute(&rig.admin_pool)
    .await
    .unwrap();
    sqlx::query(&format!("DROP FUNCTION {function}()"))
        .execute(&rig.admin_pool)
        .await
        .unwrap();
    reviews.append(actor, c).await.unwrap();
}

#[tokio::test]
async fn grouping_and_judgments_share_the_union_payload_budget() {
    let (rig, _, reviews, actor, space, r, checked) = setup().await;
    let mut roots = vec![];
    for _ in 0..2 {
        let mut tx = rig.runtime_pool.begin().await.unwrap();
        let mut basis = vec![];
        for _ in 0..24 {
            basis.push(ExactRef::Block(
                support::references::seed_content(
                    &mut tx,
                    actor,
                    space,
                    ContentDraft::V1(support::command(&"x".repeat(180000)).draft),
                    None,
                )
                .await,
            ));
        }
        let hub = support::references::seed_content(
            &mut tx,
            actor,
            space,
            ContentDraft::V2(ContentV2 {
                intent: Intent::Note,
                language: "en".into(),
                title: "hub".into(),
                body: BodyV2::Text(support::command("hub").draft.payload),
                basis_refs: basis,
                requires_context: vec![],
                source_run: None,
            }),
            None,
        )
        .await;
        tx.commit().await.unwrap();
        roots.push(hub);
    }
    for root in &roots {
        assert_eq!(
            reviews
                .group_sources(actor, vec![root.clone()])
                .await
                .unwrap()
                .len(),
            1
        );
    }
    assert!(
        matches!(reviews.group_sources(actor, roots.clone()).await, Err(ContentError::Invalid(s)) if s == "reference_budget_exceeded")
    );
    let mut c = judgment(&r, &checked);
    c.evidence.extend(roots);
    assert!(
        matches!(reviews.append(actor, c).await, Err(ContentError::Invalid(s)) if s == "reference_budget_exceeded")
    );
}
async fn blocked(admin: &PgPool, waiter: i32, blocker: i32) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if sqlx::query_scalar::<_, bool>("SELECT $1=ANY(pg_blocking_pids($2))")
                .bind(blocker)
                .bind(waiter)
                .fetch_one(admin)
                .await
                .unwrap()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("expected database lock wait");
}

#[tokio::test]
async fn withdrawal_and_new_judgment_serialize_in_both_commit_orders() {
    for withdrawal_first in [true, false] {
        let (rig, _, reviews, actor, _, r, checked) = setup().await;
        let c = judgment(&r, &checked);
        let (a, apid) = connection().await;
        let (b, bpid) = connection().await;
        let mut lock = rig.admin_pool.begin().await.unwrap();
        let lock_pid = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *lock)
            .await
            .unwrap();
        sqlx::query("SELECT id FROM relation WHERE id=$1 FOR UPDATE")
            .bind(r.reference.relation_id)
            .execute(&mut *lock)
            .await
            .unwrap();
        let mut withdraw = review(&r);
        withdraw.expected_previous = Some(checked.reference.review_id);
        withdraw.state = RelationReviewState::Withdrawn;
        let gate = std::sync::Arc::new(Barrier::new(2));
        let g = gate.clone();
        let first_command = c.clone();
        let second_command = c.clone();
        let first_withdraw = withdraw.clone();
        let first_pool = a.clone();
        let first = tokio::spawn(async move {
            g.wait().await;
            if withdrawal_first {
                RelationStore::new(first_pool)
                    .review(actor, first_withdraw)
                    .await
                    .map(|_| None)
            } else {
                ReviewStore::new(first_pool)
                    .append(actor, first_command)
                    .await
                    .map(Some)
            }
        });
        gate.wait().await;
        blocked(&rig.admin_pool, apid, lock_pid).await;
        let second_pool = b.clone();
        let second = tokio::spawn(async move {
            if withdrawal_first {
                ReviewStore::new(second_pool)
                    .append(actor, second_command)
                    .await
                    .map(Some)
            } else {
                RelationStore::new(second_pool)
                    .review(actor, withdraw)
                    .await
                    .map(|_| None)
            }
        });
        // The second lock queues behind the first writer; PostgreSQL reports the soft blocker.
        blocked(&rig.admin_pool, bpid, apid).await;
        lock.commit().await.unwrap();
        let first = first.await.unwrap().unwrap();
        let second = second.await.unwrap();
        if withdrawal_first {
            assert!(matches!(second, Err(ContentError::Invalid(_))));
        } else {
            second.unwrap();
            let fixed = first.unwrap();
            assert_eq!(
                reviews.read(actor, fixed.reference.clone()).await.unwrap(),
                ReviewProjection::Available(fixed.clone())
            );
            assert_eq!(reviews.append(actor, c).await.unwrap(), fixed);
        }
        a.close().await;
        b.close().await;
    }
}

#[tokio::test]
async fn first_and_update_cas_races_and_identical_requests_commit_once() {
    let (rig, _, reviews, actor, space, r, checked) = setup().await;
    let mut c = judgment(&r, &checked);
    // Empty selections exercise stream-key serialization without relation locks.
    c.state = EpistemicState::Testing;
    c.relations.clear();
    c.evidence.clear();
    let (pa, _) = connection().await;
    let (pb, _) = connection().await;
    let a = ReviewStore::new(pa.clone());
    let b = ReviewStore::new(pb.clone());
    let mut previous = None;
    for _ in 0..2 {
        let mut left = c.clone();
        left.request_id = Uuid::new_v4();
        left.expected_previous = previous;
        let mut right = left.clone();
        right.request_id = Uuid::new_v4();
        let gate = Barrier::new(2);
        let (x, y) = tokio::join!(
            async {
                gate.wait().await;
                a.append(actor, left).await
            },
            async {
                gate.wait().await;
                b.append(actor, right).await
            }
        );
        let outcomes = [x, y];
        assert_eq!(outcomes.iter().filter(|r| r.is_ok()).count(), 1);
        let winner = outcomes.iter().find_map(|r| r.as_ref().ok()).unwrap();
        assert!(outcomes.iter().any(|r| matches!(r, Err(ContentError::Conflict { current_revision_id }) if *current_revision_id == winner.reference.review_id)));
        previous = Some(winner.reference.clone());
    }
    c.expected_previous = previous;
    c.request_id = Uuid::new_v4();
    let gate = Barrier::new(2);
    let (x, y) = tokio::join!(
        async {
            gate.wait().await;
            a.append(actor, c.clone()).await
        },
        async {
            gate.wait().await;
            b.append(actor, c.clone()).await
        }
    );
    assert_eq!(x.unwrap(), y.unwrap());
    let count: (i64,i64) = sqlx::query_as("SELECT (SELECT count(*) FROM epistemic_stream WHERE space_id=$1),(SELECT count(*) FROM epistemic_review WHERE space_id=$1)").bind(space).fetch_one(&rig.admin_pool).await.unwrap();
    assert_eq!(count, (1, 3));
    let mut bad = c.clone();
    bad.request_id = Uuid::new_v4();
    bad.target = r.from.clone();
    assert!(matches!(
        reviews.append(actor, bad).await,
        Err(ContentError::Invalid(_))
    ));
    pa.close().await;
    pb.close().await;
}

#[tokio::test]
async fn stable_cross_space_cas_conflicts_authorize_head_without_requiring_audit_parent() {
    let (rig, _, reviews, actor, _, r, checked) = setup().await;
    let (owner, evidence_space) = rig.seed_actor_space(true).await;
    let evidence = rig
        .store
        .create(
            owner,
            evidence_space,
            support::command("cross-space evidence"),
        )
        .await
        .unwrap();
    rig.grant(actor, evidence_space, false).await;
    let mut original = judgment(&r, &checked);
    original.state = EpistemicState::Testing;
    original.relations.clear();
    original.evidence = vec![exact(&evidence)];
    let first = reviews.append(actor, original.clone()).await.unwrap();
    let mut stale = original.clone();
    stale.request_id = Uuid::new_v4();
    stale.evidence.clear();
    // The existing head predates both attempts. An unchanged retry must not
    // permanently report that authorization changed during discovery.
    for _ in 0..2 {
        let result = reviews.append(actor, stale.clone()).await;
        assert!(
            matches!(result, Err(ContentError::Conflict { current_revision_id }) if current_revision_id == first.reference.review_id),
            "expected readable head conflict, got {result:?}"
        );
    }
    rig.revoke(actor, evidence_space).await;
    assert!(matches!(
        reviews.append(actor, stale.clone()).await,
        Err(ContentError::NotFound)
    ));
    assert!(matches!(
        reviews.append(actor, original).await,
        Err(ContentError::NotFound)
    ));
    // A correct CAS may replace a hidden audit predecessor with a judgment
    // whose necessary evidence is fully readable.
    stale.expected_previous = Some(first.reference.clone());
    let second = reviews.append(actor, stale.clone()).await.unwrap();
    assert_eq!(second.previous, None);
    assert_eq!(second.reference.stream_id, first.reference.stream_id);
    assert_eq!(reviews.append(actor, stale).await.unwrap(), second);
    rig.grant(actor, evidence_space, false).await;
    let mut restored = second.clone();
    restored.previous = Some(first.reference);
    assert_eq!(
        reviews.read(actor, second.reference).await.unwrap(),
        ReviewProjection::Available(restored)
    );
}

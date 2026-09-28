mod support;

use learning_core::*;
use learning_db::{QueryStore, VersionedContentStore};
use support::{TestRig, assembly as a, reading as r, references as refs};
use uuid::Uuid;

fn note(basis: Vec<ExactRef>) -> ContentDraft {
    let draft = ContentDraft::V2(ContentV2 {
        intent: Intent::Note,
        language: "en".into(),
        title: "bounded shared closure".into(),
        body: BodyV2::Text(support::command("shared closure fixture").draft.payload),
        basis_refs: basis,
        requires_context: vec![],
        source_run: None,
    });
    draft.validate().unwrap();
    draft
}

/// A query must not turn many individually valid B3 authorization checks into
/// an operation-wide failure. Only H is displayed; S_i are Context members
/// reached from H and all have the same K closure of 250 simple leaves.
#[tokio::test]
async fn direct_context_candidates_do_not_accumulate_b3_session_work() {
    let rig = TestRig::from_env().await;
    let (actor, space) = rig.seed_actor_space(true).await;
    let mut tx = rig.runtime_pool.begin().await.unwrap();
    let target = refs::seed_content(&mut tx, actor, space, note(vec![]), None).await;
    let mut leaves = Vec::with_capacity(250);
    for _ in 0..250 {
        leaves.push(refs::seed_content(&mut tx, actor, space, note(vec![]), None).await);
    }
    let shared = refs::seed_content(
        &mut tx,
        actor,
        space,
        note(leaves.into_iter().map(ExactRef::Block).collect()),
        None,
    )
    .await;
    let mut sources = Vec::with_capacity(175);
    for _ in 0..175 {
        sources.push(
            refs::seed_content(
                &mut tx,
                actor,
                space,
                note(vec![
                    ExactRef::Block(target.clone()),
                    ExactRef::Block(shared.clone()),
                ]),
                None,
            )
            .await,
        );
    }
    tx.commit().await.unwrap();

    let content = VersionedContentStore::new(rig.runtime_pool.clone());
    let mut aggregates = Vec::with_capacity(3);
    for chunk in sources.chunks(60) {
        let saved = content
            .create(
                actor,
                space,
                CreateContent {
                    request_id: Uuid::new_v4(),
                    draft: note(chunk.iter().cloned().map(ExactRef::Block).collect()),
                    reason: "group valid source roots".into(),
                },
            )
            .await
            .unwrap();
        aggregates.push(BlockRef {
            block_id: saved.block_id,
            revision_id: saved.revision_id,
        });
    }
    let saved = content
        .create(
            actor,
            space,
            CreateContent {
                request_id: Uuid::new_v4(),
                draft: note(aggregates.into_iter().map(ExactRef::Block).collect()),
                reason: "display grouped valid roots".into(),
            },
        )
        .await
        .unwrap();
    let displayed = BlockRef {
        block_id: saved.block_id,
        revision_id: saved.revision_id,
    };

    let composition = rig
        .compositions()
        .save(
            actor,
            space,
            a::doc(vec![NodeTarget::Block(displayed.clone())]),
        )
        .await
        .unwrap();
    let reading = r::store(&rig)
        .create(actor, space, r::create(composition.reference))
        .await
        .unwrap();
    // These B3 checks distinguish a valid, readable fixed scope from a
    // broken fixture that merely happens to make the new query fail.
    assert!(content.read(actor, displayed).await.unwrap().is_some());
    let fixed = r::store(&rig)
        .read_versioned(actor, reading.view.clone(), ReadingMode::Fused)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fixed.items.len(), 1);

    let query = ImpactQuery {
        start: ImpactStart::Block(target.clone()),
        scope: ImpactScope::Reading {
            view: reading.view,
            mode: ReadingMode::Fused,
        },
        families: vec![ImpactFamily::Necessary],
        max_depth: 1,
        limit: 200,
        work_limit: 4096,
        after: None,
    };
    // The traversal path already gives each candidate a fresh full B3
    // authorization in this same fixed scope. It is a real-query control,
    // independent of the direct API under test.
    let control = QueryStore::new(rig.runtime_pool.clone())
        .traverse(actor, query.clone())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(control.status(), PageStatus::Complete));
    assert_eq!(control.consumers().len(), 175);
    let result = QueryStore::new(rig.runtime_pool.clone())
        .direct(actor, query)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.start_membership(), ImpactMembership::Context);
    assert!(matches!(result.status(), PageStatus::Complete));
    assert_eq!(result.consumers().len(), 175);
    assert_eq!(control, result);
    for source in &sources {
        let group = result
            .consumers()
            .iter()
            .find(|group| group.consumer == ImpactNode::Block(source.clone()))
            .unwrap();
        assert!(group.locations.is_empty());
        assert!(
            matches!(&group.explanations[..], [ImpactExplanation { steps }]
            if matches!(&steps[..], [step]
                if step.from == ImpactNode::Block(target.clone())
                    && step.to == ImpactNode::Block(source.clone())
                    && step.family == ImpactFamily::Necessary
                    && step.dependency_role == Some(DependencyRole::Basis)
                    && step.dependency_position == Some(0)
                    && step.reason == ImpactReason::RequiresExactRevision))
        );
    }
}

use learning_core::*;
use uuid::Uuid;

fn command() -> AppendEpistemicReview {
    AppendEpistemicReview {
        request_id: Uuid::new_v4(),
        scope: RelationScope::Space {
            space_id: Uuid::new_v4(),
        },
        target: BlockRef {
            block_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
        },
        expected_previous: None,
        state: EpistemicState::Testing,
        relations: vec![],
        evidence: vec![],
        conditions: " exact bytes \n".into(),
        explanation: "human judgment".into(),
    }
}

#[test]
fn epistemic_commands_are_strict_bounded_and_keep_manual_states() {
    let c = command();
    assert!(c.validate().is_ok());
    let mut json = serde_json::to_value(&c).unwrap();
    json["reviewer_id"] = serde_json::json!(Uuid::new_v4());
    assert!(serde_json::from_value::<AppendEpistemicReview>(json).is_err());
    for state in [
        EpistemicState::Untested,
        EpistemicState::Testing,
        EpistemicState::Inconclusive,
        EpistemicState::Superseded,
    ] {
        let mut c = c.clone();
        c.state = state;
        c.conditions.clear();
        c.explanation.clear();
        assert!(c.validate().is_ok());
    }
    for state in [
        EpistemicState::SupportedWithinScope,
        EpistemicState::RefutedWithinScope,
    ] {
        let mut c = c.clone();
        c.state = state;
        c.conditions = " \t".into();
        assert!(c.validate().is_err());
        c.conditions = "bounded".into();
        c.explanation = "\n".into();
        assert!(c.validate().is_err());
    }
    for text in ["x".repeat(10001), "\0".into(), "界".repeat(3334)] {
        let mut c = c.clone();
        c.conditions = text.clone();
        assert!(c.validate().is_err());
        c.conditions.clear();
        c.explanation = text;
        assert!(c.validate().is_err());
    }
    let mut c = c;
    c.evidence = vec![c.target.clone(), c.target.clone()];
    assert!(c.validate().is_err());
    c.evidence = (0..257)
        .map(|_| BlockRef {
            block_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
        })
        .collect();
    assert!(c.validate().is_err());
    c.evidence.clear();
    let r = RelationRef {
        relation_id: Uuid::new_v4(),
        revision_id: Uuid::new_v4(),
    };
    c.relations = vec![
        RelationSelection {
            relation: r.clone(),
            review: None
        };
        2
    ];
    assert!(c.validate().is_err());
    c.relations.truncate(1);
    c.relations[0].review = Some(RelationReviewRef {
        relation: RelationRef {
            relation_id: r.relation_id,
            revision_id: Uuid::new_v4(),
        },
        review_id: Uuid::new_v4(),
    });
    assert!(c.validate().is_err());
}

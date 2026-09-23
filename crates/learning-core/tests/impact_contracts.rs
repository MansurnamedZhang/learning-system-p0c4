use learning_core::*;
use serde_json::{Value, json};
use uuid::Uuid;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn block(revision: u128) -> ExactRef {
    ExactRef::Block(BlockRef {
        block_id: id(1),
        revision_id: id(revision),
    })
}
fn query() -> Value {
    json!({"start":{"type":"block","block_id":id(1),"revision_id":id(2)},
        "scope":{"type":"reading","view":{"view_id":id(3),"revision_id":id(4)},"mode":"fused"}})
}

#[test]
fn query_defaults_and_strict_input() {
    let q: ImpactQuery = serde_json::from_value(query()).unwrap();
    assert_eq!(q.start, block(2));
    assert_eq!(
        q.families,
        vec![ImpactFamily::Structural, ImpactFamily::Necessary]
    );
    assert_eq!((q.max_depth, q.limit, q.work_limit), (3, 50, 4096));
    for (field, value) in [
        ("max_depth", json!(0)),
        ("max_depth", json!(9)),
        ("limit", json!(0)),
        ("limit", json!(201)),
        ("work_limit", json!(0)),
        ("work_limit", json!(131073)),
        ("families", json!([])),
        ("families", Value::Null),
        ("families", json!(["structural", "structural"])),
    ] {
        let mut v = query();
        v[field] = value;
        assert!(serde_json::from_value::<ImpactQuery>(v).is_err(), "{field}");
    }
    for (field, value) in [("actor_id", json!(id(8))), ("candidate_count", json!(1))] {
        let mut v = query();
        v[field] = value;
        assert!(serde_json::from_value::<ImpactQuery>(v).is_err());
    }
    for path in ["start", "scope", "view"] {
        let mut v = query();
        if path == "view" {
            v["scope"]["view"]["extra"] = json!(true);
        } else {
            v[path]["extra"] = json!(true);
        }
        assert!(serde_json::from_value::<ImpactQuery>(v).is_err(), "{path}");
    }
    let mut v = query();
    v["scope"] = json!({"type":"release","release_id":id(5)});
    assert!(serde_json::from_value::<ImpactQuery>(v).is_ok());
    let mut v = query();
    v["scope"]["mode"] = json!("current");
    assert!(serde_json::from_value::<ImpactQuery>(v).is_err());
}

fn group(revision: u128, paths: usize) -> ImpactConsumerGroup {
    let node = ImpactNode::Block(BlockRef {
        block_id: id(1),
        revision_id: id(revision),
    });
    ImpactConsumerGroup {
        consumer: node.clone(),
        locations: (0..paths)
            .map(|i| ImpactLocation::Occurrence {
                path: vec![id(100 + i as u128)],
            })
            .collect(),
        explanations: (0..paths)
            .map(|i| ImpactExplanation {
                steps: vec![ImpactStep {
                    from: ImpactNode::Block(BlockRef {
                        block_id: id(1),
                        revision_id: id(2),
                    }),
                    to: node.clone(),
                    family: ImpactFamily::Structural,
                    direction: None,
                    relation_type: None,
                    provenance: ImpactProvenance::Stored,
                    location: Some(ImpactLocation::Occurrence {
                        path: vec![id(100 + i as u128)],
                    }),
                    reason: ImpactReason::ReferencesOldRevision,
                }],
            })
            .collect(),
    }
}

#[test]
fn pages_keep_all_locations_and_paths_for_each_exact_consumer() {
    let groups = vec![group(9, 2), group(8, 1)];
    let mut budget = VisibleWorkBudget::new(4096);
    let first = paginate_visible_groups(groups.clone(), None, 1, &mut budget).unwrap();
    assert_eq!(first.consumers.len(), 1);
    assert_eq!(
        first.consumers[0].locations.len(),
        first.consumers[0].explanations.len()
    );
    assert_eq!(first.consumers[0].locations.len(), 1);
    let PageStatus::Truncated { after } = first.status else {
        panic!("expected next page")
    };
    let mut next_budget = VisibleWorkBudget::new(4096);
    let second = paginate_visible_groups(groups, Some(&after), 1, &mut next_budget).unwrap();
    assert_eq!(second.consumers.len(), 1);
    assert_eq!(second.consumers[0].locations.len(), 2);
    assert_eq!(second.status, PageStatus::Complete);
    assert_ne!(second.consumers[0].consumer, first.consumers[0].consumer);
}

#[test]
fn visible_budget_is_operation_wide_and_never_counts_hidden_candidates() {
    let mut budget = VisibleWorkBudget::new(2);
    budget.charge_step().unwrap();
    budget.charge_step().unwrap();
    assert_eq!(budget.charge_step(), Err(BudgetExceeded));
    let mut budget = VisibleWorkBudget::new(4096);
    for _ in 0..1000 { /* hidden candidate screening does not call the visible budget */ }
    budget.charge_edge().unwrap();
    let node = ImpactNode::Block(BlockRef {
        block_id: id(1),
        revision_id: id(2),
    });
    budget.charge_node(node.clone()).unwrap();
    budget.charge_node(node).unwrap();
    assert_eq!((budget.edges(), budget.nodes()), (1, 1));
}

#[test]
fn oversized_group_fails_without_partial_page() {
    let mut budget = VisibleWorkBudget::new(4096);
    let mut huge = group(8, 4097);
    huge.locations.clear();
    assert_eq!(
        paginate_visible_groups(vec![huge], None, 1, &mut budget),
        Err(BudgetExceeded)
    );
}

#[test]
fn cursor_is_visible_structured_and_nested_fields_are_strict() {
    let mut budget = VisibleWorkBudget::new(4096);
    let page =
        paginate_visible_groups(vec![group(8, 1), group(9, 1)], None, 1, &mut budget).unwrap();
    let PageStatus::Truncated { after } = page.status else {
        panic!("missing cursor")
    };
    let mut v = query();
    v["after"] = serde_json::to_value(&after).unwrap();
    assert!(serde_json::from_value::<ImpactQuery>(v.clone()).is_ok());
    v["after"]["internal_offset"] = json!(1);
    assert!(serde_json::from_value::<ImpactQuery>(v).is_err());
    let mut v = query();
    v["after"] = serde_json::to_value(&after).unwrap();
    v["after"]["consumer"]["candidate_count"] = json!(3);
    assert!(serde_json::from_value::<ImpactQuery>(v).is_err());
    let mut v = query();
    v["after"] = serde_json::to_value(&after).unwrap();
    v["after"]["hops"] = json!(9);
    assert!(serde_json::from_value::<ImpactQuery>(v).is_err());
}

#[test]
fn exact_revisions_and_duplicate_consumer_fragments_merge_without_losing_paths() {
    let mut budget = VisibleWorkBudget::new(4096);
    let page = paginate_visible_groups(
        vec![group(8, 1), group(9, 1), group(8, 2)],
        None,
        3,
        &mut budget,
    )
    .unwrap();
    assert_eq!(page.status, PageStatus::Complete);
    assert_eq!(page.consumers.len(), 2);
    assert_eq!(
        page.consumers[0].consumer,
        ImpactNode::Block(BlockRef {
            block_id: id(1),
            revision_id: id(8)
        })
    );
    assert_eq!(page.consumers[0].locations.len(), 2);
    assert_eq!(page.consumers[0].explanations.len(), 2);
    assert_eq!(
        page.consumers[1].consumer,
        ImpactNode::Block(BlockRef {
            block_id: id(1),
            revision_id: id(9)
        })
    );
}

#[test]
fn visible_work_and_node_caps_are_inclusive() {
    let mut budget = VisibleWorkBudget::new(1);
    budget.charge_edge().unwrap();
    assert_eq!(
        budget.charge_node(ImpactNode::Release { release_id: id(4) }),
        Err(BudgetExceeded)
    );
    assert_eq!((budget.edges(), budget.nodes(), budget.work()), (1, 0, 1));
    let mut budget = VisibleWorkBudget::new(MAX_VISIBLE_WORK);
    for n in 0..MAX_VISIBLE_NODES {
        budget
            .charge_node(ImpactNode::Release {
                release_id: id(n as u128 + 1),
            })
            .unwrap();
    }
    assert_eq!(budget.nodes(), MAX_VISIBLE_NODES);
    assert_eq!(
        budget.charge_node(ImpactNode::Release {
            release_id: id(9999)
        }),
        Err(BudgetExceeded)
    );
    budget
        .charge_node(ImpactNode::Release { release_id: id(1) })
        .unwrap();
    assert_eq!(budget.nodes(), MAX_VISIBLE_NODES);
}

#[test]
fn budget_status_cannot_expose_a_partial_consumer() {
    let scope = ImpactScope::Release { release_id: id(55) };
    let result = ImpactResult::from_page(
        block(2),
        ImpactMembership::Context,
        scope,
        ImpactPage {
            consumers: vec![group(8, 1)],
            status: PageStatus::BudgetExceeded,
        },
    );
    assert!(result.consumers().is_empty());
    assert_eq!(result.status(), &PageStatus::BudgetExceeded);
    let json = serde_json::to_value(&result).unwrap();
    assert_eq!(json["consumers"], json!([]));
    assert_eq!(json["status"]["type"], json!("budget_exceeded"));
    assert_eq!(json["start_membership"], json!("context"));
}

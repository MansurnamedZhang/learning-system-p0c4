use learning_core::*;
use serde_json::{Value, json};
use uuid::Uuid;

fn draft() -> Value {
    json!({"request_id":Uuid::nil(),"composition_id":null,"base_revision_id":null,
        "kind":"document","title":"Attention","nodes":[],"reason":"初版"})
}
fn node() -> Value {
    json!({"occurrence_id":null,"target":{"type":"block","block_id":Uuid::from_u128(1),"revision_id":Uuid::from_u128(2)}})
}
fn publish() -> Value {
    json!({"request_id":Uuid::nil(),"roots":[{"composition_id":Uuid::from_u128(1),"revision_id":Uuid::from_u128(2),"expected_head_revision_id":Uuid::from_u128(2),"expected_release_id":null}],"reason":"发布"})
}
#[test]
fn creation_requires_paired_identity_and_base() {
    let mut value = draft();
    value["base_revision_id"] = json!(Uuid::new_v4());
    assert!(serde_json::from_value::<SaveComposition>(value).is_err());
    let mut direct: SaveComposition = serde_json::from_value(draft()).unwrap();
    direct.composition_id = Some(Uuid::new_v4());
    assert!(direct.validate().is_err());
}
#[test]
fn nested_unknown_fields_are_rejected() {
    for level in ["outer", "node", "target"] {
        let mut value = draft();
        value["nodes"] = json!([node()]);
        match level {
            "outer" => value["author_id"] = json!(Uuid::new_v4()),
            "node" => value["nodes"][0]["position"] = json!(0),
            _ => value["nodes"][0]["latest"] = json!(true),
        }
        assert!(
            serde_json::from_value::<SaveComposition>(value).is_err(),
            "{level}"
        );
    }
    let mut value = publish();
    value["roots"][0]["latest"] = json!(true);
    assert!(serde_json::from_value::<PublishCommand>(value).is_err());
}
#[test]
fn composition_limits_apply_to_json_and_direct_values() {
    for (field, valid, invalid) in [
        ("title", "中".repeat(300), "中".repeat(301)),
        ("reason", "理".repeat(1000), "理".repeat(1001)),
    ] {
        let mut value = draft();
        value[field] = json!(valid);
        assert!(serde_json::from_value::<SaveComposition>(value.clone()).is_ok());
        value[field] = json!(invalid);
        assert!(serde_json::from_value::<SaveComposition>(value).is_err());
    }
    let mut direct: SaveComposition = serde_json::from_value(draft()).unwrap();
    direct.nodes = vec![serde_json::from_value(node()).unwrap(); 512];
    assert!(direct.validate().is_ok());
    direct.nodes.push(direct.nodes[0].clone());
    assert!(direct.validate().is_err());
    for field in ["title", "reason"] {
        let mut value = draft();
        value[field] = json!("a\0b");
        assert!(serde_json::from_value::<SaveComposition>(value).is_err());
    }
    let mut value = draft();
    value["reason"] = json!("");
    assert!(serde_json::from_value::<SaveComposition>(value).is_err());
}
#[test]
fn existing_occurrence_cannot_appear_twice_in_one_revision() {
    let mut value = draft();
    let mut n = node();
    n["occurrence_id"] = json!(Uuid::new_v4());
    value["nodes"] = json!([n, n]);
    assert!(serde_json::from_value::<SaveComposition>(value).is_err());
}
#[test]
fn release_requires_unique_bounded_nonempty_roots() {
    let mut cmd: PublishCommand = serde_json::from_value(publish()).unwrap();
    cmd.roots.clear();
    assert!(cmd.validate().is_err());
    for i in 0..16 {
        let mut r: PublishRoot = serde_json::from_value(publish()["roots"][0].clone()).unwrap();
        r.composition_id = Uuid::from_u128(i);
        cmd.roots.push(r);
    }
    assert!(cmd.validate().is_ok());
    cmd.roots.push(cmd.roots[0].clone());
    assert!(cmd.validate().is_err());
    cmd.roots.truncate(2);
    cmd.roots[1] = cmd.roots[0].clone();
    assert!(cmd.validate().is_err());
    cmd.roots.truncate(1);
    cmd.reason = "\0".into();
    assert!(cmd.validate().is_err());
}
#[test]
fn request_digest_treats_roots_as_a_set_and_nodes_as_a_sequence() {
    let mut a: PublishCommand = serde_json::from_value(publish()).unwrap();
    let mut second = a.roots[0].clone();
    second.composition_id = Uuid::from_u128(3);
    a.roots.push(second);
    let mut b = a.clone();
    b.roots.reverse();
    assert_eq!(a.digest(Uuid::nil()), b.digest(Uuid::nil()));
    b.reason.push('x');
    assert_ne!(a.digest(Uuid::nil()), b.digest(Uuid::nil()));
    let mut c: SaveComposition = serde_json::from_value(draft()).unwrap();
    let x: NodeDraft = serde_json::from_value(node()).unwrap();
    let mut y = x.clone();
    y.target = NodeTarget::Block(BlockRef {
        block_id: Uuid::from_u128(3),
        revision_id: Uuid::from_u128(4),
    });
    c.nodes = vec![x, y];
    let mut d = c.clone();
    d.nodes.reverse();
    assert_ne!(c.digest(Uuid::nil()), d.digest(Uuid::nil()));
}
#[test]
fn composition_digest_matches_independent_golden_and_tracks_order() {
    let nodes = vec![Occurrence {
        occurrence_id: Uuid::from_u128(3),
        target: NodeTarget::Block(BlockRef {
            block_id: Uuid::from_u128(1),
            revision_id: Uuid::from_u128(2),
        }),
    }];
    assert_eq!(
        composition_digest(CompositionKind::Document, "Attention", &nodes),
        "ac50ebc23e7e32af602a65c879111120acc2137c85462b6658aed3b577a77939"
    );
    assert_ne!(
        composition_digest(CompositionKind::Document, "Attention", &nodes),
        composition_digest(CompositionKind::Section, "Attention", &nodes)
    );
}

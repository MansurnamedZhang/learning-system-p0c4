use learning_core::*;
use serde_json::json;
use uuid::Uuid;

fn value() -> serde_json::Value {
    json!({
        "request_id":Uuid::new_v4(),"operation":"derive",
        "inputs":[{"block_id":Uuid::new_v4(),"revision_id":Uuid::new_v4()}],
        "outputs":[{"contract_version":1,"draft":{"kind":"text","intent":"note","language":"en","title":"new","payload":{"format":"markdown","text":"body"}}}],
        "reason":"transform"
    })
}

#[test]
fn versioned_lineage_command_is_strict_and_roundtrips() {
    let mut source = value();
    let command: LineageCommand = serde_json::from_value(source.clone()).unwrap();
    assert_eq!(serde_json::to_value(command).unwrap(), source);
    for path in ["command", "input", "output", "draft"] {
        let mut bad = source.clone();
        match path {
            "command" => bad["author_id"] = json!(Uuid::new_v4()),
            "input" => bad["inputs"][0]["space_id"] = json!(Uuid::new_v4()),
            "output" => bad["outputs"][0]["block_id"] = json!(Uuid::new_v4()),
            _ => bad["outputs"][0]["draft"]["basis_refs"] = json!([]),
        }
        assert!(
            serde_json::from_value::<LineageCommand>(bad).is_err(),
            "{path}"
        );
    }
    for reason in [" ".to_owned(), "x".repeat(1001), "null\0byte".to_owned()] {
        let mut bad = source.clone();
        bad["reason"] = json!(reason);
        assert!(serde_json::from_value::<LineageCommand>(bad).is_err());
    }
    source["outputs"][0]["contract_version"] = json!(3);
    assert!(serde_json::from_value::<LineageCommand>(source).is_err());

    let mut overflow = value();
    overflow["operation"] = json!("merge");
    overflow["inputs"] = json!(
        (0..33)
            .map(|_| json!({"block_id":Uuid::new_v4(),"revision_id":Uuid::new_v4()}))
            .collect::<Vec<_>>()
    );
    assert!(serde_json::from_value::<LineageCommand>(overflow).is_err());
    let mut overflow = value();
    overflow["operation"] = json!("split");
    overflow["outputs"] = json!(vec![overflow["outputs"][0].clone(); 33]);
    assert!(serde_json::from_value::<LineageCommand>(overflow).is_err());
}

#[test]
fn mixed_versions_require_explicit_dispatch_and_system_types_are_separate() {
    let mut source = value();
    source["operation"] = json!("split");
    source["outputs"][0]["draft"]["payload"]["format"] = json!("markdown");
    source["outputs"].as_array_mut().unwrap().push(json!({"contract_version":2,"draft":{
        "intent":"note","language":"en","title":"adapted", "body":{"kind":"text","payload":{"format":"markdown","text":"body"}},
        "basis_refs":[],"requires_context":[],"source_run":null
    }}));
    let cmd: LineageCommand = serde_json::from_value(source.clone()).unwrap();
    assert_eq!(cmd.outputs[0].contract_version(), 1);
    assert_eq!(cmd.outputs[1].contract_version(), 2);
    assert_eq!(serde_json::to_value(cmd).unwrap(), source);
    for op in ["derived_from", "split_from", "merged_from", "unknown"] {
        assert!(serde_json::from_value::<RelationType>(json!(op)).is_err());
        assert!(serde_json::from_value::<LineageOperation>(json!(op)).is_err());
    }
}

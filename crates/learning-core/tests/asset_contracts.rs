use learning_core::{
    AssetRef, AssetUseRef, BlockRef, ContentDraft, ContentV3, Dependency, DependencyRole, ExactRef,
    ResourceVersionRef,
};
use serde_json::{Value, json};
use uuid::Uuid;

const SPACE: &str = "10000000-0000-0000-0000-000000000001";
const ASSET: &str = "20000000-0000-0000-0000-000000000002";
const BLOCK: &str = "30000000-0000-0000-0000-000000000003";
const REVISION: &str = "40000000-0000-0000-0000-000000000004";

const V1_JSON: &str = r#"{"kind":"text","intent":"note","language":"zh-CN","title":"注记","payload":{"format":"markdown","text":"保留空格  与换行\n"}}"#;
const V2_JSON: &str = r#"{"intent":"note","language":"zh-CN","title":"注记","body":{"kind":"text","payload":{"format":"markdown","text":"保留空格  与换行\n"}},"basis_refs":[],"requires_context":[],"source_run":null}"#;
const FIGURE_JSON: &str = r#"{"intent":"note","language":"zh-CN","title":"图","body":{"kind":"figure","asset":{"space_id":"10000000-0000-0000-0000-000000000001","asset_id":"20000000-0000-0000-0000-000000000002"},"usage":"lecture_diagram","caption":"注意力示意图","alt":"查询、键和值之间的连线","decorative":false},"basis_refs":[],"requires_context":[],"source_run":null}"#;
const ATTACHMENT_JSON: &str = r#"{"intent":"note","language":"en","title":"Data","body":{"kind":"attachment","asset":{"space_id":"10000000-0000-0000-0000-000000000001","asset_id":"20000000-0000-0000-0000-000000000002"},"display_name":"lecture.pdf"},"basis_refs":[],"requires_context":[],"source_run":null}"#;

fn parsed(value: &str) -> Value {
    serde_json::from_str(value).unwrap()
}

#[test]
fn frozen_v1_v2_json_and_digest_stay_identical() {
    let v1 = ContentDraft::decode(1, parsed(V1_JSON)).unwrap();
    let v2 = ContentDraft::decode(2, parsed(V2_JSON)).unwrap();
    assert_eq!(serde_json::to_string(&v1).unwrap(), V1_JSON);
    assert_eq!(serde_json::to_string(&v2).unwrap(), V2_JSON);
    assert_eq!(
        v1.digest(),
        "35db28db30d4e6d1b4f81c32a365ba9d2b34757bbb6694e09ab308baab1037ec"
    );
    assert_eq!(
        v2.digest(),
        "1c89f2f7dccc9a4175d8266483d9f6430b3a25e1512d376557820aba9cb4537a"
    );
    assert_eq!(
        ContentDraft::decode(1, serde_json::to_value(v1).unwrap())
            .unwrap()
            .contract_version(),
        1
    );
    assert_eq!(
        ContentDraft::decode(2, serde_json::to_value(v2).unwrap())
            .unwrap()
            .contract_version(),
        2
    );
}

#[test]
fn figure_and_attachment_literals_round_trip_and_extract_exact_asset() {
    for literal in [FIGURE_JSON, ATTACHMENT_JSON] {
        let content: ContentV3 = serde_json::from_str(literal).unwrap();
        assert_eq!(serde_json::to_string(&content).unwrap(), literal);
        assert_eq!(serde_json::from_str::<ContentV3>(literal).unwrap(), content);
        assert_eq!(
            content.asset_ref(),
            Some(&AssetRef {
                space_id: Uuid::parse_str(SPACE).unwrap(),
                asset_id: Uuid::parse_str(ASSET).unwrap(),
            })
        );
        assert_eq!(content.dependencies(), Vec::<Dependency>::new());
    }
    let mut text = parsed(FIGURE_JSON);
    text["body"] = json!({"kind":"text","payload":{"format":"markdown","text":"plain"}});
    assert_eq!(
        serde_json::from_value::<ContentV3>(text)
            .unwrap()
            .asset_ref(),
        None
    );
}

#[test]
fn figure_requires_alt_or_explicit_decoration() {
    let mut figure = parsed(FIGURE_JSON);
    figure["body"]["alt"] = json!("  ");
    assert!(serde_json::from_value::<ContentV3>(figure.clone()).is_err());
    figure["body"]["decorative"] = json!(true);
    figure["body"]["alt"] = json!("");
    assert!(serde_json::from_value::<ContentV3>(figure.clone()).is_ok());
    figure["body"]["alt"] = json!("still described");
    assert!(serde_json::from_value::<ContentV3>(figure).is_err());
}

#[test]
fn asset_fields_reject_missing_malformed_and_unbounded_metadata() {
    let mut figure = parsed(FIGURE_JSON);
    figure["body"]["asset"]["asset_id"] = json!("not-a-uuid");
    assert!(serde_json::from_value::<ContentV3>(figure).is_err());

    let mut figure = parsed(FIGURE_JSON);
    figure["body"]["asset"]["other"] = json!(1);
    assert!(serde_json::from_value::<ContentV3>(figure).is_err());

    let mut figure = parsed(FIGURE_JSON);
    figure["body"]["usage"] = json!("");
    assert!(serde_json::from_value::<ContentV3>(figure.clone()).is_err());
    figure["body"]["usage"] = json!("x".repeat(101));
    assert!(serde_json::from_value::<ContentV3>(figure).is_err());

    let mut attachment = parsed(ATTACHMENT_JSON);
    attachment["body"]["display_name"] = json!(" ");
    assert!(serde_json::from_value::<ContentV3>(attachment.clone()).is_err());
    attachment["body"]["display_name"] = json!("x\0y");
    assert!(serde_json::from_value::<ContentV3>(attachment.clone()).is_err());
    attachment["body"]["display_name"] = json!("x".repeat(256));
    assert!(serde_json::from_value::<ContentV3>(attachment).is_err());
}

#[test]
fn v3_retains_v2_dependencies_in_order_and_separate_digest_domain() {
    let target = BlockRef {
        block_id: Uuid::parse_str(BLOCK).unwrap(),
        revision_id: Uuid::parse_str(REVISION).unwrap(),
    };
    let mut value = parsed(FIGURE_JSON);
    value["basis_refs"] = json!([{"type":"block","block_id":BLOCK,"revision_id":REVISION}]);
    value["requires_context"] = json!([{"block_id":BLOCK,"revision_id":REVISION}]);
    value["source_run"] = json!({"block_id":BLOCK,"revision_id":REVISION});
    let content: ContentV3 = serde_json::from_value(value).unwrap();
    assert_eq!(
        content.dependencies(),
        vec![
            Dependency {
                role: DependencyRole::Basis,
                target: ExactRef::Block(target.clone())
            },
            Dependency {
                role: DependencyRole::RequiresContext,
                target: ExactRef::Block(target.clone())
            },
            Dependency {
                role: DependencyRole::SourceRun,
                target: ExactRef::Block(target)
            },
        ]
    );
    assert_eq!(content.digest().len(), 64);
    assert_ne!(
        content.digest(),
        ContentDraft::decode(2, parsed(V2_JSON)).unwrap().digest()
    );
    let mut invalid = content.clone();
    invalid.basis_refs.push(invalid.basis_refs[0].clone());
    assert!(invalid.validate().is_err());
}

#[test]
fn asset_use_refs_preserve_exact_resource_version_and_block_identity() {
    let reference = ResourceVersionRef {
        space_id: Uuid::parse_str(SPACE).unwrap(),
        resource_id: Uuid::parse_str(BLOCK).unwrap(),
        version_id: Uuid::parse_str(REVISION).unwrap(),
    };
    let use_ref = AssetUseRef::Resource(reference.clone());
    let literal =
        json!({"type":"resource","space_id":SPACE,"resource_id":BLOCK,"version_id":REVISION});
    assert_eq!(serde_json::to_value(&use_ref).unwrap(), literal);
    assert_eq!(
        serde_json::from_value::<AssetUseRef>(literal).unwrap(),
        use_ref
    );
    assert_eq!(
        serde_json::from_value::<ResourceVersionRef>(serde_json::to_value(reference).unwrap())
            .unwrap()
            .space_id,
        Uuid::parse_str(SPACE).unwrap()
    );
    let block = AssetUseRef::Block(BlockRef {
        block_id: Uuid::parse_str(BLOCK).unwrap(),
        revision_id: Uuid::parse_str(REVISION).unwrap(),
    });
    assert_eq!(
        serde_json::to_value(&block).unwrap(),
        json!({"type":"block","block_id":BLOCK,"revision_id":REVISION})
    );
}

#[test]
fn v3_draft_dispatch_preserves_frozen_v1_v2_wires_and_extracts_asset_separately() {
    let literal = parsed(FIGURE_JSON);
    let draft = ContentDraft::decode(3, literal.clone()).unwrap();
    assert_eq!(draft.contract_version(), 3);
    assert_eq!(serde_json::to_value(&draft).unwrap(), literal);
    assert_eq!(
        draft.asset_ref().unwrap().asset_id,
        Uuid::parse_str(ASSET).unwrap()
    );
    assert_eq!(draft.dependencies(), Vec::<Dependency>::new());
    assert_eq!(
        draft.digest(),
        serde_json::from_value::<ContentV3>(literal)
            .unwrap()
            .digest()
    );
    assert_eq!(
        serde_json::to_string(&ContentDraft::decode(1, parsed(V1_JSON)).unwrap()).unwrap(),
        V1_JSON
    );
    assert_eq!(
        serde_json::to_string(&ContentDraft::decode(2, parsed(V2_JSON)).unwrap()).unwrap(),
        V2_JSON
    );
}

#[test]
fn v3_rejects_unknown_fields_and_bad_common_metadata() {
    let mut value = parsed(ATTACHMENT_JSON);
    value["surprise"] = json!(true);
    assert!(serde_json::from_value::<ContentV3>(value).is_err());
    let mut value = parsed(ATTACHMENT_JSON);
    value["language"] = json!("");
    assert!(serde_json::from_value::<ContentV3>(value).is_err());
    let mut value = parsed(ATTACHMENT_JSON);
    value["body"]["extra"] = json!(true);
    assert!(serde_json::from_value::<ContentV3>(value).is_err());
    let mut value = parsed(ATTACHMENT_JSON);
    value["body"]["asset"]["space_id"] = json!("bad");
    assert!(serde_json::from_value::<ContentV3>(value).is_err());
}

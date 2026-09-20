use learning_core::{BodyV2, ContentDraft, ContentV2, Intent, TextDraft, TextFormat, TextPayload};
use serde_json::json;

fn v1_value() -> serde_json::Value {
    json!({"kind":"text","intent":"note","language":"zh-CN","title":"注记",
        "payload":{"format":"markdown","text":"保留空格  与换行\n"}})
}

#[test]
fn dispatch_preserves_v1_digest_and_rejects_unknown_versions() {
    let v1: TextDraft = serde_json::from_value(v1_value()).unwrap();
    assert_eq!(
        v1.digest(),
        "35db28db30d4e6d1b4f81c32a365ba9d2b34757bbb6694e09ab308baab1037ec"
    );
    assert_eq!(ContentDraft::V1(v1.clone()).digest(), v1.digest());
    assert_eq!(
        ContentDraft::decode(1, v1_value()).unwrap(),
        ContentDraft::V1(v1)
    );
    assert!(ContentDraft::decode(99, json!({})).is_err());
}

#[test]
fn v2_digest_has_a_stable_domain_and_preserves_body_bytes() {
    let draft = ContentDraft::V2(ContentV2 {
        intent: Intent::Note,
        language: "zh-CN".into(),
        title: "注记".into(),
        body: BodyV2::Text(TextPayload {
            format: TextFormat::Markdown,
            text: "保留空格  与换行\n".into(),
        }),
        basis_refs: vec![],
        requires_context: vec![],
        source_run: None,
    });
    // Independently reviewed canonical JSON and SHA-256 fixture.
    assert_eq!(
        draft.digest(),
        "1c89f2f7dccc9a4175d8266483d9f6430b3a25e1512d376557820aba9cb4537a"
    );
}

#[test]
fn v1_and_v2_reject_unknown_or_mistyped_fields() {
    let mut old = v1_value();
    old["target"] = json!({});
    assert!(ContentDraft::decode(1, old).is_err());

    let raw = json!({
        "intent":"note", "language":"en", "title":"x",
        "body":{"kind":"text","payload":{"format":"markdown","text":"x","extra":true}},
        "basis_refs":[], "requires_context":[], "source_run":null
    });
    assert!(ContentDraft::decode(2, raw).is_err());
    let wrong = json!({
        "intent":"note", "language":"en", "title":"x",
        "body":{"kind":"reference","target":{"block_id":7,"revision_id":"00000000-0000-0000-0000-000000000001"}},
        "basis_refs":[], "requires_context":[], "source_run":null
    });
    assert!(ContentDraft::decode(2, wrong).is_err());
}

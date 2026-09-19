use learning_core::TextDraft;
use serde_json::{Value, json};

fn input() -> Value {
    json!({"kind":"text","intent":"note","language":"zh-CN","title":"注记",
        "payload":{"format":"markdown","text":"保留空格  与换行\n"}})
}

#[test]
fn client_cannot_supply_an_author_or_unimplemented_reference() {
    for (key, value) in [("author_id", json!("forged")), ("basis_refs", json!(["hidden"]))] {
        let mut raw = input();
        raw[key] = value;
        assert!(serde_json::from_value::<TextDraft>(raw).is_err());
    }
}

#[test]
fn unsupported_kind_intent_and_format_are_rejected() {
    for (key, value) in [("kind", "code"), ("intent", "proven_truth")] {
        let mut raw = input(); raw[key] = json!(value);
        let parsed = serde_json::from_value::<TextDraft>(raw);
        assert!(parsed.is_err() || parsed.unwrap().validate().is_err());
    }
    let mut raw = input(); raw["payload"]["format"] = json!("html");
    let parsed = serde_json::from_value::<TextDraft>(raw);
    assert!(parsed.is_err() || parsed.unwrap().validate().is_err());
}

#[test]
fn payload_does_not_hide_unchecked_fields() {
    let mut raw = input(); raw["payload"]["target"] = json!("hidden");
    assert!(serde_json::from_value::<TextDraft>(raw).is_err());
}

#[test]
fn title_and_text_limits_are_enforced_without_trimming() {
    let mut draft: TextDraft = serde_json::from_value(input()).unwrap();
    assert_eq!(draft.payload.text, "保留空格  与换行\n");
    draft.title = "长".repeat(301);
    assert!(draft.validate().is_err());
    draft.title = "".into(); draft.payload.text = "a".repeat(200_001);
    assert!(draft.validate().is_err());
    draft.payload.text.clear();
    assert!(draft.validate().is_ok());
}

#[test]
fn digest_ignores_json_key_order_but_tracks_content() {
    let a: TextDraft = serde_json::from_value(input()).unwrap();
    let b: TextDraft = serde_json::from_str(r#"{"title":"注记","language":"zh-CN",
      "kind":"text","payload":{"text":"保留空格  与换行\n","format":"markdown"},"intent":"note"}"#).unwrap();
    assert_eq!(a.digest(), b.digest());
    assert_eq!(a.digest().len(), 64);
    let mut changed = b; changed.payload.text.push('改');
    assert_ne!(a.digest(), changed.digest());
}

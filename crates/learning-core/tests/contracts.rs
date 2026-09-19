use learning_core::TextDraft;
use serde_json::{Value, json};

fn input() -> Value {
    json!({"kind":"text","intent":"note","language":"zh-CN","title":"注记",
        "payload":{"format":"markdown","text":"保留空格  与换行\n"}})
}

#[test]
fn client_cannot_supply_an_author_or_unimplemented_reference() {
    for (key, value) in [
        ("author_id", json!("forged")),
        ("basis_refs", json!(["hidden"])),
    ] {
        let mut raw = input();
        raw[key] = value;
        assert!(serde_json::from_value::<TextDraft>(raw).is_err());
    }
}

#[test]
fn unsupported_kind_intent_and_format_are_rejected() {
    for (key, value) in [("kind", "code"), ("intent", "proven_truth")] {
        let mut raw = input();
        raw[key] = json!(value);
        let parsed = serde_json::from_value::<TextDraft>(raw);
        assert!(parsed.is_err() || parsed.unwrap().validate().is_err());
    }
    let mut raw = input();
    raw["payload"]["format"] = json!("html");
    let parsed = serde_json::from_value::<TextDraft>(raw);
    assert!(parsed.is_err() || parsed.unwrap().validate().is_err());
}

#[test]
fn payload_does_not_hide_unchecked_fields() {
    let mut raw = input();
    raw["payload"]["target"] = json!("hidden");
    assert!(serde_json::from_value::<TextDraft>(raw).is_err());
}

#[test]
fn title_and_text_limits_are_enforced_without_trimming() {
    let mut draft: TextDraft = serde_json::from_value(input()).unwrap();
    assert_eq!(draft.payload.text, "保留空格  与换行\n");
    draft.title = "长".repeat(301);
    assert!(draft.validate().is_err());
    draft.title = "".into();
    draft.payload.text = "a".repeat(200_001);
    assert!(draft.validate().is_err());
    draft.payload.text.clear();
    assert!(draft.validate().is_ok());
}

#[test]
fn digest_ignores_json_key_order_but_tracks_content() {
    let a: TextDraft = serde_json::from_value(input()).unwrap();
    let b: TextDraft = serde_json::from_str(
        r#"{"title":"注记","language":"zh-CN",
      "kind":"text","payload":{"text":"保留空格  与换行\n","format":"markdown"},"intent":"note"}"#,
    )
    .unwrap();
    assert_eq!(a.digest(), b.digest());
    assert_eq!(a.digest().len(), 64);
    let mut changed = b;
    changed.payload.text.push('改');
    assert_ne!(a.digest(), changed.digest());
}

#[test]
fn language_is_ascii_and_bounded_even_for_internal_drafts() {
    let mut draft: TextDraft = serde_json::from_value(input()).unwrap();
    for language in ["中文".to_string(), String::new(), "a".repeat(36)] {
        draft.language = language;
        assert!(draft.validate().is_err());
    }
}

#[test]
fn deserialization_enforces_lengths_not_only_explicit_validation() {
    let mut raw = input();
    raw["title"] = json!("字".repeat(301));
    assert!(serde_json::from_value::<TextDraft>(raw).is_err());
}

#[test]
fn jsonb_unrepresentable_nul_is_rejected_at_the_contract_boundary() {
    for field in ["title", "language", "text"] {
        let mut raw = input();
        if field == "text" {
            raw["payload"][field] = json!("a\u{0}b");
        } else {
            raw[field] = json!("a\u{0}b");
        }
        assert!(serde_json::from_value::<TextDraft>(raw).is_err());
    }
}

#[test]
fn digest_includes_the_contract_version_golden_bytes() {
    let draft: TextDraft = serde_json::from_value(input()).unwrap();
    let golden = r#"{"contract_version":1,"intent":"note","kind":"text","language":"zh-CN","payload":{"format":"markdown","text":"保留空格  与换行\n"},"title":"注记"}"#;
    // Literal bytes are reviewed independently of the serializer.
    assert_eq!(learning_core::canonical_content(&draft), golden.as_bytes());
    // Independently calculated with Python hashlib from the reviewed UTF-8 bytes.
    assert_eq!(
        draft.digest(),
        "35db28db30d4e6d1b4f81c32a365ba9d2b34757bbb6694e09ab308baab1037ec"
    );
}

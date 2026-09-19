use crate::{CONTRACT_VERSION, TextDraft};
use sha2::{Digest, Sha256};

/// Content-v1 uses sorted object keys, UTF-8, compact JSON, no text normalization.
pub fn canonical_content(draft: &TextDraft) -> Vec<u8> {
    let mut value = serde_json::to_value(draft).expect("text contract serializes infallibly");
    value["contract_version"] = CONTRACT_VERSION.into();
    canonical_json(&value).into_bytes()
}
pub fn content_digest(draft: &TextDraft) -> String {
    hex_digest(&canonical_content(draft))
}
pub fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let sorted: std::collections::BTreeMap<_, _> = map.iter().collect();
            let parts: Vec<_> = sorted
                .into_iter()
                .map(|(k, v)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap(),
                        canonical_json(v)
                    )
                })
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        serde_json::Value::Array(items) => format!(
            "[{}]",
            items
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        other => other.to_string(),
    }
}
pub fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

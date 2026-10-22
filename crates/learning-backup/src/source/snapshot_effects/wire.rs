//! The versioned adjunct is separate from the twelve-file v1 source proof.
use crate::{BackupError, FileRecord};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub(crate) const ENCODING: &str = "root_frame_v1";
pub(crate) const MAX_FILE: u64 = 64 * 1024 * 1024;
pub(crate) const MAX_FILES: usize = 100_000;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Descriptor {
    pub format_version: u8,
    pub capability: String,
    pub storage_encoding: String,
    pub deployment_id: Uuid,
    pub epoch_id: Uuid,
    pub binding_id: Uuid,
    pub source_group_id: Uuid,
    pub source_application_commit: String,
    pub source_build_sha256: String,
    pub broker_binary_sha256: String,
    pub broker_pid: u32,
    pub broker_uid: u32,
    pub broker_gid: u32,
    pub broker_binary_dev: u64,
    pub broker_binary_ino: u64,
    pub broker_binary_bytes: u64,
    pub source_database: String,
    pub source_database_oid: i64,
    pub source_system_identifier: String,
    pub source_postmaster_start: String,
    pub companion_root_dev: u64,
    pub companion_root_ino: u64,
    pub export_root_dev: u64,
    pub export_root_ino: u64,
    pub journal_root_dev: u64,
    pub journal_root_ino: u64,
    pub effect_lock_dev: u64,
    pub effect_lock_ino: u64,
    pub initial_namespace_sha256: String,
    pub writer_contract_sha256: String,
    pub work_socket_dev: u64,
    pub work_socket_ino: u64,
    pub management_socket_dev: u64,
    pub management_socket_ino: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reference {
    pub sequence: u64,
    pub sha256: String,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Attempt {
    pub attempt: i32,
    pub claim_token: Uuid,
    pub lease_expires_at: String,
    pub claim_record: Reference,
    #[serde(deserialize_with = "required_option")]
    pub revoked_before_write: Option<Reference>,
    #[serde(deserialize_with = "required_option")]
    pub write_issued: Option<Reference>,
    #[serde(deserialize_with = "required_option")]
    pub business_commit: Option<Reference>,
    #[serde(deserialize_with = "required_option")]
    pub write_closed: Option<Reference>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KnownJob {
    pub job_id: Uuid,
    pub source_job_binding_sha256: String,
    pub current_rows_sha256: String,
    pub attempts: Vec<Attempt>,
    pub last_event: Reference,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Ledger {
    pub format_version: u8,
    pub capability: String,
    pub storage_encoding: String,
    pub epoch_id: Uuid,
    pub binding_id: Uuid,
    pub jobs: Vec<KnownJob>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Cut {
    pub format_version: u8,
    pub capability: String,
    pub storage_encoding: String,
    pub backup_id: Uuid,
    pub capture_id: Uuid,
    pub epoch_id: Uuid,
    pub binding_id: Uuid,
    pub previous_head: Reference,
    pub cut_record: Reference,
    pub descriptor_sha256: String,
    pub ledger_sha256: String,
    pub active_writers: u64,
    pub active_operations: u64,
    pub pools_closed: bool,
    pub observed_work_peers: Vec<WorkPeer>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkPeer {
    pub request_id: Uuid,
    pub process_id: i32,
    pub uid: u32,
    pub gid: u32,
    pub work_socket_dev: u64,
    pub work_socket_ino: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Capture {
    pub format_version: u8,
    pub capability: String,
    pub storage_encoding: String,
    pub backup_id: Uuid,
    pub capture_id: Uuid,
    pub manifest_sha256: String,
    pub source_group_id: Uuid,
    pub source_registry_generation_sha256: String,
    pub companion_authority_sha256: String,
    pub companion_generation_sha256: String,
    pub binding_sha256: String,
    pub epoch_sha256: String,
    pub journal_head_sequence: u64,
    pub journal_head_sha256: String,
    pub cut_event_sha256: String,
    pub rows_sha256: String,
    pub namespace_sha256: String,
    pub files: Vec<FileRecord>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct NamespaceEntry {
    pub path: String,
    pub directory: bool,
    pub size: u64,
}

/// Parsed complete prefix. There is deliberately no public constructor and no
/// serde implementation for this checked representation.
pub(crate) struct Prefix {
    pub records: BTreeMap<u64, (String, Value)>,
}

pub(crate) fn invalid() -> BackupError {
    BackupError::Invalid("snapshot effects v2 contract")
}
fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}
pub(crate) fn hash(bytes: &[u8]) -> String {
    crate::digest(bytes)
}
pub(crate) fn uuid(value: &Value) -> Result<Uuid, BackupError> {
    let text = value.as_str().ok_or_else(invalid)?;
    if !crate::registry::v4(text) {
        return Err(invalid());
    }
    Uuid::parse_str(text).map_err(|_| invalid())
}
pub(crate) fn path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 512
        && path.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        })
}

impl Prefix {
    pub fn check(
        files: &BTreeMap<String, Vec<u8>>,
        descriptor: &[u8],
        epoch: Uuid,
        binding: Uuid,
        head: &Reference,
    ) -> Result<Self, BackupError> {
        let header = files.get("journal/epoch.header").ok_or_else(invalid)?;
        if header.len() != 40
            || &header[..8] != b"KWSJEPO2"
            || &header[8..24] != epoch.as_bytes()
            || &header[24..40] != binding.as_bytes()
            || head.sequence < 2
            || files.len() != head.sequence as usize + 1
        {
            return Err(invalid());
        }
        let mut previous = hash(header);
        let mut records = BTreeMap::new();
        for sequence in 1..=head.sequence {
            let raw = files
                .get(&format!("journal/{sequence:020}.frame"))
                .ok_or_else(invalid)?;
            if !(120..=4 * 1024 * 1024).contains(&raw.len())
                || &raw[..8] != b"KWSJBYT2"
                || raw[8..16] != sequence.to_be_bytes()
                || &raw[16..32] != epoch.as_bytes()
                || &raw[32..48] != binding.as_bytes()
                || hex::encode(&raw[48..80]) != previous
                || hex::encode(&raw[80..112]) != hash(&raw[120..])
                || u64::from_be_bytes(raw[112..120].try_into().map_err(|_| invalid())?)
                    != (raw.len() - 120) as u64
            {
                return Err(invalid());
            }
            let body = &raw[120..];
            let value = unique_json(body)?;
            if sequence == 1 {
                if body != descriptor {
                    return Err(invalid());
                }
            } else {
                Self::envelope(&value, epoch, binding, sequence, &previous)?;
            }
            previous = hash(raw);
            records.insert(sequence, (previous.clone(), value));
        }
        if previous != head.sha256 {
            return Err(invalid());
        }
        Ok(Self { records })
    }

    fn envelope(
        value: &Value,
        epoch: Uuid,
        binding: Uuid,
        seq: u64,
        prev: &str,
    ) -> Result<(), BackupError> {
        let kinds = [
            "CLAIM_INTENT",
            "CLAIM_COMMITTED",
            "CLAIM_NOT_ACQUIRED",
            "JOB_TRANSITION_KNOWN",
            "REVOKED_BEFORE_WRITE",
            "CAPTURE_CUT",
        ];
        if value["capability"] == "snapshot_effect_journal_event_v2" {
            let keys: BTreeSet<_> = value
                .as_object()
                .ok_or_else(invalid)?
                .keys()
                .map(String::as_str)
                .collect();
            if keys
                != BTreeSet::from([
                    "format_version",
                    "capability",
                    "epoch_id",
                    "binding_id",
                    "sequence",
                    "previous_sha256",
                    "kind",
                    "data",
                ])
                || value["format_version"] != 2
                || uuid(&value["epoch_id"])? != epoch
                || uuid(&value["binding_id"])? != binding
                || value["sequence"] != seq
                || !kinds.contains(&value["kind"].as_str().ok_or_else(invalid)?)
                || bytes_hash(&value["previous_sha256"])? != prev
            {
                return Err(invalid());
            }
            let keys: &[&str] = match value["kind"].as_str().ok_or_else(invalid)? {
                "CLAIM_INTENT" => &[
                    "request_id",
                    "worker_request_id",
                    "job_id",
                    "candidate_attempt",
                    "previous_attempt",
                    "before_job",
                    "source_job_binding_sha256",
                    "lease_ms",
                    "identity",
                ],
                "CLAIM_COMMITTED" => &[
                    "request_id",
                    "job_id",
                    "attempt",
                    "claim_token",
                    "lease_expires_at",
                    "before_job",
                    "after_job",
                    "source_job_binding_sha256",
                    "lease_ms",
                    "sql_before",
                    "sql_after",
                    "identity",
                ],
                "CLAIM_NOT_ACQUIRED" => &[
                    "request_id",
                    "job_id",
                    "before_job",
                    "after_job",
                    "reason",
                    "sql_before",
                    "sql_after",
                    "identity",
                ],
                "JOB_TRANSITION_KNOWN" => &[
                    "job_id",
                    "attempt",
                    "claim_record_sha256",
                    "operation",
                    "before_job",
                    "after_job",
                    "sql_return",
                    "lease_ms",
                    "error_class",
                    "retry",
                    "sql_before",
                    "sql_after",
                    "identity",
                ],
                "REVOKED_BEFORE_WRITE" => &[
                    "job_id",
                    "attempt",
                    "claim_record_sha256",
                    "slot_generation",
                    "reason",
                ],
                "CAPTURE_CUT" => &[
                    "capture_id",
                    "source_backup_id",
                    "previous_head_sequence",
                    "previous_head_sha256",
                    "descriptor_sha256",
                    "ledger_sha256",
                    "active_writers",
                    "active_operations",
                    "pools_closed",
                    "observed_work_peers_sha256",
                ],
                _ => return Err(invalid()),
            };
            exact_keys(&value["data"], keys)?;
        } else {
            let schema = value["schema"].as_str().ok_or_else(invalid)?;
            let event = value["event"].as_str().ok_or_else(invalid)?;
            if !matches!(
                (schema, event),
                (
                    "knowweave.snapshot-first-write.c2d0.issue",
                    "WRITE_AUTHORITY_ISSUED"
                ) | (
                    "knowweave.snapshot-first-write.c2d0.staged",
                    "PACKAGE_STAGED"
                ) | (
                    "knowweave.snapshot-publication.d1.business-commit-known",
                    "BUSINESS_COMMIT_KNOWN"
                ) | (
                    "knowweave.snapshot-publication.d1.write-closed",
                    "WRITE_CLOSED"
                )
            ) || value["version"] != 1
                || uuid(&value["epoch"])? != epoch
                || uuid(&value["binding"])? != binding
                || value["previous"]["sequence"] != seq - 1
                || bytes_hash(&value["previous"]["sha256"])? != prev
            {
                return Err(invalid());
            }
            let keys: &[&str] = match event {
                "WRITE_AUTHORITY_ISSUED" => &[
                    "schema",
                    "version",
                    "event",
                    "epoch",
                    "binding",
                    "request",
                    "previous",
                    "claim_intent",
                    "claim_committed",
                    "claim",
                    "input_sha256",
                    "rows",
                    "database_clock",
                    "runtime",
                    "permit_nonce",
                ],
                "PACKAGE_STAGED" => &[
                    "schema",
                    "version",
                    "event",
                    "epoch",
                    "binding",
                    "request",
                    "previous",
                    "issue",
                    "claim_committed",
                    "claim",
                    "manifest_sha256",
                    "verified_files",
                    "verified_bytes",
                    "permit_nonce",
                ],
                "BUSINESS_COMMIT_KNOWN" => &[
                    "schema",
                    "version",
                    "event",
                    "epoch",
                    "binding",
                    "request",
                    "previous",
                    "issue",
                    "package",
                    "claim_committed",
                    "permit_nonce",
                    "before",
                    "after",
                    "runtime",
                    "sql_before",
                    "sql_after",
                    "commit",
                ],
                "WRITE_CLOSED" => &[
                    "schema",
                    "version",
                    "event",
                    "epoch",
                    "binding",
                    "request",
                    "previous",
                    "business_commit",
                    "issue",
                    "package",
                    "permit_nonce",
                    "manifest_sha256",
                    "outcome",
                ],
                _ => return Err(invalid()),
            };
            exact_keys(value, keys)?;
            for name in [
                "previous",
                "claim_intent",
                "claim_committed",
                "issue",
                "package",
                "business_commit",
            ] {
                if !value[name].is_null() {
                    exact_keys(&value[name], &["sequence", "sha256"])?;
                }
            }
            if !value["claim"].is_null() {
                exact_keys(
                    &value["claim"],
                    &["job_id", "token", "attempts", "expires_at"],
                )?;
            }
        }
        Ok(())
    }

    pub fn get(&self, reference: &Reference, kind: &str) -> Result<&Value, BackupError> {
        let (digest, value) = self.records.get(&reference.sequence).ok_or_else(invalid)?;
        if digest != &reference.sha256
            || value["kind"].as_str().or_else(|| value["event"].as_str()) != Some(kind)
        {
            return Err(invalid());
        }
        Ok(value)
    }
}

pub(crate) fn exact_keys(value: &Value, keys: &[&str]) -> Result<(), BackupError> {
    let map = value.as_object().ok_or_else(invalid)?;
    if map.len() != keys.len() || keys.iter().any(|key| !map.contains_key(*key)) {
        return Err(invalid());
    }
    Ok(())
}

/// Value's ordinary map parser overwrites duplicate keys. Journal bodies must
/// reject duplicates at every depth, even inside a job payload/checkpoint.
fn unique_json(raw: &[u8]) -> Result<Value, BackupError> {
    struct Node(Value);
    impl<'de> Deserialize<'de> for Node {
        fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            struct Visitor;
            impl<'de> serde::de::Visitor<'de> for Visitor {
                type Value = Node;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str("unique-key JSON")
                }
                fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Node, E> {
                    Ok(Node(Value::Bool(v)))
                }
                fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Node, E> {
                    Ok(Node(v.into()))
                }
                fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Node, E> {
                    Ok(Node(v.into()))
                }
                fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Node, E> {
                    Err(E::custom("noninteger journal number"))
                }
                fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Node, E> {
                    Ok(Node(v.into()))
                }
                fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Node, E> {
                    Ok(Node(v.into()))
                }
                fn visit_unit<E: serde::de::Error>(self) -> Result<Node, E> {
                    Ok(Node(Value::Null))
                }
                fn visit_seq<A: serde::de::SeqAccess<'de>>(
                    self,
                    mut a: A,
                ) -> Result<Node, A::Error> {
                    let mut values = Vec::new();
                    while let Some(node) = a.next_element::<Node>()? {
                        values.push(node.0);
                    }
                    Ok(Node(Value::Array(values)))
                }
                fn visit_map<A: serde::de::MapAccess<'de>>(
                    self,
                    mut a: A,
                ) -> Result<Node, A::Error> {
                    let mut values = serde_json::Map::new();
                    while let Some(key) = a.next_key::<String>()? {
                        if values.contains_key(&key) {
                            return Err(serde::de::Error::custom("duplicate journal key"));
                        }
                        values.insert(key, a.next_value::<Node>()?.0);
                    }
                    Ok(Node(Value::Object(values)))
                }
            }
            d.deserialize_any(Visitor)
        }
    }
    Ok(serde_json::from_slice::<Node>(raw)?.0)
}

pub(crate) fn bytes_hash(value: &Value) -> Result<String, BackupError> {
    let bytes = value.as_array().ok_or_else(invalid)?;
    if bytes.len() != 32 {
        return Err(invalid());
    }
    let mut raw = [0u8; 32];
    for (slot, value) in raw.iter_mut().zip(bytes) {
        *slot = u8::try_from(value.as_u64().ok_or_else(invalid)?).map_err(|_| invalid())?;
    }
    Ok(hex::encode(raw))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn effects_paths_cannot_escape_or_alias_the_adjunct() {
        for bad in ["", "/a", "a//b", "a/../b", "./x", "a\\b", "a\0b"] {
            assert!(!path(bad));
        }
        assert!(path("journal/00000000000000000001.frame"));
        assert!(path(
            "packages/24ca3a56-538b-4804-929b-a3ec8fe51260/manifest.json"
        ));
    }
    #[test]
    fn a_json_object_or_unlinked_empty_prefix_cannot_create_a_checked_epoch() {
        let id = Uuid::new_v4();
        let head = Reference {
            sequence: 2,
            sha256: "1".repeat(64),
        };
        assert!(Prefix::check(&BTreeMap::new(), b"{}", id, Uuid::new_v4(), &head).is_err());
        assert!(bytes_hash(&serde_json::json!([0, 1])).is_err());
    }
    fn frame(sequence: u64, epoch: Uuid, binding: Uuid, previous: &[u8], body: &[u8]) -> Vec<u8> {
        let mut raw = b"KWSJBYT2".to_vec();
        raw.extend(sequence.to_be_bytes());
        raw.extend(epoch.as_bytes());
        raw.extend(binding.as_bytes());
        raw.extend(hex::decode(hash(previous)).unwrap());
        raw.extend(hex::decode(hash(body)).unwrap());
        raw.extend((body.len() as u64).to_be_bytes());
        raw.extend(body);
        raw
    }
    #[test]
    fn complete_binary_prefix_rejects_gap_body_tamper_and_duplicate_nested_key() {
        let epoch = Uuid::new_v4();
        let binding = Uuid::new_v4();
        let mut header = b"KWSJEPO2".to_vec();
        header.extend(epoch.as_bytes());
        header.extend(binding.as_bytes());
        let first = frame(1, epoch, binding, &header, b"{}");
        let event = serde_json::json!({"format_version":2,"capability":"snapshot_effect_journal_event_v2","epoch_id":epoch,"binding_id":binding,"sequence":2,"previous_sha256":hex::decode(hash(&first)).unwrap(),"kind":"CAPTURE_CUT","data":{
            "capture_id":Uuid::new_v4(),"source_backup_id":Uuid::new_v4(),"previous_head_sequence":1,"previous_head_sha256":hash(&first),"descriptor_sha256":hash(b"{}"),"ledger_sha256":"a".repeat(64),"active_writers":0,"active_operations":0,"pools_closed":true,"observed_work_peers_sha256":"b".repeat(64)}});
        let second = frame(
            2,
            epoch,
            binding,
            &first,
            &serde_json::to_vec(&event).unwrap(),
        );
        let head = Reference {
            sequence: 2,
            sha256: hash(&second),
        };
        let files = BTreeMap::from([
            ("journal/epoch.header".into(), header),
            ("journal/00000000000000000001.frame".into(), first),
            ("journal/00000000000000000002.frame".into(), second),
        ]);
        assert!(Prefix::check(&files, b"{}", epoch, binding, &head).is_ok());
        let mut gap = files.clone();
        gap.remove("journal/00000000000000000001.frame");
        assert!(Prefix::check(&gap, b"{}", epoch, binding, &head).is_err());
        let mut tamper = files.clone();
        *tamper
            .get_mut("journal/00000000000000000002.frame")
            .unwrap()
            .last_mut()
            .unwrap() ^= 1;
        assert!(Prefix::check(&tamper, b"{}", epoch, binding, &head).is_err());
        assert!(unique_json(br#"{"nested":{"same":1,"same":2}}"#).is_err());
        assert!(unique_json(br#"{"float":1.5}"#).is_err());
    }
}

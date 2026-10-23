//! Semantic joins never issue capabilities. The caller must additionally own
//! the original enrolled cut or the independently copied destination object.
use super::{
    files::{Budget, encode},
    wire::*,
};
use crate::{BackupError, ExternalEffectFinding, full_restore::jobs::JobInventory};
use learning_assets::root_journal::SharedJournalBudget;
use learning_db::snapshot_observation::{JobObservation, OutboxObservation};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub(crate) fn cut(
    descriptor: &Descriptor,
    ledger: &Ledger,
    cut: &Cut,
    head: (u64, [u8; 32]),
    descriptor_raw: &[u8],
    ledger_raw: &[u8],
    backup: Uuid,
) -> Result<(), BackupError> {
    if ledger.format_version != 2
        || ledger.capability != "snapshot_effect_known_ledger_v2"
        || ledger.storage_encoding != ENCODING
        || cut.format_version != 2
        || cut.capability != "snapshot_effect_capture_cut_v2"
        || cut.storage_encoding != ENCODING
        || ledger.epoch_id != descriptor.epoch_id
        || cut.epoch_id != descriptor.epoch_id
        || ledger.binding_id != descriptor.binding_id
        || cut.binding_id != descriptor.binding_id
        || cut.backup_id != backup
        || cut.capture_id.get_version_num() != 4
        || cut.descriptor_sha256 != hash(descriptor_raw)
        || cut.ledger_sha256 != hash(ledger_raw)
        || cut.active_writers != 0
        || cut.active_operations != 0
        || !cut.pools_closed
        || cut.cut_record.sequence != head.0
        || cut.cut_record.sha256 != hex::encode(head.1)
        || cut.previous_head.sequence.checked_add(1) != Some(cut.cut_record.sequence)
        || ledger.jobs.len() > MAX_FILES
        || ledger.jobs.windows(2).any(|w| w[0].job_id >= w[1].job_id)
    {
        return Err(invalid());
    }
    for peer in &cut.observed_work_peers {
        if peer.uid != 65532
            || peer.gid != 65532
            || peer.process_id <= 0
            || peer.work_socket_dev != descriptor.work_socket_dev
            || peer.work_socket_ino != descriptor.work_socket_ino
        {
            return Err(invalid());
        }
    }
    Ok(())
}

pub(crate) fn prefix_cut(
    prefix: &Prefix,
    cut: &Cut,
    descriptor_raw: &[u8],
    ledger_raw: &[u8],
    budget: &Budget,
    journal: &SharedJournalBudget,
) -> Result<(), BackupError> {
    let record = prefix.get(&cut.cut_record, "CAPTURE_CUT")?;
    let data = &record["data"];
    if prefix
        .records
        .last_key_value()
        .is_none_or(|(seq, _)| *seq != cut.cut_record.sequence)
        || data["capture_id"] != cut.capture_id.to_string()
        || data["source_backup_id"] != cut.backup_id.to_string()
        || data["previous_head_sequence"] != cut.previous_head.sequence
        || data["previous_head_sha256"] != cut.previous_head.sha256
        || prefix
            .records
            .get(&cut.previous_head.sequence)
            .is_none_or(|(sha, _)| sha != &cut.previous_head.sha256)
        || data["descriptor_sha256"] != hash(descriptor_raw)
        || data["ledger_sha256"] != hash(ledger_raw)
        || data["active_writers"] != 0
        || data["active_operations"] != 0
        || data["pools_closed"] != true
        || data["observed_work_peers_sha256"]
            != hash(&super::files::typed_bytes(
                &cut.observed_work_peers,
                budget,
                journal,
            )?)
    {
        return Err(invalid());
    }
    for (_, record) in prefix.records.values() {
        if record["kind"] == "CLAIM_INTENT"
            && !cut
                .observed_work_peers
                .iter()
                .any(|peer| record["data"]["worker_request_id"] == peer.request_id.to_string())
        {
            return Err(invalid());
        }
    }
    Ok(())
}

fn rows<'a>(inventory: &'a JobInventory, table: &str) -> Result<&'a [Value], BackupError> {
    inventory
        .tables()
        .get(table)
        .map(Vec::as_slice)
        .ok_or_else(invalid)
}
fn one<'a>(rows: &'a [Value], field: &str, id: Uuid) -> Result<&'a Value, BackupError> {
    let mut found = rows
        .iter()
        .filter(|r| r[field].as_str() == Some(id.to_string().as_str()));
    let result = found.next().ok_or_else(invalid)?;
    if found.next().is_some() {
        return Err(invalid());
    }
    Ok(result)
}
pub(crate) fn receipt(inventory: &JobInventory, job: Uuid) -> Result<&Value, BackupError> {
    one(rows(inventory, "snapshot_export_result")?, "job_id", job)
}
fn typed_job(value: &Value) -> Result<JobObservation, BackupError> {
    Ok(serde_json::from_value(value.clone())?)
}
fn typed_outbox(value: &Value) -> Result<OutboxObservation, BackupError> {
    Ok(serde_json::from_value(value.clone())?)
}

fn transition(
    data: &Value,
    before: &JobObservation,
    token: Uuid,
    attempt: i32,
) -> Result<JobObservation, BackupError> {
    exact_keys(
        data,
        &[
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
    )?;
    let actual_before = typed_job(&data["before_job"])?;
    let after = typed_job(&data["after_job"])?;
    if &actual_before != before
        || data["job_id"] != before.id.to_string()
        || data["attempt"] != attempt
        || after.id != before.id
        || after.outbox_id != before.outbox_id
        || after.idempotency_key != before.idempotency_key
        || after.created_at != before.created_at
        || after.checkpoint != before.checkpoint
        || after.attempt_count != before.attempt_count
        || before.attempt_count != attempt
        || !data["sql_return"].is_boolean()
    {
        return Err(invalid());
    }
    let low = normalize_time(&data["sql_before"])?;
    let high = normalize_time(&data["sql_after"])?;
    let mut clock = serde_json::to_value(before)?;
    clock["created_at"] = low;
    clock["updated_at"] = high;
    let clock = typed_job(&clock)?;
    if clock.created_at > clock.updated_at {
        return Err(invalid());
    }
    let operation = data["operation"].as_str().ok_or_else(invalid)?;
    if operation == "renew" {
        if !data["error_class"].is_null()
            || !data["retry"].is_null()
            || data["lease_ms"]
                .as_i64()
                .is_none_or(|n| !(1..=300_000).contains(&n))
        {
            return Err(invalid());
        }
    } else if operation == "fail" {
        if !data["lease_ms"].is_null()
            || !matches!(
                (data["error_class"].as_str(), data["retry"].as_bool()),
                (Some("transient_storage"), Some(true)) | (Some("invalid_input"), Some(false))
            )
        {
            return Err(invalid());
        }
    } else {
        return Err(invalid());
    }
    if data["sql_return"] == false {
        if after != *before {
            return Err(invalid());
        }
        return Ok(after);
    }
    if before.status != "snapshot_running"
        || before.lease_token != Some(token)
        || before
            .lease_expires_at
            .is_none_or(|expiry| expiry <= clock.created_at)
        || after.updated_at < clock.created_at
        || after.updated_at > clock.updated_at
        || after.output_digest.is_some()
    {
        return Err(invalid());
    }
    if operation == "renew" {
        let millis = data["lease_ms"].as_i64().ok_or_else(invalid)?;
        let expiry = after.lease_expires_at.ok_or_else(invalid)?;
        let since_low = expiry
            .signed_duration_since(clock.created_at)
            .num_microseconds()
            .ok_or_else(invalid)?;
        let since_high = expiry
            .signed_duration_since(clock.updated_at)
            .num_microseconds()
            .ok_or_else(invalid)?;
        if after.status != before.status
            || after.lease_token != before.lease_token
            || after.next_attempt_at != before.next_attempt_at
            || after.last_error_class != before.last_error_class
            || expiry < before.lease_expires_at.ok_or_else(invalid)?
            || since_low < millis * 1000
            || since_high > millis * 1000
        {
            return Err(invalid());
        }
    } else {
        let retry = data["retry"] == true && attempt < 3;
        if after.status
            != if retry {
                "snapshot_retry_wait"
            } else {
                "failed"
            }
            || after.lease_token.is_some()
            || after.lease_expires_at.is_some()
            || after.last_error_class.as_deref() != data["error_class"].as_str()
        {
            return Err(invalid());
        }
        if retry {
            let next = after.next_attempt_at.ok_or_else(invalid)?;
            let millis = if attempt == 1 { 1000 } else { 2000 };
            if next
                .signed_duration_since(clock.created_at)
                .num_microseconds()
                .ok_or_else(invalid)?
                < millis * 1000
                || next
                    .signed_duration_since(clock.updated_at)
                    .num_microseconds()
                    .ok_or_else(invalid)?
                    > millis * 1000
            {
                return Err(invalid());
            }
        } else if after.next_attempt_at.is_some() {
            return Err(invalid());
        }
    }
    Ok(after)
}
fn normalize_time(value: &Value) -> Result<Value, BackupError> {
    // Reuse the existing exact typed timestamp parser without adding another
    // date grammar. All unrelated fields have fixed, valid observation values.
    let id = "c13e17fb-4686-4d7d-92da-6e9765c7f2a2";
    let parsed: OutboxObservation = serde_json::from_value(serde_json::json!({
        "id":id,"business_key":"timestamp-parser","event_type":"timestamp-parser",
        "payload_version":1,"payload":{},"actor_id":id,"processor_version":1,
        "created_at":value,"dispatched_at":null
    }))?;
    Ok(serde_json::to_value(parsed.created_at)?)
}
fn receipt_equal(a: &Value, b: &Value) -> Result<bool, BackupError> {
    let mut a = a.clone();
    let mut b = b.clone();
    a["created_at"] = normalize_time(&a["created_at"])?;
    b["created_at"] = normalize_time(&b["created_at"])?;
    Ok(a == b)
}
fn ref_matches(value: &Value, reference: &Reference) -> Result<bool, BackupError> {
    Ok(
        value["sequence"] == reference.sequence
            && bytes_hash(&value["sha256"])? == reference.sha256,
    )
}

fn timestamp(value: &Value) -> Result<OutboxObservation, BackupError> {
    Ok(serde_json::from_value(
        serde_json::json!({"id":"c13e17fb-4686-4d7d-92da-6e9765c7f2a2",
        "business_key":"clock","event_type":"clock","payload_version":1,"payload":{},
        "actor_id":"c13e17fb-4686-4d7d-92da-6e9765c7f2a2","processor_version":1,"created_at":value,"dispatched_at":null}),
    )?)
}
fn invariant(a: &JobObservation, b: &JobObservation) -> bool {
    a.id == b.id
        && a.outbox_id == b.outbox_id
        && a.idempotency_key == b.idempotency_key
        && a.created_at == b.created_at
        && a.checkpoint == b.checkpoint
}
fn identity(value: &Value, descriptor: &Value, role: &str) -> Result<(), BackupError> {
    exact_keys(
        value,
        &[
            "database",
            "oid",
            "postmaster",
            "session_user",
            "current_user",
            "backend",
            "transaction",
        ],
    )?;
    if value["database"] != descriptor["source_database"]
        || value["oid"] != descriptor["source_database_oid"]
        || timestamp(&value["postmaster"])?.created_at
            != timestamp(&descriptor["source_postmaster_start"])?.created_at
        || value["session_user"] != role
        || value["current_user"] != role
        || value["backend"]
            .as_i64()
            .is_none_or(|id| id <= 0 || id > i64::from(i32::MAX))
        || value["transaction"].as_str().is_none_or(|id| {
            id.parse::<u64>()
                .ok()
                .filter(|n| *n > 0)
                .is_none_or(|n| n.to_string() != id)
        })
    {
        return Err(invalid());
    }
    Ok(())
}
fn runtime_identity(value: &Value, descriptor: &Value) -> Result<(), BackupError> {
    exact_keys(
        value,
        &[
            "database",
            "oid",
            "postmaster",
            "session_user",
            "current_user",
            "original_login",
            "restricted_login",
            "server_address",
            "server_port",
        ],
    )?;
    if value["database"] != descriptor["source_database"]
        || value["oid"] != descriptor["source_database_oid"]
        || timestamp(&value["postmaster"])?.created_at
            != timestamp(&descriptor["source_postmaster_start"])?.created_at
        || value["session_user"] != "learning_runtime"
        || value["current_user"] != "learning_runtime"
        || value["restricted_login"] != true
        || value["original_login"]
            .as_str()
            .and_then(|login| login.split_once(':'))
            .is_none_or(|(method, user)| method.is_empty() || user != "learning_runtime")
    {
        return Err(invalid());
    }
    Ok(())
}
fn claim_delta(
    intent: &Value,
    data: &Value,
    before: &JobObservation,
    after: &JobObservation,
    descriptor: &Value,
) -> Result<(), BackupError> {
    uuid(&intent["request_id"])?;
    identity(
        &intent["identity"],
        descriptor,
        "learning_snapshot_claim_broker",
    )?;
    identity(
        &data["identity"],
        descriptor,
        "learning_snapshot_claim_broker",
    )?;
    let low = timestamp(&data["sql_before"])?.created_at;
    let high = timestamp(&data["sql_after"])?.created_at;
    let millis = data["lease_ms"]
        .as_i64()
        .filter(|ms| (1..=300000).contains(ms))
        .ok_or_else(invalid)?;
    let expiry = after.lease_expires_at.ok_or_else(invalid)?;
    if intent["identity"] != data["identity"]
        || intent["lease_ms"] != data["lease_ms"]
        || low > high
        || !invariant(before, after)
        || after.attempt_count != before.attempt_count + 1
        || after.id != uuid(&data["job_id"])?
        || after.status != "snapshot_running"
        || after.lease_token != Some(uuid(&data["claim_token"])?)
        || expiry != timestamp(&data["lease_expires_at"])?.created_at
        || after.next_attempt_at.is_some()
        || after.last_error_class.is_some()
        || after.output_digest.is_some()
        || after.updated_at < low
        || after.updated_at > high
        || expiry
            .signed_duration_since(low)
            .num_microseconds()
            .ok_or_else(invalid)?
            < millis * 1000
        || expiry
            .signed_duration_since(high)
            .num_microseconds()
            .ok_or_else(invalid)?
            > millis * 1000
        || !match before.status.as_str() {
            "snapshot_queued" => {
                before.attempt_count == 0
                    && before.lease_token.is_none()
                    && before.lease_expires_at.is_none()
                    && before.next_attempt_at.is_none()
            }
            "snapshot_running" => {
                before.attempt_count < 3
                    && before.lease_token.is_some()
                    && before.lease_expires_at.is_some_and(|expiry| expiry <= low)
            }
            "snapshot_retry_wait" => {
                before.attempt_count < 3
                    && before.lease_token.is_none()
                    && before.lease_expires_at.is_none()
                    && before.next_attempt_at.is_some_and(|next| next <= low)
            }
            _ => false,
        }
    {
        return Err(invalid());
    }
    Ok(())
}
fn not_acquired(
    intent: &Value,
    data: &Value,
    before: &JobObservation,
    after: &JobObservation,
    descriptor: &Value,
) -> Result<(), BackupError> {
    uuid(&intent["request_id"])?;
    identity(
        &intent["identity"],
        descriptor,
        "learning_snapshot_claim_broker",
    )?;
    identity(
        &data["identity"],
        descriptor,
        "learning_snapshot_claim_broker",
    )?;
    let low = timestamp(&data["sql_before"])?.created_at;
    let high = timestamp(&data["sql_after"])?.created_at;
    if intent["identity"] != data["identity"]
        || low > high
        || intent["request_id"] != data["request_id"]
        || intent["job_id"] != data["job_id"]
        || intent["lease_ms"]
            .as_i64()
            .is_none_or(|ms| !(1..=300000).contains(&ms))
        || intent["previous_attempt"] != before.attempt_count
        || intent["candidate_attempt"] != (before.attempt_count + 1).min(3)
        || typed_job(&intent["before_job"])? != *before
        || data["job_id"] != before.id.to_string()
    {
        return Err(invalid());
    }
    match data["reason"].as_str() {
        Some("ineligible") if before == after => (),
        Some("expired_at_ceiling")
            if invariant(before, after)
                && before.status == "snapshot_running"
                && before.attempt_count == 3
                && before.lease_token.is_some()
                && before.lease_expires_at.is_some_and(|expiry| expiry <= low)
                && after.status == "failed"
                && after.attempt_count == 3
                && after.lease_token.is_none()
                && after.lease_expires_at.is_none()
                && after.next_attempt_at == before.next_attempt_at
                && after.output_digest.is_none()
                && after.last_error_class.as_deref() == Some("lease_expired")
                && after.updated_at >= low
                && after.updated_at <= high => {}
        _ => return Err(invalid()),
    }
    Ok(())
}

fn full_join<'a>(
    inventory: &'a JobInventory,
    job: &JobObservation,
    outbox: &OutboxObservation,
    budget: &Budget,
    journal: &SharedJournalBudget,
) -> Result<Vec<(&'a Value, &'a Value)>, BackupError> {
    if job.id != job.outbox_id
        || job.id != outbox.id
        || job.idempotency_key != outbox.business_key
        || outbox.event_type != "snapshot_export_requested"
        || outbox.payload_version != 1
        || outbox.processor_version != 1
        || outbox.dispatched_at.is_none()
        || outbox.payload["actor_id"] != outbox.actor_id.to_string()
    {
        return Err(invalid());
    }
    let request_digest = hash(&encode(&outbox.payload, budget, journal)?);
    let mut result = Vec::new();
    let mut request_ids = BTreeSet::new();
    for request in rows(inventory, "snapshot_export_request")?
        .iter()
        .filter(|r| r["job_id"] == job.id.to_string())
    {
        if request["outbox_id"] != outbox.id.to_string()
            || request["actor_id"] != outbox.actor_id.to_string()
            || !request_ids.insert(uuid(&request["request_id"])?)
        {
            return Err(invalid());
        }
        let mut keys = rows(inventory, "request_key")?.iter().filter(|key| {
            key["actor_id"] == request["actor_id"] && key["request_id"] == request["request_id"]
        });
        let key = keys.next().ok_or_else(invalid)?;
        if keys.next().is_some()
            || key["operation"] != "snapshot_export"
            || key["request_sha256"] != request_digest
        {
            return Err(invalid());
        }
        result.push((request, key));
    }
    if result.is_empty()
        || rows(inventory, "asset_integrity_result")?
            .iter()
            .any(|r| r["job_id"] == job.id.to_string())
    {
        return Err(invalid());
    }
    Ok(result)
}

fn global_join(inventory: &JobInventory) -> Result<(), BackupError> {
    let jobs = rows(inventory, "job")?;
    let events = rows(inventory, "job_outbox")?;
    let requests = rows(inventory, "snapshot_export_request")?;
    let keys = rows(inventory, "request_key")?;
    let mut pairs = BTreeSet::new();
    for request in requests {
        exact_keys(request, &["actor_id", "request_id", "outbox_id", "job_id"])?;
        let job = one(jobs, "id", uuid(&request["job_id"])?)?;
        let event = one(events, "id", uuid(&request["outbox_id"])?)?;
        if job["outbox_id"] != request["outbox_id"]
            || event["event_type"] != "snapshot_export_requested"
            || event["actor_id"] != request["actor_id"]
            || !pairs.insert((uuid(&request["actor_id"])?, uuid(&request["request_id"])?))
        {
            return Err(invalid());
        }
    }
    let mut seen_keys = BTreeSet::new();
    for key in keys {
        let pair = (uuid(&key["actor_id"])?, uuid(&key["request_id"])?);
        if !seen_keys.insert(pair)
            || (key["operation"] == "snapshot_export" && !pairs.contains(&pair))
        {
            return Err(invalid());
        }
    }
    let mut results = BTreeSet::new();
    for receipt in rows(inventory, "snapshot_export_result")? {
        let id = uuid(&receipt["job_id"])?;
        let job = one(jobs, "id", id)?;
        let event = one(events, "id", uuid(&job["outbox_id"])?)?;
        if event["event_type"] != "snapshot_export_requested" || !results.insert(id) {
            return Err(invalid());
        }
    }
    for event in events
        .iter()
        .filter(|e| e["event_type"] == "snapshot_export_requested")
    {
        let job = one(jobs, "id", uuid(&event["id"])?)?;
        if job["outbox_id"] != event["id"] {
            return Err(invalid());
        }
    }
    Ok(())
}

fn digest_typed(
    job: &JobObservation,
    outbox: &OutboxObservation,
    budget: &Budget,
    journal: &SharedJournalBudget,
) -> Result<(String, String), BackupError> {
    let pair = super::files::typed_bytes(&(job, outbox), budget, journal)?;
    let event = super::files::typed_bytes(outbox, budget, journal)?;
    Ok((hash(&pair), hash(&event)))
}

/// Every known attempt is represented exactly once in the full prefix, and the
/// final typed source row must match the original owner's last known row hash.
pub(crate) fn inventory(
    inventory: &JobInventory,
    ledger: &Ledger,
    prefix: &Prefix,
    namespace: &[NamespaceEntry],
    budget: &Budget,
    journal: &SharedJournalBudget,
) -> Result<BTreeMap<Uuid, ExternalEffectFinding>, BackupError> {
    let descriptor = &prefix.records.get(&1).ok_or_else(invalid)?.1;
    let mut findings = BTreeMap::new();
    let jobs = rows(inventory, "job")?;
    let events = rows(inventory, "job_outbox")?;
    namespace_closed(namespace, jobs, events)?;
    global_join(inventory)?;
    let (_, (_, cut)) = prefix.records.last_key_value().ok_or_else(invalid)?;
    let cut_data = &cut["data"];
    if cut["kind"] != "CAPTURE_CUT"
        || cut_data["ledger_sha256"] != hash(&super::files::typed_bytes(ledger, budget, journal)?)
        || cut_data["active_writers"] != 0
        || cut_data["active_operations"] != 0
        || cut_data["pools_closed"] != true
    {
        return Err(invalid());
    }
    let mut seen = BTreeSet::new();
    let mut referenced = BTreeSet::new();
    for row in jobs {
        let job = typed_job(row)?;
        if !seen.insert(job.id) {
            return Err(invalid());
        }
        let outbox = typed_outbox(one(events, "id", job.outbox_id)?)?;
        if outbox.event_type != "snapshot_export_requested" {
            continue;
        }
        let pairs = match full_join(inventory, &job, &outbox, budget, journal) {
            Ok(pairs) => pairs,
            Err(BackupError::Invalid(_)) => {
                findings.insert(job.id, ExternalEffectFinding::Unknown);
                continue;
            }
            Err(error) => return Err(error),
        };
        let known = ledger.jobs.iter().find(|known| known.job_id == job.id);
        let result_rows = rows(inventory, "snapshot_export_result")?;
        let no_receipt = !result_rows
            .iter()
            .any(|r| r["job_id"] == job.id.to_string());
        let job_namespace: Vec<_> = namespace
            .iter()
            .filter(|entry| {
                entry.path == job.id.to_string() || entry.path.starts_with(&format!("{}/", job.id))
            })
            .collect();
        let Some(known) = known else {
            let no_effect = job.status == "snapshot_queued"
                && job.attempt_count == 0
                && job.lease_token.is_none()
                && job.lease_expires_at.is_none()
                && job.next_attempt_at.is_none()
                && job.checkpoint.is_none()
                && job.output_digest.is_none()
                && job.last_error_class.is_none()
                && no_receipt
                && job_namespace.is_empty()
                && !prefix.records.values().any(|(_, v)| {
                    v["data"]["job_id"] == job.id.to_string()
                        || v["claim"]["job_id"] == job.id.to_string()
                });
            findings.insert(
                job.id,
                if no_effect {
                    ExternalEffectFinding::ConfirmedNoEffect
                } else {
                    ExternalEffectFinding::Unknown
                },
            );
            continue;
        };
        let (current, binding) = digest_typed(&job, &outbox, budget, journal)?;
        if current != known.current_rows_sha256
            || binding != known.source_job_binding_sha256
            || known.attempts.is_empty()
            || known.attempts.len() > 3
            || known.attempts.len() != job.attempt_count as usize
        {
            return Err(invalid());
        }
        let mut previous_job: Option<JobObservation> = None;
        let mut previous_end = 0;
        let mut all_revoked = true;
        for (index, attempt) in known.attempts.iter().enumerate() {
            if attempt.attempt != index as i32 + 1 || attempt.claim_token.get_version_num() != 4 {
                return Err(invalid());
            }
            let claim = prefix.get(&attempt.claim_record, "CLAIM_COMMITTED")?;
            referenced.insert(attempt.claim_record.sequence);
            let data = &claim["data"];
            let before = typed_job(&data["before_job"])?;
            let after = typed_job(&data["after_job"])?;
            let intent_seq = attempt
                .claim_record
                .sequence
                .checked_sub(1)
                .ok_or_else(invalid)?;
            let (intent_hash, intent) = prefix.records.get(&intent_seq).ok_or_else(invalid)?;
            let _ = intent_hash;
            referenced.insert(intent_seq);
            claim_delta(&intent["data"], data, &before, &after, descriptor)?;
            if intent["kind"] != "CLAIM_INTENT"
                || intent["data"]["job_id"] != job.id.to_string()
                || intent["data"]["before_job"] != data["before_job"]
                || intent["data"]["request_id"] != data["request_id"]
                || intent["data"]["candidate_attempt"] != attempt.attempt
                || intent["data"]["previous_attempt"] != index
                || data["job_id"] != job.id.to_string()
                || data["attempt"] != attempt.attempt
                || data["claim_token"] != attempt.claim_token.to_string()
                || data["source_job_binding_sha256"] != binding
                || intent["data"]["source_job_binding_sha256"] != binding
                || after.id != job.id
                || after.attempt_count != attempt.attempt
                || after.status != "snapshot_running"
                || after.lease_token != Some(attempt.claim_token)
                || after.lease_expires_at.is_none()
                || after.output_digest.is_some()
                || after.checkpoint != before.checkpoint
                || after.outbox_id != before.outbox_id
                || after.created_at != before.created_at
                || after.idempotency_key != before.idempotency_key
                || before.attempt_count != index as i32
                || intent_seq <= previous_end
                || (index == 0
                    && (before.status != "snapshot_queued"
                        || before.checkpoint.is_some()
                        || before.lease_token.is_some()
                        || before.lease_expires_at.is_some()
                        || before.next_attempt_at.is_some()
                        || before.last_error_class.is_some()
                        || before.output_digest.is_some()))
                || previous_job
                    .as_ref()
                    .is_some_and(|previous| previous != &before)
            {
                return Err(invalid());
            }
            let mut latest = after;
            let mut final_expiry = latest.lease_expires_at.ok_or_else(invalid)?;
            let mut issued_seen = false;
            let end = attempt
                .revoked_before_write
                .as_ref()
                .or(attempt.write_closed.as_ref())
                .ok_or_else(invalid)?;
            for (seq, (_, event)) in prefix
                .records
                .range((attempt.claim_record.sequence + 1)..end.sequence)
            {
                if event["event"] == "WRITE_AUTHORITY_ISSUED"
                    && event["claim"]["job_id"] == job.id.to_string()
                {
                    if attempt
                        .write_issued
                        .as_ref()
                        .is_none_or(|issued| issued.sequence != *seq)
                        || issued_seen
                    {
                        return Err(invalid());
                    }
                    issue(
                        event,
                        &latest,
                        &outbox,
                        attempt,
                        descriptor,
                        &data["request_id"],
                        prefix,
                        budget,
                        journal,
                    )?;
                    issued_seen = true;
                }
                if event["kind"] == "JOB_TRANSITION_KNOWN"
                    && event["data"]["job_id"] == job.id.to_string()
                {
                    let t = &event["data"];
                    if t["claim_record_sha256"] != attempt.claim_record.sha256 {
                        return Err(invalid());
                    }
                    identity(&t["identity"], descriptor, "learning_snapshot_claim_broker")?;
                    if issued_seen && (t["operation"] != "renew" || t["sql_return"] != true) {
                        return Err(invalid());
                    }
                    latest = transition(t, &latest, attempt.claim_token, attempt.attempt)?;
                    if let Some(expiry) = latest.lease_expires_at {
                        final_expiry = expiry;
                    }
                    referenced.insert(*seq);
                }
            }
            if timestamp(&Value::String(attempt.lease_expires_at.clone()))?.created_at
                != final_expiry
            {
                return Err(invalid());
            }
            if let Some(revoked) = &attempt.revoked_before_write {
                if attempt.write_issued.is_some()
                    || attempt.business_commit.is_some()
                    || attempt.write_closed.is_some()
                    || revoked.sequence <= attempt.claim_record.sequence
                {
                    return Err(invalid());
                }
                let r = prefix.get(revoked, "REVOKED_BEFORE_WRITE")?;
                if r["data"]["job_id"] != job.id.to_string()
                    || r["data"]["attempt"] != attempt.attempt
                    || r["data"]["claim_record_sha256"] != attempt.claim_record.sha256
                    || r["data"]["reason"] != "management_quiesce"
                    || r["data"]["slot_generation"] != intent["data"]["worker_request_id"]
                {
                    return Err(invalid());
                }
                referenced.insert(revoked.sequence);
                previous_end = revoked.sequence;
                previous_job = Some(latest);
            } else {
                if index + 1 != known.attempts.len() {
                    return Err(invalid());
                }
                committed(
                    inventory,
                    &job,
                    &outbox,
                    &pairs,
                    attempt,
                    prefix,
                    &mut referenced,
                    &latest,
                    descriptor,
                    namespace,
                )?;
                previous_end = end.sequence;
                previous_job = Some(typed_job(
                    &prefix.get(
                        attempt.business_commit.as_ref().ok_or_else(invalid)?,
                        "BUSINESS_COMMIT_KNOWN",
                    )?["after"]["job"],
                )?);
                all_revoked = false;
            }
        }
        let (last_hash, last) = prefix
            .records
            .get(&known.last_event.sequence)
            .ok_or_else(invalid)?;
        if last_hash != &known.last_event.sha256 {
            return Err(invalid());
        }
        if known.last_event.sequence < previous_end {
            return Err(invalid());
        }
        if known.last_event.sequence > previous_end {
            if last["kind"] != "CLAIM_NOT_ACQUIRED"
                || last["data"]["job_id"] != job.id.to_string()
                || &typed_job(&last["data"]["before_job"])?
                    != previous_job.as_ref().ok_or_else(invalid)?
            {
                return Err(invalid());
            }
            previous_job = Some(typed_job(&last["data"]["after_job"])?);
            referenced.insert(known.last_event.sequence);
            let intent_seq = known
                .last_event
                .sequence
                .checked_sub(1)
                .ok_or_else(invalid)?;
            if prefix.records.get(&intent_seq).is_none_or(|(_, v)| {
                v["kind"] != "CLAIM_INTENT" || v["data"]["job_id"] != job.id.to_string()
            }) {
                return Err(invalid());
            }
            let intent = &prefix.records.get(&intent_seq).ok_or_else(invalid)?.1;
            if intent["data"]["source_job_binding_sha256"] != binding {
                return Err(invalid());
            }
            not_acquired(
                &intent["data"],
                &last["data"],
                &typed_job(&last["data"]["before_job"])?,
                &typed_job(&last["data"]["after_job"])?,
                descriptor,
            )?;
            referenced.insert(intent_seq);
        }
        if previous_job.as_ref() != Some(&job) {
            return Err(invalid());
        }
        let finding = if all_revoked
            && no_receipt
            && job.output_digest.is_none()
            && job.checkpoint.is_none()
            && job_namespace.is_empty()
        {
            ExternalEffectFinding::ConfirmedNoEffect
        } else if !all_revoked
            && namespace_winner(
                &job_namespace,
                &job,
                known.attempts.last().ok_or_else(invalid)?,
            )?
        {
            ExternalEffectFinding::AlreadyCommitted
        } else {
            ExternalEffectFinding::Unknown
        };
        findings.insert(job.id, finding);
    }
    if ledger
        .jobs
        .iter()
        .any(|known| !seen.contains(&known.job_id))
    {
        return Err(invalid());
    }
    // An event cannot be silently dropped from the complete history. The only
    // unreferenced records are the epoch descriptor and the final CUT.
    for (seq, (_, value)) in &prefix.records {
        if value["kind"] == "CAPTURE_CUT"
            && *seq != *prefix.records.last_key_value().ok_or_else(invalid)?.0
        {
            return Err(invalid());
        }
        if *seq != 1 && value["kind"] != "CAPTURE_CUT" && !referenced.contains(seq) {
            return Err(invalid());
        }
    }
    Ok(findings)
}

#[allow(clippy::too_many_arguments)]
fn issue(
    record: &Value,
    latest: &JobObservation,
    outbox: &OutboxObservation,
    attempt: &Attempt,
    descriptor: &Value,
    request: &Value,
    prefix: &Prefix,
    budget: &Budget,
    journal: &SharedJournalBudget,
) -> Result<(), BackupError> {
    exact_keys(&record["rows"], &["job", "outbox", "observed_at"])?;
    runtime_identity(&record["runtime"], descriptor)?;
    let clock = timestamp(&record["database_clock"])?.created_at;
    if typed_job(&record["rows"]["job"])? != *latest
        || typed_outbox(&record["rows"]["outbox"])? != *outbox
        || record["request"] != *request
        || record["claim"]["job_id"] != latest.id.to_string()
        || record["claim"]["token"] != attempt.claim_token.to_string()
        || record["claim"]["attempts"] != attempt.attempt
        || timestamp(&record["claim"]["expires_at"])?.created_at
            != latest.lease_expires_at.ok_or_else(invalid)?
        || timestamp(&record["rows"]["observed_at"])?.created_at > clock
        || latest.lease_expires_at.is_none_or(|expiry| expiry <= clock)
        || record["input_sha256"] != hash(&super::files::typed_bytes(outbox, budget, journal)?)
    {
        return Err(invalid());
    }
    let intent_sequence = attempt
        .claim_record
        .sequence
        .checked_sub(1)
        .ok_or_else(invalid)?;
    let intent_ref = Reference {
        sequence: intent_sequence,
        sha256: prefix
            .records
            .get(&intent_sequence)
            .ok_or_else(invalid)?
            .0
            .clone(),
    };
    if !ref_matches(&record["claim_intent"], &intent_ref)?
        || !ref_matches(&record["claim_committed"], &attempt.claim_record)?
    {
        return Err(invalid());
    }
    uuid(&record["permit_nonce"])?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn committed(
    inventory: &JobInventory,
    job: &JobObservation,
    outbox: &OutboxObservation,
    pairs: &[(&Value, &Value)],
    attempt: &Attempt,
    prefix: &Prefix,
    referenced: &mut BTreeSet<u64>,
    latest: &JobObservation,
    descriptor: &Value,
    namespace: &[NamespaceEntry],
) -> Result<(), BackupError> {
    let issued = attempt.write_issued.as_ref().ok_or_else(invalid)?;
    let business = attempt.business_commit.as_ref().ok_or_else(invalid)?;
    let closed = attempt.write_closed.as_ref().ok_or_else(invalid)?;
    let issue = prefix.get(issued, "WRITE_AUTHORITY_ISSUED")?;
    if !ref_matches(&issue["claim_committed"], &attempt.claim_record)?
        || issue["claim"]["job_id"] != job.id.to_string()
        || issue["claim"]["token"] != attempt.claim_token.to_string()
        || issue["claim"]["attempts"] != attempt.attempt
    {
        return Err(invalid());
    }
    let package_seq = business.sequence.checked_sub(1).ok_or_else(invalid)?;
    let (package_hash, package) = prefix.records.get(&package_seq).ok_or_else(invalid)?;
    let package_ref = Reference {
        sequence: package_seq,
        sha256: package_hash.clone(),
    };
    let b = prefix.get(business, "BUSINESS_COMMIT_KNOWN")?;
    let c = prefix.get(closed, "WRITE_CLOSED")?;
    let receipt = receipt(inventory, job.id)?;
    let after_request = &b["after"]["request"];
    let after_key = &b["after"]["key"];
    for full in [&b["before"], &b["after"]] {
        exact_keys(
            full,
            &["job", "outbox", "request", "key", "receipt", "observed_at"],
        )?;
        exact_keys(
            &full["request"],
            &["actor_id", "request_id", "outbox_id", "job_id"],
        )?;
        exact_keys(
            &full["key"],
            &["actor_id", "request_id", "request_sha256", "operation"],
        )?;
    }
    exact_keys(
        &b["after"]["receipt"],
        &[
            "job_id",
            "stage_token",
            "capability",
            "manifest_sha256",
            "plan_sha256",
            "required_spaces",
            "created_at",
        ],
    )?;
    exact_keys(&b["runtime"], &["runtime", "backend", "transaction"])?;
    runtime_identity(&b["runtime"]["runtime"], descriptor)?;
    let low = timestamp(&b["sql_before"])?.created_at;
    let high = timestamp(&b["sql_after"])?.created_at;
    let before = typed_job(&b["before"]["job"])?;
    let created = timestamp(&receipt["created_at"])?.created_at;
    let package_prefix = format!("{}/{}.ready/", job.id, attempt.claim_token);
    let verified_bytes = namespace
        .iter()
        .filter(|entry| {
            !entry.directory
                && entry.path.starts_with(&package_prefix)
                && !entry.path.ends_with("/manifest.json")
        })
        .try_fold(0u64, |n, entry| {
            n.checked_add(entry.size).ok_or(BackupError::Overflow)
        })?;
    if job.status != "succeeded"
        || job.lease_token.is_some()
        || job.lease_expires_at.is_some()
        || job.next_attempt_at.is_some()
        || job.last_error_class.is_some()
        || receipt["stage_token"] != attempt.claim_token.to_string()
        || receipt["job_id"] != job.id.to_string()
        || receipt["capability"] != "reading_copy_v1"
        || &before != latest
        || !invariant(&before, job)
        || before.attempt_count != job.attempt_count
        || before.status != "snapshot_running"
        || before.lease_token != Some(attempt.claim_token)
        || before.lease_expires_at.is_none_or(|expiry| expiry <= low)
        || low > high
        || job.updated_at < low
        || job.updated_at > high
        || created < low
        || created > high
        || timestamp(&b["before"]["observed_at"])?.created_at > low
        || timestamp(&b["after"]["observed_at"])?.created_at < high
        || b["runtime"]["backend"]
            .as_i64()
            .is_none_or(|id| id <= 0 || id > i64::from(i32::MAX))
        || b["runtime"]["transaction"].as_str().is_none_or(|id| {
            id.parse::<u64>()
                .ok()
                .filter(|n| *n > 0)
                .is_none_or(|n| n.to_string() != id)
        })
        || b["runtime"]["runtime"]["server_address"] != issue["runtime"]["server_address"]
        || b["runtime"]["runtime"]["server_port"] != issue["runtime"]["server_port"]
        || receipt["manifest_sha256"].as_str() != job.output_digest.as_deref()
        || !crate::valid_digest(receipt["plan_sha256"].as_str().ok_or_else(invalid)?)
        || package["event"] != "PACKAGE_STAGED"
        || package["verified_files"] != 3
        || package["verified_bytes"] != verified_bytes
        || package["request"] != issue["request"]
        || b["request"] != issue["request"]
        || c["request"] != issue["request"]
        || !ref_matches(&package["claim_committed"], &attempt.claim_record)?
        || package["claim"]["job_id"] != job.id.to_string()
        || package["claim"]["token"] != attempt.claim_token.to_string()
        || package["claim"]["attempts"] != attempt.attempt
        || timestamp(&package["claim"]["expires_at"])?.created_at
            != latest.lease_expires_at.ok_or_else(invalid)?
        || after_request["request_id"] != issue["request"]
        || !ref_matches(&package["issue"], issued)?
        || package["permit_nonce"] != issue["permit_nonce"]
        || package["manifest_sha256"] != receipt["manifest_sha256"]
        || !ref_matches(&b["issue"], issued)?
        || !ref_matches(&b["package"], &package_ref)?
        || !ref_matches(&b["claim_committed"], &attempt.claim_record)?
        || b["commit"] != "same_original_business_commit_returned_success"
        || b["permit_nonce"] != issue["permit_nonce"]
        || typed_job(&b["after"]["job"])? != *job
        || typed_outbox(&b["after"]["outbox"])? != *outbox
        || typed_outbox(&b["before"]["outbox"])? != *outbox
        || !b["before"]["receipt"].is_null()
        || b["before"]["request"] != *after_request
        || b["before"]["key"] != *after_key
        || !pairs
            .iter()
            .any(|(request, key)| *request == after_request && *key == after_key)
        || !receipt_equal(receipt, &b["after"]["receipt"])?
        || !ref_matches(&c["business_commit"], business)?
        || !ref_matches(&c["issue"], issued)?
        || !ref_matches(&c["package"], &package_ref)?
        || c["permit_nonce"] != issue["permit_nonce"]
        || c["manifest_sha256"] != receipt["manifest_sha256"]
        || c["outcome"] != "succeeded"
        || !(attempt.claim_record.sequence < issued.sequence
            && issued.sequence < package_seq
            && package_seq < business.sequence
            && business.sequence < closed.sequence)
    {
        return Err(invalid());
    }
    let spaces = receipt["required_spaces"].as_array().ok_or_else(invalid)?;
    let mut previous = None;
    for space in spaces {
        let id = uuid(space)?;
        if previous.is_some_and(|p| p >= id) {
            return Err(invalid());
        }
        previous = Some(id);
    }
    for seq in [
        issued.sequence,
        package_seq,
        business.sequence,
        closed.sequence,
    ] {
        referenced.insert(seq);
    }
    Ok(())
}

fn namespace_winner(
    entries: &[&NamespaceEntry],
    job: &JobObservation,
    attempt: &Attempt,
) -> Result<bool, BackupError> {
    let job_name = job.id.to_string();
    let ready = format!("{}/{}.ready", job.id, attempt.claim_token);
    let expected = BTreeSet::from([
        "reading.md",
        "reading.html",
        "validation.json",
        "manifest.json",
    ]);
    let files: BTreeSet<_> = entries
        .iter()
        .filter_map(|entry| {
            entry
                .path
                .strip_prefix(&format!("{ready}/"))
                .map(|name| (name, entry.directory))
        })
        .collect();
    Ok(entries.len() == 6
        && entries
            .iter()
            .any(|entry| entry.path == job_name && entry.directory)
        && entries
            .iter()
            .any(|entry| entry.path == ready && entry.directory)
        && files.len() == 4
        && files
            .iter()
            .all(|(name, is_dir)| expected.contains(name) && !*is_dir))
}

fn namespace_closed(
    namespace: &[NamespaceEntry],
    jobs: &[Value],
    events: &[Value],
) -> Result<(), BackupError> {
    if namespace.len() < 2 || namespace.windows(2).any(|w| w[0].path >= w[1].path) {
        return Err(invalid());
    }
    let mut snapshot_jobs = BTreeSet::new();
    for job in jobs {
        let id = uuid(&job["id"])?;
        let event = one(events, "id", uuid(&job["outbox_id"])?)?;
        if event["event_type"] == "snapshot_export_requested" {
            snapshot_jobs.insert(id);
        }
    }
    let mut base = BTreeSet::new();
    for entry in namespace {
        if matches!(entry.path.as_str(), "effect.lock" | "export-root-v2.json") {
            if entry.directory || !base.insert(entry.path.as_str()) {
                return Err(invalid());
            }
            continue;
        }
        let first = entry.path.split('/').next().ok_or_else(invalid)?;
        if !path(&entry.path)
            || !crate::registry::v4(first)
            || !snapshot_jobs.contains(&Uuid::parse_str(first).map_err(|_| invalid())?)
        {
            return Err(invalid());
        }
    }
    if base != BTreeSet::from(["effect.lock", "export-root-v2.json"]) {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actual_producer_p6_full_inventory_and_issue_digest_binding() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/snapshot-effects-p6-capture-v2.json"
        ))
        .unwrap();
        let files: BTreeMap<String, Vec<u8>> = fixture["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| {
                let raw = hex::decode(entry["hex"].as_str().unwrap()).unwrap();
                assert_eq!(raw.len() as u64, entry["bytes"].as_u64().unwrap());
                assert_eq!(hash(&raw), entry["sha256"].as_str().unwrap());
                (entry["path"].as_str().unwrap().to_owned(), raw)
            })
            .collect();
        assert_eq!(files.len(), 14);
        let descriptor: Descriptor = serde_json::from_slice(&files["epoch.json"]).unwrap();
        let ledger: Ledger = serde_json::from_slice(&files["ledger.json"]).unwrap();
        let cut: Cut = serde_json::from_slice(&files["cut.json"]).unwrap();
        let rows: JobInventory = serde_json::from_slice(&files["source-rows.json"]).unwrap();
        let namespace: Vec<NamespaceEntry> =
            serde_json::from_slice(&files["namespace.json"]).unwrap();
        let journal_files = files
            .iter()
            .filter(|(path, _)| path.starts_with("journal/"))
            .map(|(path, raw)| (path.clone(), raw.clone()))
            .collect();
        let prefix = Prefix::check(
            &journal_files,
            &files["epoch.json"],
            descriptor.epoch_id,
            descriptor.binding_id,
            &cut.cut_record,
        )
        .unwrap();
        super::cut(
            &descriptor,
            &ledger,
            &cut,
            (
                cut.cut_record.sequence,
                hex::decode(&cut.cut_record.sha256)
                    .unwrap()
                    .try_into()
                    .unwrap(),
            ),
            &files["epoch.json"],
            &files["ledger.json"],
            cut.backup_id,
        )
        .unwrap();
        let budget =
            std::sync::Arc::new(std::sync::Mutex::new(crate::registry::ScanBudget::default()));
        let meter = SharedJournalBudget::default();
        prefix_cut(
            &prefix,
            &cut,
            &files["epoch.json"],
            &files["ledger.json"],
            &budget,
            &meter,
        )
        .unwrap();
        let known = &ledger.jobs[0];
        let attempt = &known.attempts[0];
        let findings = inventory(&rows, &ledger, &prefix, &namespace, &budget, &meter)
            .expect("the original producer's complete P6 capture must verify");
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[&known.job_id],
            ExternalEffectFinding::AlreadyCommitted
        );

        let issue_record = &prefix.records[&attempt.write_issued.as_ref().unwrap().sequence].1;
        let claim_record = &prefix.records[&attempt.claim_record.sequence].1;
        let latest = typed_job(&claim_record["data"]["after_job"]).unwrap();
        let outbox = typed_outbox(&issue_record["rows"]["outbox"]).unwrap();
        // writer.rs builds IssueRecord.input_sha256 from history::encode of the
        // complete typed outbox. The request-key digest separately binds payload.
        let producer_input = hash(&serde_json::to_vec(&outbox).unwrap());
        let payload_input = hash(&serde_json::to_vec(&outbox.payload).unwrap());
        assert_eq!(issue_record["input_sha256"], producer_input);
        assert_eq!(known.source_job_binding_sha256, producer_input);
        assert_ne!(producer_input, payload_input);
        let validate = |record: &Value, observed_outbox: &OutboxObservation| {
            issue(
                record,
                &latest,
                observed_outbox,
                attempt,
                &prefix.records[&1].1,
                &claim_record["data"]["request_id"],
                &prefix,
                &budget,
                &meter,
            )
        };
        assert!(validate(issue_record, &outbox).is_ok());
        let mut forged = issue_record.clone();
        forged["input_sha256"] = payload_input.clone().into();
        assert!(validate(&forged, &outbox).is_err());
        let mut changed = serde_json::to_value(&outbox).unwrap();
        changed["business_key"] = "changed outside the payload".into();
        let changed = typed_outbox(&changed).unwrap();
        assert_eq!(changed.payload, outbox.payload);
        let mut forged = issue_record.clone();
        forged["rows"]["outbox"] = serde_json::to_value(&changed).unwrap();
        assert!(validate(&forged, &changed).is_err());
        forged["input_sha256"] = payload_input.into();
        assert!(validate(&forged, &changed).is_err());
    }
    fn job() -> Value {
        let id = "c13e17fb-4686-4d7d-92da-6e9765c7f2a2";
        serde_json::json!({"id":id,"outbox_id":id,"idempotency_key":"k","status":"snapshot_running",
            "attempt_count":1,"lease_token":"d77872df-6c93-49fa-b23e-3a1469b633d7",
            "lease_expires_at":"2026-10-23T00:01:00Z","next_attempt_at":null,"last_error_class":null,
            "output_digest":null,"checkpoint":null,"created_at":"2026-10-23T00:00:00Z","updated_at":"2026-10-23T00:00:00Z"})
    }
    fn renewal() -> Value {
        let before = job();
        let mut after = before.clone();
        after["updated_at"] = "2026-10-23T00:00:01Z".into();
        after["lease_expires_at"] = "2026-10-23T00:01:01Z".into();
        serde_json::json!({"job_id":before["id"],"attempt":1,"claim_record_sha256":"a".repeat(64),
            "operation":"renew","before_job":before,"after_job":after,"sql_return":true,
            "lease_ms":60000,"error_class":null,"retry":null,
            "sql_before":"2026-10-23T00:00:01Z","sql_after":"2026-10-23T00:00:01Z","identity":{}})
    }
    #[test]
    fn effects_transition_rejects_true_arbitrary_after_and_false_mutation() {
        let data = renewal();
        let before = typed_job(&data["before_job"]).unwrap();
        let token = before.lease_token.unwrap();
        assert!(transition(&data, &before, token, 1).is_ok());
        for (key, value) in [
            ("attempt_count", serde_json::json!(2)),
            ("status", serde_json::json!("succeeded")),
            ("lease_token", serde_json::json!(Uuid::new_v4())),
            ("output_digest", serde_json::json!("a".repeat(64))),
            (
                "lease_expires_at",
                serde_json::json!("2026-10-23T00:05:00Z"),
            ),
        ] {
            let mut bad = data.clone();
            bad["after_job"][key] = value;
            assert!(transition(&bad, &before, token, 1).is_err(), "{key}");
        }
        let mut bad = data.clone();
        bad["sql_return"] = false.into();
        assert!(transition(&bad, &before, token, 1).is_err());
        bad["after_job"] = bad["before_job"].clone();
        assert!(transition(&bad, &before, token, 1).is_ok());
        bad["operation"] = "unknown_operation".into();
        assert!(transition(&bad, &before, token, 1).is_err());
    }
    #[test]
    fn effects_transition_requires_real_failure_shape_and_full_inputs() {
        let mut data = renewal();
        let before = typed_job(&data["before_job"]).unwrap();
        let token = before.lease_token.unwrap();
        data["operation"] = "fail".into();
        data["lease_ms"] = Value::Null;
        data["error_class"] = "transient_storage".into();
        data["retry"] = true.into();
        data["after_job"]["status"] = "snapshot_retry_wait".into();
        data["after_job"]["lease_token"] = Value::Null;
        data["after_job"]["lease_expires_at"] = Value::Null;
        data["after_job"]["last_error_class"] = "transient_storage".into();
        data["after_job"]["next_attempt_at"] = "2026-10-23T00:00:02Z".into();
        assert!(transition(&data, &before, token, 1).is_ok());
        data["after_job"]["next_attempt_at"] = "2026-10-23T00:00:03Z".into();
        assert!(transition(&data, &before, token, 1).is_err());
        data.as_object_mut().unwrap().remove("sql_after");
        assert!(transition(&data, &before, token, 1).is_err());
    }
    #[test]
    fn effects_namespace_rejects_orphan_extra_and_duplicate_root_entries() {
        let base = vec![
            NamespaceEntry {
                path: "effect.lock".into(),
                directory: false,
                size: 0,
            },
            NamespaceEntry {
                path: "export-root-v2.json".into(),
                directory: false,
                size: 10,
            },
        ];
        assert!(namespace_closed(&base, &[], &[]).is_ok());
        for name in [
            "orphan",
            "c13e17fb-4686-4d7d-92da-6e9765c7f2a2",
            "effect.lock/extra",
        ] {
            let mut bad = base.clone();
            bad.push(NamespaceEntry {
                path: name.into(),
                directory: true,
                size: 0,
            });
            bad.sort_by(|a, b| a.path.cmp(&b.path));
            assert!(namespace_closed(&bad, &[], &[]).is_err());
        }
        let mut duplicate = base.clone();
        duplicate.push(base[0].clone());
        duplicate.sort_by(|a, b| a.path.cmp(&b.path));
        assert!(namespace_closed(&duplicate, &[], &[]).is_err());
    }
    fn database_identity() -> Value {
        serde_json::json!({"database":"source","oid":16384,"postmaster":"2026-10-23T00:00:00Z","session_user":"learning_snapshot_claim_broker","current_user":"learning_snapshot_claim_broker","backend":12,"transaction":"42"})
    }
    fn descriptor() -> Value {
        serde_json::json!({"source_database":"source","source_database_oid":16384,"source_postmaster_start":"2026-10-23T00:00:00Z"})
    }
    #[test]
    fn acquired_claim_requires_actual_tx_and_microsecond_lease_bounds() {
        let mut before = job();
        before["status"] = "snapshot_queued".into();
        before["attempt_count"] = 0.into();
        before["lease_token"] = Value::Null;
        before["lease_expires_at"] = Value::Null;
        let after = job();
        let id = database_identity();
        let intent =
            serde_json::json!({"request_id":Uuid::new_v4(),"lease_ms":60000,"identity":id});
        let mut data = serde_json::json!({"job_id":after["id"],"claim_token":after["lease_token"],"lease_expires_at":after["lease_expires_at"],"lease_ms":60000,"identity":id,"sql_before":"2026-10-23T00:00:00Z","sql_after":"2026-10-23T00:00:00Z"});
        let b = typed_job(&before).unwrap();
        let a = typed_job(&after).unwrap();
        assert!(claim_delta(&intent, &data, &b, &a, &descriptor()).is_ok());
        data["identity"]["transaction"] = "43".into();
        assert!(claim_delta(&intent, &data, &b, &a, &descriptor()).is_err());
        data["identity"] = id;
        let mut too_early = after.clone();
        too_early["lease_expires_at"] = "2026-10-23T00:00:59.999999Z".into();
        data["lease_expires_at"] = too_early["lease_expires_at"].clone();
        assert!(
            claim_delta(
                &intent,
                &data,
                &b,
                &typed_job(&too_early).unwrap(),
                &descriptor()
            )
            .is_err()
        );
    }
    #[test]
    fn ceiling_none_never_refunds_or_invents_attempt_four() {
        let mut before = job();
        before["attempt_count"] = 3.into();
        before["lease_expires_at"] = "2026-10-23T00:00:00Z".into();
        let mut after = before.clone();
        after["status"] = "failed".into();
        after["lease_token"] = Value::Null;
        after["lease_expires_at"] = Value::Null;
        after["last_error_class"] = "lease_expired".into();
        after["updated_at"] = "2026-10-23T00:00:01Z".into();
        let request = Uuid::new_v4();
        let identity = database_identity();
        let mut intent = serde_json::json!({"job_id":before["id"],"request_id":request,"previous_attempt":3,"candidate_attempt":3,"before_job":before,"identity":identity,"lease_ms":60000});
        let data = serde_json::json!({"job_id":before["id"],"request_id":request,"reason":"expired_at_ceiling","identity":identity,"sql_before":"2026-10-23T00:00:01Z","sql_after":"2026-10-23T00:00:01Z"});
        let b = typed_job(&before).unwrap();
        let a = typed_job(&after).unwrap();
        assert!(not_acquired(&intent, &data, &b, &a, &descriptor()).is_ok());
        intent["candidate_attempt"] = 4.into();
        assert!(not_acquired(&intent, &data, &b, &a, &descriptor()).is_err());
        intent["candidate_attempt"] = 3.into();
        after["attempt_count"] = 0.into();
        assert!(
            not_acquired(
                &intent,
                &data,
                &b,
                &typed_job(&after).unwrap(),
                &descriptor()
            )
            .is_err()
        );
    }
    #[test]
    fn queued_zero_shape_requires_full_join_and_closed_namespace() {
        let budget =
            std::sync::Arc::new(std::sync::Mutex::new(crate::registry::ScanBudget::default()));
        let meter = SharedJournalBudget::default();
        let mut job = job();
        job["status"] = "snapshot_queued".into();
        job["attempt_count"] = 0.into();
        job["lease_token"] = Value::Null;
        job["lease_expires_at"] = Value::Null;
        let id = uuid(&job["id"]).unwrap();
        let actor = Uuid::new_v4();
        let request = Uuid::new_v4();
        let payload = serde_json::json!({"actor_id":actor});
        let outbox = serde_json::json!({"id":id,"business_key":"k","event_type":"snapshot_export_requested","payload_version":1,"payload":payload,"actor_id":actor,"processor_version":1,"created_at":"2026-10-23T00:00:00Z","dispatched_at":"2026-10-23T00:00:00Z"});
        let mut tables = serde_json::json!({"tables":{"job":[job],"job_outbox":[outbox],"asset_integrity_result":[],"snapshot_export_result":[],"snapshot_export_request":[{"actor_id":actor,"request_id":request,"outbox_id":id,"job_id":id}],"request_key":[{"actor_id":actor,"request_id":request,"operation":"snapshot_export","request_sha256":hash(&serde_json::to_vec(&payload).unwrap())}]}});
        let source_rows: JobInventory = serde_json::from_value(tables.clone()).unwrap();
        let epoch = Uuid::new_v4();
        let binding = Uuid::new_v4();
        let ledger = Ledger {
            format_version: 2,
            capability: "snapshot_effect_known_ledger_v2".into(),
            storage_encoding: ENCODING.into(),
            epoch_id: epoch,
            binding_id: binding,
            jobs: vec![],
        };
        let prefix = Prefix {
            records: BTreeMap::from([
                (1, ("a".repeat(64), serde_json::json!({}))),
                (
                    2,
                    (
                        "b".repeat(64),
                        serde_json::json!({"kind":"CAPTURE_CUT","data":{"ledger_sha256":hash(&super::super::files::typed_bytes(&ledger,&budget,&meter).unwrap()),"active_writers":0,"active_operations":0,"pools_closed":true}}),
                    ),
                ),
            ]),
        };
        let mut namespace = vec![
            NamespaceEntry {
                path: "effect.lock".into(),
                directory: false,
                size: 0,
            },
            NamespaceEntry {
                path: "export-root-v2.json".into(),
                directory: false,
                size: 10,
            },
        ];
        assert_eq!(
            inventory(&source_rows, &ledger, &prefix, &namespace, &budget, &meter).unwrap()[&id],
            ExternalEffectFinding::ConfirmedNoEffect
        );
        namespace.push(NamespaceEntry {
            path: id.to_string(),
            directory: true,
            size: 0,
        });
        namespace.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(
            inventory(&source_rows, &ledger, &prefix, &namespace, &budget, &meter).unwrap()[&id],
            ExternalEffectFinding::Unknown
        );
        tables["tables"]["snapshot_export_request"][0]["job_id"] =
            Uuid::new_v4().to_string().into();
        let orphan: JobInventory = serde_json::from_value(tables).unwrap();
        assert!(inventory(&orphan, &ledger, &prefix, &namespace, &budget, &meter).is_err());
    }
}

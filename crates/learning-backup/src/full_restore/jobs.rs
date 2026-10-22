//! Management-only job normalization. No external-effect authority is inferred
//! from the current registry: its v1 roster contains no snapshot export root.
//! Positive findings require the original captured-and-independently-copied v2 capability.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use crate::{
    BackupError, ExternalEffectFinding, JobRecoveryAction, RestoredJob, classify_restored_job,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Write,
    sync::{Arc, Mutex},
    time::SystemTime,
};
use uuid::Uuid;

#[cfg(target_os = "linux")]
use super::ImportedTarget;
#[cfg(target_os = "linux")]
use learning_assets::backup_fs::BackupDir;
#[cfg(target_os = "linux")]
use std::io::Read;
#[cfg(target_os = "linux")]
use std::{future::Future, pin::Pin};

const MAX_ROWS: usize = 100_000;
const MAX_INVENTORY_BYTES: usize = 64 * 1024 * 1024;
#[cfg(target_os = "linux")]
const MAX_ROW_BYTES: i64 = 1024 * 1024;
const MAX_RECORD_BYTES: u64 = 256 * 1024 * 1024;
type MetadataBudget = Arc<Mutex<crate::registry::ScanBudget>>;
const TABLES: [(&str, &str); 6] = [
    ("asset_integrity_result", "job_id"),
    ("job", "id"),
    ("job_outbox", "id"),
    ("request_key", "actor_id,request_id"),
    ("snapshot_export_request", "actor_id,request_id"),
    ("snapshot_export_result", "job_id"),
];
#[cfg(target_os = "linux")]
const INTENT_FILES: [&str; 5] = [
    "before.json",
    "effects.json",
    "holds.json",
    "plan.json",
    "delta.json",
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JobInventory {
    tables: BTreeMap<String, Vec<Value>>,
}
impl JobInventory {
    pub(crate) fn tables(&self) -> &BTreeMap<String, Vec<Value>> {
        &self.tables
    }
}

/// No caller enum or directory can create positive reconciliation. Only the
/// independently copied v2 capability can populate the private finding map.
pub(crate) struct ExternalReconciliation {
    identity: Value,
    before: JobInventory,
    observations: Value,
    holds: Vec<Value>,
    findings: BTreeMap<Uuid, ExternalEffectFinding>,
    #[cfg(target_os = "linux")]
    copied: Option<crate::destination::snapshot_effects::CopiedSnapshotEffectsV2>,
}

/// A committed normalization delta, never recovery/activation eligibility.
/// Source tokens and complete row images occur only in private sidecar evidence.
pub(crate) struct JobRecoveryReport {
    evidence: Value,
    committed: JobInventory,
    #[cfg(test)]
    held: usize,
    has_holds: bool,
    budget: MetadataBudget,
}

/// Re-read all six job/reference tables after the remaining application checks.
/// Only the original committed report supplies the expected rows.
#[cfg(target_os = "linux")]
pub(crate) async fn verify_committed_rows(
    target: &mut ImportedTarget,
    report: &JobRecoveryReport,
) -> Result<Value, BackupError> {
    target.recheck().await?;
    let current = inventory(target.connection(), &report.budget).await?;
    require_committed_rows(&current, &report.committed)?;
    target.recheck().await?;
    Ok(
        json!({"format_version":1,"capability":"recovery_committed_job_rows_rechecked_v1",
        "tables":TABLES.map(|(name,_)|name),"rows_sha256":digest_charged(&current,Some(&report.budget))?,
        "normalization_report_sha256":report.canonical_sha256()?,"all_columns_compared":true}),
    )
}

fn require_committed_rows(
    current: &JobInventory,
    committed: &JobInventory,
) -> Result<(), BackupError> {
    validate_inventory_budget(current)?;
    validate_inventory_budget(committed)?;
    if current != committed {
        return Err(BackupError::Invalid(
            "recovery normalized rows changed after commit",
        ));
    }
    Ok(())
}

impl JobRecoveryReport {
    pub(crate) fn has_holds(&self) -> bool {
        self.has_holds
    }
    #[cfg(test)]
    pub(crate) fn held_jobs(&self) -> usize {
        self.held
    }
    #[cfg(test)]
    pub(crate) fn evidence(&self) -> &Value {
        &self.evidence
    }
    pub(crate) fn canonical_sha256(&self) -> Result<String, BackupError> {
        digest_charged(&self.evidence, Some(&self.budget))
    }
}

#[derive(Debug, PartialEq, Serialize)]
struct PlannedChange {
    job_id: Uuid,
    status: &'static str,
    retry_at_recovery_time: bool,
    error_class: &'static str,
}

#[cfg(test)]
fn digest(value: &impl Serialize) -> Result<String, BackupError> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(value)?)))
}

fn charge(budget: &MetadataBudget, bytes: u64) -> Result<(), BackupError> {
    budget
        .lock()
        .map_err(|_| BackupError::Invalid("recovery metadata budget poisoned"))?
        .charge(bytes)
}

// Charge serialization as it is produced, before extending the output buffer.
// The same meter is retained by every recovery phase; no local inventory read,
// receipt write, canonical readback or report hash gets a fresh allowance.
fn encode_charged(
    value: &impl Serialize,
    budget: Option<&MetadataBudget>,
) -> Result<Vec<u8>, BackupError> {
    let Some(budget) = budget else {
        return Ok(serde_json::to_vec(value)?);
    };
    struct Output<'a> {
        budget: &'a MetadataBudget,
        bytes: Vec<u8>,
        failure: Option<BackupError>,
    }
    impl Write for Output<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let next = self.bytes.len().checked_add(bytes.len());
            let result = match next {
                Some(next) if next as u64 <= MAX_RECORD_BYTES => {
                    charge(self.budget, bytes.len() as u64)
                }
                _ => Err(BackupError::Capacity("recovery serialized record")),
            };
            if let Err(error) = result {
                self.failure = Some(error);
                return Err(std::io::Error::other("recovery metadata budget"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = Output {
        budget,
        bytes: Vec::new(),
        failure: None,
    };
    let result = serde_json::to_writer(&mut output, value);
    if let Some(error) = output.failure {
        return Err(error);
    }
    result?;
    Ok(output.bytes)
}

fn digest_charged(
    value: &impl Serialize,
    budget: Option<&MetadataBudget>,
) -> Result<String, BackupError> {
    Ok(hex::encode(Sha256::digest(encode_charged(value, budget)?)))
}

// A bounded size walk avoids making an uncharged second serialized copy merely
// to enforce the per-inventory sublimit.
fn inventory_weight(value: &Value) -> Result<usize, BackupError> {
    let add = |a: usize, b: usize| a.checked_add(b).ok_or(BackupError::Overflow);
    let string_size = |s: &str| {
        s.bytes().try_fold(2usize, |n, byte| {
            let length = match byte {
                b'"' | b'\\' | b'\x08' | b'\x0c' | b'\n' | b'\r' | b'\t' => 2,
                0..=31 => 6,
                _ => 1,
            };
            add(n, length)
        })
    };
    match value {
        Value::Null | Value::Bool(_) => Ok(5),
        Value::Number(_) => Ok(32),
        Value::String(s) => string_size(s),
        Value::Array(values) => values
            .iter()
            .try_fold(2usize, |sum, v| add(add(sum, 1)?, inventory_weight(v)?)),
        Value::Object(values) => values.iter().try_fold(2usize, |sum, (k, v)| {
            let key = add(string_size(k)?, 2)?;
            add(add(sum, key)?, inventory_weight(v)?)
        }),
    }
}

fn field<'a>(row: &'a Value, key: &str) -> Result<&'a Value, BackupError> {
    row.get(key)
        .ok_or(BackupError::Invalid("recovery job row field missing"))
}

fn text<'a>(row: &'a Value, key: &str) -> Result<&'a str, BackupError> {
    field(row, key)?
        .as_str()
        .ok_or(BackupError::Invalid("recovery job row text"))
}

fn id(row: &Value, key: &str) -> Result<Uuid, BackupError> {
    let raw = text(row, key)?;
    let value = Uuid::parse_str(raw).map_err(|_| BackupError::Invalid("recovery job row UUID"))?;
    if value.to_string() != raw {
        return Err(BackupError::Invalid("recovery job row UUID spelling"));
    }
    Ok(value)
}

fn rows<'a>(inventory: &'a JobInventory, table: &str) -> Result<&'a [Value], BackupError> {
    inventory
        .tables
        .get(table)
        .map(Vec::as_slice)
        .ok_or(BackupError::Invalid("recovery inventory table missing"))
}

fn indexed<'a>(rows: &'a [Value], key: &str) -> Result<BTreeMap<Uuid, &'a Value>, BackupError> {
    let mut result = BTreeMap::new();
    for row in rows {
        if result.insert(id(row, key)?, row).is_some() {
            return Err(BackupError::Invalid("duplicate recovery row identity"));
        }
    }
    Ok(result)
}

fn validate_inventory_budget(inventory: &JobInventory) -> Result<(), BackupError> {
    if inventory.tables.len() != TABLES.len()
        || TABLES
            .iter()
            .any(|(name, _)| !inventory.tables.contains_key(*name))
    {
        return Err(BackupError::Invalid("recovery inventory exact tables"));
    }
    let count = inventory.tables.values().try_fold(0usize, |n, rows| {
        n.checked_add(rows.len()).ok_or(BackupError::Overflow)
    })?;
    let weight = inventory
        .tables
        .values()
        .flatten()
        .try_fold(0usize, |sum, row| {
            sum.checked_add(inventory_weight(row)?)
                .ok_or(BackupError::Overflow)
        })?;
    if count > MAX_ROWS || weight > MAX_INVENTORY_BYTES {
        return Err(BackupError::Capacity("recovery job inventory"));
    }
    Ok(())
}

fn policy_row(job: &Value, event_type: &str) -> Result<RestoredJob, BackupError> {
    let status = text(job, "status")?;
    let running = matches!(status, "running" | "snapshot_running");
    let retry = matches!(status, "retry_wait" | "snapshot_retry_wait");
    let token = field(job, "lease_token")?;
    let expiry = field(job, "lease_expires_at")?;
    let next = field(job, "next_attempt_at")?;
    if running == token.is_null()
        || running == expiry.is_null()
        || retry == next.is_null()
        || (!expiry.is_null() && !expiry.is_string())
        || (!next.is_null() && !next.is_string())
    {
        return Err(BackupError::Invalid(
            "recovery lease or retry deadline shape",
        ));
    }
    if !token.is_null() {
        id(job, "lease_token")?;
    }
    let attempts = field(job, "attempt_count")?
        .as_i64()
        .and_then(|n| i32::try_from(n).ok())
        .ok_or(BackupError::Invalid("recovery attempt count"))?;
    let checkpoint = field(job, "checkpoint")?;
    if !checkpoint.is_null() && !checkpoint.is_object() {
        return Err(BackupError::Invalid("recovery checkpoint shape"));
    }
    if matches!(status, "queued" | "snapshot_queued")
        && (!field(job, "last_error_class")?.is_null() || !field(job, "output_digest")?.is_null())
    {
        return Err(BackupError::Invalid("recovery queued result or error"));
    }
    Ok(RestoredJob {
        status: status.to_owned(),
        event_type: event_type.to_owned(),
        attempt_count: attempts,
        lease_token_present: !token.is_null(),
        checkpoint_present: !checkpoint.is_null(),
        external_effect: ExternalEffectFinding::Unknown,
    })
}

#[cfg(test)]
fn plan_changes(inventory: &JobInventory) -> Result<Vec<PlannedChange>, BackupError> {
    plan_changes_with_findings(inventory, &BTreeMap::new())
}

fn plan_changes_with_findings(
    inventory: &JobInventory,
    findings: &BTreeMap<Uuid, ExternalEffectFinding>,
) -> Result<Vec<PlannedChange>, BackupError> {
    validate_inventory_budget(inventory)?;
    let jobs = indexed(rows(inventory, "job")?, "id")?;
    let events = indexed(rows(inventory, "job_outbox")?, "id")?;
    if findings.iter().any(|(job, finding)| {
        !jobs.contains_key(job)
            || !matches!(
                finding,
                ExternalEffectFinding::ConfirmedNoEffect | ExternalEffectFinding::AlreadyCommitted
            )
    }) {
        return Err(BackupError::Invalid("positive snapshot finding roster"));
    }
    let mut seen_outbox = std::collections::BTreeSet::new();
    let mut changes = Vec::new();
    for (job_id, job) in jobs {
        let outbox_id = id(job, "outbox_id")?;
        let event = events
            .get(&outbox_id)
            .ok_or(BackupError::Invalid("recovery job outbox missing"))?;
        if !seen_outbox.insert(outbox_id)
            || text(job, "idempotency_key")? != text(event, "business_key")?
            || field(event, "dispatched_at")?.is_null()
        {
            return Err(BackupError::Invalid("recovery job outbox binding"));
        }
        let mut row = policy_row(job, text(event, "event_type")?)?;
        if row.event_type == "snapshot_export_requested" {
            row.external_effect = findings
                .get(&job_id)
                .copied()
                .unwrap_or(ExternalEffectFinding::Unknown);
        } else if findings.contains_key(&job_id) {
            return Err(BackupError::Invalid(
                "snapshot finding attached to another job family",
            ));
        }
        let change = match classify_restored_job(&row)? {
            JobRecoveryAction::RetryWait => Some((
                if row.event_type == "snapshot_export_requested" {
                    "snapshot_retry_wait"
                } else {
                    "retry_wait"
                },
                true,
                "recovery_interrupted",
            )),
            JobRecoveryAction::FailedRetryBudget => Some(("failed", false, "recovery_interrupted")),
            JobRecoveryAction::AwaitExternalReconciliation
                if matches!(
                    row.status.as_str(),
                    "snapshot_queued" | "snapshot_running" | "snapshot_retry_wait"
                ) =>
            {
                Some(("cancelled", false, "recovery_external_unknown"))
            }
            _ => None,
        };
        if let Some((status, retry_at_recovery_time, error_class)) = change {
            changes.push(PlannedChange {
                job_id,
                status,
                retry_at_recovery_time,
                error_class,
            });
        }
    }
    Ok(changes)
}

fn request_identity(row: &Value) -> Result<(Uuid, Uuid), BackupError> {
    Ok((id(row, "actor_id")?, id(row, "request_id")?))
}

// The SQL FKs do not encode the application-level snapshot request joins.
// Inspect every receipt, including those attached to a different event family;
// iterating only snapshot outboxes would silently discard these legal anomalies.
fn reconciliation_join_holds_metered(
    before: &JobInventory,
    budget: Option<&MetadataBudget>,
) -> Result<Vec<Value>, BackupError> {
    let jobs = indexed(rows(before, "job")?, "id")?;
    let events = indexed(rows(before, "job_outbox")?, "id")?;
    let mut keys = BTreeMap::new();
    for row in rows(before, "request_key")? {
        if keys.insert(request_identity(row)?, row).is_some() {
            return Err(BackupError::Invalid("duplicate recovery request key"));
        }
    }
    let mut seen_requests = std::collections::BTreeSet::new();
    let mut holds = Vec::new();
    for request in rows(before, "snapshot_export_request")? {
        let pair = request_identity(request)?;
        if !seen_requests.insert(pair) {
            return Err(BackupError::Invalid("duplicate recovery snapshot request"));
        }
        let outbox_id = id(request, "outbox_id")?;
        let requested_job_id = id(request, "job_id")?;
        let event = events.get(&outbox_id).copied();
        let job = jobs.get(&requested_job_id).copied();
        let key = keys.get(&pair).copied();
        let mut reasons = Vec::new();
        if requested_job_id != outbox_id {
            reasons.push("REQUEST_JOB_ID_DIFFERS_FROM_STABLE_OUTBOX_ID");
        }
        if let Some(event) = event {
            if text(event, "event_type")? != "snapshot_export_requested" {
                reasons.push("REQUEST_OUTBOX_IS_NOT_SNAPSHOT_EVENT");
            }
            if id(event, "actor_id")? != pair.0 {
                reasons.push("REQUEST_ACTOR_DIFFERS_FROM_OUTBOX_ACTOR");
            }
            if let Some(key) = key
                && (text(key, "operation")? != "snapshot_export"
                    || text(key, "request_sha256")?
                        != digest_charged(field(event, "payload")?, budget)?)
            {
                reasons.push("REQUEST_KEY_OPERATION_OR_PAYLOAD_DIGEST_DIFFERS");
            }
            if job.is_none() && !field(event, "dispatched_at")?.is_null() {
                reasons.push("DISPATCHED_REQUEST_JOB_MISSING");
            }
        } else {
            reasons.push("REQUEST_OUTBOX_MISSING");
        }
        if key.is_none() {
            reasons.push("REQUEST_KEY_MISSING");
        }
        if let Some(job) = job
            && id(job, "outbox_id")? != outbox_id
        {
            reasons.push("REQUEST_JOB_BELONGS_TO_DIFFERENT_OUTBOX");
        }
        if !reasons.is_empty() {
            holds.push(json!({
                "finding":"UNKNOWN","reason":"SNAPSHOT_REQUEST_JOIN_CONFLICT","details":reasons,
                "outbox_id":outbox_id,"job_id":job.map(|j| id(j,"id")).transpose()?,
                "imported_job":job,"imported_outbox":event,
                "imported_request":request,"imported_request_key":key,
                "external_bytes_verified":false,"replay_authorized":false
            }));
        }
    }
    for (pair, key) in keys {
        if text(key, "operation")? == "snapshot_export" && !seen_requests.contains(&pair) {
            holds.push(json!({
                "finding":"UNKNOWN","reason":"ORPHAN_SNAPSHOT_REQUEST_KEY",
                "job_id":null,"imported_request_key":key,
                "external_bytes_verified":false,"replay_authorized":false
            }));
        }
    }
    for result in rows(before, "snapshot_export_result")? {
        let result_job_id = id(result, "job_id")?;
        let job = jobs.get(&result_job_id).copied();
        let event = job
            .map(|j| id(j, "outbox_id"))
            .transpose()?
            .and_then(|outbox_id| events.get(&outbox_id).copied());
        let coherent = match (job, event) {
            (Some(job), Some(event)) => {
                text(event, "event_type")? == "snapshot_export_requested"
                    && result_job_id == id(event, "id")?
                    && text(job, "status")? == "succeeded"
                    && field(job, "output_digest")? == field(result, "manifest_sha256")?
            }
            _ => false,
        };
        if !coherent {
            holds.push(json!({
                "finding":"UNKNOWN","reason":"SNAPSHOT_RESULT_JOIN_CONFLICT",
                "job_id":job.map(|j| id(j,"id")).transpose()?,
                "referenced_job_id":result_job_id,"imported_job":job,
                "imported_outbox":event,"imported_result":result,
                "external_bytes_verified":false,"replay_authorized":false
            }));
        }
    }
    Ok(holds)
}

#[cfg(test)]
fn reconciliation_join_holds(before: &JobInventory) -> Result<Vec<Value>, BackupError> {
    reconciliation_join_holds_metered(before, None)
}

#[cfg(test)]
fn unknown_effects(
    identity: Value,
    before: JobInventory,
    registry_observation: Value,
) -> Result<ExternalReconciliation, BackupError> {
    unknown_effects_metered(identity, before, registry_observation, None)
}

fn unknown_effects_metered(
    identity: Value,
    before: JobInventory,
    registry_observation: Value,
    budget: Option<&MetadataBudget>,
) -> Result<ExternalReconciliation, BackupError> {
    assemble_effects(
        identity,
        before,
        registry_observation,
        budget,
        BTreeMap::new(),
        None,
    )
}

// Private assembly is also exercised by policy tests. Production positive maps
// originate only in inspect_copied_external_effects, never in a caller payload.
fn assemble_effects(
    identity: Value,
    before: JobInventory,
    registry_observation: Value,
    budget: Option<&MetadataBudget>,
    findings: BTreeMap<Uuid, ExternalEffectFinding>,
    captured_evidence: Option<Value>,
) -> Result<ExternalReconciliation, BackupError> {
    plan_changes_with_findings(&before, &findings)?;
    let mut jobs = BTreeMap::new();
    for job in rows(&before, "job")? {
        jobs.insert(id(job, "outbox_id")?, job);
    }
    let mut requests_by_event: BTreeMap<Uuid, Vec<Value>> = BTreeMap::new();
    for request in rows(&before, "snapshot_export_request")? {
        requests_by_event
            .entry(id(request, "outbox_id")?)
            .or_default()
            .push(request.clone());
    }
    let mut results_by_job: BTreeMap<Uuid, Vec<Value>> = BTreeMap::new();
    for result in rows(&before, "snapshot_export_result")? {
        results_by_job
            .entry(id(result, "job_id")?)
            .or_default()
            .push(result.clone());
    }
    let mut holds = reconciliation_join_holds_metered(&before, budget)?;
    if !findings.is_empty() && !holds.is_empty() {
        // A wrong request may name a different job while sharing this outbox;
        // checking only hold.job_id would miss that conflict. No positive
        // normalization begins until the whole imported join roster closes.
        return Err(BackupError::Invalid(
            "positive finding conflicts with imported join",
        ));
    }
    for event in rows(&before, "job_outbox")? {
        if text(event, "event_type")? != "snapshot_export_requested" {
            continue;
        }
        let outbox_id = id(event, "id")?;
        let job = jobs.get(&outbox_id).copied();
        let job_id = job.map(|job| id(job, "id")).transpose()?;
        let requests = requests_by_event.remove(&outbox_id).unwrap_or_default();
        let results = job_id
            .and_then(|value| results_by_job.remove(&value))
            .unwrap_or_default();
        if job_id.is_some_and(|id| findings.contains_key(&id)) {
            continue;
        }
        holds.push(json!({
            "finding":"UNKNOWN", "reason":if captured_evidence.is_some() {"SNAPSHOT_V2_EVIDENCE_INCOMPLETE"} else {"SNAPSHOT_EXPORT_ROOT_NOT_ENROLLED_IN_REGISTRY_V1"},
            "outbox_id":outbox_id, "job_id":job_id,
            "imported_job":job, "imported_outbox":event,
            "imported_requests":requests, "imported_results":results,
            "external_bytes_verified":false, "replay_authorized":false
        }));
    }
    let observations = json!({
        "format_version":1, "capability":"recovery_external_effect_observation_v1",
        "identity":identity, "before_sha256":digest_charged(&before, budget)?,
        "registry_observation":registry_observation,
        "export_authority_available":captured_evidence.is_some(),
        "positive_snapshot_findings":findings.len(),
        "snapshot_effects_are_unknown":findings.is_empty(),
        "snapshot_capture":captured_evidence,
        "hold_count":holds.len()
    });
    let mut observations = observations;
    if findings.is_empty() && observations["snapshot_capture"].is_null() {
        observations["external_artifacts_opened"] = json!(0);
    }
    Ok(ExternalReconciliation {
        identity,
        before,
        observations,
        holds,
        findings,
        #[cfg(target_os = "linux")]
        copied: None,
    })
}

fn recovery_micros(time: SystemTime) -> Result<i64, BackupError> {
    let duration = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|_| BackupError::Invalid("recovery time before epoch"))?;
    i64::try_from(duration.as_micros()).map_err(|_| BackupError::Overflow)
}

fn compare_delta(
    before: &JobInventory,
    after: &JobInventory,
    changes: &[PlannedChange],
    recovery_timestamp: &Value,
) -> Result<Vec<Value>, BackupError> {
    validate_inventory_budget(after)?;
    for (name, _) in TABLES {
        if name != "job" && rows(before, name)? != rows(after, name)? {
            return Err(BackupError::Invalid("recovery changed non-job history"));
        }
    }
    let old = indexed(rows(before, "job")?, "id")?;
    let new = indexed(rows(after, "job")?, "id")?;
    if old.keys().ne(new.keys()) {
        return Err(BackupError::Invalid("recovery job identity set changed"));
    }
    let mut deltas = Vec::new();
    let planned: BTreeMap<_, _> = changes
        .iter()
        .map(|change| (change.job_id, change))
        .collect();
    if planned.len() != changes.len() {
        return Err(BackupError::Invalid("duplicate recovery planned job"));
    }
    for (job_id, before_row) in old {
        let after_row = new[&job_id];
        let Some(change) = planned.get(&job_id) else {
            if before_row != after_row {
                return Err(BackupError::Invalid("recovery changed untouched job"));
            }
            continue;
        };
        let mut expected = before_row.clone();
        let object = expected
            .as_object_mut()
            .ok_or(BackupError::Invalid("recovery job object"))?;
        object.insert("status".into(), json!(change.status));
        object.insert("lease_token".into(), Value::Null);
        object.insert("lease_expires_at".into(), Value::Null);
        object.insert(
            "next_attempt_at".into(),
            if change.retry_at_recovery_time {
                recovery_timestamp.clone()
            } else {
                Value::Null
            },
        );
        object.insert("last_error_class".into(), json!(change.error_class));
        // The unchanged business trigger sets this real DB timestamp. It is
        // recorded, never rewritten to the injected recovery clock.
        if !field(after_row, "updated_at")?.is_string() {
            return Err(BackupError::Invalid("recovery trigger timestamp"));
        }
        object.insert("updated_at".into(), field(after_row, "updated_at")?.clone());
        if &expected != after_row {
            return Err(BackupError::Invalid(
                "recovery delta exceeds allowed fields",
            ));
        }
        let changed_fields: Vec<_> = before_row
            .as_object()
            .unwrap()
            .iter()
            .filter_map(|(k, v)| (after_row.get(k) != Some(v)).then_some(k))
            .collect();
        deltas.push(json!({"job_id":job_id,"before":before_row,"after":after_row,"changed_fields":changed_fields}));
    }
    if deltas.len() != changes.len() {
        return Err(BackupError::Invalid("recovery planned delta count"));
    }
    Ok(deltas)
}

#[cfg(target_os = "linux")]
pub(crate) async fn inventory(
    conn: &mut sqlx::PgConnection,
    budget: &MetadataBudget,
) -> Result<JobInventory, BackupError> {
    // Stable timestamp JSON across the original guard and independent top-level
    // transaction. Only this held session's transaction-local setting changes.
    sqlx::query("SELECT pg_catalog.set_config('TimeZone','UTC',true)")
        .execute(&mut *conn)
        .await?;
    let mut tables = BTreeMap::new();
    let mut total = 0usize;
    let mut bytes = 0usize;
    for (name, order) in TABLES {
        let mut values = Vec::new();
        loop {
            // Fixed identifiers only; 64 bounded rows per read prevents an
            // oversized checkpoint/payload from allocating an unbounded batch.
            let sql = format!(
                "SELECT CASE WHEN octet_length(to_jsonb(t)::text)<=$3 THEN to_jsonb(t) ELSE NULL END FROM public.{name} t ORDER BY {order} LIMIT $1 OFFSET $2"
            );
            let page: Vec<Option<Value>> = sqlx::query_scalar(&sql)
                .bind(64i64)
                .bind(i64::try_from(values.len()).map_err(|_| BackupError::Overflow)?)
                .bind(MAX_ROW_BYTES)
                .fetch_all(&mut *conn)
                .await?;
            let count = page.len();
            for value in page {
                let value = value.ok_or(BackupError::Capacity("recovery job row bytes"))?;
                total = total.checked_add(1).ok_or(BackupError::Overflow)?;
                bytes = bytes
                    .checked_add(encode_charged(&value, Some(budget))?.len())
                    .ok_or(BackupError::Overflow)?;
                if total > MAX_ROWS || bytes > MAX_INVENTORY_BYTES {
                    return Err(BackupError::Capacity("recovery job inventory"));
                }
                values.push(value);
            }
            if count < 64 {
                break;
            }
        }
        tables.insert(name.to_owned(), values);
    }
    let result = JobInventory { tables };
    validate_inventory_budget(&result)?;
    Ok(result)
}

/// The existing registry is rechecked, but its lack of export enrollment never
/// supplies positive external-effect authority.
#[cfg(target_os = "linux")]
pub(crate) fn inspect_external_effects<'a>(
    target: &'a mut ImportedTarget,
    registry: &'a crate::RegistryLease,
) -> Pin<Box<dyn Future<Output = Result<ExternalReconciliation, BackupError>> + Send + 'a>> {
    Box::pin(async move {
        let result = async {
            let budget = target.recovery_metadata_budget();
            registry.recheck()?;
            target.recheck().await?;
            let before = inventory(target.connection(), &budget).await?;
            registry.recheck()?;
            target.recheck().await?;
            unknown_effects_metered(
                target.recovery_identity(),
                before,
                json!({
                    "generation":registry.generation().generation,
                    "generation_sha256":digest_charged(registry.generation(),Some(&budget))?,
                    "snapshot_export_root_enrolled":false
                }),
                Some(&budget),
            )
        }
        .await;
        isolate_error(target, result).await
    })
}

/// A narrower conservative adapter for a fresh local rehearsal. It accepts no
/// claimed finding or root, and cannot prove NoEffect or AlreadyCommitted.
#[cfg(all(test, target_os = "linux"))]
pub(crate) async fn inspect_external_effects_without_export_authority(
    target: &mut ImportedTarget,
) -> Result<ExternalReconciliation, BackupError> {
    let result = async {
        let budget = target.recovery_metadata_budget();
        target.recheck().await?;
        let before = inventory(target.connection(), &budget).await?;
        target.recheck().await?;
        unknown_effects_metered(
            target.recovery_identity(),
            before,
            json!({"available":false}),
            Some(&budget),
        )
    }
    .await;
    isolate_error(target, result).await
}

/// Bind the independently copied, frozen v2 evidence to the actual imported
/// six-table projection. A current registry/path/receipt is never a substitute.
#[cfg(target_os = "linux")]
pub(crate) fn inspect_copied_external_effects<'a>(
    target: &'a mut ImportedTarget,
    registry: &'a crate::RegistryLease,
    copied: crate::destination::snapshot_effects::CopiedSnapshotEffectsV2,
) -> Pin<Box<dyn Future<Output = Result<ExternalReconciliation, BackupError>> + Send + 'a>> {
    Box::pin(async move {
        let result = async {
            let budget = target.recovery_metadata_budget();
            registry.recheck()?;
            target.recheck().await?;
            let before = inventory(target.connection(), &budget).await?;
            let identity = target.recovery_identity();
            let verified = copied.reconcile_with_budget(&identity, &before, &budget)?;
            let mut findings = BTreeMap::new();
            for row in rows(&before, "job")? {
                let job_id = id(row, "id")?;
                let finding = verified.finding(job_id);
                if matches!(
                    finding,
                    ExternalEffectFinding::ConfirmedNoEffect
                        | ExternalEffectFinding::AlreadyCommitted
                ) {
                    findings.insert(job_id, finding);
                }
            }
            let observation = verified.evidence().clone();
            charge(&budget, serde_json::to_vec(&observation)?.len() as u64)?;
            registry.recheck()?;
            target.recheck().await?;
            let mut effects = assemble_effects(
                identity,
                before,
                json!({"generation":registry.generation().generation,
                    "generation_sha256":digest_charged(registry.generation(),Some(&budget))?,
                    "snapshot_export_root_enrolled":true}),
                Some(&budget),
                findings,
                Some(observation),
            )?;
            effects.copied = Some(copied);
            Ok(effects)
        }
        .await;
        isolate_error(target, result).await
    })
}

/// Real application delivery against the original held management transaction.
/// Every file comes from the independently copied destination-only package. The
/// private authorization and iterator keep grant locks alive for all byte reads.
#[cfg(target_os = "linux")]
pub(crate) async fn verify_copied_deliveries(
    target: &mut ImportedTarget,
    effects: &ExternalReconciliation,
) -> Result<Value, BackupError> {
    let budget = target.recovery_metadata_budget();
    if effects.identity != target.recovery_identity() {
        return Err(BackupError::Invalid("snapshot delivery target changed"));
    }
    let Some(copied) = &effects.copied else {
        if !effects.findings.is_empty() {
            return Err(BackupError::Invalid(
                "positive finding lost copied authority",
            ));
        }
        return Ok(
            json!({"count":0,"files":0,"bytes":0,"execution":"NO_POSITIVE_SNAPSHOT_FINDING"}),
        );
    };
    let deadline = target.recovery_phase_deadline(std::time::Duration::from_secs(60));
    let identity = target.recovery_identity();
    let delivery_root = target.recovery_control()?.create_dir("snapshot-delivery")?;
    let restored =
        tokio::task::block_in_place(|| copied.restore_deliveries(&identity, &delivery_root))?;
    if restored.target_identity() != &identity || std::time::Instant::now() >= deadline {
        return Err(BackupError::Invalid(
            "snapshot restored delivery target or deadline differs",
        ));
    }
    target.recheck().await?;
    let mut delivered = Vec::new();
    for (job_id, finding) in &effects.findings {
        if *finding != ExternalEffectFinding::AlreadyCommitted {
            continue;
        }
        target.recheck().await?;
        let job = rows(&effects.before, "job")?
            .iter()
            .find(|row| id(row, "id").ok() == Some(*job_id))
            .ok_or(BackupError::Invalid("snapshot delivery job absent"))?;
        let outbox_id = id(job, "outbox_id")?;
        let event = rows(&effects.before, "job_outbox")?
            .iter()
            .find(|row| id(row, "id").ok() == Some(outbox_id))
            .ok_or(BackupError::Invalid("snapshot delivery event absent"))?;
        let actor = learning_core::Principal {
            actor_id: id(event, "actor_id")?,
        };
        let authorization = learning_db::SnapshotStore::authorize_recovery_delivery(
            target.recovery_reading_transaction(),
            actor,
            *job_id,
        )
        .await
        .map_err(|_| BackupError::Invalid("held snapshot delivery authorization failed"))?;
        let token = authorization.stage_token();
        let manifest = authorization.manifest_sha256().to_owned();
        let receipt = rows(&effects.before, "snapshot_export_result")?
            .iter()
            .find(|row| id(row, "job_id").ok() == Some(*job_id))
            .ok_or(BackupError::Invalid("snapshot delivery receipt absent"))?;
        if id(receipt, "stage_token")? != token || text(receipt, "manifest_sha256")? != manifest {
            return Err(BackupError::Invalid(
                "snapshot delivery winning receipt differs",
            ));
        }
        let files = restored.with_package(*job_id, |package| {
            authorization
                .open_files(package)
                .map_err(|_| BackupError::Invalid("snapshot copied delivery package differs"))
        })?;
        let file_evidence = tokio::task::block_in_place(|| {
            let mut records = Vec::new();
            let mut buffer = [0_u8; 65536];
            for file in files {
                let (name, mut file) = file.map_err(|_| {
                    BackupError::Invalid("snapshot copied file verification failed")
                })?;
                let expected_size = file.metadata()?.len();
                charge(
                    &budget,
                    expected_size.checked_add(1).ok_or(BackupError::Overflow)?,
                )?;
                let mut hash = Sha256::new();
                let mut bytes = 0_u64;
                loop {
                    if std::time::Instant::now() >= deadline {
                        return Err(BackupError::Capacity("snapshot delivery deadline"));
                    }
                    let count = file.read(&mut buffer)?;
                    if std::time::Instant::now() >= deadline {
                        return Err(BackupError::Capacity("snapshot delivery deadline"));
                    }
                    if count == 0 {
                        break;
                    }
                    bytes = bytes
                        .checked_add(count as u64)
                        .ok_or(BackupError::Overflow)?;
                    hash.update(&buffer[..count]);
                }
                if std::time::Instant::now() >= deadline {
                    return Err(BackupError::Capacity("snapshot delivery deadline"));
                }
                if bytes != expected_size {
                    return Err(BackupError::Invalid(
                        "snapshot sealed delivery size changed",
                    ));
                }
                records
                    .push(json!({"path":name,"bytes":bytes,"sha256":hex::encode(hash.finalize())}));
            }
            if records.is_empty() {
                return Err(BackupError::Invalid("snapshot delivered no package files"));
            }
            Ok::<_, BackupError>(records)
        })?;
        // No borrow of the original transaction survives the completed reader.
        if std::time::Instant::now() >= deadline {
            return Err(BackupError::Capacity("snapshot delivery deadline"));
        }
        target.recheck().await?;
        if std::time::Instant::now() >= deadline {
            return Err(BackupError::Capacity("snapshot delivery deadline"));
        }
        delivered.push(json!({"job_id":job_id,"stage_token":token,"manifest_sha256":manifest,"files":file_evidence}));
    }
    let evidence = json!({"count":delivered.len(),"deliveries":delivered,
        "execution":"REAL_SNAPSHOT_DELIVERY_CORE_ON_HELD_MANAGEMENT_TRANSACTION_RECOVERY_CONTROL_COPY",
        "recovery_copy_receipt_sha256":restored.receipt_sha256(),
        "source_access":false,"runtime_reconnected":false,"republished":false});
    charge(&budget, serde_json::to_vec(&evidence)?.len() as u64)?;
    if std::time::Instant::now() >= deadline {
        return Err(BackupError::Capacity("snapshot delivery deadline"));
    }
    Ok(evidence)
}

#[cfg(target_os = "linux")]
async fn isolate_error<T>(
    target: &mut ImportedTarget,
    result: Result<T, BackupError>,
) -> Result<T, BackupError> {
    if result.is_err() && target.quarantine().await.is_err() {
        return Err(BackupError::Invalid("recovery jobs isolation unconfirmed"));
    }
    result
}

#[cfg(target_os = "linux")]
fn persist(
    dir: &BackupDir,
    name: &str,
    value: &impl Serialize,
    budget: &MetadataBudget,
) -> Result<Value, BackupError> {
    let bytes = encode_charged(value, Some(budget))?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err(BackupError::Capacity("recovery job evidence bytes"));
    }
    charge(budget, bytes.len() as u64)?;
    let mut file = dir.create_file(name)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    dir.sync()?;
    Ok(json!({"size":bytes.len(),"sha256":hex::encode(Sha256::digest(&bytes))}))
}

/// Called by the bound transaction adapter immediately before its own durable
/// commit-intent. The argument is the held normalization directory, not its parent.
#[cfg(target_os = "linux")]
pub(crate) fn verify_normalization_intent(
    root: &BackupDir,
    identity: &Value,
    budget: &MetadataBudget,
) -> Result<String, BackupError> {
    fn read(root: &BackupDir, name: &str, budget: &MetadataBudget) -> Result<Vec<u8>, BackupError> {
        let mut file = root.open_file(name)?;
        let size = file.metadata()?.len();
        if size > MAX_RECORD_BYTES {
            return Err(BackupError::Capacity("recovery job intent bytes"));
        }
        charge(budget, size.checked_add(1).ok_or(BackupError::Overflow)?)?;
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(size + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 != size {
            return Err(BackupError::Invalid("recovery job evidence changed"));
        }
        Ok(bytes)
    }
    let bytes = read(root, "commit-intent.json", budget)?;
    let intent: Value = serde_json::from_slice(&bytes)?;
    if encode_charged(&intent, Some(budget))? != bytes
        || intent.as_object().map(|value| value.len()) != Some(4)
        || intent.get("format_version") != Some(&json!(1))
        || intent.get("capability") != Some(&json!("recovery_jobs_commit_intent_v1"))
        || intent.get("identity") != Some(identity)
        || intent
            .get("files")
            .and_then(Value::as_object)
            .map(|m| m.len())
            != Some(INTENT_FILES.len())
    {
        return Err(BackupError::Invalid("recovery job commit intent binding"));
    }
    for name in INTENT_FILES {
        let content = read(root, name, budget)?;
        let parsed: Value = serde_json::from_slice(&content)?;
        if encode_charged(&parsed, Some(budget))? != content
            || intent["files"].get(name)
                != Some(
                    &json!({"size":content.len(),"sha256":hex::encode(Sha256::digest(&content))}),
                )
        {
            return Err(BackupError::Invalid("recovery job intent file mismatch"));
        }
    }
    Ok(hex::encode(Sha256::digest(&bytes)))
}

#[cfg(target_os = "linux")]
pub(crate) fn normalize_restored_jobs<'a>(
    target: &'a mut ImportedTarget,
    effects: &'a ExternalReconciliation,
    recovery_time: SystemTime,
) -> Pin<Box<dyn Future<Output = Result<JobRecoveryReport, BackupError>> + Send + 'a>> {
    Box::pin(async move {
        let result = normalize_inner(target, effects, recovery_time).await;
        isolate_error(target, result).await
    })
}

#[cfg(target_os = "linux")]
async fn normalize_inner(
    target: &mut ImportedTarget,
    effects: &ExternalReconciliation,
    recovery_time: SystemTime,
) -> Result<JobRecoveryReport, BackupError> {
    target.recheck().await?;
    let budget = target.recovery_metadata_budget();
    let identity = target.recovery_identity();
    if identity != effects.identity {
        return Err(BackupError::Invalid("recovery effect target differs"));
    }
    let micros = recovery_micros(recovery_time)?;
    let changes = plan_changes_with_findings(&effects.before, &effects.findings)?;
    let root = target.recovery_control()?;
    // create-exclusive directory is the irreversible no-replay fence, including
    // partial writes and a crash before any SQL. No existing directory is repaired.
    let dir = root.create_dir("normalization")?;
    root.sync()?;
    let mut refs = serde_json::Map::new();
    refs.insert(
        "before.json".into(),
        persist(&dir, "before.json", &effects.before, &budget)?,
    );
    refs.insert(
        "effects.json".into(),
        persist(&dir, "effects.json", &effects.observations, &budget)?,
    );
    refs.insert(
        "holds.json".into(),
        persist(&dir, "holds.json", &effects.holds, &budget)?,
    );
    refs.insert(
        "plan.json".into(),
        persist(
            &dir,
            "plan.json",
            &json!({
                "identity":identity,"recovery_epoch_micros":micros,"changes":changes,
                "allowed_trigger_change":"updated_at", "attempt_counts_reset":false,
                "checkpoint_or_output_digest_changes_authorized":false
            }),
            &budget,
        )?,
    );
    target.recheck().await?;
    let mut transaction = target.begin_recovery_transaction().await?;
    sqlx::query("LOCK TABLE public.asset_integrity_result,public.job,public.job_outbox,public.request_key,public.snapshot_export_request,public.snapshot_export_result IN SHARE ROW EXCLUSIVE MODE")
        .execute(transaction.connection()).await?;
    let locked_before = inventory(transaction.connection(), &budget).await?;
    if locked_before != effects.before {
        return Err(BackupError::Invalid(
            "recovery jobs changed after inspection",
        ));
    }
    let timestamp: Value = sqlx::query_scalar(
        "SELECT to_jsonb(TIMESTAMPTZ 'epoch' + ($1::bigint::text || ' microseconds')::interval)",
    )
    .bind(micros)
    .fetch_one(transaction.connection())
    .await?;
    for change in &changes {
        let changed = sqlx::query("UPDATE public.job SET status=$2,lease_token=NULL,lease_expires_at=NULL,next_attempt_at=CASE WHEN $3 THEN TIMESTAMPTZ 'epoch' + ($4::bigint::text || ' microseconds')::interval ELSE NULL END,last_error_class=$5 WHERE id=$1")
            .bind(change.job_id).bind(change.status).bind(change.retry_at_recovery_time)
            .bind(micros).bind(change.error_class).execute(transaction.connection()).await?;
        if changed.rows_affected() != 1 {
            return Err(BackupError::Invalid("recovery job update count"));
        }
    }
    let after = inventory(transaction.connection(), &budget).await?;
    let deltas = compare_delta(&effects.before, &after, &changes, &timestamp)?;
    refs.insert(
        "delta.json".into(),
        persist(
            &dir,
            "delta.json",
            &json!({"after":after,"applied_delta":deltas}),
            &budget,
        )?,
    );
    persist(
        &dir,
        "commit-intent.json",
        &json!({"format_version":1,"capability":"recovery_jobs_commit_intent_v1","identity":identity,"files":refs}),
        &budget,
    )?;
    verify_normalization_intent(&dir, &identity, &budget)?;
    transaction.commit(target).await?;
    target.recheck().await?;
    let committed = inventory(target.connection(), &budget).await?;
    if committed != after {
        return Err(BackupError::Invalid("recovery committed readback differs"));
    }
    let held = effects
        .holds
        .iter()
        .filter_map(|row| row["job_id"].as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let evidence = json!({
        "format_version":1,"capability":"recovery_job_normalization_v1",
        "status":"COMMITTED_NORMALIZATION_NOT_RECOVERY_ELIGIBILITY",
        "identity":identity,"before":effects.before,"after":committed,
        "recovery_epoch_micros":micros,"planned_delta":changes,"applied_delta":deltas,
        "holds":effects.holds,"held_jobs":held,"has_holds":!effects.holds.is_empty(),
        "external_snapshot_bytes_verified":effects.findings.values().any(|finding| *finding == ExternalEffectFinding::AlreadyCommitted),
        "positive_snapshot_findings":effects.findings.len(),"normalization_commit_known":true,
        "native_old_token_rejection_verified":false,"usable":false
    });
    persist(&dir, "committed.json", &evidence, &budget)?;
    Ok(JobRecoveryReport {
        evidence,
        committed,
        #[cfg(test)]
        held,
        has_holds: !effects.holds.is_empty(),
        budget,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_committed_rows_reject_any_job_column_change_or_reference_table_residue() {
        let committed = fixture("snapshot_retry_wait", true, 2);
        require_committed_rows(&committed, &committed).unwrap();
        for field in [
            "attempts",
            "updated_at",
            "status",
            "claim_token",
            "last_error_class",
        ] {
            let mut changed = committed.clone();
            changed.tables.get_mut("job").unwrap()[0][field] = json!("changed");
            assert!(
                require_committed_rows(&changed, &committed).is_err(),
                "{field}"
            );
        }
        for (name, _) in TABLES {
            if name == "job" {
                continue;
            }
            let mut changed = committed.clone();
            changed
                .tables
                .get_mut(name)
                .unwrap()
                .push(json!({"unexpected":"row"}));
            assert!(
                require_committed_rows(&changed, &committed).is_err(),
                "{name}"
            );
        }
    }

    fn fixture(status: &str, snapshot: bool, attempts: i32) -> JobInventory {
        let job_id = Uuid::from_u128(1);
        let token = Uuid::from_u128(2);
        let running = matches!(status, "running" | "snapshot_running");
        let retry = matches!(status, "retry_wait" | "snapshot_retry_wait");
        let mut tables = TABLES
            .iter()
            .map(|(name, _)| (name.to_string(), vec![]))
            .collect::<BTreeMap<_, _>>();
        tables.insert("job".into(),vec![json!({"id":job_id,"outbox_id":job_id,"idempotency_key":"fixed-key","status":status,"attempt_count":attempts,"lease_token":if running {json!(token)} else {Value::Null},"lease_expires_at":if running {json!("2030-01-01T00:00:00+00:00")} else {Value::Null},"next_attempt_at":if retry {json!("2020-01-01T00:00:00+00:00")} else {Value::Null},"last_error_class":null,"output_digest":null,"checkpoint":null,"created_at":"2020-01-01T00:00:00+00:00","updated_at":"2020-01-01T00:00:00+00:00"})]);
        tables.insert("job_outbox".into(),vec![json!({"id":job_id,"business_key":"fixed-key","event_type":if snapshot {"snapshot_export_requested"} else {"asset_integrity_requested"},"dispatched_at":"2020-01-01T00:00:00+00:00"})]);
        JobInventory { tables }
    }

    #[test]
    fn planner_keeps_attempt_budget_and_never_retries_snapshot_unknown() {
        for n in 1..=3 {
            let c2 = fixture("running", false, n);
            let plan = plan_changes(&c2).unwrap();
            assert_eq!(plan[0].status, if n < 3 { "retry_wait" } else { "failed" });
            assert_eq!(c2.tables["job"][0]["attempt_count"], n);
            let c3 = fixture("snapshot_running", true, n);
            let plan = plan_changes(&c3).unwrap();
            assert_eq!(plan[0].status, "cancelled");
            assert!(!plan[0].retry_at_recovery_time);
        }
    }

    #[test]
    fn private_planner_consumes_findings_without_refunding_attempts_or_republishing() {
        // Policy-only inputs, deliberately not evidence/authority constructors.
        // The live adapter must obtain this map from an opaque verified copy.
        let job = Uuid::from_u128(1);
        for attempt in 1..=3 {
            let before = fixture("snapshot_running", true, attempt);
            let findings = BTreeMap::from([(job, ExternalEffectFinding::ConfirmedNoEffect)]);
            let changes = plan_changes_with_findings(&before, &findings).unwrap();
            assert_eq!(changes.len(), 1);
            assert_eq!(
                changes[0].status,
                if attempt < 3 {
                    "snapshot_retry_wait"
                } else {
                    "failed"
                }
            );
            assert_eq!(changes[0].retry_at_recovery_time, attempt < 3);
            assert_eq!(before.tables["job"][0]["attempt_count"], attempt);
        }
        let mut terminal = fixture("succeeded", true, 1);
        terminal.tables.get_mut("job").unwrap()[0]["output_digest"] = json!("a".repeat(64));
        let findings = BTreeMap::from([(job, ExternalEffectFinding::AlreadyCommitted)]);
        assert!(
            plan_changes_with_findings(&terminal, &findings)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn private_planner_rejects_foreign_job_family_or_unrecognized_finding_roster() {
        let valid =
            BTreeMap::from([(Uuid::from_u128(1), ExternalEffectFinding::ConfirmedNoEffect)]);
        assert!(plan_changes_with_findings(&fixture("running", false, 1), &valid).is_err());
        let foreign = BTreeMap::from([(
            Uuid::from_u128(99),
            ExternalEffectFinding::ConfirmedNoEffect,
        )]);
        assert!(
            plan_changes_with_findings(&fixture("snapshot_running", true, 1), &foreign).is_err()
        );
        let conflict = BTreeMap::from([(Uuid::from_u128(1), ExternalEffectFinding::Conflict)]);
        assert!(
            plan_changes_with_findings(&fixture("snapshot_running", true, 1), &conflict).is_err()
        );
    }

    #[test]
    fn all_snapshot_rows_hold_even_success_receipt_and_terminal_rows_are_untouched() {
        for (state, n) in [
            ("snapshot_queued", 0),
            ("snapshot_running", 1),
            ("snapshot_retry_wait", 2),
            ("succeeded", 1),
            ("failed", 3),
            ("cancelled", 0),
        ] {
            let mut inv = fixture(state, true, n);
            if state == "succeeded" {
                inv.tables.get_mut("job").unwrap()[0]["output_digest"] = json!("a".repeat(64));
                inv.tables
                    .get_mut("snapshot_export_result")
                    .unwrap()
                    .push(json!({"job_id":Uuid::from_u128(1),"manifest_sha256":"a".repeat(64)}));
            }
            let effects =
                unknown_effects(json!({"target":"one"}), inv, json!({"available":false})).unwrap();
            assert_eq!(effects.holds.len(), 1);
            assert_eq!(effects.holds[0]["finding"], "UNKNOWN");
            assert_eq!(
                plan_changes(&effects.before).unwrap().is_empty(),
                matches!(state, "succeeded" | "failed" | "cancelled")
            );
        }
    }

    #[test]
    fn malformed_lease_deadline_counter_and_event_pair_are_rejected() {
        for key in ["lease_token", "lease_expires_at"] {
            let mut inv = fixture("running", false, 1);
            inv.tables.get_mut("job").unwrap()[0][key] = Value::Null;
            assert!(plan_changes(&inv).is_err());
        }
        for n in [-1, 4] {
            assert!(plan_changes(&fixture("running", false, n)).is_err());
        }
        let mut inv = fixture("retry_wait", false, 1);
        inv.tables.get_mut("job").unwrap()[0]["next_attempt_at"] = Value::Null;
        assert!(plan_changes(&inv).is_err());
        let mut inv = fixture("running", false, 1);
        inv.tables.get_mut("job_outbox").unwrap()[0]["business_key"] = json!("different");
        assert!(plan_changes(&inv).is_err());
    }

    #[test]
    fn full_delta_rejects_checkpoint_attempt_receipt_or_untouched_changes() {
        let before = fixture("running", false, 1);
        let plan = plan_changes(&before).unwrap();
        let timestamp = json!("2030-01-02T00:00:00+00:00");
        let mut after = before.clone();
        let row = &mut after.tables.get_mut("job").unwrap()[0];
        row["status"] = json!("retry_wait");
        row["lease_token"] = Value::Null;
        row["lease_expires_at"] = Value::Null;
        row["next_attempt_at"] = timestamp.clone();
        row["last_error_class"] = json!("recovery_interrupted");
        row["updated_at"] = json!("2026-10-20T00:00:00+00:00");
        assert_eq!(
            compare_delta(&before, &after, &plan, &timestamp)
                .unwrap()
                .len(),
            1
        );
        for (key, value) in [
            ("attempt_count", json!(0)),
            ("checkpoint", json!({"lost":true})),
            ("output_digest", json!("f".repeat(64))),
        ] {
            let mut bad = after.clone();
            bad.tables.get_mut("job").unwrap()[0][key] = value;
            assert!(compare_delta(&before, &bad, &plan, &timestamp).is_err());
        }
        let mut bad = after.clone();
        bad.tables
            .get_mut("request_key")
            .unwrap()
            .push(json!({"receipt":"new"}));
        assert!(compare_delta(&before, &bad, &plan, &timestamp).is_err());
        let terminal = fixture("succeeded", false, 1);
        let mut bad = terminal.clone();
        bad.tables.get_mut("job").unwrap()[0]["updated_at"] = json!("changed");
        assert!(compare_delta(&terminal, &bad, &[], &timestamp).is_err());
    }

    #[test]
    fn recovery_clock_is_injected_not_wall_clock_or_lease_deadline() {
        assert_eq!(
            recovery_micros(SystemTime::UNIX_EPOCH + std::time::Duration::from_micros(1_234_567))
                .unwrap(),
            1_234_567
        );
        assert!(
            recovery_micros(SystemTime::UNIX_EPOCH - std::time::Duration::from_secs(1)).is_err()
        );
        let future = fixture("running", false, 1);
        let mut expired = future.clone();
        expired.tables.get_mut("job").unwrap()[0]["lease_expires_at"] =
            json!("2000-01-01T00:00:00+00:00");
        assert_eq!(
            plan_changes(&future).unwrap(),
            plan_changes(&expired).unwrap()
        );
    }

    #[test]
    fn pending_snapshot_event_without_job_still_blocks_recovery() {
        let mut inv = fixture("snapshot_queued", true, 0);
        inv.tables.get_mut("job").unwrap().clear();
        inv.tables.get_mut("job_outbox").unwrap()[0]["dispatched_at"] = Value::Null;
        let effects = unknown_effects(json!({"target":"one"}), inv, json!({})).unwrap();
        assert_eq!(effects.holds.len(), 1);
        assert!(effects.holds[0]["job_id"].is_null());
        assert!(!effects.holds[0]["imported_outbox"].is_null());
        assert!(plan_changes(&effects.before).unwrap().is_empty());
    }

    #[test]
    fn inventory_bound_and_duplicate_identity_fail_before_planning_mutations() {
        let mut inv = fixture("queued", false, 0);
        let duplicate = inv.tables["job"][0].clone();
        inv.tables.get_mut("job").unwrap().push(duplicate);
        assert!(plan_changes(&inv).is_err());
        let mut inv = fixture("queued", false, 0);
        inv.tables
            .insert("request_key".into(), vec![Value::Null; MAX_ROWS]);
        assert!(matches!(plan_changes(&inv), Err(BackupError::Capacity(_))));
    }

    fn add_snapshot_request(inv: &mut JobInventory, requested_job_id: Uuid) {
        let actor = Uuid::from_u128(3);
        let request = Uuid::from_u128(4);
        let event = &mut inv.tables.get_mut("job_outbox").unwrap()[0];
        event["actor_id"] = json!(actor);
        event["payload"] = json!({"version":1,"kind":"snapshot_export","actor_id":actor,"space_id":Uuid::from_u128(5),"request":{"reading":{"view_id":Uuid::from_u128(6),"revision_id":Uuid::from_u128(7)},"mode":"original","include_personal":false,"include_originals":false,"resource_versions":[],"source_segments":[]}});
        let request_sha256 = digest(&event["payload"]).unwrap();
        inv.tables.get_mut("request_key").unwrap().push(json!({"actor_id":actor,"request_id":request,"operation":"snapshot_export","request_sha256":request_sha256}));
        inv.tables.get_mut("snapshot_export_request").unwrap().push(json!({"actor_id":actor,"request_id":request,"outbox_id":Uuid::from_u128(1),"job_id":requested_job_id}));
    }

    #[test]
    fn positive_private_assembly_removes_only_its_generic_unknown_hold() {
        let id = Uuid::from_u128(1);
        let mut inventory = fixture("snapshot_running", true, 2);
        add_snapshot_request(&mut inventory, id);
        let effects = assemble_effects(
            json!({"target":"one"}),
            inventory,
            json!({}),
            None,
            BTreeMap::from([(id, ExternalEffectFinding::ConfirmedNoEffect)]),
            Some(json!({"policy_test_only":true})),
        )
        .unwrap();
        assert!(effects.holds.is_empty());
        assert_eq!(effects.observations["positive_snapshot_findings"], 1);
        let planned = plan_changes_with_findings(&effects.before, &effects.findings).unwrap();
        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].status, "snapshot_retry_wait");
        assert_eq!(effects.before.tables["job"][0]["attempt_count"], 2);
    }

    #[test]
    fn positive_private_assembly_cannot_erase_request_join_conflict() {
        let id = Uuid::from_u128(1);
        let mut inventory = fixture("snapshot_running", true, 1);
        add_snapshot_request(&mut inventory, Uuid::from_u128(99));
        assert!(
            assemble_effects(
                json!({}),
                inventory,
                json!({}),
                None,
                BTreeMap::from([(id, ExternalEffectFinding::ConfirmedNoEffect)]),
                Some(json!({"policy_test_only":true}))
            )
            .is_err()
        );
    }

    #[test]
    fn snapshot_request_attached_to_integrity_event_is_held_instead_of_discarded() {
        let mut inv = fixture("queued", false, 0);
        add_snapshot_request(&mut inv, Uuid::from_u128(1));
        let effects = unknown_effects(json!({"target":"one"}), inv, json!({})).unwrap();
        assert_eq!(effects.holds.len(), 1);
        assert_eq!(effects.holds[0]["reason"], "SNAPSHOT_REQUEST_JOIN_CONFLICT");
        assert!(
            effects.holds[0]["details"]
                .as_array()
                .unwrap()
                .contains(&json!("REQUEST_OUTBOX_IS_NOT_SNAPSHOT_EVENT"))
        );
        assert!(!effects.holds[0]["imported_request_key"].is_null());
    }

    #[test]
    fn snapshot_request_wrong_reserved_job_is_held_even_when_outbox_exists() {
        let mut inv = fixture("snapshot_queued", true, 0);
        add_snapshot_request(&mut inv, Uuid::from_u128(99));
        let effects = unknown_effects(json!({"target":"one"}), inv, json!({})).unwrap();
        let conflict = effects
            .holds
            .iter()
            .find(|h| h["reason"] == "SNAPSHOT_REQUEST_JOIN_CONFLICT")
            .unwrap();
        assert!(
            conflict["details"]
                .as_array()
                .unwrap()
                .contains(&json!("REQUEST_JOB_ID_DIFFERS_FROM_STABLE_OUTBOX_ID"))
        );
        assert_eq!(
            conflict["imported_request"]["job_id"],
            json!(Uuid::from_u128(99))
        );
    }

    #[test]
    fn orphan_snapshot_request_key_is_held_but_unrelated_request_key_is_preserved() {
        let mut inv = fixture("queued", false, 0);
        inv.tables.get_mut("request_key").unwrap().push(json!({"actor_id":Uuid::from_u128(3),"request_id":Uuid::from_u128(4),"operation":"snapshot_export","request_sha256":"a".repeat(64)}));
        let effects = unknown_effects(json!({"target":"one"}), inv.clone(), json!({})).unwrap();
        assert_eq!(effects.holds.len(), 1);
        assert_eq!(effects.holds[0]["reason"], "ORPHAN_SNAPSHOT_REQUEST_KEY");
        assert_eq!(
            effects.holds[0]["imported_request_key"],
            inv.tables["request_key"][0]
        );
        inv.tables.get_mut("request_key").unwrap()[0]["operation"] = json!("content_v1");
        assert!(
            unknown_effects(json!({"target":"one"}), inv, json!({}))
                .unwrap()
                .holds
                .is_empty()
        );
    }

    #[test]
    fn wrong_request_operation_digest_or_actor_never_becomes_a_positive_finding() {
        let mut baseline = fixture("snapshot_queued", true, 0);
        add_snapshot_request(&mut baseline, Uuid::from_u128(1));
        assert!(reconciliation_join_holds(&baseline).unwrap().is_empty());
        assert_eq!(
            unknown_effects(json!({}), baseline.clone(), json!({}))
                .unwrap()
                .holds
                .len(),
            1
        );
        for (column, value) in [
            ("operation", json!("content_v1")),
            ("request_sha256", json!("0".repeat(64))),
        ] {
            let mut inv = baseline.clone();
            inv.tables.get_mut("request_key").unwrap()[0][column] = value;
            assert!(!reconciliation_join_holds(&inv).unwrap().is_empty());
        }
        baseline.tables.get_mut("job_outbox").unwrap()[0]["actor_id"] = json!(Uuid::from_u128(88));
        assert!(!reconciliation_join_holds(&baseline).unwrap().is_empty());
    }

    #[test]
    fn unconsumed_snapshot_result_is_retained_as_a_hold() {
        let mut inv = fixture("succeeded", false, 1);
        inv.tables
            .get_mut("snapshot_export_result")
            .unwrap()
            .push(json!({"job_id":Uuid::from_u128(1),"manifest_sha256":"a".repeat(64)}));
        let effects = unknown_effects(json!({}), inv, json!({})).unwrap();
        assert_eq!(effects.holds.len(), 1);
        assert_eq!(effects.holds[0]["reason"], "SNAPSHOT_RESULT_JOIN_CONFLICT");
        assert!(!effects.holds[0]["imported_result"].is_null());
        assert!(plan_changes(&effects.before).unwrap().is_empty());
    }

    fn allowance(remaining: u64) -> MetadataBudget {
        let budget = Arc::new(Mutex::new(crate::registry::ScanBudget::default()));
        charge(&budget, crate::registry::MAX_SCAN_BYTES - remaining).unwrap();
        budget
    }

    #[test]
    fn small_remaining_allowance_is_shared_across_metadata_serializations() {
        let value = json!({"body":"1234567890123456"});
        let size = serde_json::to_vec(&value).unwrap().len() as u64;
        let budget = allowance(size * 2 - 1);
        let same_operation = Arc::clone(&budget);
        assert_eq!(
            encode_charged(&value, Some(&budget)).unwrap(),
            serde_json::to_vec(&value).unwrap()
        );
        assert!(matches!(
            encode_charged(&value, Some(&same_operation)),
            Err(BackupError::Capacity(_))
        ));
        // A reset meter would wrongly admit this second independently small record.
        let reset = allowance(size * 2 - 1);
        assert!(encode_charged(&value, Some(&reset)).is_ok());
        assert!(budget.lock().unwrap().bytes <= crate::registry::MAX_SCAN_BYTES);
    }

    #[test]
    fn repeated_real_inventory_hashes_cannot_reset_the_operation_budget() {
        let inv = fixture("queued", false, 0);
        let size = serde_json::to_vec(&inv).unwrap().len() as u64;
        let budget = allowance(size * 2 - 1);
        assert!(unknown_effects_metered(json!({}), inv.clone(), json!({}), Some(&budget)).is_ok());
        assert!(matches!(
            unknown_effects_metered(json!({}), inv, json!({}), Some(&Arc::clone(&budget))),
            Err(BackupError::Capacity(_))
        ));
    }

    #[test]
    fn budgeted_json_preserves_exact_bytes_for_escaped_unicode_metadata() {
        let value = json!({"中文":"quote\" slash\\ newline\n control\u{1}","numbers":[1,-2,3.5],"flags":[true,false,null]});
        let budget = Arc::new(Mutex::new(crate::registry::ScanBudget::default()));
        let raw = encode_charged(&value, Some(&budget)).unwrap();
        assert_eq!(raw, serde_json::to_vec(&value).unwrap());
        assert_eq!(budget.lock().unwrap().bytes, raw.len() as u64);
        assert!(inventory_weight(&value).unwrap() >= raw.len());
    }
}

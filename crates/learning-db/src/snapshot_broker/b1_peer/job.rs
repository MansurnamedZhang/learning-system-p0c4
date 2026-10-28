//! Fixed typed DATA and original-channel codec. Parsing grants no process qualification.
use super::{Error, Result};
use learning_assets::root_journal::SharedJournalBudget;
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned};

pub(super) const MAX_MESSAGE: usize = 4096;
pub(super) const MAX_FRAMES: u64 = 64;
pub(super) const MAX_CONTROL_BYTES: u64 = 262_400;

pub(super) trait Record: Serialize {
    fn validate(&self) -> Result<()>;
}

pub(super) fn parse<T: Record + DeserializeOwned>(raw: &[u8], maximum: usize) -> Result<T> {
    if raw.is_empty() || raw.len() > maximum {
        return Err(Error::Shape);
    }
    // Typed derive rejects duplicate/unknown keys at every nested struct.
    let value: T = serde_json::from_slice(raw).map_err(|_| Error::Shape)?;
    value.validate()?;
    if canonical(&value)? != raw {
        return Err(Error::Shape);
    }
    Ok(value)
}

pub(super) fn canonical(value: &impl Serialize) -> Result<Vec<u8>> {
    fn order(value: serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(fields) => {
                let sorted: std::collections::BTreeMap<_, _> = fields.into_iter().collect();
                serde_json::Value::Object(sorted.into_iter().map(|(k, v)| (k, order(v))).collect())
            }
            serde_json::Value::Array(items) => {
                serde_json::Value::Array(items.into_iter().map(order).collect())
            }
            other => other,
        }
    }
    serde_json::to_vec(&order(
        serde_json::to_value(value).map_err(|_| Error::Shape)?,
    ))
    .map_err(|_| Error::Shape)
}

pub(super) fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn digest_bytes(value: &str) -> Result<[u8; 32]> {
    if !hex(value, 64) {
        return Err(Error::Shape);
    }
    let mut out = [0; 32];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        let digit = |c: u8| if c <= b'9' { c - b'0' } else { c - b'a' + 10 };
        out[index] = digit(chunk[0]) * 16 + digit(chunk[1]);
    }
    Ok(out)
}
pub(super) fn uuid(value: &str, v4: bool) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| {
        id.hyphenated().to_string() == value
            && (!v4 || (id.get_version_num() == 4 && id.get_variant() == uuid::Variant::RFC4122))
    })
}
pub(super) fn path(value: &str) -> bool {
    value.starts_with('/')
        && value.len() <= 1024
        && !value.as_bytes().contains(&0)
        && value[1..]
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != "..")
}
fn namespace(value: &str, prefix: &str) -> bool {
    value
        .strip_prefix(prefix)
        .and_then(|v| v.strip_suffix(']'))
        .is_some_and(|n| {
            !n.starts_with('0') && n.parse::<u64>().is_ok_and(|v| v > 0 && v.to_string() == n)
        })
}
fn pattern(value: &str, pattern: &str) -> bool {
    match pattern {
        "^(learning-system-p0c4-[A-Za-z0-9_-]+|kwc4c-[0-9a-f]{32})$" => {
            value
                .strip_prefix("learning-system-p0c4-")
                .is_some_and(|s| {
                    !s.is_empty()
                        && s.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
                })
                || value.strip_prefix("kwc4c-").is_some_and(|s| hex(s, 32))
        }
        "^/(?!/)(?!.*(?:^|/)\\.\\.?(?:/|$))[^\\x00]+$" => path(value),
        "^PGDATABASE=learning_backup_c4_task3_[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$" => {
            value.strip_prefix("PGDATABASE=").is_some_and(database)
        }
        "^[0-9a-f]{40}$" => hex(value, 40),
        "^[0-9a-f]{64}$" => hex(value, 64),
        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$" => {
            uuid(value, true)
        }
        "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$" => uuid(value, false),
        "^[1-9][0-9]{0,19}$" => {
            !value.starts_with('0')
                && value
                    .parse::<u64>()
                    .is_ok_and(|v| v > 0 && v.to_string() == value)
        }
        "^[A-Za-z0-9:_-]+$" => {
            !value.is_empty()
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b":_-".contains(&b))
        }
        "^learning_backup_c4_task3_[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$" => {
            database(value)
        }
        "^mnt:\\[[1-9][0-9]*\\]$" => namespace(value, "mnt:["),
        "^pid:\\[[1-9][0-9]*\\]$" => namespace(value, "pid:["),
        "^sha256:[0-9a-f]{64}$" => value.strip_prefix("sha256:").is_some_and(|v| hex(v, 64)),
        _ => false,
    }
}
pub(super) fn database(value: &str) -> bool {
    value.len() <= 63
        && value
            .strip_prefix("learning_backup_c4_task3_")
            .is_some_and(|v| uuid(v, true))
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CaptureExecRecipe {
    pub(super) schema_version: u64,
    pub(super) entrypoint: String,
    pub(super) arguments: Vec<String>,
    pub(super) wrapper_code: CaptureExecRecipeWrapperCode,
    pub(super) wrapper_libraries: Vec<CodeFile>,
    pub(super) final_capture_path: String,
    pub(super) same_exec_pid_start_no_fork: bool,
}
impl Record for CaptureExecRecipe {
    fn validate(&self) -> Result<()> {
        if self.schema_version == 1
            && self.entrypoint == "/usr/bin/env"
            && self.arguments.len() >= 9
            && self.arguments.len() <= 9
            && self.arguments.first().is_some_and(|item| item == "-i")
            && self.arguments.get(1).is_some_and(|item| item == "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
            && self.arguments.get(2).is_some_and(|item| item == "HOME=/root")
            && self.arguments.get(3).is_some_and(|item| item == "LANG=C")
            && self.arguments.get(4).is_some_and(|item| item == "PGHOST=/var/run/postgresql")
            && self.arguments.get(5).is_some_and(|item| item == "PGPORT=5432")
            && self.arguments.get(6).is_some_and(|item| item == "PGUSER=learning_admin")
            && self.arguments.get(7).is_some_and(|item| item.len() <= 74 && pattern(item, "^PGDATABASE=learning_backup_c4_task3_[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"))
            && self.arguments.get(8).is_some_and(|item| item == "/opt/knowweave-c4/b1-capture/knowweave-c4-b1-capture")
            && self.wrapper_code.validate().is_ok()
            && self.wrapper_libraries.len() <= 32
            && self.wrapper_libraries.iter().all(|item| item.validate().is_ok())
            && self.wrapper_libraries.windows(2).all(|w|w[0].path<w[1].path)
            && self.final_capture_path == "/opt/knowweave-c4/b1-capture/knowweave-c4-b1-capture"
            && self.same_exec_pid_start_no_fork { Ok(()) } else { Err(Error::Shape) }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CaptureExecRecipeWrapperCode {
    pub(super) path: String,
    pub(super) stamp: FileStamp,
    pub(super) sha256: String,
}
impl Record for CaptureExecRecipeWrapperCode {
    fn validate(&self) -> Result<()> {
        if self.path == "/usr/bin/env"
            && self.stamp.validate().is_ok()
            && self.sha256.len() <= 64
            && pattern(&self.sha256, "^[0-9a-f]{64}$")
        {
            Ok(())
        } else {
            Err(Error::Shape)
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InstalledRuntime {
    pub(super) compose_project: String,
    pub(super) pg_container_id: String,
    pub(super) pg_image_ref: String,
    pub(super) daemon_id: String,
    pub(super) internal_network_id: String,
    pub(super) capture_mount_recipe_sha256: String,
    pub(super) source_root_records_sha256: String,
    pub(super) pg_pid_namespace: String,
    pub(super) pg_mount_namespace: String,
    pub(super) postmaster_start_ticks: u64,
    pub(super) admitted_pg_image_id: String,
    pub(super) pg_image_digest: String,
}
impl Record for InstalledRuntime {
    fn validate(&self) -> Result<()> {
        if self.compose_project.len() <= 63
            && pattern(
                &self.compose_project,
                "^(learning-system-p0c4-[A-Za-z0-9_-]+|kwc4c-[0-9a-f]{32})$",
            )
            && self.pg_container_id.len() <= 64
            && pattern(&self.pg_container_id, "^[0-9a-f]{64}$")
            && self.pg_image_ref
                == "postgres:18.6-bookworm@sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d"
            && self.daemon_id.len() <= 128
            && pattern(&self.daemon_id, "^[A-Za-z0-9:_-]+$")
            && self.internal_network_id.len() <= 64
            && pattern(&self.internal_network_id, "^[0-9a-f]{64}$")
            && self.capture_mount_recipe_sha256.len() <= 64
            && pattern(&self.capture_mount_recipe_sha256, "^[0-9a-f]{64}$")
            && self.source_root_records_sha256.len() <= 64
            && pattern(&self.source_root_records_sha256, "^[0-9a-f]{64}$")
            && self.pg_pid_namespace.len() <= 64
            && pattern(&self.pg_pid_namespace, "^pid:\\[[1-9][0-9]*\\]$")
            && self.pg_mount_namespace.len() <= 64
            && pattern(&self.pg_mount_namespace, "^mnt:\\[[1-9][0-9]*\\]$")
            && self.postmaster_start_ticks >= 1
            && self.admitted_pg_image_id.len() <= 71
            && pattern(&self.admitted_pg_image_id, "^sha256:[0-9a-f]{64}$")
            && self.pg_image_digest
                == "sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d"
        {
            Ok(())
        } else {
            Err(Error::Shape)
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Hold {
    pub(super) schema_version: u64,
    pub(super) capability: String,
    pub(super) job_id: String,
    pub(super) phase: String,
    pub(super) reason_code: String,
    pub(super) retry_allowed: bool,
    pub(super) owner_settlement: String,
}
impl Record for Hold {
    fn validate(&self) -> Result<()> {
        if self.schema_version == 1
            && self.capability == "b1_source_job_hold_v1"
            && self.job_id.len() <= 36
            && pattern(
                &self.job_id,
                "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
            )
            && [
                "Bootstrap",
                "Install",
                "Quiesce",
                "Capture",
                "Transfer",
                "Settle",
                "Final",
            ]
            .contains(&self.phase.as_str())
            && [
                "InputShape",
                "ProcessChanged",
                "RootOwnerLost",
                "CookieMismatch",
                "Phase",
                "StaleInitialProof",
                "Budget",
                "ChildExit",
                "UnexpectedEof",
                "ExtraBytes",
                "Deadline",
                "SettlementUnconfirmed",
            ]
            .contains(&self.reason_code.as_str())
            && !self.retry_allowed
            && ["Unconfirmed", "ObservedStopped"].contains(&self.owner_settlement.as_str())
        {
            Ok(())
        } else {
            Err(Error::Shape)
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CaptureSettlement {
    pub(super) schema_version: u64,
    pub(super) capability: String,
    pub(super) job_id: String,
    pub(super) backup_id: String,
    pub(super) capture_start_sha256: String,
    pub(super) captured_data_sha256: String,
    pub(super) exec_id: String,
    pub(super) child_start_ticks: u64,
    pub(super) natural_exit_code: u64,
    pub(super) stdout_eof: bool,
    pub(super) stderr_eof: bool,
    pub(super) extra_bytes: bool,
    pub(super) owner_settled: bool,
    pub(super) stdout_sha256: String,
    pub(super) stderr_sha256: String,
}
impl Record for CaptureSettlement {
    fn validate(&self) -> Result<()> {
        if self.schema_version == 1
            && self.capability == "b1_capture_natural_settlement_observation_v1"
            && self.job_id.len() <= 36
            && pattern(
                &self.job_id,
                "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
            )
            && self.backup_id.len() <= 36
            && pattern(
                &self.backup_id,
                "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
            )
            && self.capture_start_sha256.len() <= 64
            && pattern(&self.capture_start_sha256, "^[0-9a-f]{64}$")
            && self.captured_data_sha256.len() <= 64
            && pattern(&self.captured_data_sha256, "^[0-9a-f]{64}$")
            && self.exec_id.len() <= 64
            && pattern(&self.exec_id, "^[0-9a-f]{64}$")
            && self.child_start_ticks >= 1
            && self.natural_exit_code == 0
            && self.stdout_eof
            && self.stderr_eof
            && !self.extra_bytes
            && self.owner_settled
            && self.stdout_sha256.len() <= 64
            && pattern(&self.stdout_sha256, "^[0-9a-f]{64}$")
            && self.stderr_sha256.len() <= 64
            && pattern(&self.stderr_sha256, "^[0-9a-f]{64}$")
        {
            Ok(())
        } else {
            Err(Error::Shape)
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CapturedData {
    pub(super) schema_version: u64,
    pub(super) capability: String,
    pub(super) job_id: String,
    pub(super) backup_id: String,
    pub(super) capture_start_sha256: String,
    pub(super) capture_generation_sha256: String,
    pub(super) source_group_id: String,
    pub(super) source_registry_generation_sha256: String,
    pub(super) manifest_sha256: String,
    pub(super) captured_roster_sha256: String,
    pub(super) snapshot_effects_roster_sha256: String,
}
impl Record for CapturedData {
    fn validate(&self) -> Result<()> {
        if self.schema_version == 1
            && self.capability == "b1_captured_input_data_only_v1"
            && self.job_id.len() <= 36
            && pattern(
                &self.job_id,
                "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
            )
            && self.backup_id.len() <= 36
            && pattern(
                &self.backup_id,
                "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
            )
            && self.capture_start_sha256.len() <= 64
            && pattern(&self.capture_start_sha256, "^[0-9a-f]{64}$")
            && self.capture_generation_sha256.len() <= 64
            && pattern(&self.capture_generation_sha256, "^[0-9a-f]{64}$")
            && self.source_group_id.len() <= 36
            && pattern(
                &self.source_group_id,
                "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
            )
            && self.source_registry_generation_sha256.len() <= 64
            && pattern(&self.source_registry_generation_sha256, "^[0-9a-f]{64}$")
            && self.manifest_sha256.len() <= 64
            && pattern(&self.manifest_sha256, "^[0-9a-f]{64}$")
            && self.captured_roster_sha256.len() <= 64
            && pattern(&self.captured_roster_sha256, "^[0-9a-f]{64}$")
            && self.snapshot_effects_roster_sha256.len() <= 64
            && pattern(&self.snapshot_effects_roster_sha256, "^[0-9a-f]{64}$")
        {
            Ok(())
        } else {
            Err(Error::Shape)
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GenerationBinding {
    pub(super) schema_version: u64,
    pub(super) capability: String,
    pub(super) job_id: String,
    pub(super) backup_id: String,
    pub(super) phase: String,
    pub(super) request_id: String,
    pub(super) capture_start_sha256: String,
    pub(super) isolation_bytes: u64,
    pub(super) isolation_sha256: String,
    pub(super) inspection_bytes: u64,
    pub(super) inspection_sha256: String,
    pub(super) lock_dev: u64,
    pub(super) lock_ino: u64,
    pub(super) observed_unix_ms: u64,
}
impl Record for GenerationBinding {
    fn validate(&self) -> Result<()> {
        if self.schema_version == 1
            && self.capability == "b1_immutable_isolation_generation_v1"
            && self.job_id.len() <= 36
            && pattern(
                &self.job_id,
                "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
            )
            && self.backup_id.len() <= 36
            && pattern(
                &self.backup_id,
                "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
            )
            && ["Install", "Capture"].contains(&self.phase.as_str())
            && self.request_id.len() <= 36
            && pattern(
                &self.request_id,
                "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
            )
            && self.capture_start_sha256.len() <= 64
            && pattern(&self.capture_start_sha256, "^[0-9a-f]{64}$")
            && self.isolation_bytes >= 1
            && self.isolation_bytes <= 4096
            && self.isolation_sha256.len() <= 64
            && pattern(&self.isolation_sha256, "^[0-9a-f]{64}$")
            && self.inspection_bytes >= 1
            && self.inspection_bytes <= 1048576
            && self.inspection_sha256.len() <= 64
            && pattern(&self.inspection_sha256, "^[0-9a-f]{64}$")
            && self.lock_dev >= 1
            && self.lock_ino >= 1
            && self.observed_unix_ms >= 1
        {
            Ok(())
        } else {
            Err(Error::Shape)
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CaptureStart {
    pub(super) schema_version: u64,
    pub(super) job_id: String,
    pub(super) launch_id: String,
    pub(super) catalog_sha256: String,
    pub(super) boot_id: String,
    pub(super) root_cookie: String,
    pub(super) capability: String,
    pub(super) backup_id: String,
    pub(super) capture_nonce: String,
    pub(super) capture_request_id: String,
    pub(super) source_start_sha256: String,
    pub(super) container_id: String,
    pub(super) exec_id: String,
    pub(super) image_id: String,
    pub(super) mount_recipe_sha256: String,
    pub(super) capture: ProcessRef,
    pub(super) native: Native,
    pub(super) image_ref: String,
    pub(super) image_digest: String,
    pub(super) exec_process_config: CaptureStartExecProcessConfig,
    pub(super) capture_exec_recipe_sha256: String,
}
impl Record for CaptureStart {
    fn validate(&self) -> Result<()> {
        if self.schema_version == 1
            && self.job_id.len() <= 36
            && pattern(
                &self.job_id,
                "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
            )
            && self.launch_id.len() <= 36
            && pattern(
                &self.launch_id,
                "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
            )
            && self.catalog_sha256.len() <= 64
            && pattern(&self.catalog_sha256, "^[0-9a-f]{64}$")
            && self.boot_id.len() <= 36
            && pattern(
                &self.boot_id,
                "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$",
            )
            && self.root_cookie.len() <= 64
            && pattern(&self.root_cookie, "^[0-9a-f]{64}$")
            && self.capability == "b1_capture_process_start_v1"
            && self.backup_id.len() <= 36
            && pattern(
                &self.backup_id,
                "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
            )
            && self.capture_nonce.len() <= 64
            && pattern(&self.capture_nonce, "^[0-9a-f]{64}$")
            && self.capture_request_id.len() <= 36
            && pattern(
                &self.capture_request_id,
                "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
            )
            && self.source_start_sha256.len() <= 64
            && pattern(&self.source_start_sha256, "^[0-9a-f]{64}$")
            && self.container_id.len() <= 64
            && pattern(&self.container_id, "^[0-9a-f]{64}$")
            && self.exec_id.len() <= 64
            && pattern(&self.exec_id, "^[0-9a-f]{64}$")
            && self.image_id.len() <= 71
            && pattern(&self.image_id, "^sha256:[0-9a-f]{64}$")
            && self.mount_recipe_sha256.len() <= 64
            && pattern(&self.mount_recipe_sha256, "^[0-9a-f]{64}$")
            && self.capture.validate().is_ok()
            && self.native.validate().is_ok()
            && self.image_ref
                == "postgres:18.6-bookworm@sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d"
            && self.image_digest
                == "sha256:9e73daeb439141c2b11eea2463f5f1a3b269fd90d897b41cddb7cb440f21aa5d"
            && self.exec_process_config.validate().is_ok()
            && self.capture_exec_recipe_sha256.len() <= 64
            && pattern(&self.capture_exec_recipe_sha256, "^[0-9a-f]{64}$")
        {
            Ok(())
        } else {
            Err(Error::Shape)
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CaptureStartExecProcessConfig {
    pub(super) entrypoint: String,
    pub(super) arguments: Vec<String>,
    pub(super) tty: bool,
    pub(super) privileged: bool,
    pub(super) user: String,
}
impl Record for CaptureStartExecProcessConfig {
    fn validate(&self) -> Result<()> {
        if self.entrypoint == "/usr/bin/env"
            && self.arguments.len() >= 9
            && self.arguments.len() <= 9
            && self.arguments.first().is_some_and(|item| item == "-i")
            && self.arguments.get(1).is_some_and(|item| item == "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
            && self.arguments.get(2).is_some_and(|item| item == "HOME=/root")
            && self.arguments.get(3).is_some_and(|item| item == "LANG=C")
            && self.arguments.get(4).is_some_and(|item| item == "PGHOST=/var/run/postgresql")
            && self.arguments.get(5).is_some_and(|item| item == "PGPORT=5432")
            && self.arguments.get(6).is_some_and(|item| item == "PGUSER=learning_admin")
            && self.arguments.get(7).is_some_and(|item| item.len() <= 74 && pattern(item, "^PGDATABASE=learning_backup_c4_task3_[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"))
            && self.arguments.get(8).is_some_and(|item| item == "/opt/knowweave-c4/b1-capture/knowweave-c4-b1-capture")
            && !self.tty
            && !self.privileged
            && self.user == "0:0" { Ok(()) } else { Err(Error::Shape) }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Native {
    pub(super) postmaster_start_ticks: u64,
    pub(super) data_dev: u64,
    pub(super) data_ino: u64,
    pub(super) socket_dev: u64,
    pub(super) socket_ino: u64,
}
impl Record for Native {
    fn validate(&self) -> Result<()> {
        if self.postmaster_start_ticks >= 1
            && self.data_dev >= 1
            && self.data_ino >= 1
            && self.socket_dev >= 1
            && self.socket_ino >= 1
        {
            Ok(())
        } else {
            Err(Error::Shape)
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SourceStart {
    pub(super) schema_version: u64,
    pub(super) job_id: String,
    pub(super) launch_id: String,
    pub(super) catalog_sha256: String,
    pub(super) boot_id: String,
    pub(super) root_cookie: String,
    pub(super) capability: String,
    pub(super) launcher: ProcessRef,
    pub(super) source: ProcessRef,
    pub(super) source_control_fd: u64,
    pub(super) source_socket_cookie: u64,
    pub(super) job_lock_dev: u64,
    pub(super) job_lock_ino: u64,
    pub(super) started_monotonic_ns: u64,
}
impl Record for SourceStart {
    fn validate(&self) -> Result<()> {
        if self.schema_version == 1
            && self.job_id.len() <= 36
            && pattern(
                &self.job_id,
                "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
            )
            && self.launch_id.len() <= 36
            && pattern(
                &self.launch_id,
                "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
            )
            && self.catalog_sha256.len() <= 64
            && pattern(&self.catalog_sha256, "^[0-9a-f]{64}$")
            && self.boot_id.len() <= 36
            && pattern(
                &self.boot_id,
                "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$",
            )
            && self.root_cookie.len() <= 64
            && pattern(&self.root_cookie, "^[0-9a-f]{64}$")
            && self.capability == "b1_source_process_start_v1"
            && self.launcher.validate().is_ok()
            && self.source.validate().is_ok()
            && self.source_control_fd == 3
            && self.source_socket_cookie >= 1
            && self.job_lock_dev >= 1
            && self.job_lock_ino >= 1
            && self.started_monotonic_ns >= 1
        {
            Ok(())
        } else {
            Err(Error::Shape)
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExecutionCatalog {
    pub(super) schema_version: u64,
    pub(super) capability: String,
    pub(super) profile: String,
    pub(super) launch_id: String,
    pub(super) installed_source: InstalledSource,
    pub(super) peer_endpoint: PeerEndpoint,
    pub(super) capture_recipe_sha256: String,
    pub(super) images: Vec<RoleImage>,
    pub(super) installed_runtime: InstalledRuntime,
    pub(super) capture_exec_recipe: CaptureExecRecipe,
}
impl Record for ExecutionCatalog {
    fn validate(&self) -> Result<()> {
        if self.schema_version == 1
            && self.capability == "b1_source_peer_execution_v1"
            && self.profile == "task9-b1-limited-v1"
            && self.launch_id.len() <= 36
            && pattern(
                &self.launch_id,
                "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
            )
            && self.installed_source.validate().is_ok()
            && self.peer_endpoint.validate().is_ok()
            && self.capture_recipe_sha256.len() <= 64
            && pattern(&self.capture_recipe_sha256, "^[0-9a-f]{64}$")
            && self.images.len() >= 3
            && self.images.len() <= 3
            && self.images.iter().all(|item| item.validate().is_ok())
            && self.installed_runtime.validate().is_ok()
            && self.capture_exec_recipe.validate().is_ok()
            && self
                .images
                .iter()
                .map(|i| i.role.as_str())
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == 3
            && self.capture_exec_recipe.arguments.get(7)
                == Some(&format!("PGDATABASE={}", self.installed_source.database))
        {
            Ok(())
        } else {
            Err(Error::Shape)
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RoleImage {
    pub(super) role: String,
    pub(super) executable: CodeFile,
    pub(super) launch_manifest: RecordRef,
    pub(super) libraries: Vec<CodeFile>,
    pub(super) code_commit: String,
    pub(super) build_input_sha256: String,
}
impl Record for RoleImage {
    fn validate(&self) -> Result<()> {
        if ["PublicationSource", "Capture", "RootLauncher"].contains(&self.role.as_str())
            && self.executable.validate().is_ok()
            && self.launch_manifest.validate().is_ok()
            && self.libraries.len() <= 32
            && self.libraries.iter().all(|item| item.validate().is_ok())
            && self.code_commit.len() <= 40
            && pattern(&self.code_commit, "^[0-9a-f]{40}$")
            && self.build_input_sha256.len() <= 64
            && pattern(&self.build_input_sha256, "^[0-9a-f]{64}$")
            && match self.role.as_str() {
                "PublicationSource" => {
                    self.executable.path == "/opt/knowweave-c4/bin/knowweave-c4-b1-source"
                        && self.launch_manifest.record == "launch-manifest.json"
                }
                "Capture" => {
                    self.executable.path == "/opt/knowweave-c4/b1-capture/knowweave-c4-b1-capture"
                        && self.launch_manifest.record == "capture-launch-manifest.json"
                }
                "RootLauncher" => {
                    self.executable.path == "/opt/knowweave-c4/bin/knowweave-c4-b1-source-launcher"
                        && self.launch_manifest.record == "launcher-launch-manifest.json"
                }
                _ => false,
            }
            && self.libraries.windows(2).all(|w| w[0].path < w[1].path)
        {
            Ok(())
        } else {
            Err(Error::Shape)
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PeerEndpoint {
    pub(super) capture_directory: String,
    pub(super) source_directory: String,
    pub(super) port: u64,
    pub(super) socket_leaf: String,
    pub(super) directory_dev: u64,
    pub(super) directory_ino: u64,
    pub(super) socket_dev: u64,
    pub(super) socket_ino: u64,
    pub(super) admin_role: String,
    pub(super) broker_role: String,
    pub(super) runtime_role: String,
    pub(super) original_login: String,
}
impl Record for PeerEndpoint {
    fn validate(&self) -> Result<()> {
        if self.capture_directory == "/var/run/postgresql"
            && self.source_directory == "/run/knowweave-c4-source-pg"
            && self.port == 5432
            && self.socket_leaf == ".s.PGSQL.5432"
            && self.directory_dev >= 1
            && self.directory_ino >= 1
            && self.socket_dev >= 1
            && self.socket_ino >= 1
            && self.admin_role == "learning_admin"
            && self.broker_role == "learning_snapshot_claim_broker"
            && self.runtime_role == "learning_runtime"
            && self.original_login == "peer:root"
        {
            Ok(())
        } else {
            Err(Error::Shape)
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InstalledSource {
    pub(super) deployment_id: String,
    pub(super) source_group_id: String,
    pub(super) application_commit: String,
    pub(super) application_build_sha256: String,
    pub(super) source_auth_public_key_sha256: String,
    pub(super) registry_authority_sha256: String,
    pub(super) source_group_record_sha256: String,
    pub(super) source_control_binding_sha256: String,
    pub(super) database: String,
    pub(super) database_oid: u64,
    pub(super) system_identifier: String,
}
impl Record for InstalledSource {
    fn validate(&self) -> Result<()> {
        if self.deployment_id.len() <= 36
            && pattern(
                &self.deployment_id,
                "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
            )
            && self.source_group_id.len() <= 36
            && pattern(
                &self.source_group_id,
                "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
            )
            && self.application_commit.len() <= 40
            && pattern(&self.application_commit, "^[0-9a-f]{40}$")
            && self.application_build_sha256.len() <= 64
            && pattern(&self.application_build_sha256, "^[0-9a-f]{64}$")
            && self.source_auth_public_key_sha256.len() <= 64
            && pattern(&self.source_auth_public_key_sha256, "^[0-9a-f]{64}$")
            && self.registry_authority_sha256.len() <= 64
            && pattern(&self.registry_authority_sha256, "^[0-9a-f]{64}$")
            && self.source_group_record_sha256.len() <= 64
            && pattern(&self.source_group_record_sha256, "^[0-9a-f]{64}$")
            && self.source_control_binding_sha256.len() <= 64
            && pattern(&self.source_control_binding_sha256, "^[0-9a-f]{64}$")
            && self.database.len() <= 63
            && pattern(
                &self.database,
                "^learning_backup_c4_task3_[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
            )
            && self.database_oid >= 1
            && self.database_oid <= 4294967295
            && self.system_identifier.len() <= 20
            && pattern(&self.system_identifier, "^[1-9][0-9]{0,19}$")
        {
            Ok(())
        } else {
            Err(Error::Shape)
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProcessRef {
    pub(super) host_pid: u64,
    pub(super) namespace_pid: u64,
    pub(super) start_ticks: u64,
    pub(super) pid_namespace: String,
    pub(super) mount_namespace: String,
    pub(super) exe_sha256: String,
    pub(super) exe_dev: u64,
    pub(super) exe_ino: u64,
    pub(super) real_uid: u64,
    pub(super) effective_uid: u64,
    pub(super) saved_uid: u64,
    pub(super) real_gid: u64,
    pub(super) effective_gid: u64,
    pub(super) saved_gid: u64,
}
impl Record for ProcessRef {
    fn validate(&self) -> Result<()> {
        if self.host_pid >= 1
            && self.host_pid <= 4294967295
            && self.namespace_pid >= 1
            && self.namespace_pid <= 4294967295
            && self.start_ticks >= 1
            && self.pid_namespace.len() <= 64
            && pattern(&self.pid_namespace, "^pid:\\[[1-9][0-9]*\\]$")
            && self.mount_namespace.len() <= 64
            && pattern(&self.mount_namespace, "^mnt:\\[[1-9][0-9]*\\]$")
            && self.exe_sha256.len() <= 64
            && pattern(&self.exe_sha256, "^[0-9a-f]{64}$")
            && self.exe_dev >= 1
            && self.exe_ino >= 1
            && self.real_uid == 0
            && self.effective_uid == 0
            && self.saved_uid == 0
            && self.real_gid == 0
            && self.effective_gid == 0
            && self.saved_gid == 0
        {
            Ok(())
        } else {
            Err(Error::Shape)
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CodeFile {
    pub(super) path: String,
    pub(super) stamp: FileStamp,
    pub(super) sha256: String,
}
impl Record for CodeFile {
    fn validate(&self) -> Result<()> {
        if self.path.len() <= 1024
            && pattern(&self.path, "^/(?!/)(?!.*(?:^|/)\\.\\.?(?:/|$))[^\\x00]+$")
            && self.stamp.validate().is_ok()
            && self.sha256.len() <= 64
            && pattern(&self.sha256, "^[0-9a-f]{64}$")
        {
            Ok(())
        } else {
            Err(Error::Shape)
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileStamp {
    pub(super) dev: u64,
    pub(super) ino: u64,
    pub(super) uid: u64,
    pub(super) gid: u64,
    pub(super) mode: u64,
    pub(super) nlink: u64,
    pub(super) bytes: u64,
    pub(super) mtime_ns: u64,
    pub(super) ctime_ns: u64,
}
impl Record for FileStamp {
    fn validate(&self) -> Result<()> {
        if self.dev >= 1
            && self.ino >= 1
            && self.uid == 0
            && self.gid <= 4294967295
            && self.mode <= 65535
            && self.nlink == 1
            && self.bytes >= 1
            && self.bytes <= 134217728
        {
            Ok(())
        } else {
            Err(Error::Shape)
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RecordRef {
    pub(super) record: String,
    pub(super) bytes: u64,
    pub(super) sha256: String,
}
impl Record for RecordRef {
    fn validate(&self) -> Result<()> {
        if [
            "launch-manifest.json",
            "capture-launch-manifest.json",
            "launcher-launch-manifest.json",
            "source-peer-execution.json",
            "source-start.json",
            "capture-start.json",
            "generation-binding.json",
            "captured-data.json",
            "capture-settlement.json",
        ]
        .contains(&self.record.as_str())
            && self.bytes >= 1
            && self.bytes <= 16384
            && self.sha256.len() <= 64
            && pattern(&self.sha256, "^[0-9a-f]{64}$")
        {
            Ok(())
        } else {
            Err(Error::Shape)
        }
    }
}
// Exact human-approved C11 record DATA. None of these values can construct
// an installed binding, session, process or original journal owner.
fn c11_text(value: &str, maximum: usize) -> bool {
    value.len() <= maximum && !value.contains('\0')
}
fn c11_path(value: &str) -> bool {
    c11_text(value, 4096)
        && value.starts_with('/')
        && value != "/"
        && value[1..]
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != "..")
}
#[derive(Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub(super) enum InstallOptionalString {
    #[default]
    Missing,
    Present(Option<String>),
}
// A serde field default is absence; the custom deserializer wraps even JSON
// null as Present(None). Serialization skips Missing before this is called.
fn install_optional<'de, D: Deserializer<'de>>(
    d: D,
) -> std::result::Result<InstallOptionalString, D::Error> {
    Option::<String>::deserialize(d).map(InstallOptionalString::Present)
}
impl InstallOptionalString {
    fn missing(&self) -> bool {
        matches!(self, Self::Missing)
    }
    fn valid(&self, maximum: usize) -> bool {
        match self {
            Self::Present(Some(value)) => c11_text(value, maximum),
            _ => true,
        }
    }
}
fn install_required_nullable<'de, D: Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<String>, D::Error> {
    Option::<String>::deserialize(d)
}
#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct InstallAssetsMount {
    #[serde(rename = "Type")]
    pub(super) kind: String,
    #[serde(rename = "Source")]
    pub(super) source: String,
    #[serde(rename = "Destination")]
    pub(super) destination: String,
    #[serde(rename = "RW")]
    pub(super) rw: bool,
    #[serde(
        rename = "Name",
        default,
        deserialize_with = "install_optional",
        skip_serializing_if = "InstallOptionalString::missing"
    )]
    pub(super) name: InstallOptionalString,
    #[serde(
        rename = "Driver",
        default,
        deserialize_with = "install_optional",
        skip_serializing_if = "InstallOptionalString::missing"
    )]
    pub(super) driver: InstallOptionalString,
    #[serde(
        rename = "Mode",
        default,
        deserialize_with = "install_optional",
        skip_serializing_if = "InstallOptionalString::missing"
    )]
    pub(super) mode: InstallOptionalString,
    #[serde(
        rename = "Propagation",
        default,
        deserialize_with = "install_optional",
        skip_serializing_if = "InstallOptionalString::missing"
    )]
    pub(super) propagation: InstallOptionalString,
}
#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct InstallMountProjection {
    pub(super) destination: String,
    pub(super) source: String,
    pub(super) kind: String,
    pub(super) rw: bool,
    #[serde(deserialize_with = "install_required_nullable")]
    pub(super) name: Option<String>,
}
#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct InstallContainerProjection {
    pub(super) container_id: String,
    pub(super) image_sha256: String,
    pub(super) running: bool,
    pub(super) process_id: u64,
    pub(super) started_at: String,
    pub(super) restart_count: u64,
    #[serde(deserialize_with = "install_required_nullable")]
    pub(super) project: Option<String>,
    #[serde(deserialize_with = "install_required_nullable")]
    pub(super) service: Option<String>,
    pub(super) mounts: Vec<InstallMountProjection>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InstallHostCutover {
    pub(super) container_id: String,
    pub(super) image_sha256: String,
    pub(super) export_mount: String,
    pub(super) journal_mount: String,
    pub(super) assets_mount: InstallAssetsMount,
    pub(super) legacy_producers: Vec<InstallContainerProjection>,
    pub(super) complete_container_projection: Vec<InstallContainerProjection>,
    pub(super) daemon_id: String,
    pub(super) mountinfo_sha256: String,
    pub(super) docker_cli_sha256: String,
}
impl InstallHostCutover {
    fn validate(&self) -> Result<()> {
        let assets = &self.assets_mount;
        if !hex(&self.container_id, 64)
            || !self
                .image_sha256
                .strip_prefix("sha256:")
                .is_some_and(|s| hex(s, 64))
            || !c11_path(&self.export_mount)
            || !c11_path(&self.journal_mount)
            || !c11_text(&self.daemon_id, 128)
            || !hex(&self.mountinfo_sha256, 64)
            || !hex(&self.docker_cli_sha256, 64)
            || self.complete_container_projection.is_empty()
            || self.complete_container_projection.len() > 1024
            || self.legacy_producers.len() > 1024
            || !matches!(assets.kind.as_str(), "bind" | "volume")
            || !c11_path(&assets.source)
            || !c11_path(&assets.destination)
            || assets.rw
            || !assets.name.valid(4096)
            || !assets.driver.valid(128)
            || !assets.mode.valid(256)
            || !assets.propagation.valid(128)
        {
            return Err(Error::Shape);
        }
        for rows in [&self.complete_container_projection, &self.legacy_producers] {
            if rows
                .windows(2)
                .any(|w| w[0].container_id >= w[1].container_id)
            {
                return Err(Error::Shape);
            }
            for row in rows {
                if !hex(&row.container_id, 64)
                    || !row
                        .image_sha256
                        .strip_prefix("sha256:")
                        .is_some_and(|s| hex(s, 64))
                    || !c11_text(&row.started_at, 128)
                    || row.project.as_ref().is_some_and(|s| !c11_text(s, 1024))
                    || row.service.as_ref().is_some_and(|s| !c11_text(s, 1024))
                    || row.mounts.len() > 100_000
                    || row
                        .mounts
                        .windows(2)
                        .any(|w| w[0].destination >= w[1].destination)
                {
                    return Err(Error::Shape);
                }
                for mount in &row.mounts {
                    if !c11_path(&mount.source)
                        || !c11_path(&mount.destination)
                        || !c11_text(&mount.kind, 64)
                        || mount.name.as_ref().is_some_and(|s| !c11_text(s, 4096))
                    {
                        return Err(Error::Shape);
                    }
                }
            }
        }
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InstallCutoverObservation {
    pub(super) schema_version: u64,
    pub(super) capability: String,
    pub(super) job_id: String,
    pub(super) backup_id: String,
    pub(super) phase: String,
    pub(super) request_id: String,
    pub(super) binding_id: String,
    pub(super) epoch_id: String,
    pub(super) generation_binding_sha256: String,
    pub(super) descriptor_sha256: String,
    pub(super) epoch_header_sha256: String,
    pub(super) genesis_frame_sha256: String,
    pub(super) observed_unix_ms: u64,
    pub(super) host_cutover: InstallHostCutover,
}
impl Record for InstallCutoverObservation {
    fn validate(&self) -> Result<()> {
        if self.schema_version != 1
            || self.capability != "b1_install_cutover_observation_v1"
            || self.phase != "Install"
            || ![
                &self.job_id,
                &self.backup_id,
                &self.request_id,
                &self.binding_id,
                &self.epoch_id,
            ]
            .into_iter()
            .all(|s| uuid(s, true))
            || ![
                &self.generation_binding_sha256,
                &self.descriptor_sha256,
                &self.epoch_header_sha256,
                &self.genesis_frame_sha256,
            ]
            .into_iter()
            .all(|s| hex(s, 64))
            || self.observed_unix_ms == 0
        {
            return Err(Error::Shape);
        }
        self.host_cutover.validate()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub(super) enum ControlData {
    SourceBootstrap {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
        source_start_sha256: String,
    },
    SpawnCapture {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
    },
    CaptureBootstrap {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
        source_start_sha256: String,
    },
    CaptureInit {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
        backup_id: String,
        capture_nonce: String,
        request_id: String,
    },
    CaptureStartBound {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
        backup_id: String,
        capture_start_sha256: String,
    },
    RequestInstallProof {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
        backup_id: String,
        request_id: String,
    },
    InstallProofBound {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
        backup_id: String,
        request_id: String,
        generation_binding_sha256: String,
    },
    BrokerInstalled {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
        backup_id: String,
        request_id: String,
        binding_id: String,
        epoch_id: String,
        descriptor_sha256: String,
    },
    BrokerInstallObserved {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
        backup_id: String,
        request_id: String,
        binding_id: String,
        epoch_id: String,
        descriptor_sha256: String,
        observation_sha256: String,
    },
    RequestCaptureProof {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
        backup_id: String,
        request_id: String,
    },
    CaptureProofBound {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
        backup_id: String,
        request_id: String,
        generation_binding_sha256: String,
    },
    CapturedDataReady {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
        backup_id: String,
        captured_data_sha256: String,
    },
    SettleCapture {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
        backup_id: String,
        captured_data_sha256: String,
    },
    CaptureSettled {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
        backup_id: String,
        capture_settlement_sha256: String,
    },
    Abort {
        schema_version: u64,
        job_id: String,
        root_cookie: String,
        sequence: u64,
        reason_code: String,
    },
}
impl Record for ControlData {
    fn validate(&self) -> Result<()> {
        match self {
            Self::SourceBootstrap {
                schema_version,
                job_id,
                root_cookie,
                sequence: _,
                source_start_sha256,
            } => {
                if *schema_version == 1
                    && job_id.len() <= 36
                    && pattern(
                        job_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && root_cookie.len() <= 64
                    && pattern(root_cookie, "^[0-9a-f]{64}$")
                    && source_start_sha256.len() <= 64
                    && pattern(source_start_sha256, "^[0-9a-f]{64}$")
                {
                    Ok(())
                } else {
                    Err(Error::Shape)
                }
            }
            Self::SpawnCapture {
                schema_version,
                job_id,
                root_cookie,
                sequence: _,
            } => {
                if *schema_version == 1
                    && job_id.len() <= 36
                    && pattern(
                        job_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && root_cookie.len() <= 64
                    && pattern(root_cookie, "^[0-9a-f]{64}$")
                {
                    Ok(())
                } else {
                    Err(Error::Shape)
                }
            }
            Self::CaptureBootstrap {
                schema_version,
                job_id,
                root_cookie,
                sequence: _,
                source_start_sha256,
            } => {
                if *schema_version == 1
                    && job_id.len() <= 36
                    && pattern(
                        job_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && root_cookie.len() <= 64
                    && pattern(root_cookie, "^[0-9a-f]{64}$")
                    && source_start_sha256.len() <= 64
                    && pattern(source_start_sha256, "^[0-9a-f]{64}$")
                {
                    Ok(())
                } else {
                    Err(Error::Shape)
                }
            }
            Self::CaptureInit {
                schema_version,
                job_id,
                root_cookie,
                sequence: _,
                backup_id,
                capture_nonce,
                request_id,
            } => {
                if *schema_version == 1
                    && job_id.len() <= 36
                    && pattern(
                        job_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && root_cookie.len() <= 64
                    && pattern(root_cookie, "^[0-9a-f]{64}$")
                    && backup_id.len() <= 36
                    && pattern(
                        backup_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && capture_nonce.len() <= 64
                    && pattern(capture_nonce, "^[0-9a-f]{64}$")
                    && request_id.len() <= 36
                    && pattern(
                        request_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                {
                    Ok(())
                } else {
                    Err(Error::Shape)
                }
            }
            Self::CaptureStartBound {
                schema_version,
                job_id,
                root_cookie,
                sequence: _,
                backup_id,
                capture_start_sha256,
            } => {
                if *schema_version == 1
                    && job_id.len() <= 36
                    && pattern(
                        job_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && root_cookie.len() <= 64
                    && pattern(root_cookie, "^[0-9a-f]{64}$")
                    && backup_id.len() <= 36
                    && pattern(
                        backup_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && capture_start_sha256.len() <= 64
                    && pattern(capture_start_sha256, "^[0-9a-f]{64}$")
                {
                    Ok(())
                } else {
                    Err(Error::Shape)
                }
            }
            Self::RequestInstallProof {
                schema_version,
                job_id,
                root_cookie,
                sequence: _,
                backup_id,
                request_id,
            } => {
                if *schema_version == 1
                    && job_id.len() <= 36
                    && pattern(
                        job_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && root_cookie.len() <= 64
                    && pattern(root_cookie, "^[0-9a-f]{64}$")
                    && backup_id.len() <= 36
                    && pattern(
                        backup_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && request_id.len() <= 36
                    && pattern(
                        request_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                {
                    Ok(())
                } else {
                    Err(Error::Shape)
                }
            }
            Self::InstallProofBound {
                schema_version,
                job_id,
                root_cookie,
                sequence: _,
                backup_id,
                request_id,
                generation_binding_sha256,
            } => {
                if *schema_version == 1
                    && job_id.len() <= 36
                    && pattern(
                        job_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && root_cookie.len() <= 64
                    && pattern(root_cookie, "^[0-9a-f]{64}$")
                    && backup_id.len() <= 36
                    && pattern(
                        backup_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && request_id.len() <= 36
                    && pattern(
                        request_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && generation_binding_sha256.len() <= 64
                    && pattern(generation_binding_sha256, "^[0-9a-f]{64}$")
                {
                    Ok(())
                } else {
                    Err(Error::Shape)
                }
            }
            Self::BrokerInstalled {
                schema_version,
                job_id,
                root_cookie,
                sequence: _,
                backup_id,
                request_id,
                binding_id,
                epoch_id,
                descriptor_sha256,
            }
            | Self::BrokerInstallObserved {
                schema_version,
                job_id,
                root_cookie,
                sequence: _,
                backup_id,
                request_id,
                binding_id,
                epoch_id,
                descriptor_sha256,
                ..
            } => {
                if uuid(binding_id, true)
                    && uuid(epoch_id, true)
                    && hex(descriptor_sha256, 64)
                    && match self {
                        Self::BrokerInstallObserved {
                            observation_sha256, ..
                        } => hex(observation_sha256, 64),
                        _ => true,
                    }
                    && *schema_version == 1
                    && job_id.len() <= 36
                    && pattern(
                        job_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && root_cookie.len() <= 64
                    && pattern(root_cookie, "^[0-9a-f]{64}$")
                    && backup_id.len() <= 36
                    && pattern(
                        backup_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && request_id.len() <= 36
                    && pattern(
                        request_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                {
                    Ok(())
                } else {
                    Err(Error::Shape)
                }
            }
            Self::RequestCaptureProof {
                schema_version,
                job_id,
                root_cookie,
                sequence: _,
                backup_id,
                request_id,
            } => {
                if *schema_version == 1
                    && job_id.len() <= 36
                    && pattern(
                        job_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && root_cookie.len() <= 64
                    && pattern(root_cookie, "^[0-9a-f]{64}$")
                    && backup_id.len() <= 36
                    && pattern(
                        backup_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && request_id.len() <= 36
                    && pattern(
                        request_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                {
                    Ok(())
                } else {
                    Err(Error::Shape)
                }
            }
            Self::CaptureProofBound {
                schema_version,
                job_id,
                root_cookie,
                sequence: _,
                backup_id,
                request_id,
                generation_binding_sha256,
            } => {
                if *schema_version == 1
                    && job_id.len() <= 36
                    && pattern(
                        job_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && root_cookie.len() <= 64
                    && pattern(root_cookie, "^[0-9a-f]{64}$")
                    && backup_id.len() <= 36
                    && pattern(
                        backup_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && request_id.len() <= 36
                    && pattern(
                        request_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && generation_binding_sha256.len() <= 64
                    && pattern(generation_binding_sha256, "^[0-9a-f]{64}$")
                {
                    Ok(())
                } else {
                    Err(Error::Shape)
                }
            }
            Self::CapturedDataReady {
                schema_version,
                job_id,
                root_cookie,
                sequence: _,
                backup_id,
                captured_data_sha256,
            } => {
                if *schema_version == 1
                    && job_id.len() <= 36
                    && pattern(
                        job_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && root_cookie.len() <= 64
                    && pattern(root_cookie, "^[0-9a-f]{64}$")
                    && backup_id.len() <= 36
                    && pattern(
                        backup_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && captured_data_sha256.len() <= 64
                    && pattern(captured_data_sha256, "^[0-9a-f]{64}$")
                {
                    Ok(())
                } else {
                    Err(Error::Shape)
                }
            }
            Self::SettleCapture {
                schema_version,
                job_id,
                root_cookie,
                sequence: _,
                backup_id,
                captured_data_sha256,
            } => {
                if *schema_version == 1
                    && job_id.len() <= 36
                    && pattern(
                        job_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && root_cookie.len() <= 64
                    && pattern(root_cookie, "^[0-9a-f]{64}$")
                    && backup_id.len() <= 36
                    && pattern(
                        backup_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && captured_data_sha256.len() <= 64
                    && pattern(captured_data_sha256, "^[0-9a-f]{64}$")
                {
                    Ok(())
                } else {
                    Err(Error::Shape)
                }
            }
            Self::CaptureSettled {
                schema_version,
                job_id,
                root_cookie,
                sequence: _,
                backup_id,
                capture_settlement_sha256,
            } => {
                if *schema_version == 1
                    && job_id.len() <= 36
                    && pattern(
                        job_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && root_cookie.len() <= 64
                    && pattern(root_cookie, "^[0-9a-f]{64}$")
                    && backup_id.len() <= 36
                    && pattern(
                        backup_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && capture_settlement_sha256.len() <= 64
                    && pattern(capture_settlement_sha256, "^[0-9a-f]{64}$")
                {
                    Ok(())
                } else {
                    Err(Error::Shape)
                }
            }
            Self::Abort {
                schema_version,
                job_id,
                root_cookie,
                sequence: _,
                reason_code,
            } => {
                if *schema_version == 1
                    && job_id.len() <= 36
                    && pattern(
                        job_id,
                        "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
                    )
                    && root_cookie.len() <= 64
                    && pattern(root_cookie, "^[0-9a-f]{64}$")
                    && [
                        "InputShape",
                        "ProcessChanged",
                        "RootOwnerLost",
                        "CookieMismatch",
                        "Phase",
                        "StaleInitialProof",
                        "Budget",
                        "ChildExit",
                        "UnexpectedEof",
                        "ExtraBytes",
                        "Deadline",
                        "SettlementUnconfirmed",
                    ]
                    .contains(&reason_code.as_str())
                {
                    Ok(())
                } else {
                    Err(Error::Shape)
                }
            }
        }
    }
}
impl ControlData {
    pub(super) fn kind(&self) -> &'static str {
        match self {
            Self::SourceBootstrap { .. } => "SourceBootstrap",
            Self::SpawnCapture { .. } => "SpawnCapture",
            Self::CaptureBootstrap { .. } => "CaptureBootstrap",
            Self::CaptureInit { .. } => "CaptureInit",
            Self::CaptureStartBound { .. } => "CaptureStartBound",
            Self::RequestInstallProof { .. } => "RequestInstallProof",
            Self::InstallProofBound { .. } => "InstallProofBound",
            Self::BrokerInstalled { .. } => "BrokerInstalled",
            Self::BrokerInstallObserved { .. } => "BrokerInstallObserved",
            Self::RequestCaptureProof { .. } => "RequestCaptureProof",
            Self::CaptureProofBound { .. } => "CaptureProofBound",
            Self::CapturedDataReady { .. } => "CapturedDataReady",
            Self::SettleCapture { .. } => "SettleCapture",
            Self::CaptureSettled { .. } => "CaptureSettled",
            Self::Abort { .. } => "Abort",
        }
    }
    pub(super) fn common(&self) -> (&str, &str, u64) {
        match self {
            Self::SourceBootstrap {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
            Self::SpawnCapture {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
            Self::CaptureBootstrap {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
            Self::CaptureInit {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
            Self::CaptureStartBound {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
            Self::RequestInstallProof {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
            Self::InstallProofBound {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
            Self::BrokerInstalled {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
            Self::BrokerInstallObserved {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
            Self::RequestCaptureProof {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
            Self::CaptureProofBound {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
            Self::CapturedDataReady {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
            Self::SettleCapture {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
            Self::CaptureSettled {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
            Self::Abort {
                job_id,
                root_cookie,
                sequence,
                ..
            } => (job_id, root_cookie, *sequence),
        }
    }
}

/// This is a DATA-only phase recognizer, not a Root/process qualification.
/// Per-process reservations cannot certify Root's aggregate two-channel cap.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ProcessRole {
    Source,
    Capture,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Direction {
    Receive,
    Send,
}

pub(super) struct ControlState {
    role: ProcessRole,
    step: usize,
    next_receive: u64,
    next_send: u64,
    frames: u64,
    bytes: u64,
    failed: bool,
    job_id: Option<String>,
    cookie: Option<String>,
    backup: Option<String>,
    request: Option<String>,
    captured_digest: Option<String>,
    installed: Option<(String, String, String)>,
}
impl ControlState {
    pub(super) fn new(role: ProcessRole) -> Self {
        Self {
            role,
            step: 0,
            next_receive: 0,
            next_send: 0,
            frames: 0,
            bytes: 0,
            failed: false,
            job_id: None,
            cookie: None,
            backup: None,
            request: None,
            captured_digest: None,
            installed: None,
        }
    }
    pub(super) fn is_failed(&self) -> bool {
        self.failed
    }
    fn phase(&self) -> usize {
        self.step
    }
    pub(super) fn poison(&mut self) {
        self.failed = true;
    }
    /// Reserve before allocation/IO. Failure is sticky, even at zero bytes.
    pub(super) fn reserve(&mut self, length: usize, scan: &SharedJournalBudget) -> Result<()> {
        if self.failed {
            return Err(Error::Poisoned);
        }
        if length == 0 || length > MAX_MESSAGE {
            self.failed = true;
            return Err(Error::Capacity);
        }
        self.reserve_prefix(scan)?;
        self.reserve_body(length, scan)
    }
    fn reserve_prefix(&mut self, scan: &SharedJournalBudget) -> Result<()> {
        if self.failed {
            return Err(Error::Poisoned);
        }
        self.failed = true;
        let frames = self.frames.checked_add(1).ok_or(Error::Capacity)?;
        let bytes = self.bytes.checked_add(4).ok_or(Error::Capacity)?;
        if frames > MAX_FRAMES || bytes > MAX_CONTROL_BYTES {
            return Err(Error::Capacity);
        }
        scan.charge(4, 1).map_err(|_| Error::Capacity)?;
        self.frames = frames;
        self.bytes = bytes;
        self.failed = false;
        Ok(())
    }
    fn reserve_body(&mut self, length: usize, scan: &SharedJournalBudget) -> Result<()> {
        if self.failed {
            return Err(Error::Poisoned);
        }
        self.failed = true;
        if length == 0 || length > MAX_MESSAGE {
            return Err(Error::Capacity);
        }
        let amount = u64::try_from(length).map_err(|_| Error::Capacity)?;
        let bytes = self.bytes.checked_add(amount).ok_or(Error::Capacity)?;
        if bytes > MAX_CONTROL_BYTES {
            return Err(Error::Capacity);
        }
        scan.charge(amount, 0).map_err(|_| Error::Capacity)?;
        self.bytes = bytes;
        self.failed = false;
        Ok(())
    }
    pub(super) fn observe(&mut self, data: &ControlData, direction: Direction) -> Result<()> {
        if self.failed {
            return Err(Error::Poisoned);
        }
        self.failed = true;
        data.validate()?;
        let (job, cookie, sequence) = data.common();
        let expected = match direction {
            Direction::Receive => self.next_receive,
            Direction::Send => self.next_send,
        };
        if sequence != expected {
            return Err(Error::Phase);
        }
        if self.job_id.as_deref().is_some_and(|v| v != job)
            || self.cookie.as_deref().is_some_and(|v| v != cookie)
        {
            return Err(Error::Identity);
        }
        if data.kind() == "Abort" {
            return Err(Error::RootLost);
        }
        let sequence_table: &[(Direction, &str)] = match self.role {
            ProcessRole::Source => &[
                (Direction::Receive, "SourceBootstrap"),
                (Direction::Send, "SpawnCapture"),
                (Direction::Receive, "CapturedDataReady"),
                (Direction::Send, "SettleCapture"),
                (Direction::Receive, "CaptureSettled"),
            ],
            ProcessRole::Capture => &[
                (Direction::Receive, "CaptureBootstrap"),
                (Direction::Send, "CaptureInit"),
                (Direction::Receive, "CaptureStartBound"),
                (Direction::Send, "RequestInstallProof"),
                (Direction::Receive, "InstallProofBound"),
                (Direction::Send, "BrokerInstalled"),
                (Direction::Receive, "BrokerInstallObserved"),
                (Direction::Send, "RequestCaptureProof"),
                (Direction::Receive, "CaptureProofBound"),
                (Direction::Send, "CapturedDataReady"),
                (Direction::Receive, "SettleCapture"),
            ],
        };
        if sequence_table.get(self.step) != Some(&(direction, data.kind())) {
            return Err(Error::Phase);
        }
        match data {
            ControlData::CaptureInit {
                backup_id,
                request_id,
                ..
            } => {
                self.backup = Some(backup_id.clone());
                self.request = Some(request_id.clone());
            }
            ControlData::RequestInstallProof {
                backup_id,
                request_id,
                ..
            }
            | ControlData::RequestCaptureProof {
                backup_id,
                request_id,
                ..
            } => {
                if self.backup.as_deref() != Some(backup_id) {
                    return Err(Error::Identity);
                }
                self.request = Some(request_id.clone());
            }
            ControlData::InstallProofBound {
                backup_id,
                request_id,
                ..
            }
            | ControlData::CaptureProofBound {
                backup_id,
                request_id,
                ..
            } => {
                if self.backup.as_deref() != Some(backup_id)
                    || self.request.as_deref() != Some(request_id)
                {
                    return Err(Error::Identity);
                }
            }
            ControlData::BrokerInstalled {
                backup_id,
                request_id,
                binding_id,
                epoch_id,
                descriptor_sha256,
                ..
            } => {
                if self.backup.as_deref() != Some(backup_id)
                    || self.request.as_deref() != Some(request_id)
                    || self.installed.is_some()
                {
                    return Err(Error::Identity);
                }
                self.installed = Some((
                    binding_id.clone(),
                    epoch_id.clone(),
                    descriptor_sha256.clone(),
                ));
            }
            ControlData::BrokerInstallObserved {
                backup_id,
                request_id,
                binding_id,
                epoch_id,
                descriptor_sha256,
                ..
            } => {
                let installed = self.installed.as_ref().ok_or(Error::Phase)?;
                if self.backup.as_deref() != Some(backup_id)
                    || self.request.as_deref() != Some(request_id)
                    || (&installed.0, &installed.1, &installed.2)
                        != (binding_id, epoch_id, descriptor_sha256)
                {
                    return Err(Error::Identity);
                }
            }
            ControlData::CaptureStartBound { backup_id, .. } => {
                if self.backup.as_deref() != Some(backup_id) {
                    return Err(Error::Identity);
                }
            }
            ControlData::CapturedDataReady {
                backup_id,
                captured_data_sha256,
                ..
            } => {
                if self.backup.as_deref().is_some_and(|v| v != backup_id) {
                    return Err(Error::Identity);
                }
                self.backup = Some(backup_id.clone());
                self.captured_digest = Some(captured_data_sha256.clone());
            }
            ControlData::SettleCapture {
                backup_id,
                captured_data_sha256,
                ..
            } => {
                if self.backup.as_deref() != Some(backup_id)
                    || self.captured_digest.as_deref() != Some(captured_data_sha256)
                {
                    return Err(Error::Identity);
                }
            }
            ControlData::CaptureSettled { backup_id, .. }
                if self.backup.as_deref() != Some(backup_id) =>
            {
                return Err(Error::Identity);
            }
            _ => {}
        }
        self.job_id.get_or_insert_with(|| job.to_owned());
        self.cookie.get_or_insert_with(|| cookie.to_owned());
        match direction {
            Direction::Receive => {
                self.next_receive = self.next_receive.checked_add(1).ok_or(Error::Capacity)?
            }
            Direction::Send => {
                self.next_send = self.next_send.checked_add(1).ok_or(Error::Capacity)?
            }
        }
        self.step += 1;
        self.failed = false;
        Ok(())
    }
}

/// Owns one original endpoint. No constructors are public outside this module.
/// No authority-bearing ctx can be built from ControlState or decoded records.
enum OriginalTransport {
    Source(std::os::unix::net::UnixStream),
    Capture {
        input: std::fs::File,
        output: std::fs::File,
    },
}
pub(super) struct OriginalControlOwner {
    transport: OriginalTransport,
    state: ControlState,
    scan: SharedJournalBudget,
    action_failed: std::cell::Cell<bool>,
    descriptors: Vec<DescriptorIdentity>,
}
struct DescriptorIdentity {
    dev: u64,
    ino: u64,
    mode: u32,
    uid: u32,
    gid: u32,
}
impl DescriptorIdentity {
    fn observe(fd: std::os::fd::RawFd, scan: &SharedJournalBudget) -> Result<Self> {
        use std::mem::MaybeUninit;
        scan.charge(0, 1).map_err(|_| Error::Capacity)?;
        let mut value = MaybeUninit::<libc::stat>::uninit();
        if unsafe { libc::fstat(fd, value.as_mut_ptr()) } != 0 {
            return Err(Error::Identity);
        }
        let v = unsafe { value.assume_init() };
        Ok(Self {
            dev: v.st_dev,
            ino: v.st_ino,
            mode: v.st_mode,
            uid: v.st_uid,
            gid: v.st_gid,
        })
    }
    fn same(&self, other: &Self) -> bool {
        self.dev == other.dev
            && self.ino == other.ino
            && self.mode == other.mode
            && self.uid == other.uid
            && self.gid == other.gid
    }
}
struct EnteredAction<'a>(&'a std::cell::Cell<bool>);
impl<'a> EnteredAction<'a> {
    fn enter(failed: &'a std::cell::Cell<bool>) -> Result<Self> {
        if failed.replace(true) {
            return Err(Error::Poisoned);
        }
        Ok(Self(failed))
    }
    fn complete(self) {
        self.0.set(false);
    }
}
// Dropping an entered action leaves the original owner poisoned. Drop never
// claims async cancellation/physical child stop/natural Root settlement.
impl OriginalControlOwner {
    /// Called only by the once-burned synchronous factory. No duplicated FD and
    /// no public raw handle, constructor, Clone or test success-owner exists.
    pub(super) fn take_installed(role: ProcessRole, scan: SharedJournalBudget) -> Result<Self> {
        use std::os::fd::{AsRawFd, FromRawFd};
        // This is a pre-runtime exec-isolation barrier, never pre_exec close-all.
        scan.charge(0, 1).map_err(|_| Error::Capacity)?;
        if unsafe { libc::syscall(libc::SYS_close_range, 3_u32, u32::MAX, 4_u32) } != 0 {
            return Err(Error::Identity);
        }
        let transport = match role {
            ProcessRole::Source => {
                scan.charge(0, 5).map_err(|_| Error::Capacity)?;
                super::process::mark_cloexec_nonblocking(3)?;
                OriginalTransport::Source(unsafe { std::os::unix::net::UnixStream::from_raw_fd(3) })
            }
            ProcessRole::Capture => {
                scan.charge(0, 10).map_err(|_| Error::Capacity)?;
                super::process::mark_cloexec_nonblocking(0)?;
                super::process::mark_cloexec_nonblocking(1)?;
                OriginalTransport::Capture {
                    input: unsafe { std::fs::File::from_raw_fd(0) },
                    output: unsafe { std::fs::File::from_raw_fd(1) },
                }
            }
        };
        let descriptors = match &transport {
            OriginalTransport::Source(s) => {
                vec![DescriptorIdentity::observe(s.as_raw_fd(), &scan)?]
            }
            OriginalTransport::Capture { input, output } => vec![
                DescriptorIdentity::observe(input.as_raw_fd(), &scan)?,
                DescriptorIdentity::observe(output.as_raw_fd(), &scan)?,
            ],
        };
        match role {
            ProcessRole::Source => {
                if descriptors[0].mode & libc::S_IFMT != libc::S_IFSOCK {
                    return Err(Error::Identity);
                }
            }
            ProcessRole::Capture => {
                if descriptors
                    .iter()
                    .any(|v| v.mode & libc::S_IFMT != libc::S_IFIFO)
                    || descriptors[0].ino == descriptors[1].ino
                {
                    return Err(Error::Identity);
                }
            }
        }
        let value = Self {
            transport,
            state: ControlState::new(role),
            scan,
            action_failed: std::cell::Cell::new(false),
            descriptors,
        };
        value.revalidate_transport()?;
        Ok(value)
    }
    pub(super) fn revalidate_transport(&self) -> Result<()> {
        use std::os::fd::AsRawFd;
        if self.action_failed.get() || self.state.is_failed() {
            return Err(Error::Poisoned);
        }
        let fds = match &self.transport {
            OriginalTransport::Source(s) => vec![s.as_raw_fd()],
            OriginalTransport::Capture { input, output } => {
                vec![input.as_raw_fd(), output.as_raw_fd()]
            }
        };
        for (index, fd) in fds.into_iter().enumerate() {
            self.scan.charge(0, 2).map_err(|_| Error::Capacity)?;
            super::process::require_cloexec(fd)?;
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags < 0 || flags & libc::O_NONBLOCK == 0 {
                return Err(Error::Identity);
            }
            if self.state.role == ProcessRole::Capture
                && flags & libc::O_ACCMODE
                    != (if index == 0 {
                        libc::O_RDONLY
                    } else {
                        libc::O_WRONLY
                    })
            {
                return Err(Error::Identity);
            }
            if !self.descriptors[index].same(&DescriptorIdentity::observe(fd, &self.scan)?) {
                return Err(Error::Identity);
            }
        }
        Ok(())
    }
    pub(super) fn source_facts(&self) -> Result<(u32, u64)> {
        use std::os::fd::AsRawFd;
        self.revalidate_transport()?;
        self.scan.charge(0, 5).map_err(|_| Error::Capacity)?;
        match &self.transport {
            OriginalTransport::Source(s) => super::process::source_socket_facts(s.as_raw_fd()),
            _ => Err(Error::Identity),
        }
    }
    pub(super) fn phase_data(&self) -> usize {
        self.state.phase()
    }
    pub(super) fn source_spawn_before(
        &mut self,
        total: std::time::Instant,
        send: std::time::Instant,
        stop: &std::sync::atomic::AtomicBool,
    ) -> Result<(String, String)> {
        if self.state.role != ProcessRole::Source || self.state.phase() != 1 {
            self.action_failed.set(true);
            return Err(Error::Phase);
        }
        let request = ControlData::SpawnCapture {
            schema_version: 1,
            job_id: self.state.job_id.clone().ok_or(Error::Phase)?,
            root_cookie: self.state.cookie.clone().ok_or(Error::Phase)?,
            sequence: self.state.next_send,
        };
        self.write_control_frame_before(&request, send, stop)?;
        match self.read_control_frame_before(total, stop)? {
            ControlData::CapturedDataReady {
                backup_id,
                captured_data_sha256,
                ..
            } => Ok((backup_id, captured_data_sha256)),
            _ => Err(Error::Phase),
        }
    }
    pub(super) fn source_settle_before(
        &mut self,
        total: std::time::Instant,
        stop: &std::sync::atomic::AtomicBool,
    ) -> Result<String> {
        if self.state.role != ProcessRole::Source || self.state.phase() != 3 {
            self.action_failed.set(true);
            return Err(Error::Phase);
        }
        let request = ControlData::SettleCapture {
            schema_version: 1,
            job_id: self.state.job_id.clone().ok_or(Error::Phase)?,
            root_cookie: self.state.cookie.clone().ok_or(Error::Phase)?,
            sequence: self.state.next_send,
            backup_id: self.state.backup.clone().ok_or(Error::Phase)?,
            captured_data_sha256: self.state.captured_digest.clone().ok_or(Error::Phase)?,
        };
        // One request deadline includes its write, reply and held-record join.
        self.write_control_frame_before(&request, total, stop)?;
        match self.read_control_frame_before(total, stop)? {
            ControlData::CaptureSettled {
                capture_settlement_sha256,
                ..
            } => Ok(capture_settlement_sha256),
            _ => Err(Error::Phase),
        }
    }
    pub(super) fn capture_report_before(
        &mut self,
        digest: String,
        total: std::time::Instant,
        send: std::time::Instant,
        stop: &std::sync::atomic::AtomicBool,
    ) -> Result<(String, String)> {
        if self.state.role != ProcessRole::Capture || self.state.phase() != 9 || !hex(&digest, 64) {
            self.action_failed.set(true);
            return Err(Error::Phase);
        }
        let request = ControlData::CapturedDataReady {
            schema_version: 1,
            job_id: self.state.job_id.clone().ok_or(Error::Phase)?,
            root_cookie: self.state.cookie.clone().ok_or(Error::Phase)?,
            sequence: self.state.next_send,
            backup_id: self.state.backup.clone().ok_or(Error::Phase)?,
            captured_data_sha256: digest,
        };
        self.write_control_frame_before(&request, send, stop)?;
        match self.read_control_frame_before(total, stop)? {
            ControlData::SettleCapture {
                backup_id,
                captured_data_sha256,
                ..
            } => Ok((backup_id, captured_data_sha256)),
            _ => Err(Error::Phase),
        }
    }
    fn read_control_frame_before(
        &mut self,
        deadline: std::time::Instant,
        stop: &std::sync::atomic::AtomicBool,
    ) -> Result<ControlData> {
        let entered = EnteredAction::enter(&self.action_failed)?;
        let mut prefix = [0; 4];
        self.state.reserve_prefix(&self.scan)?;
        Self::read_control_exact(&self.transport, &mut prefix, deadline, stop, &self.scan)?;
        let length = u32::from_be_bytes(prefix) as usize;
        self.state.reserve_body(length, &self.scan)?;
        let mut body = vec![0; length];
        Self::read_control_exact(&self.transport, &mut body, deadline, stop, &self.scan)?;
        let parsed = parse::<ControlData>(&body, MAX_MESSAGE)?;
        self.state.observe(&parsed, Direction::Receive)?;
        entered.complete();
        Ok(parsed)
    }
    fn write_control_frame_before(
        &mut self,
        data: &ControlData,
        deadline: std::time::Instant,
        stop: &std::sync::atomic::AtomicBool,
    ) -> Result<()> {
        let entered = EnteredAction::enter(&self.action_failed)?;
        // New control slots prepay a fixed serialization/clone upper bound
        // before this worker starts; the old paths are not retroactively changed.
        let body = canonical(data)?;
        self.state.reserve(body.len(), &self.scan)?;
        self.state.observe(data, Direction::Send)?;
        self.state.poison();
        let mut frame = u32::try_from(body.len())
            .map_err(|_| Error::Capacity)?
            .to_be_bytes()
            .to_vec();
        frame.extend_from_slice(&body);
        Self::write_control_all(&self.transport, &frame, deadline, stop, &self.scan)?;
        self.state.failed = false;
        entered.complete();
        Ok(())
    }
    fn poll_control(
        fd: std::os::fd::RawFd,
        events: i16,
        deadline: std::time::Instant,
        stop: &std::sync::atomic::AtomicBool,
        scan: &SharedJournalBudget,
    ) -> Result<()> {
        loop {
            if stop.load(std::sync::atomic::Ordering::Acquire) {
                return Err(Error::Poisoned);
            }
            if std::time::Instant::now() >= deadline {
                return Err(Error::Deadline);
            }
            scan.charge(0, 1).map_err(|_| Error::Capacity)?;
            let slice =
                deadline.min(std::time::Instant::now() + std::time::Duration::from_millis(250));
            let timeout = slice
                .saturating_duration_since(std::time::Instant::now())
                .as_millis()
                .saturating_add(1)
                .min(i32::MAX as u128) as i32;
            let mut item = libc::pollfd {
                fd,
                events,
                revents: 0,
            };
            let code = unsafe { libc::poll(&mut item, 1, timeout) };
            // EINTR returns to this charged outer loop, never an uncharged
            // native-poll retry hidden inside the legacy helper.
            if code < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
            {
                continue;
            }
            Self::control_boundary(deadline, stop)?;
            if code < 0 {
                return Err(Error::Io);
            }
            if code == 0 {
                continue;
            }
            if item.revents & libc::POLLNVAL != 0 {
                return Err(Error::Identity);
            }
            if item.revents & events != 0 {
                return Ok(());
            }
            if item.revents & (libc::POLLERR | libc::POLLHUP) != 0 {
                return Err(Error::Eof);
            }
            return Err(Error::Io);
        }
    }
    fn control_boundary(
        deadline: std::time::Instant,
        stop: &std::sync::atomic::AtomicBool,
    ) -> Result<()> {
        if stop.load(std::sync::atomic::Ordering::Acquire) {
            return Err(Error::Poisoned);
        }
        if std::time::Instant::now() >= deadline {
            return Err(Error::Deadline);
        }
        Ok(())
    }
    fn read_control_exact(
        transport: &OriginalTransport,
        mut remaining: &mut [u8],
        deadline: std::time::Instant,
        stop: &std::sync::atomic::AtomicBool,
        scan: &SharedJournalBudget,
    ) -> Result<()> {
        use std::os::fd::AsRawFd;
        let fd = match transport {
            OriginalTransport::Source(s) => s.as_raw_fd(),
            OriginalTransport::Capture { input, .. } => input.as_raw_fd(),
        };
        while !remaining.is_empty() {
            Self::poll_control(fd, libc::POLLIN, deadline, stop, scan)?;
            scan.charge(0, 1).map_err(|_| Error::Capacity)?;
            let count = unsafe { libc::read(fd, remaining.as_mut_ptr().cast(), remaining.len()) };
            Self::control_boundary(deadline, stop)?;
            if count == 0 {
                return Err(Error::Eof);
            }
            if count < 0 {
                if matches!(
                    std::io::Error::last_os_error().kind(),
                    std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                ) {
                    continue;
                }
                return Err(Error::Io);
            }
            remaining = remaining
                .get_mut(usize::try_from(count).map_err(|_| Error::Io)?..)
                .ok_or(Error::Io)?;
        }
        Self::control_boundary(deadline, stop)
    }
    fn write_control_all(
        transport: &OriginalTransport,
        mut remaining: &[u8],
        deadline: std::time::Instant,
        stop: &std::sync::atomic::AtomicBool,
        scan: &SharedJournalBudget,
    ) -> Result<()> {
        use std::os::fd::AsRawFd;
        let fd = match transport {
            OriginalTransport::Source(s) => s.as_raw_fd(),
            OriginalTransport::Capture { output, .. } => output.as_raw_fd(),
        };
        while !remaining.is_empty() {
            Self::poll_control(fd, libc::POLLOUT, deadline, stop, scan)?;
            scan.charge(0, 1).map_err(|_| Error::Capacity)?;
            let count = unsafe { libc::write(fd, remaining.as_ptr().cast(), remaining.len()) };
            Self::control_boundary(deadline, stop)?;
            if count <= 0 {
                if count < 0
                    && matches!(
                        std::io::Error::last_os_error().kind(),
                        std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                    )
                {
                    continue;
                }
                return Err(Error::Io);
            }
            remaining = remaining
                .get(usize::try_from(count).map_err(|_| Error::Io)?..)
                .ok_or(Error::Io)?;
        }
        Self::control_boundary(deadline, stop)
    }
    pub(super) fn request_install_proof(&mut self, request: &str) -> Result<String> {
        if self.state.role != ProcessRole::Capture
            || self.state.phase() != 3
            || !uuid(request, true)
        {
            return Err(Error::Phase);
        }
        let message = ControlData::RequestInstallProof {
            schema_version: 1,
            job_id: self.state.job_id.clone().ok_or(Error::Phase)?,
            root_cookie: self.state.cookie.clone().ok_or(Error::Phase)?,
            sequence: self.state.next_send,
            backup_id: self.state.backup.clone().ok_or(Error::Phase)?,
            request_id: request.to_owned(),
        };
        self.write_frame(&message)?;
        match self.read_frame()? {
            ControlData::InstallProofBound {
                generation_binding_sha256,
                ..
            } => Ok(generation_binding_sha256),
            _ => Err(Error::Phase),
        }
    }
    pub(super) fn broker_installed(
        &mut self,
        binding_id: &str,
        epoch_id: &str,
        descriptor_sha256: &str,
    ) -> Result<([u8; 32], std::time::Instant)> {
        if self.state.role != ProcessRole::Capture
            || self.state.phase() != 5
            || !uuid(binding_id, true)
            || !uuid(epoch_id, true)
            || !hex(descriptor_sha256, 64)
        {
            return Err(Error::Phase);
        }
        // One original request deadline covers both frames and the caller's
        // subsequent held-record validation; neither read nor ACK renews it.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        // New fixed message cloning/encoding/parsing work, including malformed
        // bounded replies, is paid before it occurs on the same original scan.
        self.scan
            .charge((MAX_MESSAGE * 64) as u64, 4)
            .map_err(|_| Error::Capacity)?;
        let message = ControlData::BrokerInstalled {
            schema_version: 1,
            job_id: self.state.job_id.clone().ok_or(Error::Phase)?,
            root_cookie: self.state.cookie.clone().ok_or(Error::Phase)?,
            sequence: self.state.next_send,
            backup_id: self.state.backup.clone().ok_or(Error::Phase)?,
            request_id: self.state.request.clone().ok_or(Error::Phase)?,
            binding_id: binding_id.to_owned(),
            epoch_id: epoch_id.to_owned(),
            descriptor_sha256: descriptor_sha256.to_owned(),
        };
        self.write_frame_before(&message, deadline)?;
        let reply = self.read_frame_before(deadline)?;
        if std::time::Instant::now() >= deadline {
            self.state.poison();
            return Err(Error::Deadline);
        }
        match reply {
            ControlData::BrokerInstallObserved {
                observation_sha256, ..
            } => Ok((digest_bytes(&observation_sha256)?, deadline)),
            _ => Err(Error::Phase),
        }
    }
    pub(super) fn request_capture_proof(&mut self, request: &str) -> Result<String> {
        if self.state.role != ProcessRole::Capture
            || self.state.phase() != 7
            || !uuid(request, true)
        {
            return Err(Error::Phase);
        }
        let message = ControlData::RequestCaptureProof {
            schema_version: 1,
            job_id: self.state.job_id.clone().ok_or(Error::Phase)?,
            root_cookie: self.state.cookie.clone().ok_or(Error::Phase)?,
            sequence: self.state.next_send,
            backup_id: self.state.backup.clone().ok_or(Error::Phase)?,
            request_id: request.to_owned(),
        };
        self.write_frame(&message)?;
        match self.read_frame()? {
            ControlData::CaptureProofBound {
                generation_binding_sha256,
                ..
            } => Ok(generation_binding_sha256),
            _ => Err(Error::Phase),
        }
    }
    pub(super) fn require_idle_no_extra(&self) -> Result<()> {
        use std::os::fd::AsRawFd;
        self.revalidate_transport()?;
        self.scan.charge(1, 1).map_err(|_| Error::Capacity)?;
        let mut byte = 0_u8;
        let count = match &self.transport {
            OriginalTransport::Source(s) => unsafe {
                libc::recv(
                    s.as_raw_fd(),
                    (&mut byte as *mut u8).cast(),
                    1,
                    libc::MSG_PEEK | libc::MSG_DONTWAIT,
                )
            },
            OriginalTransport::Capture { input, .. } => unsafe {
                libc::read(input.as_raw_fd(), (&mut byte as *mut u8).cast(), 1)
            },
        };
        if count < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::WouldBlock {
            return Ok(());
        }
        self.action_failed.set(true);
        if count == 0 {
            return Err(Error::RootLost);
        }
        if count > 0 {
            return Err(Error::Shape);
        }
        Err(Error::Io)
    }
    pub(super) fn read_frame(&mut self) -> Result<ControlData> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        self.read_frame_before(deadline)
    }
    fn read_frame_before(&mut self, deadline: std::time::Instant) -> Result<ControlData> {
        let entered = EnteredAction::enter(&self.action_failed)?;
        let mut prefix = [0; 4];
        // Reserve the real prefix/frame before read. A malformed length still
        // consumes that observation; no payload allocation precedes its gate.
        self.state.reserve_prefix(&self.scan)?;
        if let Err(e) = Self::read_exact(&self.transport, &mut prefix, deadline) {
            self.state.poison();
            return Err(e);
        }
        let length = u32::from_be_bytes(prefix) as usize;
        self.state.reserve_body(length, &self.scan)?;
        let mut body = vec![0; length];
        if let Err(e) = Self::read_exact(&self.transport, &mut body, deadline) {
            self.state.poison();
            return Err(e);
        }
        let parsed = match parse::<ControlData>(&body, MAX_MESSAGE) {
            Ok(v) => v,
            Err(e) => {
                self.state.poison();
                return Err(e);
            }
        };
        self.state.observe(&parsed, Direction::Receive)?;
        entered.complete();
        Ok(parsed)
    }
    pub(super) fn write_frame(&mut self, data: &ControlData) -> Result<()> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        self.write_frame_before(data, deadline)
    }
    fn write_frame_before(
        &mut self,
        data: &ControlData,
        deadline: std::time::Instant,
    ) -> Result<()> {
        let entered = EnteredAction::enter(&self.action_failed)?;
        let body = match canonical(data) {
            Ok(v) => v,
            Err(e) => {
                self.state.poison();
                return Err(e);
            }
        };
        self.state.reserve(body.len(), &self.scan)?;
        self.state.observe(data, Direction::Send)?;
        // A write begun can never be replayed, including short writes/EINTR.
        self.state.poison();
        let length = u32::try_from(body.len()).map_err(|_| Error::Capacity)?;
        let mut frame = length.to_be_bytes().to_vec();
        frame.extend_from_slice(&body);
        Self::write_all(&self.transport, &frame, deadline)?;
        self.state.failed = false;
        entered.complete();
        Ok(())
    }
    fn read_exact(
        transport: &OriginalTransport,
        mut remaining: &mut [u8],
        deadline: std::time::Instant,
    ) -> Result<()> {
        use std::os::fd::AsRawFd;
        let fd = match transport {
            OriginalTransport::Source(s) => s.as_raw_fd(),
            OriginalTransport::Capture { input, .. } => input.as_raw_fd(),
        };
        while !remaining.is_empty() {
            super::process::poll_fd(fd, libc::POLLIN, deadline)?;
            let count = unsafe { libc::read(fd, remaining.as_mut_ptr().cast(), remaining.len()) };
            if count == 0 {
                return Err(Error::Eof);
            }
            if count < 0 {
                let e = std::io::Error::last_os_error();
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                ) {
                    continue;
                }
                return Err(Error::Io);
            }
            let count = usize::try_from(count).map_err(|_| Error::Io)?;
            remaining = remaining.get_mut(count..).ok_or(Error::Io)?;
        }
        Ok(())
    }
    fn write_all(
        transport: &OriginalTransport,
        mut remaining: &[u8],
        deadline: std::time::Instant,
    ) -> Result<()> {
        use std::os::fd::AsRawFd;
        let fd = match transport {
            OriginalTransport::Source(s) => s.as_raw_fd(),
            OriginalTransport::Capture { output, .. } => output.as_raw_fd(),
        };
        while !remaining.is_empty() {
            super::process::poll_fd(fd, libc::POLLOUT, deadline)?;
            let count = unsafe { libc::write(fd, remaining.as_ptr().cast(), remaining.len()) };
            if count <= 0 {
                if count < 0
                    && matches!(
                        std::io::Error::last_os_error().kind(),
                        std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                    )
                {
                    continue;
                }
                return Err(Error::Io);
            }
            remaining = remaining
                .get(usize::try_from(count).map_err(|_| Error::Io)?..)
                .ok_or(Error::Io)?;
        }
        Ok(())
    }
}

// Tests below call the actual finite IO primitive on a local resource only;
// they never construct OriginalControlOwner, PeerOriginal or a qualified ctx.
#[cfg(test)]
pub(super) fn control_read_negative_probe(
    raw: &[u8],
    close_peer: bool,
    expired: bool,
    cancelled: bool,
    exhausted: bool,
) -> Result<(Error, u64, bool)> {
    use std::{io::Write, os::fd::AsRawFd};
    if raw.len() > 4096 {
        return Err(Error::Shape);
    }
    let (local, mut peer) = std::os::unix::net::UnixStream::pair().map_err(|_| Error::Io)?;
    super::process::mark_cloexec_nonblocking(local.as_raw_fd())?;
    peer.write_all(raw).map_err(|_| Error::Io)?;
    let peer = (!close_peer).then_some(peer);
    let scan = SharedJournalBudget::default();
    if exhausted {
        assert!(scan.charge(u64::MAX, 1).is_err());
    }
    let stop = std::sync::atomic::AtomicBool::new(cancelled);
    let deadline = if expired {
        std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(1))
            .ok_or(Error::Deadline)?
    } else {
        std::time::Instant::now() + std::time::Duration::from_secs(30)
    };
    let transport = OriginalTransport::Source(local);
    let mut prefix = [0; 4];
    let error =
        OriginalControlOwner::read_control_exact(&transport, &mut prefix, deadline, &stop, &scan)
            .err()
            .ok_or(Error::Shape)?;
    let fd = match &transport {
        OriginalTransport::Source(s) => s.as_raw_fd(),
        _ => return Err(Error::Shape),
    };
    let mut byte = [0];
    let remaining = unsafe {
        libc::recv(
            fd,
            byte.as_mut_ptr().cast(),
            1,
            libc::MSG_PEEK | libc::MSG_DONTWAIT,
        )
    } > 0;
    let entries = scan.usage().map(|u| u.charged_entries).unwrap_or(0);
    drop(peer);
    Ok((error, entries, remaining))
}
#[cfg(test)]
pub(super) fn control_late_boundary_probe() -> Result<()> {
    let stop = std::sync::atomic::AtomicBool::new(true);
    OriginalControlOwner::control_boundary(
        std::time::Instant::now() + std::time::Duration::from_secs(30),
        &stop,
    )
}

/// Exact old network-side manifest shape. `role=source` cannot select a process
/// role or grant SourcePolicy/signing authority; compile role and actual ELF do.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LaunchManifest {
    pub(super) format_version: u32,
    pub(super) capability: String,
    pub(super) profile: String,
    pub(super) launch_id: String,
    pub(super) role: String,
    pub(super) code_commit: String,
    pub(super) build_input_sha256: String,
    pub(super) binary_sha256: String,
    pub(super) launcher_sha256: String,
    pub(super) policy_sha256: String,
    pub(super) anchor_sha256: String,
    pub(super) source_auth_public_key_sha256: String,
    pub(super) destination_verifier_public_key_sha256: String,
    pub(super) transport_public_key_sha256: String,
}
impl Record for LaunchManifest {
    fn validate(&self) -> Result<()> {
        if self.format_version != 1
            || self.capability != "task9_b1_launch_manifest_v1"
            || self.profile != "task9-b1-limited-v1"
            || self.role != "source"
            || !uuid(&self.launch_id, true)
            || !hex(&self.code_commit, 40)
            || [
                &self.build_input_sha256,
                &self.binary_sha256,
                &self.launcher_sha256,
                &self.policy_sha256,
                &self.anchor_sha256,
                &self.source_auth_public_key_sha256,
                &self.destination_verifier_public_key_sha256,
                &self.transport_public_key_sha256,
            ]
            .iter()
            .any(|v| !hex(v, 64))
        {
            return Err(Error::Shape);
        }
        Ok(())
    }
}

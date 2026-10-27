//! Strict, bounded installed-record DATA shapes, never filesystem admission.
//!
//! Nothing here opens a path or key, authenticates a peer, measures an ELF,
//! registers a root, or constructs an actor/permit. The later fixed FD loader
//! must independently establish custody and recheck actual local identities.

use super::{BuildPins, Role, profile};
use crate::{
    BackupError, digest,
    registry::{canonical_record, v4},
    valid_digest, valid_hex,
};
use serde::{Deserialize, Serialize};

mod installed;

const SOURCE_POLICY_PATH: &str = "/var/lib/knowweave-c4/b1/source-policy.json";
const DESTINATION_POLICY_PATH: &str = "/var/lib/knowweave-c4/b1/destination-policy.json";
const FENCE_ROOT: &str = "/var/lib/knowweave-c4-source-fences";
const SESSION_ROOT: &str = "/var/lib/knowweave-c4-b1-sessions";

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct RegistryAnchor {
    path: String,
    dev: u64,
    ino: u64,
    authority_sha256: String,
    lock_dev: u64,
    lock_ino: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RootIdentity {
    path: String,
    dev: u64,
    ino: u64,
    uid: u32,
    mode: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceConnection {
    method: String,
    socket_dir: String,
    database: String,
    role: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TransportPolicy {
    ssh_path: String,
    host: String,
    port: u16,
    user: String,
    known_hosts_path: String,
    transport_key_path: String,
    remote_command: String,
    destination_deployment_id: String,
    destination_policy_sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourcePolicy {
    format_version: u32,
    capability: String,
    profile: String,
    role: Role,
    deployment_id: String,
    source_group_id: String,
    registry_anchor: RegistryAnchor,
    source_control_binding_sha256: String,
    source_auth_public_key_sha256: String,
    source_auth_key_path: String,
    source_fence_anchor_sha256: String,
    manager_release_sha256: String,
    manager_build_input_sha256: String,
    destination_verifier_public_key_sha256: String,
    source_connection: SourceConnection,
    transport: TransportPolicy,
    launch_id: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DestinationPolicy {
    format_version: u32,
    capability: String,
    profile: String,
    role: Role,
    deployment_id: String,
    destination_group_id: String,
    registry_anchor: RegistryAnchor,
    source_deployment_id: String,
    source_group_id: String,
    source_auth_public_key_hex: String,
    source_manager_release_sha256: String,
    source_application_build_sha256: String,
    receiver_release_sha256: String,
    session_anchor_sha256: String,
    verifier_public_key_sha256: String,
    launch_id: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FenceAnchor {
    format_version: u32,
    capability: String,
    profile: String,
    deployment_id: String,
    registry_anchor: RegistryAnchor,
    fence_root: RootIdentity,
    launch_id: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionAnchor {
    format_version: u32,
    capability: String,
    profile: String,
    deployment_id: String,
    registry_anchor: RegistryAnchor,
    session_root: RootIdentity,
    launch_id: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LaunchManifest {
    format_version: u32,
    capability: String,
    profile: String,
    launch_id: String,
    role: Role,
    code_commit: String,
    build_input_sha256: String,
    binary_sha256: String,
    launcher_sha256: String,
    policy_sha256: String,
    anchor_sha256: String,
    source_auth_public_key_sha256: String,
    destination_verifier_public_key_sha256: String,
    transport_public_key_sha256: String,
}

// No Deserialize/Clone/Debug or public constructor: these remain private parsed
// input bundles, not typed substitutes for SourceAdmission or D authority.
pub(super) struct UntrustedSourceInputs {
    policy: SourcePolicy,
    fence: FenceAnchor,
    launch: LaunchManifest,
}

pub(super) struct UntrustedDestinationInputs {
    policy: DestinationPolicy,
    session: SessionAnchor,
    launch: LaunchManifest,
}

/// Borrowed candidate policy DATA only. No installed custody, actor, nonce or
/// source application commit/current-root proof is represented by this view.
pub(super) struct CandidateDestinationSignaturePins<'a> {
    public_key_hex: &'a str,
    source_auth_sha256: &'a str,
    source_deployment: &'a str,
    source_group: &'a str,
    source_manager: &'a str,
    source_application_build: &'a str,
    destination_deployment: &'a str,
    destination_group: &'a str,
    destination_registry_authority: &'a str,
    destination_receiver: &'a str,
}
impl CandidateDestinationSignaturePins<'_> {
    pub(super) fn public_key_data(&self) -> (&str, &str) {
        (self.public_key_hex, self.source_auth_sha256)
    }
    pub(super) fn source_data(&self) -> (&str, &str, &str, &str) {
        (
            self.source_deployment,
            self.source_group,
            self.source_manager,
            self.source_application_build,
        )
    }
    pub(super) fn destination_data(&self) -> (&str, &str, &str, &str) {
        (
            self.destination_deployment,
            self.destination_group,
            self.destination_registry_authority,
            self.destination_receiver,
        )
    }
}
impl UntrustedDestinationInputs {
    pub(super) fn signature_data_pins(&self) -> CandidateDestinationSignaturePins<'_> {
        CandidateDestinationSignaturePins {
            public_key_hex: &self.policy.source_auth_public_key_hex,
            source_auth_sha256: &self.launch.source_auth_public_key_sha256,
            source_deployment: &self.policy.source_deployment_id,
            source_group: &self.policy.source_group_id,
            source_manager: &self.policy.source_manager_release_sha256,
            source_application_build: &self.policy.source_application_build_sha256,
            destination_deployment: &self.policy.deployment_id,
            destination_group: &self.policy.destination_group_id,
            destination_registry_authority: &self.policy.registry_anchor.authority_sha256,
            destination_receiver: &self.policy.receiver_release_sha256,
        }
    }
}

fn invalid() -> BackupError {
    BackupError::Invalid("offhost B1 untrusted installed record")
}

fn parse<T: serde::de::DeserializeOwned + Serialize>(
    raw: &[u8],
    cap: u64,
) -> Result<T, BackupError> {
    // Typed deserialization rejects duplicate fields at every nested level;
    // the existing canonical check rejects alternate spellings/order/suffixes.
    canonical_record(raw, cap).map_err(|_| invalid())
}

fn envelope(version: u32, capability: &str, expected: &str, profile: &str) -> bool {
    version == 1 && capability == expected && profile == profile::PROFILE
}

fn registry(anchor: &RegistryAnchor) -> bool {
    anchor.path == crate::registry::REGISTRY_PATH
        && [anchor.dev, anchor.ino, anchor.lock_dev, anchor.lock_ino]
            .iter()
            .all(|v| *v > 0)
        && valid_digest(&anchor.authority_sha256)
}

fn root(identity: &RootIdentity, path: &str) -> bool {
    identity.path == path
        && identity.dev > 0
        && identity.ino > 0
        && identity.uid == 0
        && identity.mode == 0o700
}

fn fixed_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn host(host: &str) -> bool {
    if host.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

fn connection(value: &SourceConnection) -> bool {
    value.method == "private_unix_socket_no_password"
        && value.socket_dir == "/run/knowweave-c4-source-pg"
        && fixed_name(&value.database)
        && fixed_name(&value.role)
}

fn transport(value: &TransportPolicy) -> bool {
    value.ssh_path == "/usr/bin/ssh"
        && host(&value.host)
        && value.port != 0
        && fixed_name(&value.user)
        && !value.user.starts_with('-')
        && value.known_hosts_path == "/var/lib/knowweave-c4/b1/known-hosts"
        && value.transport_key_path == "/var/lib/knowweave-c4/b1/transport-key"
        && value.remote_command == "/usr/local/libexec/knowweave-c4-b1-receiver"
        && v4(&value.destination_deployment_id)
        && valid_digest(&value.destination_policy_sha256)
}

fn launch(
    value: &LaunchManifest,
    policy_raw: &[u8],
    anchor_raw: &[u8],
    pins: &BuildPins<'_>,
) -> bool {
    envelope(
        value.format_version,
        &value.capability,
        "task9_b1_launch_manifest_v1",
        &value.profile,
    ) && value.role == pins.role
        && v4(&value.launch_id)
        && value.code_commit == pins.commit
        && value.build_input_sha256 == pins.build_input
        && value.policy_sha256 == digest(policy_raw)
        && value.anchor_sha256 == digest(anchor_raw)
        && value.source_auth_public_key_sha256 == pins.source_auth_key
        && value.destination_verifier_public_key_sha256 == pins.verifier_key
        && [
            &value.binary_sha256,
            &value.launcher_sha256,
            &value.transport_public_key_sha256,
        ]
        .iter()
        .all(|v| valid_digest(v))
}

pub(super) fn parse_source_inputs(
    policy_raw: &[u8],
    fence_raw: &[u8],
    launch_raw: &[u8],
    pins: &BuildPins<'_>,
) -> Result<UntrustedSourceInputs, BackupError> {
    let policy: SourcePolicy = parse(policy_raw, profile::CONTROL_BYTES)?;
    let fence: FenceAnchor = parse(fence_raw, profile::ANCHOR_BYTES)?;
    let manifest: LaunchManifest = parse(launch_raw, profile::CONTROL_BYTES)?;
    if pins.role != Role::Source
        || !envelope(
            policy.format_version,
            &policy.capability,
            "task9_b1_source_policy_v1",
            &policy.profile,
        )
        || policy.role != Role::Source
        || !v4(&policy.deployment_id)
        || !v4(&policy.source_group_id)
        || !registry(&policy.registry_anchor)
        || policy.source_auth_key_path != "/var/lib/knowweave-c4/b1/source-auth.pk8"
        || Some(policy.source_control_binding_sha256.as_str()) != pins.source_binding
        || policy.source_auth_public_key_sha256 != pins.source_auth_key
        || policy.destination_verifier_public_key_sha256 != pins.verifier_key
        || policy.manager_build_input_sha256 != pins.build_input
        || policy.manager_release_sha256 != manifest.binary_sha256
        || !connection(&policy.source_connection)
        || !transport(&policy.transport)
        || !envelope(
            fence.format_version,
            &fence.capability,
            "task9_b1_source_fence_anchor_v1",
            &fence.profile,
        )
        || fence.deployment_id != policy.deployment_id
        || fence.registry_anchor != policy.registry_anchor
        || !root(&fence.fence_root, FENCE_ROOT)
        || policy.source_fence_anchor_sha256 != digest(fence_raw)
        || policy.launch_id != manifest.launch_id
        || fence.launch_id != manifest.launch_id
        || !launch(&manifest, policy_raw, fence_raw, pins)
    {
        return Err(invalid());
    }
    Ok(UntrustedSourceInputs {
        policy,
        fence,
        launch: manifest,
    })
}

pub(super) fn parse_destination_inputs(
    policy_raw: &[u8],
    session_raw: &[u8],
    launch_raw: &[u8],
    pins: &BuildPins<'_>,
) -> Result<UntrustedDestinationInputs, BackupError> {
    let policy: DestinationPolicy = parse(policy_raw, profile::CONTROL_BYTES)?;
    let session: SessionAnchor = parse(session_raw, profile::ANCHOR_BYTES)?;
    let manifest: LaunchManifest = parse(launch_raw, profile::CONTROL_BYTES)?;
    if pins.role != Role::Destination
        || !envelope(
            policy.format_version,
            &policy.capability,
            "task9_b1_destination_policy_v1",
            &policy.profile,
        )
        || policy.role != Role::Destination
        || ![
            &policy.deployment_id,
            &policy.destination_group_id,
            &policy.source_deployment_id,
            &policy.source_group_id,
        ]
        .iter()
        .all(|id| v4(id))
        || !registry(&policy.registry_anchor)
        || !valid_hex(&policy.source_auth_public_key_hex, 64)
        || ![
            &policy.source_manager_release_sha256,
            &policy.source_application_build_sha256,
        ]
        .iter()
        .all(|v| valid_digest(v))
        || policy.receiver_release_sha256 != manifest.binary_sha256
        || policy.verifier_public_key_sha256 != pins.verifier_key
        || !envelope(
            session.format_version,
            &session.capability,
            "task9_b1_session_anchor_v1",
            &session.profile,
        )
        || session.deployment_id != policy.deployment_id
        || session.registry_anchor != policy.registry_anchor
        || !root(&session.session_root, SESSION_ROOT)
        || policy.session_anchor_sha256 != digest(session_raw)
        || policy.launch_id != manifest.launch_id
        || session.launch_id != manifest.launch_id
        || !launch(&manifest, policy_raw, session_raw, pins)
    {
        return Err(invalid());
    }
    let public_key = hex::decode(&policy.source_auth_public_key_hex).map_err(|_| invalid())?;
    if digest(&public_key) != pins.source_auth_key {
        return Err(invalid());
    }
    Ok(UntrustedDestinationInputs {
        policy,
        session,
        launch: manifest,
    })
}

#[cfg(test)]
mod tests;

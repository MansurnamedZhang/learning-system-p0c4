use super::*;
use crate::{
    offhost_b1::tests::{AUTH, COMMIT, HASH, pins},
    registry::canonical,
};
use serde_json::{Value, json};

const ID: &str = "11111111-1111-4111-8111-111111111111";
const OTHER: &str = "22222222-2222-4222-8222-222222222222";

fn registry_value() -> Value {
    json!({
        "path": crate::registry::REGISTRY_PATH,
        "dev": 1, "ino": 2, "authority_sha256": HASH,
        "lock_dev": 1, "lock_ino": 3,
    })
}

fn root_value(path: &str) -> Value {
    json!({"path": path, "dev": 1, "ino": 4, "uid": 0, "mode": 448})
}

fn manifest(role: Role) -> Value {
    json!({
        "format_version": 1, "capability": "task9_b1_launch_manifest_v1",
        "profile": profile::PROFILE, "launch_id": ID, "role": role,
        "code_commit": COMMIT, "build_input_sha256": HASH,
        "binary_sha256": HASH, "launcher_sha256": HASH,
        "policy_sha256": HASH, "anchor_sha256": HASH,
        "source_auth_public_key_sha256": AUTH,
        "destination_verifier_public_key_sha256": HASH,
        "transport_public_key_sha256": HASH,
    })
}

fn source_values() -> (Value, Value, Value) {
    let fence = json!({
        "format_version": 1, "capability": "task9_b1_source_fence_anchor_v1",
        "profile": profile::PROFILE, "deployment_id": ID,
        "registry_anchor": registry_value(), "fence_root": root_value(FENCE_ROOT),
        "launch_id": ID,
    });
    let policy = json!({
        "format_version": 1, "capability": "task9_b1_source_policy_v1",
        "profile": profile::PROFILE, "role": "source", "deployment_id": ID,
        "source_group_id": ID, "registry_anchor": registry_value(),
        "source_control_binding_sha256": HASH, "source_auth_public_key_sha256": AUTH,
        "source_auth_key_path": "/var/lib/knowweave-c4/b1/source-auth.pk8",
        "source_fence_anchor_sha256": HASH, "manager_release_sha256": HASH,
        "manager_build_input_sha256": HASH, "destination_verifier_public_key_sha256": HASH,
        "source_connection": {
            "method": "private_unix_socket_no_password", "socket_dir": "/run/knowweave-c4-source-pg",
            "database": "fixture_only", "role": "fixture_owner",
        },
        "transport": {
            "ssh_path": "/usr/bin/ssh", "host": "example.invalid", "port": 22,
            "user": "fixture_transport", "known_hosts_path": "/var/lib/knowweave-c4/b1/known-hosts",
            "transport_key_path": "/var/lib/knowweave-c4/b1/transport-key",
            "remote_command": "/usr/local/libexec/knowweave-c4-b1-receiver",
            "destination_deployment_id": OTHER, "destination_policy_sha256": HASH,
        },
        "launch_id": ID,
    });
    (policy, fence, manifest(Role::Source))
}

fn destination_values() -> (Value, Value, Value) {
    let session = json!({
        "format_version": 1, "capability": "task9_b1_session_anchor_v1",
        "profile": profile::PROFILE, "deployment_id": ID,
        "registry_anchor": registry_value(), "session_root": root_value(SESSION_ROOT),
        "launch_id": ID,
    });
    let policy = json!({
        "format_version": 1, "capability": "task9_b1_destination_policy_v1",
        "profile": profile::PROFILE, "role": "destination", "deployment_id": ID,
        "destination_group_id": ID, "registry_anchor": registry_value(),
        "source_deployment_id": OTHER, "source_group_id": OTHER,
        "source_auth_public_key_hex": "00".repeat(32),
        "source_manager_release_sha256": HASH, "source_application_build_sha256": HASH,
        "receiver_release_sha256": HASH, "session_anchor_sha256": HASH,
        "verifier_public_key_sha256": HASH, "launch_id": ID,
    });
    (policy, session, manifest(Role::Destination))
}

// These fixtures only produce canonical input DATA. They never install records
// or emulate a filesystem, ELF measurement, signer, actor or successful backup.
fn encode(values: (Value, Value, Value), role: Role) -> [Vec<u8>; 3] {
    let (mut policy, anchor, mut manifest) = values;
    let anchor_raw = canonical(&anchor).unwrap();
    let anchor_field = match role {
        Role::Source => "source_fence_anchor_sha256",
        Role::Destination => "session_anchor_sha256",
    };
    policy[anchor_field] = digest(&anchor_raw).into();
    let policy_raw = canonical(&policy).unwrap();
    manifest["policy_sha256"] = digest(&policy_raw).into();
    manifest["anchor_sha256"] = digest(&anchor_raw).into();
    [policy_raw, anchor_raw, canonical(&manifest).unwrap()]
}

fn source_accepts(raw: &[Vec<u8>; 3]) -> bool {
    parse_source_inputs(&raw[0], &raw[1], &raw[2], &pins(Role::Source)).is_ok()
}

fn destination_accepts(raw: &[Vec<u8>; 3]) -> bool {
    parse_destination_inputs(&raw[0], &raw[1], &raw[2], &pins(Role::Destination)).is_ok()
}

#[test]
fn canonical_bundles_are_data_only_and_role_separated() {
    let source = encode(source_values(), Role::Source);
    let destination = encode(destination_values(), Role::Destination);
    assert!(source_accepts(&source));
    assert!(destination_accepts(&destination));
    assert!(!destination_accepts(&source));
    assert!(!source_accepts(&destination));
}

#[test]
fn unknown_fields_at_top_and_nested_levels_refuse() {
    for nested in [false, true] {
        let (mut p, a, l) = source_values();
        if nested {
            p["source_connection"]["password"] = "synthetic-test-text".into();
        } else {
            p["permit"] = true.into();
        }
        assert!(!source_accepts(&encode((p, a, l), Role::Source)));
    }
}

#[test]
fn duplicates_trailing_bytes_and_noncanonical_spellings_refuse() {
    let original = encode(source_values(), Role::Source);
    let text = String::from_utf8(original[0].clone()).unwrap();
    for changed in [
        text.replacen(
            "\"format_version\":1",
            "\"format_version\":1,\"format_version\":1",
            1,
        ),
        text.replacen("\"dev\":1", "\"dev\":1,\"dev\":1", 1),
        format!("{text}\n"),
        text.replacen("\"format_version\":1", "\"format_version\":1.0", 1),
    ] {
        let mut raw = original.clone();
        raw[0] = changed.into_bytes();
        assert!(!source_accepts(&raw));
    }
}

#[test]
fn source_paths_command_socket_and_host_cannot_be_overridden() {
    for field in [
        "ssh_path",
        "remote_command",
        "transport_key_path",
        "known_hosts_path",
        "host",
    ] {
        let (mut p, a, l) = source_values();
        p["transport"][field] = "-oProxyCommand=forbidden".into();
        assert!(!source_accepts(&encode((p, a, l), Role::Source)));
    }
    let (mut p, a, l) = source_values();
    p["source_connection"]["socket_dir"] = "/tmp/other".into();
    assert!(!source_accepts(&encode((p, a, l), Role::Source)));
}

#[test]
fn root_identity_and_registry_anchor_linkage_are_exact() {
    for field in ["uid", "mode", "ino"] {
        let (p, mut a, l) = source_values();
        a["fence_root"][field] = 0.into();
        // uid=0 is the expected value; change it explicitly in this case.
        if field == "uid" {
            a["fence_root"][field] = 1.into();
        }
        assert!(!source_accepts(&encode((p, a, l), Role::Source)));
    }
    let (p, mut a, l) = source_values();
    a["registry_anchor"]["lock_ino"] = 9.into();
    assert!(!source_accepts(&encode((p, a, l), Role::Source)));
}

#[test]
fn launch_and_deployment_ids_cannot_be_substituted() {
    let (p, mut a, l) = source_values();
    a["launch_id"] = OTHER.into();
    assert!(!source_accepts(&encode((p, a, l), Role::Source)));
    let (p, mut a, l) = source_values();
    a["deployment_id"] = OTHER.into();
    assert!(!source_accepts(&encode((p, a, l), Role::Source)));
    let (mut p, a, l) = source_values();
    p["source_group_id"] = "11111111-1111-1111-8111-111111111111".into();
    assert!(!source_accepts(&encode((p, a, l), Role::Source)));
}

#[test]
fn actual_input_bytes_and_compile_pins_are_joined() {
    let original = encode(source_values(), Role::Source);
    let mut changed = original.clone();
    changed[2] = canonical(&manifest(Role::Source)).unwrap();
    assert!(!source_accepts(&changed));
    for field in [
        "manager_release_sha256",
        "source_control_binding_sha256",
        "source_auth_public_key_sha256",
    ] {
        let (mut p, a, l) = source_values();
        p[field] = "a".repeat(64).into();
        assert!(!source_accepts(&encode((p, a, l), Role::Source)));
    }
}

#[test]
fn destination_source_key_and_local_session_anchor_are_bound() {
    let (mut p, a, l) = destination_values();
    p["source_auth_public_key_hex"] = "11".repeat(32).into();
    assert!(!destination_accepts(&encode((p, a, l), Role::Destination)));
    let (p, mut a, l) = destination_values();
    a["session_root"]["path"] = FENCE_ROOT.into();
    assert!(!destination_accepts(&encode((p, a, l), Role::Destination)));
}

#[test]
fn oversized_installed_records_fail_before_deserialization() {
    let mut raw = encode(source_values(), Role::Source);
    raw[0] = vec![b' '; profile::CONTROL_BYTES as usize + 1];
    assert!(!source_accepts(&raw));
    let mut raw = encode(source_values(), Role::Source);
    raw[1] = vec![b' '; profile::ANCHOR_BYTES as usize + 1];
    assert!(!source_accepts(&raw));
}

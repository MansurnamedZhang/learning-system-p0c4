//! Public synthetic DATA only: no runtime key, installed holder or authority.
//! The fixed fixture is independently framed and signed outside product code.
use super::super::{
    Direction, HEADER, SignatureDataOperation, SyntaxPhase, inspect_data, inspect_header, schema,
};
use super::*;
use crate::offhost_b1::{BuildInputs, BuildPins, Role, policy};
use serde_json::{Value, json};

const FIXTURE: &[u8] = include_bytes!("fixtures/synthetic-b1-data-v1.json");
const FIXTURE_SHA: &str = "415af146ad6a899b0fc6227027d3be7c95b60d5c579292eba81de190589af1e4";
const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const ID: &str = "99999999-9999-4999-8999-999999999999";

fn fixture() -> Value {
    assert_eq!(crate::digest(FIXTURE), FIXTURE_SHA);
    let v: Value = serde_json::from_slice(FIXTURE).unwrap();
    assert_eq!(
        v["classification"],
        "PUBLIC_INSECURE_SYNTHETIC_DATA_NOT_RUNTIME_AUTHORITY"
    );
    assert_eq!(v["fixture_id"], "synthetic-b1-data-v1");
    v
}

fn raw_frame(case: &Value, index: usize) -> Vec<u8> {
    let bytes = hex::decode(case["frames"][index]["raw_hex"].as_str().unwrap()).unwrap();
    assert_eq!(crate::digest(&bytes), case["frames"][index]["raw_sha256"]);
    bytes
}

// Build only canonical input DATA through the real parser. These synthetic
// records are never installed and do not construct a holder or source actor.
fn policy_for(source: &Value, destination: &Value, public_hex: &str) -> UntrustedDestinationInputs {
    let authority = destination["registry_authority_sha256"].as_str().unwrap();
    let registry = json!({"path":crate::registry::REGISTRY_PATH,"dev":1,"ino":2,
        "authority_sha256":authority,"lock_dev":1,"lock_ino":3});
    let anchor = json!({"format_version":1,"capability":"task9_b1_session_anchor_v1",
        "profile":profile::PROFILE,"deployment_id":destination["deployment_id"],
        "registry_anchor":registry,"session_root":{"path":"/var/lib/knowweave-c4-b1-sessions",
        "dev":1,"ino":4,"uid":0,"mode":448},"launch_id":ID});
    let anchor_raw = crate::registry::canonical(&anchor).unwrap();
    let value = json!({"format_version":1,"capability":"task9_b1_destination_policy_v1",
        "profile":profile::PROFILE,"role":"destination",
        "deployment_id":destination["deployment_id"],"destination_group_id":destination["destination_group_id"],
        "registry_anchor":registry,"source_deployment_id":source["deployment_id"],
        "source_group_id":source["source_group_id"],"source_auth_public_key_hex":public_hex,
        "source_manager_release_sha256":source["manager_release_sha256"],
        "source_application_build_sha256":source["application_build_sha256"],
        "receiver_release_sha256":destination["receiver_release_sha256"],
        "session_anchor_sha256":crate::digest(&anchor_raw),"verifier_public_key_sha256":HASH,"launch_id":ID});
    let policy_raw = crate::registry::canonical(&value).unwrap();
    let public_sha = crate::digest(&hex::decode(public_hex).unwrap());
    let commit = "a".repeat(40);
    let launch = json!({"format_version":1,"capability":"task9_b1_launch_manifest_v1",
        "profile":profile::PROFILE,"launch_id":ID,"role":"destination","code_commit":commit,
        "build_input_sha256":HASH,"binary_sha256":destination["receiver_release_sha256"],
        "launcher_sha256":HASH,"policy_sha256":crate::digest(&policy_raw),
        "anchor_sha256":crate::digest(&anchor_raw),"source_auth_public_key_sha256":public_sha,
        "destination_verifier_public_key_sha256":HASH,"transport_public_key_sha256":HASH});
    let pins = BuildPins::parse(
        BuildInputs {
            role: Some("destination"),
            profile: Some(profile::PROFILE),
            source_auth_key: Some(&public_sha),
            verifier_key: Some(HASH),
            commit: Some(&commit),
            build_input: Some(HASH),
            source_binding: None,
        },
        Role::Destination,
    )
    .unwrap();
    policy::parse_destination_inputs(
        &policy_raw,
        &anchor_raw,
        &crate::registry::canonical(&launch).unwrap(),
        &pins,
    )
    .unwrap()
}

fn policy_for_case(f: &Value, case: &Value) -> UntrustedDestinationInputs {
    policy_for(
        &case["source"],
        &case["destination"],
        f["public_key_hex"].as_str().unwrap(),
    )
}

fn state_before(case: &Value, index: usize) -> WireSyntax {
    let mut state = WireSyntax::default();
    for i in 0..index {
        state.observe_untrusted(&raw_frame(case, i)).unwrap();
    }
    state
}

fn heads(state: &WireSyntax) -> [String; 2] {
    [
        hex::encode(state.transcript.head(Direction::SourceToDestination)),
        hex::encode(state.transcript.head(Direction::DestinationToSource)),
    ]
}

fn control_value(raw: &[u8]) -> Value {
    let body = inspect_data(&raw[HEADER..]).unwrap();
    serde_json::from_slice(body.control).unwrap()
}

// Only a negative-case mutation adapter, never the positive expected oracle.
fn changed_control(original: &[u8], value: &Value, body: &[u8]) -> Vec<u8> {
    changed_control_bytes(original, &crate::registry::canonical(value).unwrap(), body)
}
fn changed_control_bytes(original: &[u8], control: &[u8], body: &[u8]) -> Vec<u8> {
    inspect_header(&original[..HEADER]).unwrap();
    let mut payload = vec![original[HEADER]];
    payload.extend_from_slice(&u32::try_from(control.len()).unwrap().to_be_bytes());
    payload.extend_from_slice(control);
    payload.extend_from_slice(&u32::try_from(body.len()).unwrap().to_be_bytes());
    payload.extend_from_slice(body);
    // Keep the known fixture header, but do not run the malformed negative
    // payload through the product encoder before the entry under test sees it.
    let mut raw = original[..HEADER].to_vec();
    raw[16..HEADER].copy_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
    raw.extend_from_slice(&payload);
    raw
}

fn rejected_without_advancing(
    state: &mut WireSyntax,
    raw: &[u8],
    policy: &UntrustedDestinationInputs,
) -> WireError {
    let previous = (
        heads(state),
        state.next,
        state.auth_frame.clone(),
        state.source_key.clone(),
    );
    let error = state
        .observe_source_signature_data(raw, policy)
        .err()
        .expect("must reject DATA");
    assert_eq!(
        (
            heads(state),
            state.next,
            state.auth_frame.clone(),
            state.source_key.clone()
        ),
        previous
    );
    assert_eq!(state.phase, SyntaxPhase::Poisoned);
    assert_eq!(
        state.observe_untrusted(raw).err(),
        Some(WireError::Poisoned)
    );
    assert_eq!(
        state.observe_source_signature_data(raw, policy).err(),
        Some(WireError::Poisoned)
    );
    assert!(
        state
            .budget
            .charge_frame(profile::FrameKind::End, 0)
            .is_err()
    );
    error
}

#[test]
fn synthetic_signed_frames_match_independent_preimages_and_original_transcript() {
    let f = fixture();
    for case_name in ["baseline", "alternate_destination", "alternate_nonce"] {
        let case = &f["cases"][case_name];
        let policy = policy_for_case(&f, case);
        let mut state = WireSyntax::default();
        for i in 0..14 {
            let raw = raw_frame(case, i);
            let expected_heads = &case["frames"][i];
            assert_eq!(
                heads(&state),
                [
                    expected_heads["heads_before"]["s"].as_str().unwrap(),
                    expected_heads["heads_before"]["d"].as_str().unwrap()
                ]
            );
            if i == 2 || i == 9 {
                let header = inspect_header(&raw[..HEADER]).unwrap();
                let envelope = inspect_data(&raw[HEADER..]).unwrap();
                let control = schema::decode(raw[HEADER], envelope.control, header).unwrap();
                let name = if i == 2 {
                    "SourceAuth"
                } else {
                    "SourceFinalDecision"
                };
                let expected = &case["signed"][name];
                let preimage = transcript::candidate_preimage(&control).unwrap();
                assert_eq!(hex::encode(&preimage), expected["preimage_hex"]);
                assert_eq!(
                    preimage.len() as u64,
                    expected["preimage_bytes"].as_u64().unwrap()
                );
                assert_eq!(crate::digest(&preimage), expected["preimage_sha256"]);
                let checked = state.observe_source_signature_data(&raw, &policy).unwrap();
                assert_eq!(
                    checked.kind_data(),
                    if i == 2 {
                        SourceSignatureDataKind::SourceAuth
                    } else {
                        SourceSignatureDataKind::SourceFinalDecision
                    }
                );
                assert_eq!(
                    checked.public_key_sha256_data(),
                    f["public_key_sha256"].as_str().unwrap()
                );
                assert_eq!(
                    checked.preimage_sha256_data(),
                    expected["preimage_sha256"].as_str().unwrap()
                );
                assert!(matches!(
                    checked.frame_data().message,
                    UntrustedMessage::Data(_, UntrustedBody::Empty)
                ));
            } else {
                state.observe_untrusted(&raw).unwrap();
            }
            assert_eq!(
                heads(&state),
                [
                    expected_heads["heads_after"]["s"].as_str().unwrap(),
                    expected_heads["heads_after"]["d"].as_str().unwrap()
                ]
            );
        }
        assert_eq!(state.phase, SyntaxPhase::BothEndsSeen);
        assert!(state.finish_syntax().is_ok());
    }
}

#[test]
fn checked_final_after_syntax_only_auth_remains_exact_frame_data() {
    let f = fixture();
    let case = &f["cases"]["baseline"];
    let policy = policy_for_case(&f, case);
    let mut state = state_before(case, 9); // SourceAuth was observed only as untrusted syntax.
    let raw = raw_frame(case, 9);
    let checked = state.observe_source_signature_data(&raw, &policy).unwrap();
    assert_eq!(
        checked.kind_data(),
        SourceSignatureDataKind::SourceFinalDecision
    );
    assert_eq!(state.phase, SyntaxPhase::Result);
    // No authenticated-session bit, installation, original source or permit exists.
    assert_eq!(
        checked.preimage_sha256_data(),
        case["signed"]["SourceFinalDecision"]["preimage_sha256"]
            .as_str()
            .unwrap()
    );
}

#[test]
fn valid_source_auth_for_other_hello_nonce_or_destination_is_rejected() {
    let f = fixture();
    let base = &f["cases"]["baseline"];
    let policy = policy_for_case(&f, base);
    for other in ["alternate_destination", "alternate_nonce"] {
        let raw = raw_frame(&f["cases"][other], 2);
        let mut state = state_before(base, 2);
        assert!(matches!(
            rejected_without_advancing(&mut state, &raw, &policy),
            WireError::Binding | WireError::Transcript
        ));
    }
}

#[test]
fn valid_final_for_other_current_session_is_rejected() {
    let f = fixture();
    let base = &f["cases"]["baseline"];
    let policy = policy_for_case(&f, base);
    for other in ["alternate_destination", "alternate_nonce"] {
        let raw = raw_frame(&f["cases"][other], 9);
        let mut state = state_before(base, 9);
        assert!(matches!(
            rejected_without_advancing(&mut state, &raw, &policy),
            WireError::Binding | WireError::Transcript
        ));
    }
}

#[test]
fn candidate_policy_expected_source_and_destination_fields_are_not_message_claims() {
    let f = fixture();
    let base = &f["cases"]["baseline"];
    let raw = raw_frame(base, 2);
    for (source_side, field, value) in [
        (true, "deployment_id", ID),
        (true, "source_group_id", ID),
        (true, "manager_release_sha256", HASH),
        (true, "application_build_sha256", HASH),
        (false, "deployment_id", ID),
        (false, "destination_group_id", ID),
        (false, "registry_authority_sha256", HASH),
        (false, "receiver_release_sha256", HASH),
    ] {
        let mut source = base["source"].clone();
        let mut destination = base["destination"].clone();
        if source_side {
            source[field] = value.into();
        } else {
            destination[field] = value.into();
        }
        let policy = policy_for(&source, &destination, f["public_key_hex"].as_str().unwrap());
        let mut state = state_before(base, 2);
        assert_eq!(
            rejected_without_advancing(&mut state, &raw, &policy),
            WireError::Binding
        );
    }
}

#[test]
fn decoded_key_fingerprint_signature_bytes_and_hex_forms_are_bound() {
    let f = fixture();
    let base = &f["cases"]["baseline"];
    let raw = raw_frame(base, 2);
    let policy = policy_for_case(&f, base);
    let other_key = "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c";
    let other_policy = policy_for(&base["source"], &base["destination"], other_key);
    assert_eq!(
        rejected_without_advancing(&mut state_before(base, 2), &raw, &other_policy),
        WireError::Binding
    );
    let original = control_value(&raw);
    for (field, value) in [
        ("signature", "0".repeat(128)),
        ("signature", "a".repeat(126)),
        ("signature", "G".repeat(128)),
        ("source_auth_key_sha256", "0".repeat(64)),
        ("source_auth_key_sha256", "a".repeat(62)),
    ] {
        let mut changed = original.clone();
        changed[field] = value.into();
        let bad = changed_control(&raw, &changed, &[]);
        rejected_without_advancing(&mut state_before(base, 2), &bad, &policy);
    }
}

#[test]
fn fixed_source_auth_and_final_domains_cannot_reflect_signatures() {
    let f = fixture();
    let base = &f["cases"]["baseline"];
    let policy = policy_for_case(&f, base);
    for (index, other) in [(2, "SourceFinalDecision"), (9, "SourceAuth")] {
        let raw = raw_frame(base, index);
        let mut control = control_value(&raw);
        control["signature"] = base["signed"][other]["signature_hex"].clone();
        let bad = changed_control(&raw, &control, &[]);
        assert_eq!(
            rejected_without_advancing(&mut state_before(base, index), &bad, &policy),
            WireError::SignatureData
        );
    }
}

#[test]
fn source_signature_entry_checks_sequence_direction_prev_and_message_reflection() {
    let f = fixture();
    let base = &f["cases"]["baseline"];
    let policy = policy_for_case(&f, base);
    for index in [2, 9] {
        let raw = raw_frame(base, index);
        let original = control_value(&raw);
        for field in ["prev_s", "prev_d", "s_nonce", "d_nonce", "binding_sha256"] {
            let mut control = original.clone();
            control[field] = "0".repeat(64).into();
            rejected_without_advancing(
                &mut state_before(base, index),
                &changed_control(&raw, &control, &[]),
                &policy,
            );
        }
        let mut sequence = raw.clone();
        sequence[15] = sequence[15].wrapping_add(1);
        assert_eq!(
            rejected_without_advancing(&mut state_before(base, index), &sequence, &policy),
            WireError::Sequence
        );
        let mut direction = raw.clone();
        direction[6] = 2;
        rejected_without_advancing(&mut state_before(base, index), &direction, &policy);
    }
    // A correctly signed SourceAuth cannot occupy the Final slot, or vice versa.
    rejected_without_advancing(&mut state_before(base, 9), &raw_frame(base, 2), &policy);
    rejected_without_advancing(&mut state_before(base, 2), &raw_frame(base, 9), &policy);
}

#[test]
fn final_requires_original_ready_attempt_input_and_fence_links() {
    let f = fixture();
    let base = &f["cases"]["baseline"];
    let policy = policy_for_case(&f, base);
    let raw = raw_frame(base, 9);
    let original = control_value(&raw);
    for field in [
        "attempt_id",
        "proof_sha256",
        "snapshot_effects_proof_sha256",
        "snapshot_effects_input_sha256",
        "snapshot_effects_intent_sha256",
        "source_auth_key_sha256",
    ] {
        let mut changed = original.clone();
        changed[field] = if field == "attempt_id" {
            ID.into()
        } else {
            "0".repeat(64).into()
        };
        assert_eq!(
            rejected_without_advancing(
                &mut state_before(base, 9),
                &changed_control(&raw, &changed, &[]),
                &policy
            ),
            WireError::Binding
        );
    }
    for field in [
        "operation_id",
        "source_actor_binding_sha256",
        "anchor_sha256",
        "open_record_sha256",
        "protection_snapshot_sha256",
    ] {
        let mut changed = original.clone();
        changed["fence"][field] = if field == "operation_id" {
            ID.into()
        } else {
            "0".repeat(64).into()
        };
        let expected = if matches!(field, "operation_id" | "source_actor_binding_sha256") {
            WireError::Binding
        } else {
            WireError::SignatureData
        };
        assert_eq!(
            rejected_without_advancing(
                &mut state_before(base, 9),
                &changed_control(&raw, &changed, &[]),
                &policy
            ),
            expected
        );
    }
}

#[test]
fn complete_signature_entry_refuses_duplicate_unknown_noncanonical_or_body_bytes() {
    let f = fixture();
    let base = &f["cases"]["baseline"];
    let policy = policy_for_case(&f, base);
    for index in [2, 9] {
        let raw = raw_frame(base, index);
        let mut value = control_value(&raw);
        let canonical = crate::registry::canonical(&value).unwrap();
        value["authority"] = true.into();
        let unknown = changed_control(&raw, &value, &[]);
        let text = String::from_utf8(canonical.clone()).unwrap();
        let duplicate = text.replacen(
            "\"schema_version\":1",
            "\"schema_version\":1,\"schema_version\":1",
            1,
        );
        assert_ne!(duplicate, text);
        let noncanonical = format!(" {text}");
        for bad in [
            unknown,
            changed_control_bytes(&raw, duplicate.as_bytes(), &[]),
            changed_control_bytes(&raw, noncanonical.as_bytes(), &[]),
            changed_control_bytes(&raw, &canonical, b"x"),
        ] {
            rejected_without_advancing(&mut state_before(base, index), &bad, &policy);
        }
    }
}

#[test]
fn only_current_source_signed_frames_enter_and_errors_poison_original_syntax() {
    let f = fixture();
    let base = &f["cases"]["baseline"];
    let policy = policy_for_case(&f, base);
    for index in [0, 1, 3, 4, 8, 10, 12, 13] {
        rejected_without_advancing(
            &mut state_before(base, index),
            &raw_frame(base, index),
            &policy,
        );
    }
    let mut state = state_before(base, 2);
    let raw = raw_frame(base, 2);
    state.observe_source_signature_data(&raw, &policy).unwrap();
    assert_eq!(
        rejected_without_advancing(&mut state, &raw, &policy),
        WireError::Sequence
    );
}

#[test]
fn signed_data_uses_original_exhausted_or_in_flight_budget_without_reset() {
    let f = fixture();
    let base = &f["cases"]["baseline"];
    let policy = policy_for_case(&f, base);
    let raw = raw_frame(base, 2);
    let mut exhausted = state_before(base, 2);
    for _ in 2..profile::FRAMES {
        exhausted
            .budget
            .charge_frame(profile::FrameKind::End, 0)
            .unwrap();
    }
    assert_eq!(
        rejected_without_advancing(&mut exhausted, &raw, &policy),
        WireError::Budget
    );
    let mut pending = state_before(base, 2);
    pending.budget.reserve_header().unwrap();
    assert_eq!(
        rejected_without_advancing(&mut pending, &raw, &policy),
        WireError::Budget
    );
    assert!(pending.budget.finish_reserved_frame().is_err());
}

#[test]
fn rfc8032_test1_primitive_is_not_a_b1_message_or_authority() {
    let public =
        hex::decode("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a").unwrap();
    let signature = hex::decode(concat!(
        "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555f",
        "b8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
    ))
    .unwrap();
    UnparsedPublicKey::new(&ED25519, &public)
        .verify(b"", &signature)
        .unwrap();
    assert!(
        UnparsedPublicKey::new(&ED25519, &public)
            .verify(b"x", &signature)
            .is_err()
    );
    let mut changed = signature.clone();
    changed[0] ^= 1;
    assert!(
        UnparsedPublicKey::new(&ED25519, &public)
            .verify(b"", &changed)
            .is_err()
    );
}

#[test]
fn complete_signature_operation_unwind_poison_is_the_original_owner() {
    let mut syntax = WireSyntax::default();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _operation = SignatureDataOperation {
            owner: &mut syntax,
            finished: false,
        };
        panic!("synthetic untrusted operation unwind");
    }));
    assert!(result.is_err());
    assert_eq!(syntax.phase, SyntaxPhase::Poisoned);
    assert!(
        syntax
            .budget
            .charge_frame(profile::FrameKind::End, 0)
            .is_err()
    );
}

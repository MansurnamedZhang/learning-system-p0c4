//! Pure DATA fixtures only: no fixture is an authenticated actor or v2 proof.
use super::*;
use serde_json::{Value, json};

const ID: &str = "11111111-1111-4111-8111-111111111111";
const NONCE_S: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const NONCE_D: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const HEX: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const MANIFEST: &[u8] = b"opaque-manifest-not-yet-validated";
const COPY: &[u8] = b"opaque-copy-not-yet-validated";
const INTENT: &[u8] = b"opaque-intent-not-yet-validated";
const RECEIPT: &[u8] = b"{\"z\":0,\"a\":1}";

fn root(role: &str, i: usize) -> Value {
    json!({"role":role,"enrollment_id":format!("{i:08x}-1111-4111-8111-111111111111"),
        "path":format!("/example/{role}"),"dev":1,"ino":i+1,"uid":0,"mode":448,"record_sha256":HEX})
}
fn source() -> Value {
    json!({"deployment_id":ID,"source_group_id":ID,"backup_id":ID,"registry_authority_sha256":HEX,
        "registry_generation":1,"generation_sha256":HEX,"source_binding_sha256":HEX,
        "application_commit":"a".repeat(40),"application_build_sha256":HEX,"manager_release_sha256":HEX,
        "roots":[root("assets",1),root("control",2),root("local_pins",3),root("staging",4)],
        "manifest_sha256":HEX,"source_control_sha256":HEX,"captured_roster_sha256":HEX,
        "snapshot_effects_roster_sha256":HEX,"package_bytes":MANIFEST.len(),"evidence_bytes":0,
        "file_count":1,"package_roster_sha256":HEX,"source_operation_id":ID,
        "snapshot_effects_transport_index_sha256":HEX,"snapshot_effects_input_sha256":HEX})
}
fn destination() -> Value {
    json!({"deployment_id":ID,"destination_group_id":ID,"attempt_id":ID,"registry_authority_sha256":HEX,
        "registry_generation":1,"generation_sha256":HEX,"receiver_release_sha256":HEX,
        "roots":[root("destination",5),root("evidence",6)]})
}
fn metadata(direction: Direction, source_value: Value) -> Value {
    match direction {
        Direction::SourceToDestination => json!({"schema_version":1,"profile":profile::PROFILE,
            "kind":"HelloS","direction":"S>D","sequence":0,"s_nonce":NONCE_S,"source":source_value}),
        Direction::DestinationToSource => json!({"schema_version":1,"profile":profile::PROFILE,
            "kind":"HelloD","direction":"D>S","sequence":0,"s_nonce":NONCE_S,"d_nonce":NONCE_D,"destination":destination()}),
    }
}
fn raw_header(kind: Kind, direction: Direction, sequence: u64, length: u32) -> [u8; 20] {
    let mut h = [0_u8; 20];
    h[..4].copy_from_slice(b"KWB1");
    h[4] = 1;
    h[5] = kind.octet();
    h[6] = direction.octet();
    h[8..16].copy_from_slice(&sequence.to_be_bytes());
    h[16..].copy_from_slice(&length.to_be_bytes());
    h
}
fn raw_frame(kind: Kind, d: Direction, sequence: u64, payload: &[u8]) -> Vec<u8> {
    let mut raw = raw_header(kind, d, sequence, u32::try_from(payload.len()).unwrap()).to_vec();
    raw.extend_from_slice(payload);
    raw
}
fn raw_data(tag: u8, control: &[u8], body: &[u8], d: Direction, sequence: u64) -> Vec<u8> {
    let mut p = vec![tag];
    p.extend_from_slice(&u32::try_from(control.len()).unwrap().to_be_bytes());
    p.extend_from_slice(control);
    p.extend_from_slice(&u32::try_from(body.len()).unwrap().to_be_bytes());
    p.extend_from_slice(body);
    raw_frame(Kind::Data, d, sequence, &p)
}
fn hello(direction: Direction, value: Value) -> Vec<u8> {
    let bytes = registry::canonical(&metadata(direction, value)).unwrap();
    encode_frame(Kind::Metadata, direction, 0, &bytes).unwrap()
}
fn hellos_with(value: Value) -> WireSyntax {
    let mut s = WireSyntax::default();
    s.observe_untrusted(&hello(Direction::SourceToDestination, value))
        .unwrap();
    s.observe_untrusted(&hello(Direction::DestinationToSource, source()))
        .unwrap();
    s
}
fn control(s: &WireSyntax, tag: u8) -> Value {
    let names = [
        "SourceAuth",
        "AuthAccepted",
        "FileBegin",
        "FileChunk",
        "FileEnd",
        "SourceDataEnd",
        "DestinationReady",
        "SourceFinalDecision",
        "CommittedResult",
        "SourceResultAck",
    ];
    let d = if matches!(tag, 2 | 7 | 9) {
        Direction::DestinationToSource
    } else {
        Direction::SourceToDestination
    };
    let mut c = json!({"schema_version":1,"profile":profile::PROFILE,"kind":names[usize::from(tag-1)],
        "direction":d,"sequence":s.next[d.index()],"s_nonce":s.s_nonce,"d_nonce":s.d_nonce,
        "prev_s":hex::encode(s.transcript.head(Direction::SourceToDestination)),
        "prev_d":hex::encode(s.transcript.head(Direction::DestinationToSource)),"binding_sha256":s.binding});
    let extra = match tag {
        1 => {
            json!({"source":s.source,"destination":s.destination,"source_auth_key_sha256":HEX,"signature":"0".repeat(128)})
        }
        2 => json!({"source_auth_frame_sha256":s.auth_frame}),
        3 => {
            json!({"file_no":s.files,"role":"PackageManifest","roster_index":0,"size":MANIFEST.len(),"sha256":crate::digest(MANIFEST)})
        }
        4 => json!({"file_no":s.files,"offset":s.file.as_ref().map_or(0,|f|f.offset)}),
        5 => {
            let f = s.file.as_ref().unwrap();
            json!({"file_no":f.file_no,"size":f.size,"sha256":f.sha256})
        }
        6 => json!({"package_roster_sha256":HEX,"captured_roster_sha256":HEX,
            "snapshot_effects_roster_sha256":HEX,"files":s.files,"bytes":s.actual_bytes}),
        7 => {
            json!({"attempt_id":ID,"proof_sha256":HEX,"snapshot_effects_proof_sha256":crate::digest(COPY),
            "manifest_sha256":HEX,"snapshot_effects_input_sha256":HEX,"snapshot_effects_intent_sha256":crate::digest(INTENT)})
        }
        8 => {
            json!({"attempt_id":ID,"proof_sha256":HEX,"snapshot_effects_proof_sha256":crate::digest(COPY),
            "source_auth_key_sha256":HEX,"signature":"0".repeat(128),"snapshot_effects_input_sha256":HEX,
            "snapshot_effects_intent_sha256":crate::digest(INTENT),"fence":{"operation_id":ID,
                "anchor_sha256":HEX,"open_record_sha256":HEX,"protection_snapshot_sha256":HEX,
                "source_actor_binding_sha256":transcript::actor_binding(ID,&s.binding,&s.s_nonce,&s.d_nonce).unwrap()}})
        }
        9 => {
            json!({"attempt_id":ID,"proof_sha256":HEX,"receipt_sha256":crate::digest(RECEIPT),"receipt_size":RECEIPT.len()})
        }
        10 => json!({"attempt_id":ID,"receipt_sha256":crate::digest(RECEIPT)}),
        _ => unreachable!(),
    };
    c.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    c
}
fn typed(tag: u8, c: &Value) -> UntrustedControl {
    let h = Header {
        kind: Kind::Data,
        direction: serde_json::from_value(c["direction"].clone()).unwrap(),
        sequence: c["sequence"].as_u64().unwrap(),
        payload_bytes: 0,
    };
    schema::decode(tag, &registry::canonical(c).unwrap(), h).unwrap()
}
fn emit(s: &mut WireSyntax, tag: u8, body: &[u8]) {
    let c = typed(tag, &control(s, tag));
    let raw = encode_control(&c, body).unwrap();
    s.observe_untrusted(&raw).unwrap();
}
fn ready_body(witness: &[u8], intent: &[u8]) -> Vec<u8> {
    let mut body = u32::try_from(witness.len()).unwrap().to_be_bytes().to_vec();
    body.extend_from_slice(witness);
    body.extend_from_slice(&u32::try_from(intent.len()).unwrap().to_be_bytes());
    body.extend_from_slice(intent);
    body
}
fn streaming_with(src: Value) -> WireSyntax {
    let mut s = hellos_with(src);
    emit(&mut s, 1, &[]);
    emit(&mut s, 2, &[]);
    s
}
fn streamed() -> WireSyntax {
    let mut s = streaming_with(source());
    emit(&mut s, 3, &[]);
    emit(&mut s, 4, MANIFEST);
    emit(&mut s, 5, &[]);
    emit(&mut s, 6, &[]);
    s
}
fn ready() -> WireSyntax {
    let mut s = streamed();
    emit(&mut s, 7, &ready_body(COPY, INTENT));
    s
}
fn raw_control(c: &Value, tag: u8, body: &[u8]) -> Vec<u8> {
    raw_data(
        tag,
        &registry::canonical(c).unwrap(),
        body,
        serde_json::from_value(c["direction"].clone()).unwrap(),
        c["sequence"].as_u64().unwrap(),
    )
}
fn rejects_and_stays_poisoned(s: &mut WireSyntax, raw: &[u8]) {
    assert!(s.observe_untrusted(raw).is_err());
    assert_eq!(
        s.observe_untrusted(&hello(Direction::SourceToDestination, source()))
            .err(),
        Some(WireError::Poisoned)
    );
}

#[test]
fn fixed_header_fields_and_preallocation_bounds() {
    let h = raw_header(Kind::Data, Direction::SourceToDestination, 0, 1);
    assert!(inspect_header(&h).is_ok());
    for i in [0, 4, 5, 6, 7] {
        let mut bad = h;
        bad[i] = 255;
        assert_eq!(inspect_header(&bad).err(), Some(WireError::Header));
    }
    assert_eq!(inspect_header(&h[..19]).err(), Some(WireError::Header));
    assert_eq!(inspect_header(&[0; 21]).err(), Some(WireError::Header));
    for (kind, cap) in [
        (Kind::Metadata, 16384),
        (Kind::Data, 65536),
        (Kind::Abort, 256),
        (Kind::End, 0),
    ] {
        assert!(inspect_header(&raw_header(kind, Direction::SourceToDestination, 0, cap)).is_ok());
        assert_eq!(
            inspect_header(&raw_header(
                kind,
                Direction::SourceToDestination,
                0,
                cap + 1
            ))
            .err(),
            Some(WireError::Length)
        );
    }
    for kind in [Kind::Metadata, Kind::Data] {
        assert_eq!(
            inspect_header(&raw_header(kind, Direction::SourceToDestination, 0, 0)).err(),
            Some(WireError::Length)
        );
    }
}

#[test]
fn truncation_suffix_and_sequence_errors_are_terminal() {
    let h = hello(Direction::SourceToDestination, source());
    for raw in [
        h[..19].to_vec(),
        h[..h.len() - 1].to_vec(),
        [h.as_slice(), b"x"].concat(),
    ] {
        rejects_and_stays_poisoned(&mut WireSyntax::default(), &raw);
    }
    let mut skipped = h.clone();
    skipped[15] = 1;
    assert_eq!(
        WireSyntax::default().observe_untrusted(&skipped).err(),
        Some(WireError::Sequence)
    );
    let mut s = WireSyntax::default();
    s.next[0] = u64::MAX;
    let raw = raw_frame(Kind::Abort, Direction::SourceToDestination, u64::MAX, &[]);
    assert_eq!(s.observe_untrusted(&raw).err(), Some(WireError::Sequence));
}

#[test]
fn each_direction_has_one_hello_and_no_early_end() {
    let mut s = hellos_with(source());
    rejects_and_stays_poisoned(&mut s, &hello(Direction::DestinationToSource, source()));
    rejects_and_stays_poisoned(
        &mut WireSyntax::default(),
        &raw_frame(Kind::End, Direction::SourceToDestination, 0, &[]),
    );
    let mut wrong = metadata(Direction::SourceToDestination, source());
    wrong["direction"] = json!("D>S");
    let raw = raw_frame(
        Kind::Metadata,
        Direction::SourceToDestination,
        0,
        &registry::canonical(&wrong).unwrap(),
    );
    rejects_and_stays_poisoned(&mut WireSyntax::default(), &raw);
}

#[test]
fn abort_is_only_the_empty_supported_subset() {
    for d in [
        Direction::SourceToDestination,
        Direction::DestinationToSource,
    ] {
        let mut s = WireSyntax::default();
        let raw = encode_frame(Kind::Abort, d, 0, &[]).unwrap();
        s.observe_untrusted(&raw).unwrap();
        assert_eq!(s.phase, SyntaxPhase::Aborted);
        assert!(s.finish_syntax().is_ok());
        assert_eq!(s.observe_untrusted(&raw).err(), Some(WireError::Closed));
    }
    let raw = raw_frame(Kind::Abort, Direction::SourceToDestination, 0, &[0]);
    assert_eq!(
        WireSyntax::default().observe_untrusted(&raw).err(),
        Some(WireError::Unsupported)
    );
    assert_eq!(
        encode_frame(Kind::Abort, Direction::SourceToDestination, 0, &[0]).err(),
        Some(WireError::Unsupported)
    );
}

#[test]
fn envelope_lengths_tags_and_suffixes_are_exact() {
    for tag in [0, 11, 255] {
        assert_eq!(
            inspect_data(&[tag, 0, 0, 0, 1, b'x', 0, 0, 0, 0]).err(),
            Some(WireError::Unsupported)
        );
    }
    let raw = raw_data(2, b"{}", &[], Direction::DestinationToSource, 1);
    let p = &raw[20..];
    assert!(inspect_data(p).is_ok());
    for cut in 0..p.len() {
        assert!(inspect_data(&p[..cut]).is_err());
    }
    let mut bad = p.to_vec();
    bad.push(0);
    assert!(inspect_data(&bad).is_err());
    let mut bad = p.to_vec();
    bad[1..5].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(inspect_data(&bad).is_err());
    let mut bad = p.to_vec();
    bad[7..11].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(inspect_data(&bad).is_err());
    for (tag, cap) in [(2, 16384), (4, 1024)] {
        let raw = raw_data(
            tag,
            &vec![b' '; cap],
            &[],
            Direction::SourceToDestination,
            1,
        );
        assert!(inspect_data(&raw[20..]).is_ok());
        let raw = raw_data(
            tag,
            &vec![b' '; cap + 1],
            &[],
            Direction::SourceToDestination,
            1,
        );
        assert_eq!(inspect_data(&raw[20..]).err(), Some(WireError::Length));
    }
}

#[test]
fn typed_records_reject_duplicate_unknown_and_missing_keys() {
    let h = hello(Direction::SourceToDestination, source());
    let canonical = std::str::from_utf8(&h[20..]).unwrap();
    let variants = [
        canonical.replacen(
            "\"schema_version\":1",
            "\"schema_version\":1,\"schema_version\":1",
            1,
        ),
        canonical.replacen("\"mode\":448", "\"mode\":448,\"mode\":448", 1),
        canonical.replacen("\"mode\":448", "\"mode\":448,\"unknown\":0", 1),
        canonical.replacen("\"schema_version\":1,", "", 1),
        canonical.replacen("\"kind\"", "\"ki\\u006ed\"", 1),
    ];
    for v in variants {
        rejects_and_stays_poisoned(
            &mut WireSyntax::default(),
            &raw_frame(
                Kind::Metadata,
                Direction::SourceToDestination,
                0,
                v.as_bytes(),
            ),
        );
    }
}

#[test]
fn canonical_json_rejects_bom_whitespace_float_bool_and_overflow() {
    let raw = registry::canonical(&metadata(Direction::SourceToDestination, source())).unwrap();
    for bad in [
        [b"\xef\xbb\xbf".as_slice(), &raw].concat(),
        [raw.as_slice(), b"\n"].concat(),
        [b" ".as_slice(), &raw].concat(),
    ] {
        rejects_and_stays_poisoned(
            &mut WireSyntax::default(),
            &raw_frame(Kind::Metadata, Direction::SourceToDestination, 0, &bad),
        );
    }
    let s = std::str::from_utf8(&raw).unwrap();
    for n in ["true", "1.0", "1e0", "-1", "18446744073709551616"] {
        let bad = s.replace(
            "\"registry_generation\":1",
            &format!("\"registry_generation\":{n}"),
        );
        rejects_and_stays_poisoned(
            &mut WireSyntax::default(),
            &raw_frame(
                Kind::Metadata,
                Direction::SourceToDestination,
                0,
                bad.as_bytes(),
            ),
        );
    }
}

#[test]
fn roots_have_exact_roles_uuid_and_path_bounds() {
    for mutate in 0..6 {
        let mut src = source();
        match mutate {
            0 => src["roots"][1]["role"] = json!("assets"),
            1 => src["roots"][1]["enrollment_id"] = src["roots"][0]["enrollment_id"].clone(),
            2 => src["roots"][0]["path"] = json!("/example/../assets"),
            3 => src["roots"][0]["uid"] = json!(1000),
            4 => src["backup_id"] = json!("11111111-1111-1111-8111-111111111111"),
            5 => src["application_build_sha256"] = json!("A".repeat(64)),
            _ => unreachable!(),
        }
        let raw = raw_frame(
            Kind::Metadata,
            Direction::SourceToDestination,
            0,
            &registry::canonical(&metadata(Direction::SourceToDestination, src)).unwrap(),
        );
        rejects_and_stays_poisoned(&mut WireSyntax::default(), &raw);
    }
    let mut src = source();
    src["roots"][0]["path"] = json!(format!("/{}", "a".repeat(1023)));
    assert!(hello(Direction::SourceToDestination, src.clone()).len() < 16384);
    src["roots"][0]["path"] = json!(format!("/{}", "a".repeat(1024)));
    let h = raw_frame(
        Kind::Metadata,
        Direction::SourceToDestination,
        0,
        &registry::canonical(&metadata(Direction::SourceToDestination, src)).unwrap(),
    );
    assert_eq!(
        WireSyntax::default().observe_untrusted(&h).err(),
        Some(WireError::Fields)
    );
}

#[test]
fn controls_keep_exact_tag_direction_sequence_and_nested_fields() {
    let s = hellos_with(source());
    let base = control(&s, 1);
    for (name, value) in [
        ("kind", json!("FileBegin")),
        ("direction", json!("D>S")),
        ("signature", json!("0".repeat(127))),
        ("extra", json!(0)),
    ] {
        let mut c = base.clone();
        c[name] = value;
        assert!(
            schema::decode(
                1,
                &registry::canonical(&c).unwrap(),
                Header {
                    kind: Kind::Data,
                    direction: Direction::SourceToDestination,
                    sequence: 1,
                    payload_bytes: 0
                }
            )
            .is_err()
        );
    }
    let mut c = base.clone();
    c["sequence"] = json!(2);
    let raw = raw_control(&c, 1, &[]);
    assert_eq!(
        hellos_with(source()).observe_untrusted(&raw).err(),
        Some(WireError::Sequence)
    );
    let c = control(&ready(), 8);
    let raw = String::from_utf8(registry::canonical(&c).unwrap())
        .unwrap()
        .replace(
            "\"anchor_sha256\":",
            &format!("\"anchor_sha256\":\"{HEX}\",\"anchor_sha256\":"),
        );
    let h = Header {
        kind: Kind::Data,
        direction: Direction::SourceToDestination,
        sequence: c["sequence"].as_u64().unwrap(),
        payload_bytes: 0,
    };
    assert_eq!(
        schema::decode(8, raw.as_bytes(), h).err(),
        Some(WireError::Json)
    );
}

#[test]
fn nonce_binding_prefix_and_hello_substitution_fail_closed() {
    for field in ["s_nonce", "d_nonce", "binding_sha256", "prev_s", "prev_d"] {
        let mut s = hellos_with(source());
        let mut c = control(&s, 1);
        c[field] = json!("f".repeat(64));
        rejects_and_stays_poisoned(&mut s, &raw_control(&c, 1, &[]));
    }
    let mut s = hellos_with(source());
    let mut c = control(&s, 1);
    c["source"]["manifest_sha256"] = json!("f".repeat(64));
    rejects_and_stays_poisoned(&mut s, &raw_control(&c, 1, &[]));
    let mut s = hellos_with(source());
    emit(&mut s, 1, &[]);
    let mut c = control(&s, 2);
    c["source_auth_frame_sha256"] = json!(HEX);
    rejects_and_stays_poisoned(&mut s, &raw_control(&c, 2, &[]));
}

#[test]
fn wrong_phase_does_not_authenticate_or_commit_anything() {
    let mut s = hellos_with(source());
    let c = control(&s, 3);
    assert_eq!(
        s.observe_untrusted(&raw_control(&c, 3, &[])).err(),
        Some(WireError::Phase)
    );
    let mut s = streaming_with(source());
    let c = control(&s, 7);
    assert_eq!(
        s.observe_untrusted(&raw_control(&c, 7, &ready_body(COPY, INTENT)))
            .err(),
        Some(WireError::Phase)
    );
    let mut s = streamed();
    let c = control(&s, 9);
    assert_eq!(
        s.observe_untrusted(&raw_control(&c, 9, RECEIPT)).err(),
        Some(WireError::Phase)
    );
}

#[test]
fn file_stream_checks_single_open_number_offset_and_actual_hash() {
    for failure in 0..5 {
        let mut s = streaming_with(source());
        emit(&mut s, 3, &[]);
        let (mut c, tag, body) = match failure {
            0 => (control(&s, 3), 3, &[][..]),
            1..=3 => (control(&s, 4), 4, MANIFEST),
            4 => (control(&s, 5), 5, &[][..]),
            _ => unreachable!(),
        };
        match failure {
            1 => c["file_no"] = json!(1),
            2 => c["offset"] = json!(1),
            3 => c["offset"] = json!(u64::MAX),
            _ => {}
        }
        rejects_and_stays_poisoned(&mut s, &raw_control(&c, tag, body));
    }
    let mut s = streaming_with(source());
    emit(&mut s, 3, &[]);
    let bytes = vec![b'x'; MANIFEST.len()];
    emit(&mut s, 4, &bytes);
    let c = control(&s, 5);
    assert_eq!(
        s.observe_untrusted(&raw_control(&c, 5, &[])).err(),
        Some(WireError::File)
    );
}

#[test]
fn chunk_body_bounds_and_forbidden_control_bodies() {
    let s = streaming_with(source());
    let c = typed(4, &control(&s, 4));
    assert!(parse_body(&c, &[0; 61440]).is_ok());
    for size in [0, 61441] {
        assert_eq!(
            parse_body(&c, &vec![0; size]).err(),
            Some(WireError::Length)
        );
    }
    let c = typed(1, &control(&hellos_with(source()), 1));
    assert_eq!(parse_body(&c, b"x").err(), Some(WireError::Length));
}

#[test]
fn empty_file_needs_no_chunk_and_duplicate_roster_is_rejected() {
    let mut src = source();
    src["file_count"] = json!(3);
    src["package_bytes"] = json!(MANIFEST.len());
    let mut s = streaming_with(src);
    emit(&mut s, 3, &[]);
    emit(&mut s, 4, MANIFEST);
    emit(&mut s, 5, &[]);
    let mut c = control(&s, 3);
    c["role"] = json!("PackageMember");
    c["roster_index"] = json!(1);
    c["size"] = json!(0);
    c["sha256"] = json!(crate::digest(&[]));
    s.observe_untrusted(&raw_control(&c, 3, &[])).unwrap();
    emit(&mut s, 5, &[]);
    let mut c = control(&s, 3);
    c["role"] = json!("PackageMember");
    c["roster_index"] = json!(1);
    c["size"] = json!(0);
    c["sha256"] = json!(crate::digest(&[]));
    assert_eq!(
        s.observe_untrusted(&raw_control(&c, 3, &[])).err(),
        Some(WireError::File)
    );
}

#[test]
fn source_data_end_matches_actual_counts_and_declared_class_totals() {
    for field in ["files", "bytes", "package_roster_sha256"] {
        let mut s = streaming_with(source());
        emit(&mut s, 3, &[]);
        emit(&mut s, 4, MANIFEST);
        emit(&mut s, 5, &[]);
        let mut c = control(&s, 6);
        c[field] = if field.ends_with("sha256") {
            json!("f".repeat(64))
        } else {
            json!(999)
        };
        assert_eq!(
            s.observe_untrusted(&raw_control(&c, 6, &[])).err(),
            Some(WireError::File)
        );
    }
    let mut s = streaming_with(source());
    let c = control(&s, 6);
    assert_eq!(
        s.observe_untrusted(&raw_control(&c, 6, &[])).err(),
        Some(WireError::File)
    );
    let mut src = source();
    src["package_bytes"] = json!(MANIFEST.len() - 1);
    let mut s = streaming_with(src);
    let c = control(&s, 3);
    assert_eq!(
        s.observe_untrusted(&raw_control(&c, 3, &[])).err(),
        Some(WireError::File)
    );
}

#[test]
fn ready_records_use_sixteen_kib_each_not_signed_witness_eight_kib() {
    let s = streamed();
    let mut c = control(&s, 7);
    let witness = [b'w'; 16384];
    let intent = [b'i'; 16384];
    c["snapshot_effects_proof_sha256"] = json!(crate::digest(&witness));
    c["snapshot_effects_intent_sha256"] = json!(crate::digest(&intent));
    let t = typed(7, &c);
    let raw = ready_body(&witness, &intent);
    let UntrustedBody::Ready {
        witness: w,
        intent: i,
    } = parse_body(&t, &raw).unwrap()
    else {
        panic!("body shape");
    };
    assert_eq!(w.as_ptr(), raw[4..].as_ptr());
    assert_eq!(w.len(), 16384);
    assert_eq!(i.len(), 16384);
    assert!(encode_control(&t, &raw).unwrap().len() <= 65536 + 20);
    for (w, i) in [
        (vec![0; 16385], vec![0; 1]),
        (vec![0; 1], vec![0; 16385]),
        (vec![], vec![0; 1]),
        (vec![0; 1], vec![]),
    ] {
        assert_eq!(
            parse_body(&t, &ready_body(&w, &i)).err(),
            Some(WireError::Length)
        );
    }
    let mut extra = raw.clone();
    extra.push(0);
    assert_eq!(parse_body(&t, &extra).err(), Some(WireError::Length));
    assert_eq!(
        parse_body(&t, &raw[..raw.len() - 1]).err(),
        Some(WireError::Length)
    );
    let mut bad = raw.clone();
    bad[4] ^= 1;
    assert_eq!(parse_body(&t, &bad).err(), Some(WireError::Binding));
}

#[test]
fn receipt_body_is_borrowed_exact_bytes_with_eight_kib_cap() {
    let s = ready();
    let mut c = control(&s, 9);
    for n in [1, 8192] {
        let body = vec![b'r'; n];
        c["receipt_size"] = json!(n);
        c["receipt_sha256"] = json!(crate::digest(&body));
        let t = typed(9, &c);
        let UntrustedBody::Receipt(raw) = parse_body(&t, &body).unwrap() else {
            panic!("body shape");
        };
        assert_eq!(raw.as_ptr(), body.as_ptr());
    }
    let t = typed(9, &control(&s, 9));
    assert_eq!(parse_body(&t, &[]).err(), Some(WireError::Length));
    assert_eq!(parse_body(&t, &[0; 8193]).err(), Some(WireError::Length));
    let UntrustedBody::Receipt(raw) = parse_body(&t, RECEIPT).unwrap() else {
        panic!("body shape");
    };
    assert_eq!(raw, RECEIPT); // Deliberately opaque/non-v2 fixture, NEVER a proof.
    let mut bad = RECEIPT.to_vec();
    bad[0] ^= 1;
    assert_eq!(parse_body(&t, &bad).err(), Some(WireError::Binding));
}

#[test]
fn final_and_result_bind_this_ready_attempt_and_negative_fence_data() {
    for field in [
        "attempt_id",
        "proof_sha256",
        "snapshot_effects_intent_sha256",
        "source_auth_key_sha256",
    ] {
        let mut s = ready();
        let mut c = control(&s, 8);
        c[field] = if field == "attempt_id" {
            json!("22222222-2222-4222-8222-222222222222")
        } else {
            json!("f".repeat(64))
        };
        rejects_and_stays_poisoned(&mut s, &raw_control(&c, 8, &[]));
    }
    let mut s = ready();
    let mut c = control(&s, 8);
    c["fence"]["source_actor_binding_sha256"] = json!(HEX);
    assert_eq!(
        s.observe_untrusted(&raw_control(&c, 8, &[])).err(),
        Some(WireError::Binding)
    );
    let mut s = ready();
    emit(&mut s, 8, &[]);
    let mut c = control(&s, 9);
    c["proof_sha256"] = json!("f".repeat(64));
    assert_eq!(
        s.observe_untrusted(&raw_control(&c, 9, RECEIPT)).err(),
        Some(WireError::Binding)
    );
}

#[test]
fn complete_syntax_is_only_data_and_requires_ack_then_two_ends() {
    let mut s = ready();
    emit(&mut s, 8, &[]);
    emit(&mut s, 9, RECEIPT);
    emit(&mut s, 10, &[]);
    assert_eq!(s.phase, SyntaxPhase::SourceEnd);
    let end = encode_frame(Kind::End, Direction::SourceToDestination, s.next[0], &[]).unwrap();
    s.observe_untrusted(&end).unwrap();
    assert_eq!(s.phase, SyntaxPhase::DestinationEnd);
    let end = encode_frame(Kind::End, Direction::DestinationToSource, s.next[1], &[]).unwrap();
    s.observe_untrusted(&end).unwrap();
    assert!(s.finish_syntax().is_ok());
    assert_eq!(s.phase, SyntaxPhase::BothEndsSeen);
    assert_eq!(s.observe_untrusted(&end).err(), Some(WireError::Closed));
    // No actor/key/FD/clock exists in this fixture: not real EOF or success.
    let mut incomplete = ready();
    assert_eq!(
        incomplete.finish_syntax().err(),
        Some(WireError::Incomplete)
    );
    assert_eq!(incomplete.finish_syntax().err(), Some(WireError::Poisoned));
}

#[test]
fn source_end_closes_source_direction_including_abort() {
    for (kind, payload) in [
        (Kind::Metadata, b"{}".as_slice()),
        (Kind::Data, b"x".as_slice()),
        (Kind::End, b"".as_slice()),
        (Kind::Abort, b"".as_slice()),
    ] {
        let mut s = ready();
        emit(&mut s, 8, &[]);
        emit(&mut s, 9, RECEIPT);
        emit(&mut s, 10, &[]);
        let end = encode_frame(Kind::End, Direction::SourceToDestination, s.next[0], &[]).unwrap();
        s.observe_untrusted(&end).unwrap();
        assert_eq!(s.phase, SyntaxPhase::DestinationEnd);

        let forbidden = raw_frame(kind, Direction::SourceToDestination, s.next[0], payload);
        assert_eq!(
            s.observe_untrusted(&forbidden).err(),
            Some(WireError::Closed)
        );
        assert_eq!(s.phase, SyntaxPhase::Poisoned);
        let destination_end =
            encode_frame(Kind::End, Direction::DestinationToSource, s.next[1], &[]).unwrap();
        assert_eq!(
            s.observe_untrusted(&destination_end).err(),
            Some(WireError::Poisoned)
        );
        assert_eq!(s.finish_syntax().err(), Some(WireError::Poisoned));
    }
}

#[test]
fn result_ack_and_end_direction_cannot_be_reordered() {
    let mut s = ready();
    emit(&mut s, 8, &[]);
    emit(&mut s, 9, RECEIPT);
    let mut ack = control(&s, 10);
    ack["receipt_sha256"] = json!(HEX);
    assert_eq!(
        s.observe_untrusted(&raw_control(&ack, 10, &[])).err(),
        Some(WireError::Binding)
    );
    let mut s = ready();
    emit(&mut s, 8, &[]);
    emit(&mut s, 9, RECEIPT);
    emit(&mut s, 10, &[]);
    let early = encode_frame(Kind::End, Direction::DestinationToSource, s.next[1], &[]).unwrap();
    assert_eq!(s.observe_untrusted(&early).err(), Some(WireError::Phase));
}

fn fill_wire(budget: &mut profile::SessionBudget, mut bytes: u64) {
    let full = profile::DATA_BYTES + profile::HEADER_BYTES;
    while bytes > full {
        budget
            .charge_frame(profile::FrameKind::Data, profile::DATA_BYTES)
            .unwrap();
        bytes -= full;
    }
    if bytes > 0 {
        assert!(bytes >= profile::HEADER_BYTES);
        let payload = bytes - profile::HEADER_BYTES;
        budget
            .charge_frame(
                if payload == 0 {
                    profile::FrameKind::End
                } else {
                    profile::FrameKind::Data
                },
                payload,
            )
            .unwrap();
    }
}

#[test]
fn both_directions_share_exact_wire_budget_without_large_payload_fixture() {
    let mut s = WireSyntax::default();
    let h = hello(Direction::SourceToDestination, source());
    fill_wire(&mut s.budget, profile::WIRE_BYTES - h.len() as u64);
    s.observe_untrusted(&h).unwrap();
    let d = hello(Direction::DestinationToSource, source());
    assert_eq!(s.observe_untrusted(&d).err(), Some(WireError::Budget));
    assert_eq!(s.observe_untrusted(&d).err(), Some(WireError::Poisoned));
}

#[test]
fn frame_count_budget_is_shared_and_includes_metadata_headers() {
    let mut s = WireSyntax::default();
    for _ in 0..profile::FRAMES - 1 {
        s.budget.charge_frame(profile::FrameKind::End, 0).unwrap();
    }
    s.observe_untrusted(&hello(Direction::SourceToDestination, source()))
        .unwrap();
    assert_eq!(
        s.observe_untrusted(&hello(Direction::DestinationToSource, source()))
            .err(),
        Some(WireError::Budget)
    );
}

#[test]
fn file_reservations_use_original_shared_budget_without_reset_or_big_allocation() {
    let mut s = streaming_with(source());
    s.budget
        .charge_file(
            profile::FileClass::Package,
            profile::PACKAGE_BYTES - MANIFEST.len() as u64,
        )
        .unwrap();
    emit(&mut s, 3, &[]); // Exact shared package cap; no 256MiB fixture allocated.
    let mut s = streaming_with(source());
    s.budget
        .charge_file(profile::FileClass::Package, profile::PACKAGE_BYTES)
        .unwrap();
    let c = control(&s, 3);
    assert_eq!(
        s.observe_untrusted(&raw_control(&c, 3, &[])).err(),
        Some(WireError::Budget)
    );
    let mut s = streaming_with(source());
    for _ in 0..profile::FILES {
        s.budget
            .charge_file(profile::FileClass::Evidence, 0)
            .unwrap();
    }
    let c = control(&s, 3);
    assert_eq!(
        s.observe_untrusted(&raw_control(&c, 3, &[])).err(),
        Some(WireError::Budget)
    );
    let mut budget = profile::SessionBudget::default();
    budget
        .charge_file(profile::FileClass::Evidence, profile::EVIDENCE_BYTES)
        .unwrap();
    assert!(budget.charge_file(profile::FileClass::Evidence, 1).is_err());
}

#[test]
fn fixed_transcript_vectors_and_full_frame_hash_use_original_raw_bytes() {
    let t = transcript::CandidateTranscript::default();
    assert_ne!(
        t.head(Direction::SourceToDestination),
        t.head(Direction::DestinationToSource)
    );
    assert_eq!(
        hex::encode(t.head(Direction::SourceToDestination)),
        "ff4dd0cf9eda6c8c5ea64dee07b4df65d406a0ed538d40a30042cad5eaed3dc9"
    );
    assert_eq!(
        hex::encode(t.head(Direction::DestinationToSource)),
        "258779b99d0dd74ec499193b0fb682208e4efeb38367d89309b7607aac1df404"
    );
    let header = raw_header(Kind::Data, Direction::SourceToDestination, 7, 3);
    let mut vector = transcript::CandidateTranscript::default();
    vector
        .observe_untrusted(Direction::SourceToDestination, &header, b"abc")
        .unwrap();
    assert_eq!(
        hex::encode(vector.head(Direction::SourceToDestination)),
        "d2d7fb9c58832b1bf7b3aeecbda31fa1b84b01183d30be051ea687221b32d9d5"
    );
    assert_eq!(
        vector.head(Direction::DestinationToSource),
        t.head(Direction::DestinationToSource)
    );
    let mut s = hellos_with(source());
    let c = typed(1, &control(&s, 1));
    let raw = encode_control(&c, &[]).unwrap();
    let expected = crate::digest(&raw);
    s.observe_untrusted(&raw).unwrap();
    assert_eq!(s.auth_frame, expected);
    let after = s.transcript.head(Direction::SourceToDestination);
    let mut other = hellos_with(source());
    let mut c = control(&other, 1);
    c["signature"] = json!("1".repeat(128));
    let raw = raw_control(&c, 1, &[]);
    other.observe_untrusted(&raw).unwrap();
    assert_ne!(after, other.transcript.head(Direction::SourceToDestination));
}

#[test]
fn unsigned_preimages_have_fixed_domains_and_no_signature_self_cycle() {
    let s = hellos_with(source());
    let c = typed(1, &control(&s, 1));
    let a = transcript::candidate_preimage(&c).unwrap();
    assert!(a.starts_with(b"KnowWeave/Task9/B1/source-auth/v1\0"));
    let suffix = &a[b"KnowWeave/Task9/B1/source-auth/v1\0".len()..];
    let v: Value = serde_json::from_slice(suffix).unwrap();
    assert!(v.get("signature").is_none());
    let mut changed = control(&s, 1);
    changed["signature"] = json!("f".repeat(128));
    assert_eq!(
        a,
        transcript::candidate_preimage(&typed(1, &changed)).unwrap()
    );
    let final_control = typed(8, &control(&ready(), 8));
    let f = transcript::candidate_preimage(&final_control).unwrap();
    assert!(f.starts_with(b"KnowWeave/Task9/B1/source-final/v1\0"));
    assert_ne!(a, f);
    assert_eq!(
        transcript::candidate_preimage(&typed(2, &control(&streaming_with(source()), 2))).err(),
        Some(WireError::Unsupported)
    );
}

#[test]
fn actor_binding_uses_exact_domain_and_all_four_data_fields() {
    let a = transcript::actor_binding(ID, HEX, NONCE_S, NONCE_D).unwrap();
    assert_eq!(
        a,
        "56c5a04c776ec2a402c78a36878bf556cc8d5b4134b6645aa1b2b0a444c543fd"
    );
    assert_ne!(
        a,
        transcript::actor_binding(ID, HEX, NONCE_D, NONCE_S).unwrap()
    );
    assert_ne!(a,crate::digest(&registry::canonical(&json!({"source_operation_id":ID,"binding_sha256":HEX,"s_nonce":NONCE_S,"d_nonce":NONCE_D})).unwrap()));
    assert_ne!(
        a,
        transcript::actor_binding(
            "22222222-2222-4222-8222-222222222222",
            HEX,
            NONCE_S,
            NONCE_D
        )
        .unwrap()
    );
}

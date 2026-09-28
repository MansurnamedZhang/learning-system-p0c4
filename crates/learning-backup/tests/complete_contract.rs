use learning_backup::{DestinationStatement, VerifierTrustConfig, verify_destination_witness};
use ring::{
    rand::SystemRandom,
    signature::{Ed25519KeyPair, KeyPair},
};
use uuid::Uuid;

fn statement() -> DestinationStatement {
    DestinationStatement {
        format_version: 1,
        backup_id: Uuid::new_v4(),
        manifest_sha256: "a".repeat(64),
        source_control_sha256: "b".repeat(64),
        source_root: "/private/source".into(),
        source_control_root: "/private/control".into(),
        source_host_id: Uuid::new_v4(),
        source_storage_id: Uuid::new_v4(),
        destination_host_id: Uuid::new_v4(),
        destination_storage_id: Uuid::new_v4(),
        destination_root: "/mnt/remote-knowweave-backups".into(),
    }
}

#[test]
fn signed_witness_is_bound_to_exact_manifest_control_and_destination() {
    let original = statement();
    let key = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
    let pair = Ed25519KeyPair::from_pkcs8(key.as_ref()).unwrap();
    let signature = pair.sign(&original.canonical_bytes().unwrap());
    verify_destination_witness(&original, pair.public_key().as_ref(), signature.as_ref()).unwrap();
    for altered in [
        DestinationStatement {
            manifest_sha256: "c".repeat(64),
            ..original.clone()
        },
        DestinationStatement {
            source_control_sha256: "d".repeat(64),
            ..original.clone()
        },
        DestinationStatement {
            source_root: "/other/source".into(),
            ..original.clone()
        },
        DestinationStatement {
            destination_storage_id: Uuid::new_v4(),
            ..original.clone()
        },
    ] {
        assert!(
            verify_destination_witness(&altered, pair.public_key().as_ref(), signature.as_ref())
                .is_err()
        );
    }
}

#[test]
fn same_host_or_storage_and_invalid_key_are_refused() {
    let mut value = statement();
    value.destination_host_id = value.source_host_id;
    assert!(value.canonical_bytes().is_err());
    value = statement();
    value.destination_storage_id = value.source_storage_id;
    assert!(value.canonical_bytes().is_err());
    value = statement();
    assert!(verify_destination_witness(&value, &[0u8; 32], &[0u8; 64]).is_err());
}

#[test]
fn verifier_config_rejects_local_identity_and_noncanonical_key_or_paths() {
    let statement = statement();
    let mut config = VerifierTrustConfig {
        format_version: 1,
        source_root: "/backup/source".into(),
        source_control_root: "/backup/control".into(),
        destination_root: statement.destination_root.clone(),
        source_host_id: statement.source_host_id,
        source_storage_id: statement.source_storage_id,
        destination_host_id: statement.destination_host_id,
        destination_storage_id: statement.destination_storage_id,
        verifier_public_key_hex: "a".repeat(64),
    };
    assert!(config.canonical_bytes().is_ok());
    config.destination_host_id = config.source_host_id;
    assert!(config.canonical_bytes().is_err());
    config.destination_host_id = statement.destination_host_id;
    config.verifier_public_key_hex = "A".repeat(64);
    assert!(config.canonical_bytes().is_err());
    config.verifier_public_key_hex = "a".repeat(64);
    config.destination_root = "/backup/../backup/destination".into();
    assert!(config.canonical_bytes().is_err());
}

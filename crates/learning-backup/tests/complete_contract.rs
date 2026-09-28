use learning_backup::{DestinationStatement, VerifierTrustConfig, verify_destination_witness};
use ring::{
    rand::SystemRandom,
    signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair},
};
use uuid::Uuid;

fn statement() -> DestinationStatement {
    DestinationStatement {
        format_version: 2,
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

fn fixed_invalid_key_statement() -> DestinationStatement {
    let mut value = statement();
    value.backup_id = Uuid::from_u128(5);
    value.source_host_id = Uuid::from_u128(10);
    value.source_storage_id = Uuid::from_u128(11);
    value.destination_host_id = Uuid::from_u128(12);
    value.destination_storage_id = Uuid::from_u128(13);
    value
}

#[test]
fn signed_witness_is_bound_to_exact_manifest_control_and_destination() {
    let original = statement();
    let rng = SystemRandom::new();
    let key = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
    let pair =
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, key.as_ref(), &rng).unwrap();
    let signature = pair
        .sign(&rng, &original.canonical_bytes().unwrap())
        .unwrap();
    verify_destination_witness(&original, pair.public_key().as_ref(), signature.as_ref()).unwrap();
    let mut changed_key = pair.public_key().as_ref().to_vec();
    changed_key[64] ^= 1;
    assert!(verify_destination_witness(&original, &changed_key, signature.as_ref()).is_err());
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
    // Fixed IDs from the reproduced Ed25519 weak-key failure. The new v2
    // protocol must reject zero/invalid P-256 points deterministically.
    value = fixed_invalid_key_statement();
    assert!(verify_destination_witness(&value, &[0u8; 65], &[0u8; 64]).is_err());
    value.format_version = 1;
    assert!(value.canonical_bytes().is_err());
    value.format_version = 2;
    let mut invalid_curve_point = [0u8; 65];
    invalid_curve_point[0] = 0x04;
    assert!(verify_destination_witness(&value, &invalid_curve_point, &[0u8; 64]).is_err());
    assert!(verify_destination_witness(&value, &[0u8; 33], &[0u8; 64]).is_err());
}

#[test]
fn verifier_config_rejects_local_identity_and_noncanonical_key_or_paths() {
    let statement = statement();
    let mut config = VerifierTrustConfig {
        format_version: 2,
        source_root: "/backup/source".into(),
        source_control_root: "/backup/control".into(),
        destination_root: statement.destination_root.clone(),
        source_host_id: statement.source_host_id,
        source_storage_id: statement.source_storage_id,
        destination_host_id: statement.destination_host_id,
        destination_storage_id: statement.destination_storage_id,
        verifier_public_key_hex: format!("04{}", "a".repeat(128)),
    };
    assert!(config.canonical_bytes().is_ok());
    config.destination_host_id = config.source_host_id;
    assert!(config.canonical_bytes().is_err());
    config.destination_host_id = statement.destination_host_id;
    config.verifier_public_key_hex = format!("04{}", "A".repeat(128));
    assert!(config.canonical_bytes().is_err());
    config.verifier_public_key_hex = format!("04{}", "a".repeat(128));
    config.destination_root = "/backup/../backup/destination".into();
    assert!(config.canonical_bytes().is_err());
}

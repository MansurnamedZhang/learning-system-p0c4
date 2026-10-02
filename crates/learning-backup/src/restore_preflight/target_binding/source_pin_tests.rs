//! Portable behavior tests of the same identity decoder used by Linux acquire.
use super::*;
use serde_json::json;
use std::{cell::RefCell, rc::Rc};

const TARGET_BATCH: &str = "550e8400-e29b-41d4-a716-446655440000";
const SOURCE_BATCH: &str = "550e8400-e29b-41d4-a716-446655440002";
const TARGET_PIN: &str = "2f2f34dfb0382c75686d3dfab460c639758349d3556707e707cb4ddef271f2f1";
const SOURCE_PIN: &str = "a296f3c923e5acc85d78ed17fc10011bf98765eabf799d2d4267f2b3ce6a8954";
const FIXTURE_PINS: GuardBirthPins = GuardBirthPins {
    target: Some(TARGET_PIN),
    source: Some(SOURCE_PIN),
};

struct Records {
    config: RestorePreflightConfig,
    target_path: PathBuf,
    root_path: PathBuf,
    birth: Vec<u8>,
    state: Vec<u8>,
    success: Value,
    evidence: Value,
    precreation: Vec<u8>,
}

impl Records {
    fn fixture(batch: &str, pin: &str) -> Self {
        let original = include_str!("../../../tests/fixtures/c4_birth_python.json");
        let birth = original.replace(TARGET_BATCH, batch).into_bytes();
        let project = format!("learning-system-p0c4-restore-{batch}");
        let database = format!("learning_restore_c4_{batch}");
        let volume = format!("{project}_pg");
        let root_path = PathBuf::from("/private/controlled-import/batches/fixture/source-control");
        let target_path = root_path.join("targets").join(batch);
        let state = json!({"state":"CREATED_QUARANTINED","batch_id":batch,
            "project":project,"database":database,"network":format!("{project}_test"),
            "subnet":"10.251.219.0/24","container_id":"a".repeat(64),
            "network_id":"b".repeat(64),"volume_name":volume,
            "volume_mountpoint":"/var/lib/docker/volumes/new/_data"});
        let success = json!({"format_version":1,"state":"BIRTH_ISSUED_NOT_RESTORE_ACCEPTANCE",
            "batch_id":batch,"project_name":project,"database_name":database,"birth_sha256":pin,
            "container_id":"a".repeat(64),"network_id":"b".repeat(64),"pg_volume_name":volume,
            "image_id":format!("sha256:{}", "d".repeat(64)),
            "volume_mountpoint":"/var/lib/docker/volumes/new/_data","volume_mount_dev":9,"volume_mount_ino":10});
        let evidence = json!({"docker_daemon_id":"DAEMON","container_id":"a".repeat(64),
            "network_id":"b".repeat(64),"image_id":format!("sha256:{}", "d".repeat(64)),
            "volume_name":volume,"volume_mountpoint":"/var/lib/docker/volumes/new/_data","birth_sha256":pin});
        let precreation = json!({"format_version":1,"state":"PIN_ONLY_PRECREATION_NOT_RESTORE_AUTHORITY",
            "target_absent_at_precreation":true,"batch_id":batch,"project":project,
            "database":database,"network":format!("{project}_test"),"volume":volume,
            "subnet":"10.251.219.0/24","root_path":root_path.to_string_lossy(),
            "root_dev":11,"root_ino":12,"targets_dev":13,"targets_ino":14,
            "docker_daemon_id":"DAEMON","initdb_path":"/private/tools/initdb.sh","initdb_sha256":"e".repeat(64)});
        Self {
            config: RestorePreflightConfig {
                destination_root: target_path.join("destination"),
                trust_path: target_path.join("trust.json"),
                control_root: target_path.join("control"),
                asset_root: target_path.join("assets"),
                expected_database: database,
            },
            target_path,
            root_path,
            birth,
            state: serde_json::to_vec(&state).unwrap(),
            success,
            evidence,
            precreation: serde_json::to_vec(&precreation).unwrap(),
        }
    }

    fn load(&self, role: GuardBirthRole) -> Result<(DockerClaim, PinPrecreation), BackupError> {
        self.load_with_pins(role, FIXTURE_PINS)
    }

    fn load_with_pins(
        &self,
        role: GuardBirthRole,
        pins: GuardBirthPins,
    ) -> Result<(DockerClaim, PinPrecreation), BackupError> {
        load_guard_identity(
            &self.config,
            role,
            pins,
            &self.target_path,
            |record| {
                Ok(match record {
                    GuardRecord::Birth => self.birth.clone(),
                    GuardRecord::State => self.state.clone(),
                    GuardRecord::Success => serde_json::to_vec(&self.success)?,
                    GuardRecord::Evidence => serde_json::to_vec(&self.evidence)?,
                    GuardRecord::Precreation => self.precreation.clone(),
                })
            },
            || Ok(false),
        )
    }
}

#[test]
fn fixed_source_role_acquires_its_distinct_identity_and_retains_both_locks() {
    // Replacing source selection with target selection must reject here.
    #[derive(Debug)]
    struct Lease(&'static str, Rc<RefCell<Vec<&'static str>>>);
    impl Drop for Lease {
        fn drop(&mut self) {
            self.1.borrow_mut().push(self.0);
        }
    }
    for (role, batch, pin) in [
        (GuardBirthRole::Target, TARGET_BATCH, TARGET_PIN),
        (GuardBirthRole::ImportSource, SOURCE_BATCH, SOURCE_PIN),
    ] {
        let records = Records::fixture(batch, pin);
        let events = Rc::new(RefCell::new(Vec::new()));
        let acquired = acquire_bound_guard(
            || {
                events.borrow_mut().push("global");
                Ok(Lease("drop-global", events.clone()))
            },
            || {
                events.borrow_mut().push("target");
                Ok(Lease("drop-target", events.clone()))
            },
            || {
                events.borrow_mut().push("identity");
                let (claim, precreation) = records.load(role)?;
                validate_precreation(
                    &precreation,
                    &claim,
                    &records.root_path,
                    (11, 12),
                    (13, 14),
                    &[
                        ".restore-target.lock".into(),
                        "pin-precreation.json".into(),
                        "targets".into(),
                    ],
                    &[batch.into()],
                )?;
                Ok((claim, json!({"restart_count":0})))
            },
        );
        assert!(
            acquired.is_ok(),
            "role should accept its own fixed birth: {acquired:?}"
        );
        let guard = acquired.unwrap();
        assert_eq!(guard.claim.database, format!("learning_restore_c4_{batch}"));
        assert_eq!(guard.claim.container_id, "a".repeat(64));
        assert_eq!(guard.claim.mount_ino, 10);
        assert_eq!(*events.borrow(), ["global", "target", "identity"]);
        drop(guard);
        assert_eq!(
            *events.borrow(),
            ["global", "target", "identity", "drop-target", "drop-global"]
        );
    }
}

#[test]
fn fixed_roles_reject_crossed_births_and_release_acquired_locks() {
    struct Lease(Rc<RefCell<usize>>);
    impl Drop for Lease {
        fn drop(&mut self) {
            *self.0.borrow_mut() += 1;
        }
    }
    for (role, batch, pin) in [
        (GuardBirthRole::Target, SOURCE_BATCH, SOURCE_PIN),
        (GuardBirthRole::ImportSource, TARGET_BATCH, TARGET_PIN),
    ] {
        let records = Records::fixture(batch, pin);
        let dropped = Rc::new(RefCell::new(0));
        let result = acquire_bound_guard(
            || Ok(Lease(dropped.clone())),
            || Ok(Lease(dropped.clone())),
            || records.load(role).map(|(claim, _)| (claim, json!({}))),
        );
        assert!(matches!(
            result,
            Err(BackupError::Invalid(
                "target birth attestation is not pinned"
            ))
        ));
        assert_eq!(*dropped.borrow(), 2);
    }
}

#[test]
fn source_and_target_fixed_pins_flow_into_issuance_and_evidence_comparisons() {
    for (role, batch, pin, crossed_pin) in [
        (GuardBirthRole::Target, TARGET_BATCH, TARGET_PIN, SOURCE_PIN),
        (
            GuardBirthRole::ImportSource,
            SOURCE_BATCH,
            SOURCE_PIN,
            TARGET_PIN,
        ),
    ] {
        let mut records = Records::fixture(batch, pin);
        records.success["birth_sha256"] = crossed_pin.into();
        assert!(matches!(
            records.load(role),
            Err(BackupError::Invalid("target issuance state differs"))
        ));
        records.success["birth_sha256"] = pin.into();
        records.evidence["birth_sha256"] = crossed_pin.into();
        assert!(matches!(
            records.load(role),
            Err(BackupError::Invalid("bound issuance evidence differs"))
        ));
        records.evidence["birth_sha256"] = pin.into();
        records.evidence["container_id"] = "c".repeat(64).into();
        assert!(matches!(
            records.load(role),
            Err(BackupError::Invalid("bound issuance evidence differs"))
        ));
    }
}

#[test]
fn fixed_role_does_not_fall_back_to_the_other_compile_pin() {
    let missing_target = GuardBirthPins {
        target: None,
        source: Some(SOURCE_PIN),
    };
    let missing_source = GuardBirthPins {
        target: Some(TARGET_PIN),
        source: None,
    };
    let records = Records::fixture(TARGET_BATCH, TARGET_PIN);
    assert!(
        records
            .load_with_pins(GuardBirthRole::Target, missing_target)
            .is_err()
    );
    assert!(
        records
            .load_with_pins(GuardBirthRole::ImportSource, missing_source)
            .is_err()
    );
}

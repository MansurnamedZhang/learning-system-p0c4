use learning_backup::{GateInspection, GatePhase, PgDumpSpec, SourceGateRecord};
use uuid::Uuid;

#[test]
fn dump_argv_is_fixed_and_never_accepts_user_options() {
    let database = "learning_backup_c4_task3_550e8400-e29b-41d4-a716-446655440000";
    let spec = PgDumpSpec::new(database, "pg", 5432).unwrap();
    assert_eq!(
        spec.args(),
        [
            "--format=custom",
            "--no-password",
            "--lock-wait-timeout=5000",
            "--host=pg",
            "--port=5432",
            "--username=learning_admin",
            "--dbname=learning_backup_c4_task3_550e8400-e29b-41d4-a716-446655440000",
        ]
    );
    assert!(PgDumpSpec::new("--dbname=other", "pg", 5432).is_err());
    assert!(PgDumpSpec::new("learning_backup_c4_task3_9fd7", "pg", 5432).is_err());
    assert!(PgDumpSpec::new(database, "pg --file=/tmp/x", 5432).is_err());
    assert!(PgDumpSpec::new(database, "pg", 0).is_err());
}

#[test]
fn source_gate_requires_durable_proofs_before_release_and_never_mints_complete() {
    let id = Uuid::new_v4();
    let mut record = SourceGateRecord::new(id);
    assert_eq!(record.phase(), GatePhase::Intent);
    assert!(record.advance(GatePhase::Released, None).is_err());
    record.advance(GatePhase::Closed, None).unwrap();
    assert!(record.advance(GatePhase::Released, None).is_err());
    record.advance(GatePhase::Drained, None).unwrap();
    assert!(
        record
            .advance(GatePhase::DumpAndIndexDurable, None)
            .is_err()
    );
    record
        .advance(GatePhase::DumpAndIndexDurable, Some(&"a".repeat(64)))
        .unwrap();
    assert!(record.advance(GatePhase::Released, None).is_err());
    assert!(
        record
            .advance(GatePhase::PinsDurable, Some("wrong"))
            .is_err()
    );
    record
        .advance(GatePhase::PinsDurable, Some(&"b".repeat(64)))
        .unwrap();
    assert!(record.advance(GatePhase::Released, None).is_err());
    record.advance(GatePhase::ReleaseReady, None).unwrap();
    record.advance(GatePhase::Released, None).unwrap();
    assert_eq!(record.phase(), GatePhase::Released);
    assert!(record.advance(GatePhase::Closed, None).is_err());
}

#[test]
fn mismatched_backup_id_or_unknown_phase_is_rejected() {
    let record = SourceGateRecord::new(Uuid::new_v4());
    assert!(record.validate_for(Uuid::new_v4()).is_err());
    let mut value = serde_json::to_value(&record).unwrap();
    value["phase"] = serde_json::json!("complete");
    assert!(serde_json::from_value::<SourceGateRecord>(value).is_err());
}

#[test]
fn gate_inspection_refuses_open_runtime_or_any_other_database_session() {
    let safe = GateInspection {
        admin_is_database_owner: true,
        runtime_can_connect: false,
        public_can_connect: false,
        runtime_can_inherit_admin: false,
        runtime_is_privileged: false,
        other_sessions: 0,
        other_login_writers: 0,
    };
    assert!(safe.validate().is_ok());
    assert!(
        GateInspection {
            runtime_can_connect: true,
            ..safe
        }
        .validate()
        .is_err()
    );
    assert!(
        GateInspection {
            public_can_connect: true,
            ..safe
        }
        .validate()
        .is_err()
    );
    assert!(
        GateInspection {
            other_sessions: 1,
            ..safe
        }
        .validate()
        .is_err()
    );
    assert!(
        GateInspection {
            other_login_writers: 1,
            ..safe
        }
        .validate()
        .is_err()
    );
    assert!(
        GateInspection {
            admin_is_database_owner: false,
            ..safe
        }
        .validate()
        .is_err()
    );
    assert!(
        GateInspection {
            runtime_can_inherit_admin: true,
            ..safe
        }
        .validate()
        .is_err()
    );
    assert!(
        GateInspection {
            runtime_is_privileged: true,
            ..safe
        }
        .validate()
        .is_err()
    );
}

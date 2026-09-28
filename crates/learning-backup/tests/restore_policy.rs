use learning_backup::MigrationRecord;
use learning_backup::{
    AssetRow, BackupError, BackupPlan, ExternalEffectFinding, JobRecoveryAction, PgRestoreSpec,
    RestoreEnvironment, RestoreTargetFacts, RestoredJob, SourceIdentity, classify_restored_job,
    validate_restored_asset_bytes, validate_restored_assets, validate_role_recipe,
};
use uuid::Uuid;

fn identity() -> SourceIdentity {
    SourceIdentity::from_migrations(
        "a".repeat(64),
        "b".repeat(40),
        18,
        vec![MigrationRecord {
            version: 1,
            checksum_hex: "c".repeat(96),
        }],
    )
    .unwrap()
}

fn asset(id: u128, sha: &str) -> AssetRow {
    AssetRow {
        space_id: Uuid::from_u128(1),
        id: Uuid::from_u128(id),
        sha256: sha.into(),
        byte_size: 3,
        storage_key: format!("sha256/{}/{}", &sha[..2], sha),
    }
}

#[test]
fn restore_identity_requires_same_build_pg_and_complete_migration_set() {
    let source = identity();
    let environment = RestoreEnvironment {
        application_build_sha256: "a".repeat(64),
        application_commit: "b".repeat(40),
        postgres_major: 18,
        migrations: vec![MigrationRecord {
            version: 1,
            checksum_hex: "c".repeat(96),
        }],
    };
    environment.validate(&source).unwrap();
    let mut other = environment.clone();
    other.application_build_sha256 = "d".repeat(64);
    assert!(other.validate(&source).is_err());
    let mut other = environment.clone();
    other.postgres_major = 17;
    assert!(other.validate(&source).is_err());
    let mut other = environment;
    other.migrations[0].checksum_hex = "e".repeat(96);
    assert!(other.validate(&source).is_err());
}

#[test]
fn dirty_or_public_target_is_rejected_before_restore_writes() {
    let clean = RestoreTargetFacts {
        non_system_relations: 0,
        asset_root_entries: 0,
        database_owner_is_admin: true,
        runtime_can_connect: false,
        public_can_connect: false,
        other_sessions: 0,
        private_asset_root: true,
    };
    clean.validate().unwrap();
    let mut dirty = clean;
    dirty.non_system_relations = 1;
    assert!(dirty.validate().is_err());
    dirty.non_system_relations = 0;
    dirty.asset_root_entries = 1;
    assert!(dirty.validate().is_err());
    dirty.asset_root_entries = 0;
    dirty.runtime_can_connect = true;
    assert!(dirty.validate().is_err());
    dirty.runtime_can_connect = false;
    dirty.private_asset_root = false;
    assert!(dirty.validate().is_err());
}

#[test]
fn every_restored_ready_asset_must_match_the_entire_index() {
    let a = asset(2, &"a".repeat(64));
    let b = asset(3, &"b".repeat(64));
    let plan = BackupPlan::from_rows(vec![b.clone(), a.clone()]).unwrap();
    validate_restored_assets(&plan, vec![b.clone(), a.clone()]).unwrap();
    assert!(validate_restored_assets(&plan, vec![a.clone()]).is_err());
    assert!(
        validate_restored_assets(&plan, vec![a.clone(), b.clone(), asset(4, &"c".repeat(64))])
            .is_err()
    );
    let mut changed = b;
    changed.id = Uuid::from_u128(4);
    assert!(validate_restored_assets(&plan, vec![a, changed]).is_err());
}

#[test]
fn role_recipe_rejects_passwords_extra_roles_and_privilege_escalation() {
    let valid = serde_json::json!({
        "format_version":1,
        "roles":[
            {"name":"learning_admin","login":true,"inherit":true,"superuser":false,"createdb":false,"createrole":false,"bypassrls":false,"replication":false,"connection_limit":-1},
            {"name":"learning_auth_lock","login":false,"inherit":true,"superuser":false,"createdb":false,"createrole":false,"bypassrls":false,"replication":false,"connection_limit":-1},
            {"name":"learning_runtime","login":true,"inherit":true,"superuser":false,"createdb":false,"createrole":false,"bypassrls":false,"replication":false,"connection_limit":-1}
        ],
        "memberships":[{"role":"learning_auth_lock","member":"learning_admin","inherit":false,"set":true,"admin":false}]
    });
    validate_role_recipe(&serde_json::to_vec(&valid).unwrap()).unwrap();
    let mut bad = valid.clone();
    bad["roles"][0]["superuser"] = true.into();
    assert!(validate_role_recipe(&serde_json::to_vec(&bad).unwrap()).is_err());
    let mut bad = valid.clone();
    bad["roles"][0]["password"] = "secret".into();
    assert!(validate_role_recipe(&serde_json::to_vec(&bad).unwrap()).is_err());
    let mut bad = valid;
    bad["roles"].as_array_mut().unwrap().pop();
    assert!(validate_role_recipe(&serde_json::to_vec(&bad).unwrap()).is_err());
}

#[test]
fn running_jobs_are_classified_for_lease_invalidation_with_retry_budget() {
    let asset_job = RestoredJob {
        status: "running".into(),
        event_type: "asset_integrity_requested".into(),
        attempt_count: 2,
        lease_token_present: true,
        checkpoint_present: false,
        external_effect: ExternalEffectFinding::Unknown,
    };
    assert_eq!(
        classify_restored_job(&asset_job).unwrap(),
        JobRecoveryAction::RetryWait
    );
    let mut exhausted = asset_job.clone();
    exhausted.attempt_count = 3;
    assert_eq!(
        classify_restored_job(&exhausted).unwrap(),
        JobRecoveryAction::FailedRetryBudget
    );
    let mut snapshot = asset_job;
    snapshot.status = "snapshot_running".into();
    snapshot.event_type = "snapshot_export_requested".into();
    assert_eq!(
        classify_restored_job(&snapshot).unwrap(),
        JobRecoveryAction::AwaitExternalReconciliation
    );
    snapshot.external_effect = ExternalEffectFinding::ConfirmedNoEffect;
    assert_eq!(
        classify_restored_job(&snapshot).unwrap(),
        JobRecoveryAction::RetryWait
    );
    snapshot.external_effect = ExternalEffectFinding::AlreadyCommitted;
    assert_eq!(
        classify_restored_job(&snapshot).unwrap(),
        JobRecoveryAction::AwaitExternalReconciliation
    );
    snapshot.status = "snapshot_queued".into();
    snapshot.attempt_count = 0;
    snapshot.lease_token_present = false;
    for finding in [
        ExternalEffectFinding::Unknown,
        ExternalEffectFinding::AlreadyCommitted,
        ExternalEffectFinding::Conflict,
    ] {
        snapshot.external_effect = finding;
        assert_eq!(
            classify_restored_job(&snapshot).unwrap(),
            JobRecoveryAction::AwaitExternalReconciliation
        );
    }
    snapshot.external_effect = ExternalEffectFinding::ConfirmedNoEffect;
    assert_eq!(
        classify_restored_job(&snapshot).unwrap(),
        JobRecoveryAction::LeaveQueued
    );
    snapshot.status = "snapshot_retry_wait".into();
    snapshot.attempt_count = 1;
    for finding in [
        ExternalEffectFinding::Unknown,
        ExternalEffectFinding::AlreadyCommitted,
        ExternalEffectFinding::Conflict,
    ] {
        snapshot.external_effect = finding;
        assert_eq!(
            classify_restored_job(&snapshot).unwrap(),
            JobRecoveryAction::AwaitExternalReconciliation
        );
    }
    snapshot.external_effect = ExternalEffectFinding::ConfirmedNoEffect;
    assert_eq!(
        classify_restored_job(&snapshot).unwrap(),
        JobRecoveryAction::LeaveWaiting
    );
    snapshot.attempt_count = 3;
    assert!(classify_restored_job(&snapshot).is_err());
    snapshot.attempt_count = 1;
    snapshot.status = "snapshot_running".into();
    snapshot.lease_token_present = false;
    assert!(matches!(
        classify_restored_job(&snapshot),
        Err(BackupError::Invalid(_))
    ));
}

#[test]
fn terminal_snapshot_jobs_require_matching_durable_external_finding() {
    let mut job = RestoredJob {
        status: "succeeded".into(),
        event_type: "snapshot_export_requested".into(),
        attempt_count: 1,
        lease_token_present: false,
        checkpoint_present: false,
        external_effect: ExternalEffectFinding::Unknown,
    };
    for finding in [
        ExternalEffectFinding::Unknown,
        ExternalEffectFinding::ConfirmedNoEffect,
        ExternalEffectFinding::Conflict,
    ] {
        job.external_effect = finding;
        assert_eq!(
            classify_restored_job(&job).unwrap(),
            JobRecoveryAction::AwaitExternalReconciliation
        );
    }
    job.external_effect = ExternalEffectFinding::AlreadyCommitted;
    assert_eq!(
        classify_restored_job(&job).unwrap(),
        JobRecoveryAction::LeaveTerminal
    );
    for status in ["failed", "cancelled"] {
        job.status = status.into();
        for finding in [
            ExternalEffectFinding::Unknown,
            ExternalEffectFinding::AlreadyCommitted,
            ExternalEffectFinding::Conflict,
        ] {
            job.external_effect = finding;
            assert_eq!(
                classify_restored_job(&job).unwrap(),
                JobRecoveryAction::AwaitExternalReconciliation
            );
        }
        job.external_effect = ExternalEffectFinding::ConfirmedNoEffect;
        assert_eq!(
            classify_restored_job(&job).unwrap(),
            JobRecoveryAction::LeaveTerminal
        );
    }
    job.event_type = "asset_integrity_requested".into();
    job.external_effect = ExternalEffectFinding::Conflict;
    assert_eq!(
        classify_restored_job(&job).unwrap(),
        JobRecoveryAction::LeaveTerminal
    );
}

#[test]
fn restored_original_bytes_are_rehashed_and_corruption_blocks_acceptance() {
    use learning_assets::{FsAssetStore, UploadDeclaration};
    use std::fs;
    let root = std::env::temp_dir().join(format!("c4-restore-bytes-{}", Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    let store = FsAssetStore::new(root.join("assets"), root.join("staging")).unwrap();
    let source = root.join("source");
    fs::write(&source, b"abc").unwrap();
    let blob = store
        .put_from_file(
            Uuid::new_v4(),
            &source,
            UploadDeclaration {
                expected_size_bytes: 3,
                max_size_bytes: 3,
            },
        )
        .unwrap();
    let row = asset(2, blob.sha256());
    let plan = BackupPlan::from_rows(vec![row.clone(), asset(3, blob.sha256())]).unwrap();
    validate_restored_asset_bytes(&store, &plan).unwrap();
    fs::write(root.join("assets").join(&row.storage_key), b"xyz").unwrap();
    assert!(validate_restored_asset_bytes(&store, &plan).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pg_restore_uses_fixed_strict_options_and_isolated_database_name() {
    let db = format!("learning_restore_c4_{}", Uuid::new_v4());
    let spec = PgRestoreSpec::new(&db, "pg", 5432).unwrap();
    let args = spec.args();
    assert!(args.contains(&"--exit-on-error".to_owned()));
    assert!(args.contains(&"--single-transaction".to_owned()));
    assert!(args.contains(&"--no-owner".to_owned()));
    assert!(args.contains(&format!("--dbname={db}")));
    assert!(
        !args
            .iter()
            .any(|v| v.contains("password") && v != "--no-password")
    );
    assert!(PgRestoreSpec::new("production", "pg", 5432).is_err());
    assert!(PgRestoreSpec::new(&db, "-bad", 5432).is_err());
    assert!(PgRestoreSpec::new(&db, "pg", 0).is_err());
}

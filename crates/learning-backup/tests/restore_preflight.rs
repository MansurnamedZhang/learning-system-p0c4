use learning_backup::{CompleteBackup, RestorePreflightConfig, preflight_restore};
use sqlx::PgPool;
use std::path::PathBuf;
use uuid::Uuid;

fn config() -> RestorePreflightConfig {
    let base = std::env::temp_dir().join(format!("c4-restore-config-{}", Uuid::new_v4()));
    RestorePreflightConfig {
        destination_root: base.join("destination"),
        trust_path: base.join("trust").join("verifier.json"),
        control_root: base.join("control"),
        asset_root: base.join("assets"),
        expected_database: format!("learning_restore_c4_{}", Uuid::new_v4()),
    }
}

#[test]
fn preflight_rejects_relative_overlapping_or_production_paths() {
    let valid = config();
    valid.validate().unwrap();
    let mut overlap = valid.clone();
    overlap.asset_root = overlap.destination_root.join("assets");
    assert!(overlap.validate().is_err());
    let mut relative = valid.clone();
    relative.control_root = PathBuf::from("relative");
    assert!(relative.validate().is_err());
    let mut production = valid;
    production.expected_database = "production".into();
    assert!(production.validate().is_err());
}

#[test]
fn preflight_signature_requires_opaque_complete_handle() {
    fn compile_only(complete: &CompleteBackup, config: &RestorePreflightConfig, pool: &PgPool) {
        let _future = preflight_restore(complete, config, pool);
    }
    let _ = compile_only as fn(&CompleteBackup, &RestorePreflightConfig, &PgPool);
}

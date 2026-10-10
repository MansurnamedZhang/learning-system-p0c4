use learning_backup::full_restore::restore_complete_backup;

// The production path requires ownership of an unforgeable CompleteBackup;
// no path, decoded content, flag, or serde value can substitute for it.
#[test]
fn local_rehearsal_cannot_construct_complete_or_usable_authority() {
    fn requires_complete<F, Fut>(_: F)
    where
        F: FnOnce(
            learning_backup::CompleteBackup,
            learning_backup::RestorePreflightConfig,
            sqlx::PgPool,
        ) -> Fut,
        Fut: std::future::Future<
                Output = Result<
                    learning_backup::full_restore::RecoveryPending,
                    learning_backup::BackupError,
                >,
            >,
    {
    }
    requires_complete(restore_complete_backup);
}
#[test]
fn complete_role_reader_is_closed_without_verified_package_authority() {
    let binary = env!("CARGO_BIN_EXE_knowweave-c4-full-roles");
    for args in [
        vec![],
        vec!["--allow-local"],
        vec!["550e8400-e29b-41d4-a716-446655440000", "--skip-proof"],
        vec!["550e8400-e29b-41d4-a716-446655440000"],
    ] {
        let output = std::process::Command::new(binary)
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, b"complete role observation refused\n");
    }
}

use learning_backup::DestinationSigner;
use std::process::Command;

#[test]
fn completion_without_independent_deployment_stays_disabled() {
    // No test-only/local key facility. Also run with --all-features: the
    // reviewed build pin and installed independent custody are still required.
    assert!(DestinationSigner::open_installed().is_err());
}

#[test]
fn destination_cli_accepts_only_one_canonical_backup_uuid() {
    for args in [
        vec![],
        vec!["--allow-local"],
        vec!["00000000-0000-0000-0000-000000000000"],
        vec![
            "4b1a22c4-1109-4e03-97f0-cba02122a756",
            "--key=/tmp/test.key",
        ],
        vec!["4B1A22C4-1109-4E03-97F0-CBA02122A756"],
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_knowweave-c4-destination"))
            .args(args)
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(result.stdout.is_empty());
        assert_eq!(
            String::from_utf8(result.stderr).unwrap().trim(),
            "destination verification refused"
        );
    }
}

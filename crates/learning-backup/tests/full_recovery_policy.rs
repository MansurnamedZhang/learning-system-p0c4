//! Pure policy coverage only. Linux held-target bodies exercise SQL fencing.
use learning_backup::{
    ExternalEffectFinding as Effect, JobRecoveryAction as Action, RestoredJob,
    classify_restored_job,
};

fn job(status: &str, snapshot: bool, attempts: i32, effect: Effect) -> RestoredJob {
    RestoredJob {
        status: status.into(),
        event_type: if snapshot {
            "snapshot_export_requested"
        } else {
            "asset_integrity_requested"
        }
        .into(),
        attempt_count: attempts,
        lease_token_present: status.ends_with("running"),
        checkpoint_present: false,
        external_effect: effect,
    }
}

#[test]
fn recovery_preserves_attempt_count_and_three_attempt_ceiling() {
    for snapshot in [false, true] {
        for attempts in 1..=3 {
            let row = job(
                if snapshot {
                    "snapshot_running"
                } else {
                    "running"
                },
                snapshot,
                attempts,
                Effect::ConfirmedNoEffect,
            );
            assert_eq!(
                classify_restored_job(&row).unwrap(),
                if attempts == 3 {
                    Action::FailedRetryBudget
                } else {
                    Action::RetryWait
                }
            );
            assert_eq!(row.attempt_count, attempts);
        }
    }
}

#[test]
fn recovery_unknown_snapshot_effect_is_unclaimable_and_blocks_usable() {
    for (status, attempts) in [
        ("snapshot_queued", 0),
        ("snapshot_running", 1),
        ("snapshot_retry_wait", 2),
        ("succeeded", 1),
        ("failed", 3),
        ("cancelled", 0),
    ] {
        let row = job(status, true, attempts, Effect::Unknown);
        assert_eq!(
            classify_restored_job(&row).unwrap(),
            Action::AwaitExternalReconciliation
        );
    }
}

#[test]
fn recovery_committed_snapshot_effect_is_not_republished() {
    assert_eq!(
        classify_restored_job(&job("succeeded", true, 1, Effect::AlreadyCommitted)).unwrap(),
        Action::LeaveTerminal
    );
    assert_eq!(
        classify_restored_job(&job("snapshot_running", true, 1, Effect::AlreadyCommitted)).unwrap(),
        Action::AwaitExternalReconciliation
    );
}

#[test]
fn recovery_rejects_queued_checkpoint_instead_of_accepting_invalid_db_projection() {
    for snapshot in [false, true] {
        let mut row = job(
            if snapshot {
                "snapshot_queued"
            } else {
                "queued"
            },
            snapshot,
            0,
            Effect::ConfirmedNoEffect,
        );
        row.checkpoint_present = true;
        assert!(
            classify_restored_job(&row).is_err(),
            "queued checkpoint violates the existing DB constraint"
        );
    }
}

use learning_core::{BlockRef, JobInput, JobStatus};
use serde_json::json;
use uuid::Uuid;

#[test]
fn exact_asset_input_has_stable_business_key_and_versioned_json() {
    let space_id = Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap();
    let block_id = Uuid::parse_str("00000000-0000-4000-8000-000000000002").unwrap();
    let revision_id = Uuid::parse_str("00000000-0000-4000-8000-000000000003").unwrap();
    let actor_id = Uuid::parse_str("00000000-0000-4000-8000-000000000004").unwrap();
    let input = JobInput::asset_integrity(
        actor_id,
        space_id,
        BlockRef {
            block_id,
            revision_id,
        },
    )
    .unwrap();
    assert_eq!(
        input.business_key(),
        "asset-integrity:v1:00000000-0000-4000-8000-000000000001:00000000-0000-4000-8000-000000000002:00000000-0000-4000-8000-000000000003"
    );
    let encoded = input.to_value();
    assert_eq!(
        encoded,
        json!({"version":1,"kind":"asset_integrity","space_id":space_id,"block_id":block_id,"revision_id":revision_id,"actor_id":actor_id})
    );
    assert_eq!(JobInput::from_value(encoded).unwrap(), input);
}

#[test]
fn job_input_rejects_unknown_version_missing_fields_and_extra_fields() {
    let valid = json!({"version":1,"kind":"asset_integrity","space_id":Uuid::new_v4(),"block_id":Uuid::new_v4(),"revision_id":Uuid::new_v4(),"actor_id":Uuid::new_v4()});
    for mutation in [
        json!({"version":2,"kind":"asset_integrity","space_id":valid["space_id"],"block_id":valid["block_id"],"revision_id":valid["revision_id"],"actor_id":valid["actor_id"]}),
        json!({"version":1,"kind":"asset_integrity","space_id":valid["space_id"],"block_id":valid["block_id"],"actor_id":valid["actor_id"]}),
        json!({"version":1,"kind":"asset_integrity","space_id":valid["space_id"],"block_id":valid["block_id"],"revision_id":valid["revision_id"],"actor_id":valid["actor_id"],"surprise":true}),
    ] {
        assert!(JobInput::from_value(mutation).is_err());
    }
    assert!(
        JobInput::asset_integrity(
            Uuid::nil(),
            Uuid::new_v4(),
            BlockRef {
                block_id: Uuid::new_v4(),
                revision_id: Uuid::new_v4()
            }
        )
        .is_err()
    );
}

#[test]
fn job_status_rejects_unknown_name_and_terminal_resurrection() {
    assert_eq!(
        JobStatus::parse("retry_wait").unwrap(),
        JobStatus::RetryWait
    );
    assert!(JobStatus::parse("complete").is_err());
    assert!(JobStatus::Queued.can_transition_to(JobStatus::Running));
    assert!(!JobStatus::Queued.can_transition_to(JobStatus::Queued));
    assert!(!JobStatus::RetryWait.can_transition_to(JobStatus::RetryWait));
    assert!(JobStatus::Running.can_transition_to(JobStatus::Running));
    assert!(!JobStatus::Queued.can_transition_to(JobStatus::Succeeded));
    for terminal in [
        JobStatus::Succeeded,
        JobStatus::Failed,
        JobStatus::Cancelled,
    ] {
        assert!(!terminal.can_transition_to(JobStatus::Queued));
        assert!(!terminal.can_transition_to(JobStatus::Running));
    }
}

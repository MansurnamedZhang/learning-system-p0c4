use super::*;

// Public, synthetic byte patterns only. No runtime key is generated or read.
pub(super) const AUTH: &str = "66687aadf862bd776c8fc18b8e9f8e20089714856ee233b3902a591d0d5f2925";
pub(super) const HASH: &str = "2222222222222222222222222222222222222222222222222222222222222222";
pub(super) const COMMIT: &str = "3333333333333333333333333333333333333333";

pub(super) fn inputs(role: Role) -> BuildInputs<'static> {
    BuildInputs {
        role: Some(match role {
            Role::Source => "source",
            Role::Destination => "destination",
        }),
        profile: Some(profile::PROFILE),
        source_auth_key: Some(AUTH),
        verifier_key: Some(HASH),
        commit: Some(COMMIT),
        build_input: Some(HASH),
        source_binding: Some(HASH),
    }
}

pub(super) fn pins(role: Role) -> BuildPins<'static> {
    BuildPins::parse(inputs(role), role).unwrap()
}

#[test]
fn missing_or_other_role_never_admits_a_runner() {
    let mut absent = inputs(Role::Source);
    absent.role = None;
    assert!(BuildPins::parse(absent, Role::Source).is_err());
    assert!(BuildPins::parse(inputs(Role::Source), Role::Destination).is_err());
    let mut unknown = inputs(Role::Source);
    unknown.role = Some("source-or-destination");
    assert!(BuildPins::parse(unknown, Role::Source).is_err());
}

#[test]
fn profile_and_each_required_compile_pin_fail_closed() {
    let mut profile = inputs(Role::Source);
    profile.profile = Some("task9-b1-limited-v2");
    assert!(BuildPins::parse(profile, Role::Source).is_err());
    for field in 0..5 {
        let mut input = inputs(Role::Source);
        match field {
            0 => input.source_auth_key = None,
            1 => input.verifier_key = Some("NOT-A-LOWER-HEX-DIGEST"),
            2 => input.commit = Some(HASH),
            3 => input.build_input = None,
            4 => input.source_binding = None,
            _ => unreachable!(),
        }
        assert!(BuildPins::parse(input, Role::Source).is_err());
    }
}

#[test]
fn destination_build_does_not_require_source_connection_binding() {
    let mut input = inputs(Role::Destination);
    input.source_binding = None;
    assert!(BuildPins::parse(input, Role::Destination).is_ok());
}

#[test]
fn even_correctly_pinned_roles_stop_before_any_io_or_authority() {
    for role in [Role::Source, Role::Destination] {
        assert!(matches!(
            refuse_unwired(inputs(role), role),
            Err(BackupError::Invalid("offhost B1 integration incomplete"))
        ));
    }
}

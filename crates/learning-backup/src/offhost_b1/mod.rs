//! Default-off off-host foundation. Parsed configuration never grants authority.
//!
//! Task1a intentionally stops before installed filesystem admission, capture,
//! actors, transport, fence creation or publication. In particular it cannot
//! reconstruct a source admission or original destination success from bytes.

// These private data-only seams are not invoked by the closed runners yet.
// Remove these dead-code allowances as their reviewed runtime consumers land;
// no Clippy correctness lint or existing product module is suppressed.
#[allow(
    dead_code,
    reason = "Task1a policy inputs have no runtime authority consumer"
)]
mod policy;
#[allow(
    dead_code,
    reason = "Task1a profile is not a wired runtime IO budget yet"
)]
mod profile;
#[allow(
    dead_code,
    reason = "Task2a untrusted codec has no runtime authority or IO consumer"
)]
mod wire;

// C11's bounded typed DATA and held-genesis reader are compiled and tested here.
// The original Root job, actual observer and authenticated ACK producer are
// separate consumers; including this module does not activate those owners.
#[allow(
    dead_code,
    reason = "C11 DATA component has no installed Root producer yet"
)]
mod source_launch;

use crate::{BackupError, valid_digest, valid_hex};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Role {
    Source,
    Destination,
}

struct BuildInputs<'a> {
    role: Option<&'a str>,
    profile: Option<&'a str>,
    source_auth_key: Option<&'a str>,
    verifier_key: Option<&'a str>,
    commit: Option<&'a str>,
    build_input: Option<&'a str>,
    source_binding: Option<&'a str>,
}

/// Build declarations, not proof that the executing ELF/launcher is trusted.
#[allow(
    dead_code,
    reason = "Installed build-pin consumers are not wired in Task1a"
)]
struct BuildPins<'a> {
    role: Role,
    source_auth_key: &'a str,
    verifier_key: &'a str,
    commit: &'a str,
    build_input: &'a str,
    source_binding: Option<&'a str>,
}

impl<'a> BuildPins<'a> {
    fn parse(input: BuildInputs<'a>, expected: Role) -> Result<Self, BackupError> {
        let role = match input.role {
            Some("source") => Role::Source,
            Some("destination") => Role::Destination,
            _ => return Err(BackupError::Invalid("offhost B1 fixed build role")),
        };
        if role != expected || input.profile != Some(profile::PROFILE) {
            return Err(BackupError::Invalid("offhost B1 fixed build profile"));
        }
        let required_digest = |value: Option<&'a str>| {
            value
                .filter(|v| valid_digest(v))
                .ok_or(BackupError::Invalid("offhost B1 required build pin"))
        };
        let source_binding = match role {
            Role::Source => Some(required_digest(input.source_binding)?),
            Role::Destination => None,
        };
        Ok(Self {
            role,
            source_auth_key: required_digest(input.source_auth_key)?,
            verifier_key: required_digest(input.verifier_key)?,
            commit: input
                .commit
                .filter(|v| valid_hex(v, 40))
                .ok_or(BackupError::Invalid("offhost B1 required code commit"))?,
            build_input: required_digest(input.build_input)?,
            source_binding,
        })
    }
}

fn fixed_build_inputs() -> BuildInputs<'static> {
    BuildInputs {
        role: option_env!("KNOWWEAVE_C4_B1_ROLE"),
        profile: option_env!("KNOWWEAVE_C4_B1_PROFILE"),
        source_auth_key: option_env!("KNOWWEAVE_C4_B1_SOURCE_AUTH_KEY_SHA256"),
        verifier_key: option_env!("KNOWWEAVE_C4_VERIFIER_KEY_SHA256"),
        // Manager release identity is separate from SourceGroup application pins.
        commit: option_env!("KNOWWEAVE_C4_B1_MANAGER_CODE_COMMIT"),
        build_input: option_env!("KNOWWEAVE_C4_B1_MANAGER_BUILD_INPUT_SHA256"),
        source_binding: option_env!("KNOWWEAVE_C4_SOURCE_CONTROL_BINDING_SHA256"),
    }
}

fn refuse_unwired(input: BuildInputs<'_>, role: Role) -> Result<(), BackupError> {
    let _pins = BuildPins::parse(input, role)?;
    // Even fully pinned builds cannot reach FS/PG/SSH or signing here. Future
    // consumers must own the real SourceAdmission + registry lease/snapshot,
    // or the original D context; an input parser is never their constructor.
    Err(BackupError::Invalid("offhost B1 integration incomplete"))
}

/// Fixed installed source entry; no caller paths, credentials, IDs or permits.
/// The foundation always refuses, including correctly pinned builds.
pub fn run_installed_offhost_b1_source() -> Result<(), BackupError> {
    refuse_unwired(fixed_build_inputs(), Role::Source)
}

/// Fixed installed stdio receiver entry. Task1a does not read stdin or any key.
/// The foundation always refuses, including correctly pinned builds.
pub fn run_installed_offhost_b1_receiver() -> Result<(), BackupError> {
    refuse_unwired(fixed_build_inputs(), Role::Destination)
}

#[cfg(test)]
mod tests;

// These unit probes exercise the actual compile-time option_env! input family.
// Public synthetic strings are DATA only, never installed keys or authority.
#[cfg(test)]
mod compile_env_tests {
    use super::*;
    use std::collections::BTreeMap;

    const APP_COMMIT: &str = "1111111111111111111111111111111111111111";
    const APP_BUILD: &str = "2222222222222222222222222222222222222222222222222222222222222222";
    const MANAGER_COMMIT: &str = "3333333333333333333333333333333333333333";
    const MANAGER_BUILD: &str = "4444444444444444444444444444444444444444444444444444444444444444";
    const BINDING: &str = "5555555555555555555555555555555555555555555555555555555555555555";
    const BAD: &str = "NOT-A-CANONICAL-COMMIT";
    const COMPILE_ID: &str = "m1b-public-compile-family-data-v1";

    fn variant() -> &'static str {
        match option_env!("KNOWWEAVE_C4_B1_TEST_VARIANT") {
            None => {
                assert!(option_env!("KNOWWEAVE_C4_B1_TEST_BUILD_ID").is_none());
                "ordinary-unset"
            }
            Some(
                name @ ("app-only" | "separate-families" | "manager-only" | "bad-manager"
                | "bad-app"),
            ) => {
                let expected_id = format!("{COMPILE_ID}/{name}");
                assert_eq!(
                    option_env!("KNOWWEAVE_C4_B1_TEST_BUILD_ID"),
                    Some(expected_id.as_str())
                );
                name
            }
            Some(_) => panic!("unknown private compile DATA variant"),
        }
    }

    fn note(case: &str) {
        println!(
            "M1B_COMPILE variant={} build={} case={} PUBLIC_SYNTHETIC_DATA_ONLY",
            variant(),
            option_env!("KNOWWEAVE_C4_B1_TEST_BUILD_ID").unwrap_or("UNSET"),
            case
        );
    }

    fn source_group_data() -> crate::registry::SourceGroup {
        // A plain existing DTO for the existing DATA join, not a RegistryLease,
        // Root admission, source selection, installed getter or peer context.
        crate::registry::SourceGroup {
            format_version: 1,
            capability: "backup_source_group_v1".into(),
            deployment_id: "11111111-1111-4111-8111-111111111111".into(),
            group_id: "22222222-2222-4222-8222-222222222222".into(),
            database: "public_compile_data_only".into(),
            database_oid: 1,
            system_identifier: "1".into(),
            source_binding_sha256: BINDING.into(),
            application_commit: APP_COMMIT.into(),
            application_build_sha256: APP_BUILD.into(),
            roots: BTreeMap::new(),
        }
    }

    #[test]
    fn actual_fixed_inputs_select_only_manager_compile_family() {
        let input = fixed_build_inputs();
        let expected = match variant() {
            "ordinary-unset" | "app-only" => (None, None),
            "bad-manager" => (Some(BAD), Some(MANAGER_BUILD)),
            _ => (Some(MANAGER_COMMIT), Some(MANAGER_BUILD)),
        };
        assert_eq!((input.commit, input.build_input), expected);
        if variant() != "ordinary-unset" {
            assert_eq!(input.role, Some("source"));
            assert_eq!(input.profile, Some(profile::PROFILE));
            assert_eq!(input.source_binding, Some(BINDING));
        }
        if variant() == "app-only" {
            assert_eq!(option_env!("KNOWWEAVE_SOURCE_COMMIT"), Some(APP_COMMIT));
            assert_eq!(option_env!("KNOWWEAVE_BUILD_ID_SHA256"), Some(APP_BUILD));
            assert!(matches!(
                BuildPins::parse(input, Role::Source),
                Err(BackupError::Invalid("offhost B1 required code commit"))
            ));
        } else if variant() == "bad-manager" {
            assert!(matches!(
                BuildPins::parse(input, Role::Source),
                Err(BackupError::Invalid("offhost B1 required code commit"))
            ));
        } else if variant() == "ordinary-unset" {
            assert!(BuildPins::parse(input, Role::Source).is_err());
        } else {
            let pins = BuildPins::parse(input, Role::Source).unwrap();
            assert_eq!(
                (pins.commit, pins.build_input),
                (MANAGER_COMMIT, MANAGER_BUILD)
            );
            assert_ne!((pins.commit, pins.build_input), (APP_COMMIT, APP_BUILD));
        }
        note("actual_fixed_inputs_select_only_manager_compile_family");
    }

    #[test]
    fn original_application_data_join_uses_legacy_compile_family() {
        let group = source_group_data();
        // Existing pub(crate) DATA join. The production original_source
        // macro callsite is separately byte-pinned; Root/PG getter is not run.
        let actual = crate::registry::validate_build_binding(
            &group,
            option_env!("KNOWWEAVE_C4_SOURCE_CONTROL_BINDING_SHA256"),
            option_env!("KNOWWEAVE_SOURCE_COMMIT"),
            option_env!("KNOWWEAVE_BUILD_ID_SHA256"),
        );
        assert_eq!(
            actual.is_ok(),
            matches!(variant(), "app-only" | "separate-families" | "bad-manager")
        );
        let manager = fixed_build_inputs();
        assert!(
            crate::registry::validate_build_binding(
                &group,
                manager.source_binding,
                manager.commit,
                manager.build_input,
            )
            .is_err()
        );
        note("original_application_data_join_uses_legacy_compile_family");
    }

    #[test]
    fn dropping_each_family_pin_cannot_borrow_the_other_family() {
        if matches!(variant(), "separate-families" | "manager-only" | "bad-app") {
            let mut missing_commit = fixed_build_inputs();
            missing_commit.commit = None;
            assert!(BuildPins::parse(missing_commit, Role::Source).is_err());
            let mut missing_build = fixed_build_inputs();
            missing_build.build_input = None;
            assert!(BuildPins::parse(missing_build, Role::Source).is_err());
        } else {
            assert!(BuildPins::parse(fixed_build_inputs(), Role::Source).is_err());
        }
        let group = source_group_data();
        assert!(
            crate::registry::validate_build_binding(&group, Some(BINDING), None, Some(APP_BUILD))
                .is_err()
        );
        assert!(
            crate::registry::validate_build_binding(&group, Some(BINDING), Some(APP_COMMIT), None)
                .is_err()
        );
        assert!(
            crate::registry::validate_build_binding(
                &group,
                Some(BINDING),
                Some(MANAGER_COMMIT),
                Some(MANAGER_BUILD)
            )
            .is_err()
        );
        note("dropping_each_family_pin_cannot_borrow_the_other_family");
    }

    #[test]
    fn actual_compiled_manager_data_does_not_open_installed_entries() {
        if matches!(variant(), "separate-families" | "manager-only" | "bad-app") {
            let pins = BuildPins::parse(fixed_build_inputs(), Role::Source).unwrap();
            assert_eq!(
                (pins.commit, pins.build_input),
                (MANAGER_COMMIT, MANAGER_BUILD)
            );
            assert!(matches!(
                run_installed_offhost_b1_source(),
                Err(BackupError::Invalid("offhost B1 integration incomplete"))
            ));
        } else {
            assert!(BuildPins::parse(fixed_build_inputs(), Role::Source).is_err());
            assert!(run_installed_offhost_b1_source().is_err());
        }
        assert!(run_installed_offhost_b1_receiver().is_err());
        note("actual_compiled_manager_data_does_not_open_installed_entries");
    }
}

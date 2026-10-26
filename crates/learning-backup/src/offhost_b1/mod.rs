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
        commit: option_env!("KNOWWEAVE_SOURCE_COMMIT"),
        build_input: option_env!("KNOWWEAVE_BUILD_ID_SHA256"),
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

//! Exact wire DATA shapes. No shape, nonce, signature text or root grants trust.
use super::{Direction, Header, WireError};
use crate::{registry, valid_digest, valid_hex};
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RootRecord {
    pub role: String,
    pub enrollment_id: String,
    pub path: String,
    pub dev: u64,
    pub ino: u64,
    pub uid: u32,
    pub mode: u32,
    pub record_sha256: String,
}

fn roots(records: &[RootRecord], roles: &[&str]) -> bool {
    records.len() == roles.len()
        && records.iter().zip(roles).all(|(r, role)| {
            r.role == *role
                && registry::v4(&r.enrollment_id)
                && registry::canonical_path(&r.path)
                && r.path.len() <= 1024
                && r.uid == 0
                && r.mode == 0o700
                && valid_digest(&r.record_sha256)
        })
        && records.iter().enumerate().all(|(i, r)| {
            records[..i]
                .iter()
                .all(|old| old.enrollment_id != r.enrollment_id)
        })
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SourceBinding {
    pub deployment_id: String,
    pub source_group_id: String,
    pub backup_id: String,
    pub registry_authority_sha256: String,
    pub registry_generation: u64,
    pub generation_sha256: String,
    pub source_binding_sha256: String,
    pub application_commit: String,
    pub application_build_sha256: String,
    pub manager_release_sha256: String,
    pub roots: Vec<RootRecord>,
    pub manifest_sha256: String,
    pub source_control_sha256: String,
    pub captured_roster_sha256: String,
    pub snapshot_effects_roster_sha256: String,
    pub package_bytes: u64,
    pub evidence_bytes: u64,
    pub file_count: u32,
    pub package_roster_sha256: String,
    pub source_operation_id: String,
    pub snapshot_effects_transport_index_sha256: String,
    pub snapshot_effects_input_sha256: String,
}
impl SourceBinding {
    pub(super) fn valid(&self) -> bool {
        [
            &self.deployment_id,
            &self.source_group_id,
            &self.backup_id,
            &self.source_operation_id,
        ]
        .iter()
        .all(|s| registry::v4(s))
            && self.registry_generation > 0
            && valid_hex(&self.application_commit, 40)
            && self.package_bytes <= super::profile::PACKAGE_BYTES
            && self.evidence_bytes <= super::profile::EVIDENCE_BYTES
            && u64::from(self.file_count) <= super::profile::FILES
            && roots(&self.roots, &["assets", "control", "local_pins", "staging"])
            && [
                &self.registry_authority_sha256,
                &self.generation_sha256,
                &self.source_binding_sha256,
                &self.application_build_sha256,
                &self.manager_release_sha256,
                &self.manifest_sha256,
                &self.source_control_sha256,
                &self.captured_roster_sha256,
                &self.snapshot_effects_roster_sha256,
                &self.package_roster_sha256,
                &self.snapshot_effects_transport_index_sha256,
                &self.snapshot_effects_input_sha256,
            ]
            .iter()
            .all(|s| valid_digest(s))
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DestinationBinding {
    pub deployment_id: String,
    pub destination_group_id: String,
    pub attempt_id: String,
    pub registry_authority_sha256: String,
    pub registry_generation: u64,
    pub generation_sha256: String,
    pub receiver_release_sha256: String,
    pub roots: Vec<RootRecord>,
}
impl DestinationBinding {
    pub(super) fn valid(&self) -> bool {
        [
            &self.deployment_id,
            &self.destination_group_id,
            &self.attempt_id,
        ]
        .iter()
        .all(|s| registry::v4(s))
            && self.registry_generation > 0
            && roots(&self.roots, &["destination", "evidence"])
            && [
                &self.registry_authority_sha256,
                &self.generation_sha256,
                &self.receiver_release_sha256,
            ]
            .iter()
            .all(|s| valid_digest(s))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HelloS {
    pub schema_version: u32,
    pub profile: String,
    pub kind: String,
    pub direction: Direction,
    pub sequence: u64,
    pub s_nonce: String,
    pub source: SourceBinding,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HelloD {
    pub schema_version: u32,
    pub profile: String,
    pub kind: String,
    pub direction: Direction,
    pub sequence: u64,
    pub s_nonce: String,
    pub d_nonce: String,
    pub destination: DestinationBinding,
}

fn envelope(version: u32, profile: &str, name: &str, expected: &str) -> bool {
    version == 1 && profile == super::profile::PROFILE && name == expected
}
impl HelloS {
    pub(super) fn valid(&self, h: Header) -> bool {
        h.kind == super::Kind::Metadata
            && envelope(self.schema_version, &self.profile, &self.kind, "HelloS")
            && self.direction == Direction::SourceToDestination
            && self.direction == h.direction
            && self.sequence == 0
            && self.sequence == h.sequence
            && valid_digest(&self.s_nonce)
            && self.source.valid()
    }
}
impl HelloD {
    pub(super) fn valid(&self, h: Header) -> bool {
        h.kind == super::Kind::Metadata
            && envelope(self.schema_version, &self.profile, &self.kind, "HelloD")
            && self.direction == Direction::DestinationToSource
            && self.direction == h.direction
            && self.sequence == 0
            && self.sequence == h.sequence
            && valid_digest(&self.s_nonce)
            && valid_digest(&self.d_nonce)
            && self.destination.valid()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(super) enum FileRole {
    PackageManifest,
    PackageMember,
    SourceEvidenceMember,
    SnapshotEffectsTransportIndex,
    SnapshotEffectsMember,
}
impl FileRole {
    pub(super) fn package(self) -> bool {
        matches!(self, Self::PackageManifest | Self::PackageMember)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Fence {
    pub operation_id: String,
    pub anchor_sha256: String,
    pub open_record_sha256: String,
    pub protection_snapshot_sha256: String,
    pub source_actor_binding_sha256: String,
}
impl Fence {
    fn valid(&self) -> bool {
        registry::v4(&self.operation_id)
            && [
                &self.anchor_sha256,
                &self.open_record_sha256,
                &self.protection_snapshot_sha256,
                &self.source_actor_binding_sha256,
            ]
            .iter()
            .all(|s| valid_digest(s))
    }
}

// Fields are expanded into each exact struct, NOT serde(flatten) or a Value map.
macro_rules! control {
    ($name:ident { $($field:ident : $ty:ty),* $(,)? }) => {
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(super) struct $name {
            pub schema_version: u32,
            pub profile: String,
            pub kind: String,
            pub direction: Direction,
            pub sequence: u64,
            pub s_nonce: String,
            pub d_nonce: String,
            pub prev_s: String,
            pub prev_d: String,
            pub binding_sha256: String,
            $(pub $field: $ty,)*
        }
        impl $name {
            fn common(&self) -> Common<'_> {
                Common {
                    schema_version: self.schema_version,
                    profile: &self.profile,
                    kind: &self.kind,
                    direction: self.direction,
                    sequence: self.sequence,
                    s_nonce: &self.s_nonce,
                    d_nonce: &self.d_nonce,
                    prev_s: &self.prev_s,
                    prev_d: &self.prev_d,
                    binding_sha256: &self.binding_sha256,
                }
            }
        }
    };
}
control!(SourceAuth {
    source: SourceBinding,
    destination: DestinationBinding,
    source_auth_key_sha256: String,
    signature: String,
});
control!(AuthAccepted {
    source_auth_frame_sha256: String
});
control!(FileBegin {
    file_no: u32,
    role: FileRole,
    roster_index: u32,
    size: u64,
    sha256: String,
});
control!(FileChunk {
    file_no: u32,
    offset: u64
});
control!(FileEnd {
    file_no: u32,
    size: u64,
    sha256: String
});
control!(SourceDataEnd {
    package_roster_sha256: String,
    captured_roster_sha256: String,
    snapshot_effects_roster_sha256: String,
    files: u32,
    bytes: u64,
});
control!(DestinationReady {
    attempt_id: String,
    proof_sha256: String,
    snapshot_effects_proof_sha256: String,
    manifest_sha256: String,
    snapshot_effects_input_sha256: String,
    snapshot_effects_intent_sha256: String,
});
control!(SourceFinalDecision {
    attempt_id: String,
    proof_sha256: String,
    snapshot_effects_proof_sha256: String,
    fence: Fence,
    source_auth_key_sha256: String,
    signature: String,
    snapshot_effects_input_sha256: String,
    snapshot_effects_intent_sha256: String,
});
control!(CommittedResult {
    attempt_id: String,
    proof_sha256: String,
    receipt_sha256: String,
    receipt_size: u32,
});
control!(SourceResultAck {
    attempt_id: String,
    receipt_sha256: String
});

pub(super) struct Common<'a> {
    pub schema_version: u32,
    pub profile: &'a str,
    pub kind: &'a str,
    pub direction: Direction,
    pub sequence: u64,
    pub s_nonce: &'a str,
    pub d_nonce: &'a str,
    pub prev_s: &'a str,
    pub prev_d: &'a str,
    pub binding_sha256: &'a str,
}

pub(super) enum UntrustedControl {
    SourceAuth(Box<SourceAuth>),
    AuthAccepted(AuthAccepted),
    FileBegin(FileBegin),
    FileChunk(FileChunk),
    FileEnd(FileEnd),
    SourceDataEnd(SourceDataEnd),
    DestinationReady(DestinationReady),
    SourceFinalDecision(SourceFinalDecision),
    CommittedResult(CommittedResult),
    SourceResultAck(SourceResultAck),
}
macro_rules! match_control {
    ($value:expr, $item:ident => $body:expr) => {
        match $value {
            UntrustedControl::SourceAuth($item) => $body,
            UntrustedControl::AuthAccepted($item) => $body,
            UntrustedControl::FileBegin($item) => $body,
            UntrustedControl::FileChunk($item) => $body,
            UntrustedControl::FileEnd($item) => $body,
            UntrustedControl::SourceDataEnd($item) => $body,
            UntrustedControl::DestinationReady($item) => $body,
            UntrustedControl::SourceFinalDecision($item) => $body,
            UntrustedControl::CommittedResult($item) => $body,
            UntrustedControl::SourceResultAck($item) => $body,
        }
    };
}
impl UntrustedControl {
    pub(super) fn common(&self) -> Common<'_> {
        match_control!(self, c => c.common())
    }
    pub(super) fn canonical(&self) -> Result<Vec<u8>, WireError> {
        match_control!(self, c => registry::canonical(c).map_err(|_| WireError::Json))
    }
    pub(super) fn tag(&self) -> u8 {
        match self {
            Self::SourceAuth(_) => 1,
            Self::AuthAccepted(_) => 2,
            Self::FileBegin(_) => 3,
            Self::FileChunk(_) => 4,
            Self::FileEnd(_) => 5,
            Self::SourceDataEnd(_) => 6,
            Self::DestinationReady(_) => 7,
            Self::SourceFinalDecision(_) => 8,
            Self::CommittedResult(_) => 9,
            Self::SourceResultAck(_) => 10,
        }
    }
    pub(super) fn valid(&self, h: Header) -> bool {
        let names = [
            "SourceAuth",
            "AuthAccepted",
            "FileBegin",
            "FileChunk",
            "FileEnd",
            "SourceDataEnd",
            "DestinationReady",
            "SourceFinalDecision",
            "CommittedResult",
            "SourceResultAck",
        ];
        let common = self.common();
        let destination = matches!(self.tag(), 2 | 7 | 9);
        if h.kind != super::Kind::Data
            || !envelope(
                common.schema_version,
                common.profile,
                common.kind,
                names[usize::from(self.tag() - 1)],
            )
            || common.direction != h.direction
            || (common.direction == Direction::DestinationToSource) != destination
            || common.sequence != h.sequence
            || [
                common.s_nonce,
                common.d_nonce,
                common.prev_s,
                common.prev_d,
                common.binding_sha256,
            ]
            .iter()
            .any(|s| !valid_digest(s))
        {
            return false;
        }
        match self {
            Self::SourceAuth(c) => {
                c.source.valid()
                    && c.destination.valid()
                    && valid_digest(&c.source_auth_key_sha256)
                    && valid_hex(&c.signature, 128)
            }
            Self::AuthAccepted(c) => valid_digest(&c.source_auth_frame_sha256),
            Self::FileBegin(c) => valid_digest(&c.sha256),
            Self::FileChunk(_) => true,
            Self::FileEnd(c) => valid_digest(&c.sha256),
            Self::SourceDataEnd(c) => [
                &c.package_roster_sha256,
                &c.captured_roster_sha256,
                &c.snapshot_effects_roster_sha256,
            ]
            .iter()
            .all(|s| valid_digest(s)),
            Self::DestinationReady(c) => {
                registry::v4(&c.attempt_id)
                    && [
                        &c.proof_sha256,
                        &c.snapshot_effects_proof_sha256,
                        &c.manifest_sha256,
                        &c.snapshot_effects_input_sha256,
                        &c.snapshot_effects_intent_sha256,
                    ]
                    .iter()
                    .all(|s| valid_digest(s))
            }
            Self::SourceFinalDecision(c) => {
                registry::v4(&c.attempt_id)
                    && c.fence.valid()
                    && valid_hex(&c.signature, 128)
                    && [
                        &c.proof_sha256,
                        &c.snapshot_effects_proof_sha256,
                        &c.source_auth_key_sha256,
                        &c.snapshot_effects_input_sha256,
                        &c.snapshot_effects_intent_sha256,
                    ]
                    .iter()
                    .all(|s| valid_digest(s))
            }
            Self::CommittedResult(c) => {
                registry::v4(&c.attempt_id)
                    && valid_digest(&c.proof_sha256)
                    && valid_digest(&c.receipt_sha256)
                    && c.receipt_size > 0
                    && u64::from(c.receipt_size) <= super::profile::RESULT_BYTES
            }
            Self::SourceResultAck(c) => {
                registry::v4(&c.attempt_id) && valid_digest(&c.receipt_sha256)
            }
        }
    }
}

pub(super) fn decode(tag: u8, raw: &[u8], header: Header) -> Result<UntrustedControl, WireError> {
    let cap = if tag == 4 {
        super::profile::CHUNK_CONTROL_BYTES
    } else {
        super::profile::CONTROL_BYTES
    };
    macro_rules! parse {
        ($name:ident) => {
            UntrustedControl::$name(
                registry::canonical_record::<$name>(raw, cap).map_err(|_| WireError::Json)?,
            )
        };
    }
    let value = match tag {
        1 => UntrustedControl::SourceAuth(Box::new(
            registry::canonical_record::<SourceAuth>(raw, cap).map_err(|_| WireError::Json)?,
        )),
        2 => parse!(AuthAccepted),
        3 => parse!(FileBegin),
        4 => parse!(FileChunk),
        5 => parse!(FileEnd),
        6 => parse!(SourceDataEnd),
        7 => parse!(DestinationReady),
        8 => parse!(SourceFinalDecision),
        9 => parse!(CommittedResult),
        10 => parse!(SourceResultAck),
        _ => return Err(WireError::Unsupported),
    };
    if !value.valid(header) {
        return Err(WireError::Fields);
    }
    Ok(value)
}

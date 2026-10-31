//! Default-off wire syntax with private pipe IO; no keys or permissions.
//! All decoded messages, phases and transcript heads remain UNTRUSTED DATA.
//! Existing Ready/receipt body semantics and real EOF require later consumers.
mod auth;
mod io;
mod schema;
mod transcript;

pub(super) use io::{
    OriginalPipeIo, OriginalPipePair, close_original_source_output_negative,
    retain_original_destination_stdio, retain_original_source_pipes,
};

use super::profile;
use crate::registry;
use schema::UntrustedControl;
pub(super) use schema::{
    CommittedResult, DestinationBinding, DestinationReady, Fence, FileBegin, FileChunk, FileEnd,
    FileRole, RootRecord, SourceAuth, SourceBinding, SourceDataEnd, SourceFinalDecision,
    SourceResultAck,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

/// The existing fixed actor-binding preimage; these arguments are DATA only.
pub(super) fn actor_binding_data(
    source_operation_id: &str,
    binding: &str,
    s_nonce: &str,
    d_nonce: &str,
) -> Result<String, WireError> {
    transcript::actor_binding(source_operation_id, binding, s_nonce, d_nonce)
}

const HEADER: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Direction {
    #[serde(rename = "S>D")]
    SourceToDestination,
    #[serde(rename = "D>S")]
    DestinationToSource,
}
impl Direction {
    fn index(self) -> usize {
        match self {
            Self::SourceToDestination => 0,
            Self::DestinationToSource => 1,
        }
    }
    fn octet(self) -> u8 {
        match self {
            Self::SourceToDestination => 1,
            Self::DestinationToSource => 2,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Metadata,
    Data,
    End,
    Abort,
}
impl Kind {
    fn octet(self) -> u8 {
        match self {
            Self::Metadata => 1,
            Self::Data => 2,
            Self::End => 3,
            Self::Abort => 4,
        }
    }
    fn budget_kind(self) -> profile::FrameKind {
        match self {
            Self::Metadata => profile::FrameKind::Metadata,
            Self::Data => profile::FrameKind::Data,
            Self::End => profile::FrameKind::End,
            Self::Abort => profile::FrameKind::Abort,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WireError {
    Header,
    Length,
    Unsupported,
    Json,
    Fields,
    Sequence,
    Phase,
    Binding,
    Transcript,
    File,
    Budget,
    Closed,
    Poisoned,
    Incomplete,
    SignatureData,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Header {
    kind: Kind,
    direction: Direction,
    sequence: u64,
    payload_bytes: u32,
}

/// Stateless preallocation inspection only. No counters or authority advance.
fn inspect_header(raw: &[u8]) -> Result<Header, WireError> {
    if raw.len() != HEADER || &raw[..4] != b"KWB1" || raw[4] != 1 || raw[7] != 0 {
        return Err(WireError::Header);
    }
    let kind = match raw[5] {
        1 => Kind::Metadata,
        2 => Kind::Data,
        3 => Kind::End,
        4 => Kind::Abort,
        _ => return Err(WireError::Header),
    };
    let direction = match raw[6] {
        1 => Direction::SourceToDestination,
        2 => Direction::DestinationToSource,
        _ => return Err(WireError::Header),
    };
    let sequence = u64::from_be_bytes(raw[8..16].try_into().map_err(|_| WireError::Header)?);
    let payload_bytes = u32::from_be_bytes(raw[16..20].try_into().map_err(|_| WireError::Header)?);
    let cap = match kind {
        Kind::Metadata => profile::CONTROL_BYTES,
        Kind::Data => profile::DATA_BYTES,
        Kind::End => 0,
        Kind::Abort => profile::ABORT_BYTES,
    };
    if u64::from(payload_bytes) > cap
        || (payload_bytes == 0 && matches!(kind, Kind::Metadata | Kind::Data))
    {
        return Err(WireError::Length);
    }
    Ok(Header {
        kind,
        direction,
        sequence,
        payload_bytes,
    })
}

struct UntrustedEnvelope<'a> {
    tag: u8,
    control: &'a [u8],
    body: &'a [u8],
}
fn u32_at(raw: &[u8], start: usize) -> Result<u32, WireError> {
    let end = start.checked_add(4).ok_or(WireError::Length)?;
    Ok(u32::from_be_bytes(
        raw.get(start..end)
            .ok_or(WireError::Length)?
            .try_into()
            .map_err(|_| WireError::Length)?,
    ))
}
fn inspect_data(raw: &[u8]) -> Result<UntrustedEnvelope<'_>, WireError> {
    if raw.len() > profile::DATA_BYTES as usize {
        return Err(WireError::Length);
    }
    let tag = *raw.first().ok_or(WireError::Length)?;
    if !(1..=10).contains(&tag) {
        return Err(WireError::Unsupported);
    }
    let control_len = usize::try_from(u32_at(raw, 1)?).map_err(|_| WireError::Length)?;
    let cap = if tag == 4 {
        profile::CHUNK_CONTROL_BYTES
    } else {
        profile::CONTROL_BYTES
    };
    if control_len == 0 || control_len > cap as usize {
        return Err(WireError::Length);
    }
    let control_end = 5_usize.checked_add(control_len).ok_or(WireError::Length)?;
    let body_len = usize::try_from(u32_at(raw, control_end)?).map_err(|_| WireError::Length)?;
    let body_start = control_end.checked_add(4).ok_or(WireError::Length)?;
    if body_start.checked_add(body_len) != Some(raw.len()) {
        return Err(WireError::Length);
    }
    Ok(UntrustedEnvelope {
        tag,
        control: raw.get(5..control_end).ok_or(WireError::Length)?,
        body: raw.get(body_start..).ok_or(WireError::Length)?,
    })
}

// Deliberately no Debug: even bounded bodies/nonce text must not enter logs.
enum UntrustedBody<'a> {
    Empty,
    Chunk(&'a [u8]),
    Ready { witness: &'a [u8], intent: &'a [u8] },
    Receipt(&'a [u8]),
}
fn parse_body<'a>(
    control: &UntrustedControl,
    raw: &'a [u8],
) -> Result<UntrustedBody<'a>, WireError> {
    match control {
        UntrustedControl::FileChunk(_) => {
            if raw.is_empty() || raw.len() > profile::CHUNK_BODY_BYTES as usize {
                return Err(WireError::Length);
            }
            Ok(UntrustedBody::Chunk(raw))
        }
        UntrustedControl::DestinationReady(c) => {
            let witness_len = usize::try_from(u32_at(raw, 0)?).map_err(|_| WireError::Length)?;
            if witness_len == 0 || witness_len > profile::READY_WITNESS_BYTES as usize {
                return Err(WireError::Length);
            }
            let witness_end = 4_usize.checked_add(witness_len).ok_or(WireError::Length)?;
            let intent_len =
                usize::try_from(u32_at(raw, witness_end)?).map_err(|_| WireError::Length)?;
            if intent_len == 0 || intent_len > profile::READY_INTENT_BYTES as usize {
                return Err(WireError::Length);
            }
            let intent_start = witness_end.checked_add(4).ok_or(WireError::Length)?;
            if intent_start.checked_add(intent_len) != Some(raw.len()) {
                return Err(WireError::Length);
            }
            let witness = raw.get(4..witness_end).ok_or(WireError::Length)?;
            let intent = raw.get(intent_start..).ok_or(WireError::Length)?;
            if crate::digest(witness) != c.snapshot_effects_proof_sha256
                || crate::digest(intent) != c.snapshot_effects_intent_sha256
            {
                return Err(WireError::Binding);
            }
            // These are not validated old CopyWitness/Intent objects. Their
            // existing schema/semantic/custody validation is still REQUIRED.
            Ok(UntrustedBody::Ready { witness, intent })
        }
        UntrustedControl::CommittedResult(c) => {
            if raw.is_empty()
                || raw.len() > profile::RESULT_BYTES as usize
                || usize::try_from(c.receipt_size).ok() != Some(raw.len())
            {
                return Err(WireError::Length);
            }
            if crate::digest(raw) != c.receipt_sha256 {
                return Err(WireError::Binding);
            }
            // Preserve old v2 receipt bytes; do not lexical-reorder them here.
            Ok(UntrustedBody::Receipt(raw))
        }
        _ if raw.is_empty() => Ok(UntrustedBody::Empty),
        _ => Err(WireError::Length),
    }
}

enum UntrustedMessage<'a> {
    HelloS(schema::HelloS),
    HelloD(schema::HelloD),
    Data(UntrustedControl, UntrustedBody<'a>),
    End,
    Abort,
}
struct UntrustedFrame<'a> {
    header: Header,
    message: UntrustedMessage<'a>,
}
fn parse_frame(raw: &[u8], header: Header) -> Result<UntrustedFrame<'_>, WireError> {
    let payload = raw.get(HEADER..).ok_or(WireError::Length)?;
    if usize::try_from(header.payload_bytes).ok() != Some(payload.len()) {
        return Err(WireError::Length);
    }
    let message = match header.kind {
        Kind::Metadata => match header.direction {
            Direction::SourceToDestination => {
                let h: schema::HelloS = registry::canonical_record(payload, profile::CONTROL_BYTES)
                    .map_err(|_| WireError::Json)?;
                if !h.valid(header) {
                    return Err(WireError::Fields);
                }
                UntrustedMessage::HelloS(h)
            }
            Direction::DestinationToSource => {
                let h: schema::HelloD = registry::canonical_record(payload, profile::CONTROL_BYTES)
                    .map_err(|_| WireError::Json)?;
                if !h.valid(header) {
                    return Err(WireError::Fields);
                }
                UntrustedMessage::HelloD(h)
            }
        },
        Kind::Data => {
            let parts = inspect_data(payload)?;
            let c = schema::decode(parts.tag, parts.control, header)?;
            let body = parse_body(&c, parts.body)?;
            UntrustedMessage::Data(c, body)
        }
        Kind::End if payload.is_empty() => UntrustedMessage::End,
        Kind::Abort if payload.is_empty() => UntrustedMessage::Abort,
        Kind::Abort => return Err(WireError::Unsupported),
        Kind::End => return Err(WireError::Length),
    };
    Ok(UntrustedFrame { header, message })
}

/// Encoders produce candidate bytes only; there is no IO or signer callback.
fn encode_frame(
    kind: Kind,
    direction: Direction,
    sequence: u64,
    payload: &[u8],
) -> Result<Vec<u8>, WireError> {
    let mut header = [0_u8; HEADER];
    header[..4].copy_from_slice(b"KWB1");
    header[4] = 1;
    header[5] = kind.octet();
    header[6] = direction.octet();
    header[8..16].copy_from_slice(&sequence.to_be_bytes());
    header[16..].copy_from_slice(
        &u32::try_from(payload.len())
            .map_err(|_| WireError::Length)?
            .to_be_bytes(),
    );
    let parsed = inspect_header(&header)?;
    if kind == Kind::Abort && !payload.is_empty() {
        return Err(WireError::Unsupported);
    }
    let mut bytes = Vec::with_capacity(HEADER + payload.len());
    bytes.extend_from_slice(&header);
    bytes.extend_from_slice(payload);
    parse_frame(&bytes, parsed)?;
    Ok(bytes)
}
fn encode_control(control: &UntrustedControl, body: &[u8]) -> Result<Vec<u8>, WireError> {
    let c = control.common();
    let header = Header {
        kind: Kind::Data,
        direction: c.direction,
        sequence: c.sequence,
        payload_bytes: 0,
    };
    if !control.valid(header) {
        return Err(WireError::Fields);
    }
    parse_body(control, body)?;
    let raw = control.canonical()?;
    let cap = if control.tag() == 4 {
        profile::CHUNK_CONTROL_BYTES
    } else {
        profile::CONTROL_BYTES
    };
    let len = 9_usize
        .checked_add(raw.len())
        .and_then(|n| n.checked_add(body.len()))
        .ok_or(WireError::Length)?;
    if raw.len() > cap as usize || len > profile::DATA_BYTES as usize {
        return Err(WireError::Length);
    }
    let mut payload = Vec::with_capacity(len);
    payload.push(control.tag());
    payload.extend_from_slice(
        &u32::try_from(raw.len())
            .map_err(|_| WireError::Length)?
            .to_be_bytes(),
    );
    payload.extend_from_slice(&raw);
    payload.extend_from_slice(
        &u32::try_from(body.len())
            .map_err(|_| WireError::Length)?
            .to_be_bytes(),
    );
    payload.extend_from_slice(body);
    encode_frame(Kind::Data, c.direction, c.sequence, &payload)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SyntaxPhase {
    HelloS,
    HelloD,
    SourceAuth,
    AuthAccepted,
    BetweenFiles,
    FileOpen,
    Ready,
    Final,
    Result,
    Ack,
    SourceEnd,
    DestinationEnd,
    BothEndsSeen,
    Aborted,
    Poisoned,
}
struct OpenFile {
    file_no: u32,
    size: u64,
    sha256: String,
    offset: u64,
    hash: Sha256,
}
struct ReadyBinding {
    proof: String,
    effect: String,
    intent: String,
}
/// Owns ONE immutable-lifetime budget for BOTH directions. No reset/extract.
/// Advancing this syntax machine never authenticates a message or grants D/P.
struct WireSyntax {
    phase: SyntaxPhase,
    next: [u64; 2],
    budget: profile::SessionBudget,
    transcript: transcript::CandidateTranscript,
    source: Option<schema::SourceBinding>,
    destination: Option<schema::DestinationBinding>,
    s_nonce: String,
    d_nonce: String,
    binding: String,
    auth_frame: String,
    source_key: String,
    seen: BTreeSet<(FileRole, u32)>,
    file: Option<OpenFile>,
    files: u32,
    reserved_package: u64,
    reserved_evidence: u64,
    actual_bytes: u64,
    ready: Option<ReadyBinding>,
    receipt: String,
}
impl Default for WireSyntax {
    fn default() -> Self {
        Self {
            phase: SyntaxPhase::HelloS,
            next: [0; 2],
            budget: profile::SessionBudget::default(),
            transcript: transcript::CandidateTranscript::default(),
            source: None,
            destination: None,
            s_nonce: String::new(),
            d_nonce: String::new(),
            binding: String::new(),
            auth_frame: String::new(),
            source_key: String::new(),
            seen: BTreeSet::new(),
            file: None,
            files: 0,
            reserved_package: 0,
            reserved_evidence: 0,
            actual_bytes: 0,
            ready: None,
            receipt: String::new(),
        }
    }
}
impl WireSyntax {
    fn observe_untrusted<'a>(&mut self, raw: &'a [u8]) -> Result<UntrustedFrame<'a>, WireError> {
        let result = self.observe_open(raw);
        if result.is_err() {
            self.poison();
        }
        result
    }
    fn check_header(&self, h: Header) -> Result<(), WireError> {
        match self.phase {
            SyntaxPhase::Poisoned => return Err(WireError::Poisoned),
            SyntaxPhase::BothEndsSeen | SyntaxPhase::Aborted => return Err(WireError::Closed),
            _ => {}
        }
        if self.phase == SyntaxPhase::DestinationEnd
            && h.direction == Direction::SourceToDestination
        {
            return Err(WireError::Closed);
        }
        if h.sequence != self.next[h.direction.index()] || h.sequence.checked_add(1).is_none() {
            return Err(WireError::Sequence);
        }
        Ok(())
    }
    fn prepare_complete_frame(&mut self, raw: &[u8]) -> Result<Header, WireError> {
        match self.phase {
            SyntaxPhase::Poisoned => return Err(WireError::Poisoned),
            SyntaxPhase::BothEndsSeen | SyntaxPhase::Aborted => return Err(WireError::Closed),
            _ => {}
        }
        let h = inspect_header(raw.get(..HEADER).ok_or(WireError::Header)?)?;
        self.check_header(h)?;
        if usize::try_from(h.payload_bytes)
            .ok()
            .and_then(|n| n.checked_add(HEADER))
            != Some(raw.len())
        {
            return Err(WireError::Length);
        }
        self.budget
            .charge_frame(h.kind.budget_kind(), u64::from(h.payload_bytes))
            .map_err(|_| WireError::Budget)?;
        Ok(h)
    }
    fn observe_open<'a>(&mut self, raw: &'a [u8]) -> Result<UntrustedFrame<'a>, WireError> {
        let header = self.prepare_complete_frame(raw)?;
        self.apply_reserved_bytes(raw, header)
    }
    /// Checks this one signed frame's DATA only. In particular a Final checked
    /// here never retroactively verifies a SourceAuth observed by pure syntax.
    /// No session-authenticated flag, clock release or authority is produced.
    fn observe_source_signature_data<'a>(
        &mut self,
        raw: &'a [u8],
        destination: &super::policy::UntrustedDestinationInputs,
    ) -> Result<auth::CheckedSourceSignatureData<'a>, WireError> {
        let mut operation = SignatureDataOperation {
            owner: self,
            finished: false,
        };
        let header = operation.owner.prepare_complete_frame(raw)?;
        let frame = parse_frame(raw, header)?;
        let checked = auth::verify_frame_data(operation.owner, frame, destination)?;
        operation
            .owner
            .apply_parsed_frame(raw, header, checked.frame_data())?;
        operation.finished = true;
        Ok(checked)
    }
    // Atomic full-frame and private original-owner reservation paths share the
    // same parsed mutation. The signed DATA path verifies before entering it.
    // None of these callers has a skip-charge flag.
    fn apply_reserved_bytes<'a>(
        &mut self,
        raw: &'a [u8],
        h: Header,
    ) -> Result<UntrustedFrame<'a>, WireError> {
        self.check_header(h)?;
        let frame = parse_frame(raw, h)?;
        self.apply_parsed_frame(raw, h, &frame)?;
        Ok(frame)
    }
    fn apply_parsed_frame(
        &mut self,
        raw: &[u8],
        h: Header,
        frame: &UntrustedFrame<'_>,
    ) -> Result<(), WireError> {
        self.check_header(h)?;
        let next = h.sequence.checked_add(1).ok_or(WireError::Sequence)?;
        match &frame.message {
            UntrustedMessage::Abort => self.phase = SyntaxPhase::Aborted,
            UntrustedMessage::HelloS(c) if self.phase == SyntaxPhase::HelloS => {
                self.s_nonce = c.s_nonce.clone();
                self.source = Some(c.source.clone());
                self.phase = SyntaxPhase::HelloD;
            }
            UntrustedMessage::HelloD(c) if self.phase == SyntaxPhase::HelloD => {
                if c.s_nonce != self.s_nonce {
                    return Err(WireError::Binding);
                }
                self.d_nonce = c.d_nonce.clone();
                self.binding = transcript::binding(
                    self.source.as_ref().ok_or(WireError::Phase)?,
                    &c.destination,
                )?;
                self.destination = Some(c.destination.clone());
                self.phase = SyntaxPhase::SourceAuth;
            }
            UntrustedMessage::Data(c, body) => {
                self.check_common(c)?;
                self.control(c, body)?;
                if matches!(c, UntrustedControl::SourceAuth(_)) {
                    self.auth_frame = transcript::frame_sha256(&raw[..HEADER], &raw[HEADER..]);
                }
            }
            UntrustedMessage::End => {
                self.phase = match (self.phase, h.direction) {
                    (SyntaxPhase::SourceEnd, Direction::SourceToDestination) => {
                        SyntaxPhase::DestinationEnd
                    }
                    (SyntaxPhase::DestinationEnd, Direction::DestinationToSource) => {
                        SyntaxPhase::BothEndsSeen
                    }
                    _ => return Err(WireError::Phase),
                };
            }
            _ => return Err(WireError::Phase),
        }
        self.transcript
            .observe_untrusted(h.direction, &raw[..HEADER], &raw[HEADER..])?;
        self.next[h.direction.index()] = next;
        Ok(())
    }
    fn check_common(&self, control: &UntrustedControl) -> Result<(), WireError> {
        let c = control.common();
        if c.s_nonce != self.s_nonce
            || c.d_nonce != self.d_nonce
            || c.binding_sha256 != self.binding
        {
            return Err(WireError::Binding);
        }
        if c.prev_s != hex::encode(self.transcript.head(Direction::SourceToDestination))
            || c.prev_d != hex::encode(self.transcript.head(Direction::DestinationToSource))
        {
            return Err(WireError::Transcript);
        }
        Ok(())
    }
    fn control(&mut self, c: &UntrustedControl, body: &UntrustedBody<'_>) -> Result<(), WireError> {
        match c {
            UntrustedControl::SourceAuth(c) if self.phase == SyntaxPhase::SourceAuth => {
                self.check_source_auth_data(c)?;
                self.source_key = c.source_auth_key_sha256.clone();
                self.phase = SyntaxPhase::AuthAccepted;
            }
            UntrustedControl::AuthAccepted(c) if self.phase == SyntaxPhase::AuthAccepted => {
                if c.source_auth_frame_sha256 != self.auth_frame {
                    return Err(WireError::Binding);
                }
                self.phase = SyntaxPhase::BetweenFiles;
            }
            UntrustedControl::FileBegin(c) if self.phase == SyntaxPhase::BetweenFiles => {
                self.begin(c)?
            }
            UntrustedControl::FileChunk(c) if self.phase == SyntaxPhase::FileOpen => {
                let UntrustedBody::Chunk(bytes) = body else {
                    return Err(WireError::Length);
                };
                let file = self.file.as_mut().ok_or(WireError::File)?;
                let end = file
                    .offset
                    .checked_add(bytes.len() as u64)
                    .ok_or(WireError::File)?;
                if c.file_no != file.file_no || c.offset != file.offset || end > file.size {
                    return Err(WireError::File);
                }
                self.actual_bytes = self
                    .actual_bytes
                    .checked_add(bytes.len() as u64)
                    .ok_or(WireError::File)?;
                file.hash.update(bytes);
                file.offset = end;
            }
            UntrustedControl::FileEnd(c) if self.phase == SyntaxPhase::FileOpen => {
                let file = self.file.take().ok_or(WireError::File)?;
                if c.file_no != file.file_no
                    || c.size != file.size
                    || c.sha256 != file.sha256
                    || file.offset != file.size
                    || hex::encode(file.hash.finalize()) != file.sha256
                {
                    return Err(WireError::File);
                }
                self.files = self.files.checked_add(1).ok_or(WireError::File)?;
                self.phase = SyntaxPhase::BetweenFiles;
            }
            UntrustedControl::SourceDataEnd(c) if self.phase == SyntaxPhase::BetweenFiles => {
                let source = self.source.as_ref().ok_or(WireError::Phase)?;
                if self.files == 0
                    || c.files != self.files
                    || c.files != source.file_count
                    || c.bytes != self.actual_bytes
                    || self.reserved_package != source.package_bytes
                    || self.reserved_evidence != source.evidence_bytes
                    || c.package_roster_sha256 != source.package_roster_sha256
                    || c.captured_roster_sha256 != source.captured_roster_sha256
                    || c.snapshot_effects_roster_sha256 != source.snapshot_effects_roster_sha256
                {
                    return Err(WireError::File);
                }
                self.phase = SyntaxPhase::Ready;
            }
            UntrustedControl::DestinationReady(c) if self.phase == SyntaxPhase::Ready => {
                let source = self.source.as_ref().ok_or(WireError::Phase)?;
                if !self.attempt(&c.attempt_id)
                    || c.manifest_sha256 != source.manifest_sha256
                    || c.snapshot_effects_input_sha256 != source.snapshot_effects_input_sha256
                {
                    return Err(WireError::Binding);
                }
                self.ready = Some(ReadyBinding {
                    proof: c.proof_sha256.clone(),
                    effect: c.snapshot_effects_proof_sha256.clone(),
                    intent: c.snapshot_effects_intent_sha256.clone(),
                });
                self.phase = SyntaxPhase::Final;
            }
            UntrustedControl::SourceFinalDecision(c) if self.phase == SyntaxPhase::Final => {
                self.check_source_final_data(c)?;
                self.phase = SyntaxPhase::Result;
            }
            UntrustedControl::CommittedResult(c) if self.phase == SyntaxPhase::Result => {
                if !self.attempt(&c.attempt_id)
                    || self
                        .ready
                        .as_ref()
                        .is_none_or(|r| r.proof != c.proof_sha256)
                {
                    return Err(WireError::Binding);
                }
                self.receipt = c.receipt_sha256.clone();
                self.phase = SyntaxPhase::Ack;
            }
            UntrustedControl::SourceResultAck(c) if self.phase == SyntaxPhase::Ack => {
                if !self.attempt(&c.attempt_id) || c.receipt_sha256 != self.receipt {
                    return Err(WireError::Binding);
                }
                self.phase = SyntaxPhase::SourceEnd;
            }
            _ => return Err(WireError::Phase),
        }
        Ok(())
    }
    fn check_source_auth_data(&self, c: &schema::SourceAuth) -> Result<(), WireError> {
        if self.phase != SyntaxPhase::SourceAuth {
            return Err(WireError::Phase);
        }
        if self.source.as_ref() != Some(&c.source)
            || self.destination.as_ref() != Some(&c.destination)
        {
            return Err(WireError::Binding);
        }
        Ok(())
    }
    fn check_source_final_data(&self, c: &schema::SourceFinalDecision) -> Result<(), WireError> {
        if self.phase != SyntaxPhase::Final {
            return Err(WireError::Phase);
        }
        let source = self.source.as_ref().ok_or(WireError::Phase)?;
        let ready = self.ready.as_ref().ok_or(WireError::Phase)?;
        if !self.attempt(&c.attempt_id)
            || c.proof_sha256 != ready.proof
            || c.snapshot_effects_proof_sha256 != ready.effect
            || c.snapshot_effects_intent_sha256 != ready.intent
            || c.source_auth_key_sha256 != self.source_key
            || c.snapshot_effects_input_sha256 != source.snapshot_effects_input_sha256
            || c.fence.operation_id != source.source_operation_id
            || c.fence.source_actor_binding_sha256
                != transcript::actor_binding(
                    &source.source_operation_id,
                    &self.binding,
                    &self.s_nonce,
                    &self.d_nonce,
                )?
        {
            return Err(WireError::Binding);
        }
        Ok(())
    }
    fn attempt(&self, id: &str) -> bool {
        self.destination
            .as_ref()
            .is_some_and(|d| d.attempt_id == id)
    }
    fn begin(&mut self, c: &schema::FileBegin) -> Result<(), WireError> {
        if c.file_no != self.files
            || self.file.is_some()
            || (c.role == FileRole::PackageManifest) != (self.files == 0)
            || self.seen.contains(&(c.role, c.roster_index))
        {
            return Err(WireError::File);
        }
        let source = self.source.as_ref().ok_or(WireError::Phase)?;
        if c.file_no >= source.file_count {
            return Err(WireError::File);
        }
        let (counter, declared, class) = if c.role.package() {
            (
                &mut self.reserved_package,
                source.package_bytes,
                profile::FileClass::Package,
            )
        } else {
            (
                &mut self.reserved_evidence,
                source.evidence_bytes,
                profile::FileClass::Evidence,
            )
        };
        let next = counter.checked_add(c.size).ok_or(WireError::File)?;
        if next > declared {
            return Err(WireError::File);
        }
        self.budget
            .charge_file(class, c.size)
            .map_err(|_| WireError::Budget)?;
        *counter = next;
        self.seen.insert((c.role, c.roster_index));
        self.file = Some(OpenFile {
            file_no: c.file_no,
            size: c.size,
            sha256: c.sha256.clone(),
            offset: 0,
            hash: Sha256::new(),
        });
        self.phase = SyntaxPhase::FileOpen;
        Ok(())
    }
    fn poison(&mut self) {
        self.phase = SyntaxPhase::Poisoned;
        self.budget.poison();
    }
    fn reserve_frame(&mut self, direction: Direction) -> Result<FrameReservation<'_>, WireError> {
        let h = Header {
            kind: Kind::End,
            direction,
            sequence: self.next[direction.index()],
            payload_bytes: 0,
        };
        if let Err(error) = self.check_header(h) {
            self.poison();
            return Err(error);
        }
        if self.budget.reserve_header().is_err() {
            self.poison();
            return Err(WireError::Budget);
        }
        Ok(FrameReservation {
            owner: self,
            direction,
            header: None,
            staged: false,
            finished: false,
        })
    }
    fn reserve_eof(&mut self) -> Result<EofReservation<'_>, WireError> {
        if self.phase == SyntaxPhase::Poisoned {
            return Err(WireError::Poisoned);
        }
        if self.budget.reserve_eof_probe().is_err() {
            self.poison();
            return Err(WireError::Budget);
        }
        Ok(EofReservation {
            owner: self,
            finished: false,
        })
    }
    /// Pure state inspection ONLY. Never a socket EOF, exit0 or success receipt.
    /// Ok includes Aborted and therefore does not mean normal completion.
    fn finish_syntax(&mut self) -> Result<(), WireError> {
        match self.phase {
            SyntaxPhase::BothEndsSeen | SyntaxPhase::Aborted => Ok(()),
            SyntaxPhase::Poisoned => Err(WireError::Poisoned),
            _ => {
                self.phase = SyntaxPhase::Poisoned;
                Err(WireError::Incomplete)
            }
        }
    }
}

// Private original owner: even unwinding during complete-frame verification
// poisons the same syntax and budget; it cannot retry via the pure entry point.
struct SignatureDataOperation<'a> {
    owner: &'a mut WireSyntax,
    finished: bool,
}
impl Drop for SignatureDataOperation<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.owner.poison();
        }
    }
}

// Mutably borrowing the original owner prevents cloning/moving its budget or
// completing another session with this reservation. Drop/cancel never refunds.
struct FrameReservation<'a> {
    owner: &'a mut WireSyntax,
    direction: Direction,
    header: Option<Header>,
    staged: bool,
    finished: bool,
}
impl FrameReservation<'_> {
    fn declare(&mut self, raw: &[u8]) -> Result<Header, WireError> {
        let result = self.declare_open(raw);
        if result.is_err() {
            self.owner.poison();
        }
        result
    }
    fn declare_open(&mut self, raw: &[u8]) -> Result<Header, WireError> {
        if self.header.is_some() {
            return Err(WireError::Budget);
        }
        let h = inspect_header(raw)?;
        if h.direction != self.direction {
            return Err(WireError::Fields);
        }
        self.owner.check_header(h)?;
        let expected_kind = match self.owner.phase {
            SyntaxPhase::HelloS | SyntaxPhase::HelloD => Kind::Metadata,
            SyntaxPhase::SourceEnd | SyntaxPhase::DestinationEnd => Kind::End,
            _ => Kind::Data,
        };
        // Kind is already knowable from the fixed header. Reject before any
        // payload allocation/read; control tags still use the one old parser.
        if h.kind != Kind::Abort && h.kind != expected_kind {
            return Err(WireError::Phase);
        }
        self.owner
            .budget
            .reserve_payload(h.kind.budget_kind(), u64::from(h.payload_bytes))
            .map_err(|_| WireError::Budget)?;
        self.header = Some(h);
        Ok(h)
    }
    fn stage<'a>(&mut self, raw: &'a [u8]) -> Result<UntrustedFrame<'a>, WireError> {
        let result = self.stage_open(raw);
        if result.is_err() {
            self.owner.poison();
        }
        result
    }
    fn stage_open<'a>(&mut self, raw: &'a [u8]) -> Result<UntrustedFrame<'a>, WireError> {
        if self.staged {
            return Err(WireError::Budget);
        }
        let h = self.header.ok_or(WireError::Budget)?;
        if inspect_header(raw.get(..HEADER).ok_or(WireError::Header)?)? != h {
            return Err(WireError::Header);
        }
        let frame = self.owner.apply_reserved_bytes(raw, h)?;
        self.staged = true;
        Ok(frame)
    }
    fn finish(mut self) -> Result<(), WireError> {
        if !self.staged {
            return Err(WireError::Budget);
        }
        self.owner
            .budget
            .finish_reserved_frame()
            .map_err(|_| WireError::Budget)?;
        self.finished = true;
        Ok(())
    }
}
impl Drop for FrameReservation<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.owner.poison();
        }
    }
}
struct EofReservation<'a> {
    owner: &'a mut WireSyntax,
    finished: bool,
}
impl EofReservation<'_> {
    fn finish(mut self) -> Result<(), WireError> {
        self.owner
            .budget
            .finish_eof_probe()
            .map_err(|_| WireError::Budget)?;
        self.finished = true;
        Ok(())
    }
}
impl Drop for EofReservation<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.owner.poison();
        }
    }
}

#[cfg(test)]
mod tests;

/// Only the fixed original-D IO loop can construct these after pre-apply
/// nonce/transcript/binding/ring verification and the actual D view checks.
/// The identity is the actual original operation Arc, never a caller ID/bool.
pub(super) struct VerifiedSourceAuthInOriginalD {
    original_operation: std::sync::Arc<std::sync::Mutex<crate::destination::OperationAuthority>>,
    control: Box<schema::SourceAuth>,
    frame_sha256: String,
}
pub(super) struct VerifiedFinalInOriginalD {
    original_operation: std::sync::Arc<std::sync::Mutex<crate::destination::OperationAuthority>>,
    control: schema::SourceFinalDecision,
    frame_sha256: String,
}
impl VerifiedSourceAuthInOriginalD {
    pub(super) fn require_original(
        &self,
        original: &mut super::destination_actor::OriginalDestinationSession<'_>,
    ) -> Result<(), crate::BackupError> {
        let mut view = original.wire_identity()?;
        view.recheck()?;
        if !std::sync::Arc::ptr_eq(&self.original_operation, view.operation_original()) {
            return Err(original_wire_refused());
        }
        view.recheck()
    }
    pub(super) fn control_data(&self) -> &schema::SourceAuth {
        &self.control
    }
    pub(super) fn frame_sha256_data(&self) -> &str {
        &self.frame_sha256
    }
}
impl VerifiedFinalInOriginalD {
    pub(super) fn require_original(
        &self,
        original: &mut super::destination_actor::OriginalDestinationSession<'_>,
    ) -> Result<(), crate::BackupError> {
        let mut view = original.wire_identity()?;
        view.recheck()?;
        if !std::sync::Arc::ptr_eq(&self.original_operation, view.operation_original()) {
            return Err(original_wire_refused());
        }
        view.recheck()
    }
    pub(super) fn control_data(&self) -> &schema::SourceFinalDecision {
        &self.control
    }
    pub(super) fn frame_sha256_data(&self) -> &str {
        &self.frame_sha256
    }
}
fn original_wire_refused() -> crate::BackupError {
    crate::BackupError::Invalid("B1 original authenticated wire operation")
}

/// A terminal observation from the actual original pipe owner only. Not a
/// wire message, DTO permission, candidate observation or caller boolean.
pub(super) struct VerifiedOriginalDestinationEnd {
    original_operation: std::sync::Arc<std::sync::Mutex<crate::destination::OperationAuthority>>,
    entry: &'static super::destination_actor::OriginalDEntryScope,
}
impl VerifiedOriginalDestinationEnd {
    pub(super) fn require_original(
        &self,
        original: &mut super::destination_actor::OriginalDestinationSession<'_>,
    ) -> Result<(), crate::BackupError> {
        let mut view = original.wire_identity()?;
        view.recheck()?;
        if !std::ptr::eq(self.entry, view.entry_original())
            || !std::sync::Arc::ptr_eq(&self.original_operation, view.operation_original())
        {
            return Err(original_wire_refused());
        }
        view.recheck()
    }
}
enum VerifiedOriginalDControl {
    Auth(VerifiedSourceAuthInOriginalD),
    Final(Box<VerifiedFinalInOriginalD>),
}
/// The caller has not applied this frame to syntax or D. Verification checks
/// the current syntax's PREVIOUS heads, not fields adopted from this message.
fn verify_original_destination_control(
    owner: &WireSyntax,
    frame: UntrustedFrame<'_>,
    header_raw: &[u8],
    payload_raw: &[u8],
    original: &mut super::destination_actor::OriginalDestinationSession<'_>,
) -> Result<VerifiedOriginalDControl, crate::BackupError> {
    use ring::signature::{ED25519, UnparsedPublicKey};
    let mut view = original.wire_identity()?;
    view.recheck()?;
    let UntrustedMessage::Data(control, UntrustedBody::Empty) = frame.message else {
        return Err(original_wire_refused());
    };
    owner
        .check_common(&control)
        .map_err(|_| original_wire_refused())?;
    let (claimed_key, signature_hex) = match &control {
        UntrustedControl::SourceAuth(c) => {
            owner
                .check_source_auth_data(c)
                .map_err(|_| original_wire_refused())?;
            (&c.source_auth_key_sha256, &c.signature)
        }
        UntrustedControl::SourceFinalDecision(c) => {
            owner
                .check_source_final_data(c)
                .map_err(|_| original_wire_refused())?;
            (&c.source_auth_key_sha256, &c.signature)
        }
        _ => return Err(original_wire_refused()),
    };
    let pins = view.signature_policy_data()?;
    let source = owner.source.as_ref().ok_or_else(original_wire_refused)?;
    let destination = owner
        .destination
        .as_ref()
        .ok_or_else(original_wire_refused)?;
    if (
        source.deployment_id.as_str(),
        source.source_group_id.as_str(),
        source.manager_release_sha256.as_str(),
        source.application_build_sha256.as_str(),
    ) != pins.source_data()
        || (
            destination.deployment_id.as_str(),
            destination.destination_group_id.as_str(),
            destination.registry_authority_sha256.as_str(),
            destination.receiver_release_sha256.as_str(),
        ) != pins.destination_data()
        || claimed_key != view.source_auth_key_sha256_data()
        || crate::digest(view.source_verifier_public_key_data()) != *claimed_key
        || !crate::valid_hex(signature_hex, 128)
    {
        return Err(original_wire_refused());
    }
    let (hex_key, pin) = pins.public_key_data();
    if pin != claimed_key || !crate::valid_hex(hex_key, 64) {
        return Err(original_wire_refused());
    }
    let mut expected_key = [0u8; 32];
    hex::decode_to_slice(hex_key, &mut expected_key).map_err(|_| original_wire_refused())?;
    if &expected_key != view.source_verifier_public_key_data() {
        return Err(original_wire_refused());
    }
    let preimage = transcript::candidate_preimage(&control).map_err(|_| original_wire_refused())?;
    let mut signature = [0u8; 64];
    hex::decode_to_slice(signature_hex, &mut signature).map_err(|_| original_wire_refused())?;
    UnparsedPublicKey::new(&ED25519, &expected_key)
        .verify(&preimage, &signature)
        .map_err(|_| original_wire_refused())?;
    let operation = std::sync::Arc::clone(view.operation_original());
    view.recheck()?;
    let frame_sha256 = transcript::frame_sha256(header_raw, payload_raw);
    match control {
        UntrustedControl::SourceAuth(control) => Ok(VerifiedOriginalDControl::Auth(
            VerifiedSourceAuthInOriginalD {
                original_operation: operation,
                control,
                frame_sha256,
            },
        )),
        UntrustedControl::SourceFinalDecision(control) => {
            view.prepay_wire_work(
                u64::try_from(std::mem::size_of::<VerifiedFinalInOriginalD>())
                    .map_err(|_| crate::BackupError::Overflow)?,
                1,
            )?;
            view.recheck()?;
            Ok(VerifiedOriginalDControl::Final(Box::new(
                VerifiedFinalInOriginalD {
                    original_operation: operation,
                    control,
                    frame_sha256,
                },
            )))
        }
        _ => Err(original_wire_refused()),
    }
}

/// Fixed Source transfer DATA derived only from the live first S. It is not an
/// actor, cut, copied proof or publication permission and has no constructor.
struct OriginalSourceTransferData {
    source: SourceBinding,
    transport_index: Vec<u8>,
    proof_raw: Vec<u8>,
    payload_manifest: Vec<u8>,
    capture: crate::source::snapshot_effects::wire::Capture,
}
fn source_transfer_data(
    first: &super::source_actor::SourceFirstS<'_>,
) -> Result<OriginalSourceTransferData, crate::BackupError> {
    let lease = first.lease_original();
    let source = first.source_original()?;
    let pin = first.pin()?;
    let metadata = first.fence_metadata()?;
    let input_bytes = pin.source_bytes().values().try_fold(0u64, |n, raw| {
        n.checked_add(raw.len() as u64)
            .ok_or(crate::BackupError::Overflow)
    })?;
    let work = (pin.manifest_raw_bytes().len() as u64)
        .checked_mul(64)
        .and_then(|n| n.checked_add(input_bytes.checked_mul(8)?))
        .and_then(|n| {
            n.checked_add(
                ((pin.manifest().files.len() + pin.effects_records().len() + 32) as u64)
                    .checked_mul(4096)?,
            )
        })
        .and_then(|n| n.checked_add(524288))
        .ok_or(crate::BackupError::Overflow)?;
    first.original_owner().prepay_wire_work(
        work,
        pin.manifest().files.len() + pin.effects_records().len() + 128,
    )?;
    let capture = pin.capture_data().clone();
    let capture_record = pin
        .effects_records()
        .iter()
        .find(|record| record.path == "capture.json")
        .ok_or_else(original_wire_refused)?;
    let transport_index = registry::canonical(
        &serde_json::json!({"schema_version":1,"profile":profile::PROFILE,
        "backup_id":pin.manifest().backup_id,"source_capture_sha256":capture_record.sha256,"files":pin.effects_records()}),
    )?;
    if transport_index.len() > profile::CONTROL_BYTES as usize {
        return Err(original_wire_refused());
    }
    let payload_manifest = registry::canonical(
        &serde_json::json!({"format_version":2,"capability":"snapshot_effect_payload_v2",
        "backup_id":pin.manifest().backup_id,"files":pin.effects_records()}),
    )?;
    let mut roots = Vec::with_capacity(4);
    for role in ["assets", "control", "local_pins", "staging"] {
        let enrolled = lease
            .roots
            .get(
                source
                    .group
                    .roots
                    .get(role)
                    .ok_or_else(original_wire_refused)?,
            )
            .ok_or_else(original_wire_refused)?;
        roots.push(schema::RootRecord {
            role: role.into(),
            enrollment_id: enrolled.enrollment_id.clone(),
            path: enrolled.path.clone(),
            dev: enrolled.dev,
            ino: enrolled.ino,
            uid: enrolled.uid,
            mode: enrolled.mode,
            record_sha256: crate::digest(&registry::canonical(enrolled)?),
        });
    }
    let manifest_sha256 = crate::digest(pin.manifest_raw_bytes());
    let captured_roster_sha256 = crate::digest(&registry::canonical(&pin.source_records())?);
    let snapshot_effects_roster_sha256 =
        crate::digest(&registry::canonical(&pin.effects_records())?);
    let effects_input = registry::canonical(
        &serde_json::json!({"backup_id":pin.manifest().backup_id,"source_group_id":source.group.group_id,
        "manifest_sha256":manifest_sha256,"capture_sha256":capture_record.sha256,"roster_sha256":snapshot_effects_roster_sha256,
        "transport_index_sha256":crate::digest(&transport_index)}),
    )?;
    let mut input_hash = Sha256::new();
    input_hash.update(b"KnowWeave/Task9/B1/effects-input/v1\0");
    input_hash.update(effects_input);
    let package_bytes = pin
        .manifest()
        .files
        .iter()
        .try_fold(pin.manifest_raw_bytes().len() as u64, |n, r| {
            n.checked_add(r.size).ok_or(crate::BackupError::Overflow)
        })?;
    let effect_bytes = pin
        .effects_records()
        .iter()
        .try_fold(transport_index.len() as u64, |n, r| {
            n.checked_add(r.size).ok_or(crate::BackupError::Overflow)
        })?;
    let evidence_bytes = input_bytes
        .checked_add(effect_bytes)
        .ok_or(crate::BackupError::Overflow)?;
    let file_count = pin
        .manifest()
        .files
        .len()
        .checked_add(pin.source_records().len())
        .and_then(|n| n.checked_add(pin.effects_records().len()))
        .and_then(|n| n.checked_add(2))
        .ok_or(crate::BackupError::Overflow)?;
    let binding = SourceBinding {
        deployment_id: source.group.deployment_id.clone(),
        source_group_id: source.group.group_id.clone(),
        backup_id: pin.manifest().backup_id.to_string(),
        registry_authority_sha256: crate::digest(&registry::canonical(&lease.authority)?),
        registry_generation: lease.generation().generation,
        generation_sha256: crate::digest(&registry::canonical(lease.generation())?),
        source_binding_sha256: source.group.source_binding_sha256.clone(),
        application_commit: source.group.application_commit.clone(),
        application_build_sha256: source.group.application_build_sha256.clone(),
        manager_release_sha256: metadata.manager_release_sha256_data().into(),
        roots,
        manifest_sha256,
        source_control_sha256: crate::digest(
            pin.source_bytes()
                .get("source-released.json")
                .ok_or_else(original_wire_refused)?,
        ),
        captured_roster_sha256,
        snapshot_effects_roster_sha256,
        package_bytes,
        evidence_bytes,
        file_count: u32::try_from(file_count).map_err(|_| original_wire_refused())?,
        package_roster_sha256: pin.package_roster_sha256(lease)?,
        source_operation_id: first.source_operation_id_data()?.to_string(),
        snapshot_effects_transport_index_sha256: crate::digest(&transport_index),
        snapshot_effects_input_sha256: hex::encode(input_hash.finalize()),
    };
    if !binding.valid() {
        return Err(original_wire_refused());
    }
    let proof_raw = crate::destination::captured_proof_record_bytes_from_files(
        pin.manifest().backup_id,
        pin.manifest(),
        pin.source_bytes(),
    )?;
    Ok(OriginalSourceTransferData {
        source: binding,
        transport_index,
        proof_raw,
        payload_manifest,
        capture,
    })
}

/// Only the fixed original Source loop supplies these typed controls, derived
/// from the current syntax and actual originals. There is no arbitrary bytes,
/// algorithm, path or domain signing endpoint.
pub(super) fn sign_source_auth_with_original_key(
    first: &super::source_actor::SourceFirstS<'_>,
    control: &mut SourceAuth,
    io: &OriginalPipeIo,
    key: &ring::signature::Ed25519KeyPair,
) -> Result<(), crate::BackupError> {
    use ring::signature::KeyPair;
    let expected = source_transfer_data(first)?;
    let syntax = io.syntax_original();
    let check_raw = registry::canonical(control)?;
    let checked: SourceAuth = registry::canonical_record(&check_raw, profile::CONTROL_BYTES)?;
    let checked = UntrustedControl::SourceAuth(Box::new(checked));
    syntax
        .check_common(&checked)
        .map_err(|_| original_wire_refused())?;
    syntax
        .check_source_auth_data(control)
        .map_err(|_| original_wire_refused())?;
    if control.source != expected.source
        || control.s_nonce != hex::encode(first.source_nonce_data()?)
        || control.source_auth_key_sha256 != first.fence_metadata()?.source_auth_sha256_data()
        || crate::digest(key.public_key().as_ref()) != control.source_auth_key_sha256
        || control.signature != "0".repeat(128)
    {
        return Err(original_wire_refused());
    }
    let unsigned = schema::UntrustedControl::SourceAuth(Box::new(SourceAuth {
        schema_version: control.schema_version,
        profile: control.profile.clone(),
        kind: control.kind.clone(),
        direction: control.direction,
        sequence: control.sequence,
        s_nonce: control.s_nonce.clone(),
        d_nonce: control.d_nonce.clone(),
        prev_s: control.prev_s.clone(),
        prev_d: control.prev_d.clone(),
        binding_sha256: control.binding_sha256.clone(),
        source: control.source.clone(),
        destination: control.destination.clone(),
        source_auth_key_sha256: control.source_auth_key_sha256.clone(),
        signature: control.signature.clone(),
    }));
    let preimage =
        transcript::candidate_preimage(&unsigned).map_err(|_| original_wire_refused())?;
    control.signature = hex::encode(key.sign(&preimage).as_ref());
    Ok(())
}
pub(super) fn sign_source_final_with_original_key(
    first: &super::source_actor::SourceFirstS<'_>,
    control: &mut SourceFinalDecision,
    io: &OriginalPipeIo,
    fence: &mut super::source_fence::DurableSourceFence<'_>,
    key: &ring::signature::Ed25519KeyPair,
) -> Result<(), crate::BackupError> {
    use ring::signature::KeyPair;
    io.syntax_original()
        .check_source_final_data(control)
        .map_err(|_| original_wire_refused())?;
    fence.recheck_held()?;
    if registry::canonical(&control.fence)? != registry::canonical(&fence.wire_data()?)?
        || control.s_nonce != hex::encode(first.source_nonce_data()?)
        || control.fence.operation_id != first.source_operation_id_data()?.to_string()
        || control.fence.protection_snapshot_sha256 != first.current_snapshot_sha256_data()?
        || control.source_auth_key_sha256 != first.fence_metadata()?.source_auth_sha256_data()
        || crate::digest(key.public_key().as_ref()) != control.source_auth_key_sha256
        || control.signature != "0".repeat(128)
    {
        return Err(original_wire_refused());
    }
    let raw = registry::canonical(control)?;
    let unsigned: schema::SourceFinalDecision =
        registry::canonical_record(&raw, profile::CONTROL_BYTES)?;
    let preimage = transcript::candidate_preimage(&UntrustedControl::SourceFinalDecision(unsigned))
        .map_err(|_| original_wire_refused())?;
    control.signature = hex::encode(key.sign(&preimage).as_ref());
    Ok(())
}

macro_rules! original_control {
    ($name:ident,$syntax:expr,$direction:expr,{$($field:ident:$value:expr),*$(,)?})=>{{
        let syntax=$syntax;let direction=$direction;
        schema::$name {schema_version:1,profile:profile::PROFILE.into(),kind:stringify!($name).into(),direction,
            sequence:syntax.next[direction.index()],s_nonce:syntax.s_nonce.clone(),d_nonce:syntax.d_nonce.clone(),
            prev_s:hex::encode(syntax.transcript.head(Direction::SourceToDestination)),
            prev_d:hex::encode(syntax.transcript.head(Direction::DestinationToSource)),binding_sha256:syntax.binding.clone(),$($field:$value),*}
    }};
}
fn replace_original_envelope(
    control: &mut UntrustedControl,
    syntax: &WireSyntax,
    direction: Direction,
) {
    macro_rules! replace {
        ($value:expr) => {{
            let value = $value;
            value.schema_version = 1;
            value.profile = profile::PROFILE.into();
            value.direction = direction;
            value.sequence = syntax.next[direction.index()];
            value.s_nonce = syntax.s_nonce.clone();
            value.d_nonce = syntax.d_nonce.clone();
            value.prev_s = hex::encode(syntax.transcript.head(Direction::SourceToDestination));
            value.prev_d = hex::encode(syntax.transcript.head(Direction::DestinationToSource));
            value.binding_sha256 = syntax.binding.clone();
        }};
    }
    match control {
        UntrustedControl::SourceAuth(c) => replace!(c),
        UntrustedControl::AuthAccepted(c) => replace!(c),
        UntrustedControl::FileBegin(c) => replace!(c),
        UntrustedControl::FileChunk(c) => replace!(c),
        UntrustedControl::FileEnd(c) => replace!(c),
        UntrustedControl::SourceDataEnd(c) => replace!(c),
        UntrustedControl::DestinationReady(c) => replace!(c),
        UntrustedControl::SourceFinalDecision(c) => replace!(c),
        UntrustedControl::CommittedResult(c) => replace!(c),
        UntrustedControl::SourceResultAck(c) => replace!(c),
    }
}
fn charged_random_nonce(
    entry: &super::destination_actor::OriginalDEntryScope,
) -> Result<String, crate::BackupError> {
    entry.recheck()?;
    entry.prepay_wire_work(96, 4)?;
    let mut bytes = [0u8; 32];
    let mut offset = 0;
    while offset < bytes.len() {
        entry.recheck()?;
        entry.prepay_wire_work((bytes.len() - offset) as u64, 1)?;
        let n = unsafe {
            libc::getrandom(bytes[offset..].as_mut_ptr().cast(), bytes.len() - offset, 0)
        };
        if n < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(original_wire_refused());
        }
        if n == 0 {
            return Err(original_wire_refused());
        }
        offset += usize::try_from(n).map_err(|_| original_wire_refused())?;
    }
    Ok(hex::encode(bytes))
}
async fn write_original_control(
    io: &mut OriginalPipeIo,
    control: &UntrustedControl,
    body: &[u8],
) -> Result<(), crate::BackupError> {
    io.prepay_original_serialization(body.len())?;
    let raw = encode_control(control, body).map_err(|_| original_wire_refused())?;
    io.write_frame(&raw)
        .await
        .map_err(|_| original_wire_refused())
}
/// The single fixed D loop. The original scope owns its native custody, IO
/// offsets and every producer. Neither a decoded control nor a signed DATA
/// test can call D publication without this actual original session.
pub(super) async fn run_original_destination(
    original: &mut super::destination_actor::OriginalDestinationSession<'_>,
) -> Result<(), crate::BackupError> {
    let pair = original.original_pipe_pair()?;
    let storage = original.original_wire_io_slot();
    let mut slot = storage.lock().await;
    if slot.is_some() {
        return Err(original_wire_refused());
    }
    *slot = Some(OriginalPipeIo::for_original_destination(pair, original)?);
    let io = slot.as_mut().ok_or_else(original_wire_refused)?;
    {
        let (incoming, verified) = io.read_original_destination_frame(original).await?;
        if verified.is_some() || !matches!(incoming.message, UntrustedMessage::HelloS(_)) {
            return Err(original_wire_refused());
        }
    }
    let destination = original.hello_data()?;
    let entry = {
        let mut view = original.wire_identity()?;
        view.recheck()?;
        view.entry_original()
    };
    entry.prepay_wire_work(131072, 24)?;
    let hello = schema::HelloD {
        schema_version: 1,
        profile: profile::PROFILE.into(),
        kind: "HelloD".into(),
        direction: Direction::DestinationToSource,
        sequence: 0,
        s_nonce: io.syntax_original().s_nonce.clone(),
        d_nonce: charged_random_nonce(entry)?,
        destination,
    };
    let raw = encode_frame(
        Kind::Metadata,
        Direction::DestinationToSource,
        0,
        &registry::canonical(&hello)?,
    )
    .map_err(|_| original_wire_refused())?;
    io.write_frame(&raw)
        .await
        .map_err(|_| original_wire_refused())?;
    let source_auth_frame = {
        let (incoming, verified) = io.read_original_destination_frame(original).await?;
        if !matches!(
            incoming.message,
            UntrustedMessage::Data(UntrustedControl::SourceAuth(_), UntrustedBody::Empty)
        ) {
            return Err(original_wire_refused());
        }
        let Some(VerifiedOriginalDControl::Auth(auth)) = verified else {
            return Err(original_wire_refused());
        };
        entry.prepay_wire_work(64, 1)?;
        let hash = auth.frame_sha256_data().to_owned();
        original.admit_authenticated_source(auth)?;
        hash
    };
    entry.prepay_wire_work(131072, 16)?;
    let accepted = original_control!(AuthAccepted,io.syntax_original(),Direction::DestinationToSource,{source_auth_frame_sha256:source_auth_frame});
    write_original_control(io, &UntrustedControl::AuthAccepted(accepted), &[]).await?;
    io.mark_original_handshake_authenticated()?;
    let ready = loop {
        let (incoming, verified) = io.read_original_destination_frame(original).await?;
        if verified.is_some() {
            return Err(original_wire_refused());
        }
        match incoming.message {
            UntrustedMessage::Data(UntrustedControl::FileBegin(begin), UntrustedBody::Empty) => {
                original.begin_fixed_file(&begin)?
            }
            UntrustedMessage::Data(
                UntrustedControl::FileChunk(chunk),
                UntrustedBody::Chunk(bytes),
            ) => original.write_file_bytes(&chunk, bytes)?,
            UntrustedMessage::Data(UntrustedControl::FileEnd(end), UntrustedBody::Empty) => {
                original.finish_fixed_file(&end)?
            }
            UntrustedMessage::Data(UntrustedControl::SourceDataEnd(end), UntrustedBody::Empty) => {
                break original.finish_source_data_and_prepare_ready(&end)?;
            }
            _ => return Err(original_wire_refused()),
        }
    };
    entry.prepay_wire_work(196608, 24)?;
    let mut ready = UntrustedControl::DestinationReady(ready);
    replace_original_envelope(
        &mut ready,
        io.syntax_original(),
        Direction::DestinationToSource,
    );
    let witness = original.ready_witness_bytes()?;
    let intent = original.ready_intent_bytes()?;
    if witness.is_empty()
        || witness.len() > profile::READY_WITNESS_BYTES as usize
        || intent.is_empty()
        || intent.len() > profile::READY_INTENT_BYTES as usize
    {
        return Err(original_wire_refused());
    }
    let mut ready_body = Vec::with_capacity(8 + witness.len() + intent.len());
    ready_body.extend_from_slice(&(witness.len() as u32).to_be_bytes());
    ready_body.extend_from_slice(witness);
    ready_body.extend_from_slice(&(intent.len() as u32).to_be_bytes());
    ready_body.extend_from_slice(intent);
    write_original_control(io, &ready, &ready_body).await?;
    let result = {
        let (incoming, verified) = io.read_original_destination_frame(original).await?;
        if !matches!(
            incoming.message,
            UntrustedMessage::Data(
                UntrustedControl::SourceFinalDecision(_),
                UntrustedBody::Empty
            )
        ) {
            return Err(original_wire_refused());
        }
        let Some(VerifiedOriginalDControl::Final(final_decision)) = verified else {
            return Err(original_wire_refused());
        };
        // This actual consumption starts the approved commit exception. After
        // it, only local D custody can fail; Source/SSH liveness is not re-read
        // as a revocation of the already consumed original decision.
        original.consume_authenticated_final(*final_decision)?
    };
    entry.prepay_wire_work(131072, 16)?;
    let mut result = UntrustedControl::CommittedResult(result);
    replace_original_envelope(
        &mut result,
        io.syntax_original(),
        Direction::DestinationToSource,
    );
    let receipt = original.committed_receipt_bytes()?;
    if receipt.is_empty() || receipt.len() > profile::RESULT_BYTES as usize {
        return Err(original_wire_refused());
    }
    write_original_control(io, &result, receipt).await?;
    {
        let (incoming, verified) = io.read_original_destination_frame(original).await?;
        if verified.is_some() {
            return Err(original_wire_refused());
        }
        let UntrustedMessage::Data(UntrustedControl::SourceResultAck(ack), UntrustedBody::Empty) =
            incoming.message
        else {
            return Err(original_wire_refused());
        };
        original.acknowledge_result(&ack)?;
    }
    {
        let (incoming, verified) = io.read_original_destination_frame(original).await?;
        if verified.is_some() || !matches!(incoming.message, UntrustedMessage::End) {
            return Err(original_wire_refused());
        }
    }
    io.expect_eof().await.map_err(|_| original_wire_refused())?;
    let end = encode_frame(
        Kind::End,
        Direction::DestinationToSource,
        io.syntax_original().next[Direction::DestinationToSource.index()],
        &[],
    )
    .map_err(|_| original_wire_refused())?;
    io.write_frame(&end)
        .await
        .map_err(|_| original_wire_refused())?;
    io.close_output_after_end()
        .map_err(|_| original_wire_refused())?;
    if io.finish_untrusted().map_err(|_| original_wire_refused())?
        != io::UntrustedPipeTermination::EndFramesAndObservedEof
    {
        return Err(original_wire_refused());
    }
    let end = {
        let mut view = original.wire_identity()?;
        view.recheck()?;
        VerifiedOriginalDestinationEnd {
            original_operation: std::sync::Arc::clone(view.operation_original()),
            entry: view.entry_original(),
        }
    };
    original.finish_original_end(end)
}

/// Semantic Ready verified in this actual S/SSH operation. Not deserializable,
/// clonable or constructible by candidate syntax/crypto DATA tests.
pub(super) struct VerifiedReadyInOriginalS {
    original: &'static super::source_actor::SourceFrontOriginal,
    source: SourceBinding,
    destination: DestinationBinding,
    ready: DestinationReady,
    s_nonce: String,
    d_nonce: String,
    binding: String,
    snapshot_sha256: String,
    expected_statement: crate::DestinationStatement,
}
impl VerifiedReadyInOriginalS {
    pub(super) fn require_original(
        &self,
        first: &super::source_actor::SourceFirstS<'_>,
    ) -> Result<(), crate::BackupError> {
        first.original_owner().prepay_wire_work(131072, 16)?;
        if !std::ptr::eq(self.original, first.original_owner())
            || self.source.source_operation_id != first.source_operation_id_data()?.to_string()
            || self.s_nonce != hex::encode(first.source_nonce_data()?)
            || self.snapshot_sha256 != first.current_snapshot_sha256_data()?
            || source_transfer_data(first)?.source != self.source
        {
            return Err(original_wire_refused());
        }
        first.original_owner().revalidate()
    }
}
fn expected_source_statement(
    first: &super::source_actor::SourceFirstS<'_>,
    data: &OriginalSourceTransferData,
    destination: &DestinationBinding,
    trust: &crate::VerifierTrustConfig,
) -> Result<crate::DestinationStatement, crate::BackupError> {
    let source = first.source_original()?;
    let local = source
        .roots
        .get("local_pins")
        .ok_or_else(original_wire_refused)?;
    let control = source
        .roots
        .get("control")
        .ok_or_else(original_wire_refused)?;
    let target = destination
        .roots
        .iter()
        .find(|root| root.role == "destination")
        .ok_or_else(original_wire_refused)?;
    if trust.source_root != local.path
        || trust.source_control_root != control.path
        || trust.destination_root != target.path
        || destination.deployment_id != first.fence_metadata()?.destination_deployment_id_data()
    {
        return Err(original_wire_refused());
    }
    let statement = crate::DestinationStatement {
        format_version: 2,
        backup_id: first.pin()?.manifest().backup_id,
        manifest_sha256: data.source.manifest_sha256.clone(),
        source_control_sha256: data.source.source_control_sha256.clone(),
        source_root: trust.source_root.clone(),
        source_control_root: trust.source_control_root.clone(),
        source_host_id: trust.source_host_id,
        source_storage_id: trust.source_storage_id,
        destination_host_id: trust.destination_host_id,
        destination_storage_id: trust.destination_storage_id,
        destination_root: trust.destination_root.clone(),
    };
    statement.canonical_bytes()?;
    Ok(statement)
}
fn verify_ready_in_original_source(
    first: &super::source_actor::SourceFirstS<'_>,
    ssh: &super::supervisor::OriginalSshOwner,
    io: &OriginalPipeIo,
    data: &OriginalSourceTransferData,
    ready: DestinationReady,
    witness: &[u8],
    intent: &[u8],
) -> Result<VerifiedReadyInOriginalS, crate::BackupError> {
    ssh.recheck_installed(first)?;
    let syntax = io.syntax_original();
    let destination = syntax
        .destination
        .as_ref()
        .ok_or_else(original_wire_refused)?;
    first.original_owner().prepay_wire_work(262144, 32)?;
    let trust = ssh.trust_data()?;
    let expected = expected_source_statement(first, data, destination, &trust)?;
    let dest = destination
        .roots
        .iter()
        .find(|r| r.role == "destination")
        .ok_or_else(original_wire_refused)?;
    let evidence = destination
        .roots
        .iter()
        .find(|r| r.role == "evidence")
        .ok_or_else(original_wire_refused)?;
    let capture_raw = data.capture.files.iter().find(|r| r.path == "capture.json");
    if capture_raw.is_some() {
        return Err(original_wire_refused());
    }
    let actual_capture = first
        .pin()?
        .effects_records()
        .iter()
        .find(|r| r.path == "capture.json")
        .ok_or_else(original_wire_refused)?;
    first.original_owner().prepay_wire_work(
        actual_capture
            .size
            .checked_mul(64)
            .ok_or(crate::BackupError::Overflow)?,
        16,
    )?;
    let capture_raw = registry::canonical(&data.capture)?;
    if crate::digest(&capture_raw) != actual_capture.sha256
        || capture_raw.len() as u64 != actual_capture.size
    {
        return Err(original_wire_refused());
    }
    let (witness_hash, intent_hash) = crate::destination::snapshot_effects::validate_b1_ready_data(
        witness,
        intent,
        &data.capture,
        &capture_raw,
        &data.payload_manifest,
        &data.proof_raw,
        &expected,
        uuid::Uuid::parse_str(&destination.attempt_id).map_err(|_| original_wire_refused())?,
        (dest.dev, dest.ino),
        (evidence.dev, evidence.ino),
        &first.lease_original().budget,
    )?;
    if ready.proof_sha256 != crate::digest(&data.proof_raw)
        || ready.manifest_sha256 != data.source.manifest_sha256
        || ready.snapshot_effects_proof_sha256 != witness_hash
        || ready.snapshot_effects_intent_sha256 != intent_hash
        || ready.snapshot_effects_input_sha256 != data.source.snapshot_effects_input_sha256
        || ready.attempt_id != destination.attempt_id
    {
        return Err(original_wire_refused());
    }
    let verified = VerifiedReadyInOriginalS {
        original: first.original_owner(),
        source: data.source.clone(),
        destination: destination.clone(),
        ready,
        s_nonce: syntax.s_nonce.clone(),
        d_nonce: syntax.d_nonce.clone(),
        binding: syntax.binding.clone(),
        snapshot_sha256: first.current_snapshot_sha256_data()?,
        expected_statement: expected,
    };
    verified.require_original(first)?;
    Ok(verified)
}

async fn send_source_file(
    first: &mut super::source_actor::SourceFirstS<'_>,
    ssh: &super::supervisor::OriginalSshOwner,
    io: &mut OriginalPipeIo,
    data: &OriginalSourceTransferData,
    role: FileRole,
    index: usize,
    file_no: u32,
) -> Result<(), crate::BackupError> {
    use sha2::Digest;
    first.recheck().await?;
    ssh.recheck_installed(first)?;
    first.original_owner().prepay_wire_work(65536, 16)?;
    let (record, memory) = match role {
        FileRole::PackageManifest => {
            if index != 0 {
                return Err(original_wire_refused());
            }
            (
                crate::FileRecord {
                    path: "manifest.json".into(),
                    size: first.pin()?.manifest_raw_bytes().len() as u64,
                    sha256: data.source.manifest_sha256.clone(),
                },
                true,
            )
        }
        FileRole::PackageMember => (
            first
                .pin()?
                .manifest()
                .files
                .get(index)
                .ok_or_else(original_wire_refused)?
                .clone(),
            false,
        ),
        FileRole::SourceEvidenceMember => (
            first
                .pin()?
                .source_records()
                .get(index)
                .ok_or_else(original_wire_refused)?
                .clone(),
            true,
        ),
        FileRole::SnapshotEffectsTransportIndex => {
            if index != 0 {
                return Err(original_wire_refused());
            }
            (
                crate::FileRecord {
                    path: "transport-index.json".into(),
                    size: data.transport_index.len() as u64,
                    sha256: crate::digest(&data.transport_index),
                },
                true,
            )
        }
        FileRole::SnapshotEffectsMember => (
            first
                .pin()?
                .effects_records()
                .get(index)
                .ok_or_else(original_wire_refused)?
                .clone(),
            false,
        ),
    };
    let begin = original_control!(FileBegin,io.syntax_original(),Direction::SourceToDestination,{file_no:file_no,role:role,roster_index:u32::try_from(index).map_err(|_|original_wire_refused())?,size:record.size,sha256:record.sha256.clone()});
    write_original_control(io, &UntrustedControl::FileBegin(begin), &[]).await?;
    if !memory {
        ssh.begin_original_file(first, role, index)?;
    }
    let mut hash = Sha256::new();
    let mut offset = 0u64;
    while offset < record.size {
        let length = usize::try_from((record.size - offset).min(profile::CHUNK_BODY_BYTES))
            .map_err(|_| original_wire_refused())?;
        first.original_owner().prepay_wire_work(
            (length as u64)
                .checked_mul(4)
                .ok_or(crate::BackupError::Overflow)?,
            4,
        )?;
        let mut bytes = vec![0u8; length];
        if memory {
            let raw = match role {
                FileRole::PackageManifest => first.pin()?.manifest_raw_bytes(),
                FileRole::SourceEvidenceMember => first
                    .pin()?
                    .source_bytes()
                    .get(&record.path)
                    .ok_or_else(original_wire_refused)?
                    .as_slice(),
                FileRole::SnapshotEffectsTransportIndex => data.transport_index.as_slice(),
                _ => return Err(original_wire_refused()),
            };
            let start = usize::try_from(offset).map_err(|_| original_wire_refused())?;
            bytes.copy_from_slice(
                raw.get(start..start + length)
                    .ok_or_else(original_wire_refused)?,
            );
        } else {
            let mut read = 0;
            while read < bytes.len() {
                let n = ssh.read_original_file(&mut bytes[read..])?;
                if n == 0 {
                    return Err(original_wire_refused());
                }
                read += n;
            }
        }
        hash.update(&bytes);
        let chunk = original_control!(FileChunk,io.syntax_original(),Direction::SourceToDestination,{file_no:file_no,offset:offset});
        write_original_control(io, &UntrustedControl::FileChunk(chunk), &bytes).await?;
        offset = offset
            .checked_add(bytes.len() as u64)
            .ok_or(crate::BackupError::Overflow)?;
    }
    if !memory {
        ssh.finish_original_file()?;
    }
    if hex::encode(hash.finalize()) != record.sha256 {
        return Err(original_wire_refused());
    }
    let end = original_control!(FileEnd,io.syntax_original(),Direction::SourceToDestination,{file_no:file_no,size:record.size,sha256:record.sha256});
    write_original_control(io, &UntrustedControl::FileEnd(end), &[]).await?;
    first.recheck().await
}

/// The one fixed Source loop, borrowing the actual first-S frame throughout.
/// Its original SSH owner and IO offsets remain in Book on cancel/unknown.
pub(super) async fn run_original_source(
    first: &mut super::source_actor::SourceFirstS<'_>,
    ssh: &'static super::supervisor::OriginalSshOwner,
    io: &mut OriginalPipeIo,
) -> Result<(), crate::BackupError> {
    first.recheck().await?;
    ssh.recheck_installed(first)?;
    let data = source_transfer_data(first)?;
    first.original_owner().prepay_wire_work(262144, 32)?;
    let hello = schema::HelloS {
        schema_version: 1,
        profile: profile::PROFILE.into(),
        kind: "HelloS".into(),
        direction: Direction::SourceToDestination,
        sequence: 0,
        s_nonce: hex::encode(first.source_nonce_data()?),
        source: data.source.clone(),
    };
    let raw = encode_frame(
        Kind::Metadata,
        Direction::SourceToDestination,
        0,
        &registry::canonical(&hello)?,
    )
    .map_err(|_| original_wire_refused())?;
    io.write_frame(&raw)
        .await
        .map_err(|_| original_wire_refused())?;
    {
        let incoming = io.read_frame().await.map_err(|_| original_wire_refused())?;
        let UntrustedMessage::HelloD(hello) = incoming.message else {
            return Err(original_wire_refused());
        };
        let trust = ssh.trust_data()?;
        expected_source_statement(first, &data, &hello.destination, &trust)?;
        // destination_policy_sha256 is an exact C installation/immutable
        // forced-launch mapping obligation. HelloD's ELF digest does not
        // replace that pin and this code does not claim C has been installed.
        if !crate::valid_digest(first.fence_metadata()?.destination_policy_sha256_data()) {
            return Err(original_wire_refused());
        }
    }
    first.recheck().await?;
    ssh.recheck_installed(first)?;
    let destination = io
        .syntax_original()
        .destination
        .as_ref()
        .ok_or_else(original_wire_refused)?
        .clone();
    let mut auth = original_control!(SourceAuth,io.syntax_original(),Direction::SourceToDestination,{source:data.source.clone(),destination:destination,
        source_auth_key_sha256:first.fence_metadata()?.source_auth_sha256_data().to_owned(),signature:"0".repeat(128)});
    ssh.sign_original_auth(first, &mut auth, io)?;
    write_original_control(io, &UntrustedControl::SourceAuth(Box::new(auth)), &[]).await?;
    {
        let frame = io.read_frame().await.map_err(|_| original_wire_refused())?;
        if !matches!(
            frame.message,
            UntrustedMessage::Data(UntrustedControl::AuthAccepted(_), UntrustedBody::Empty)
        ) {
            return Err(original_wire_refused());
        }
    }
    first.recheck().await?;
    ssh.recheck_installed(first)?;
    io.mark_original_handshake_authenticated()?;
    let mut file_no = 0u32;
    send_source_file(first, ssh, io, &data, FileRole::PackageManifest, 0, file_no).await?;
    file_no += 1;
    let package_count = first.pin()?.manifest().files.len();
    for index in 0..package_count {
        send_source_file(
            first,
            ssh,
            io,
            &data,
            FileRole::PackageMember,
            index,
            file_no,
        )
        .await?;
        file_no = file_no.checked_add(1).ok_or(crate::BackupError::Overflow)?;
    }
    let source_count = first.pin()?.source_records().len();
    for index in 0..source_count {
        send_source_file(
            first,
            ssh,
            io,
            &data,
            FileRole::SourceEvidenceMember,
            index,
            file_no,
        )
        .await?;
        file_no = file_no.checked_add(1).ok_or(crate::BackupError::Overflow)?;
    }
    send_source_file(
        first,
        ssh,
        io,
        &data,
        FileRole::SnapshotEffectsTransportIndex,
        0,
        file_no,
    )
    .await?;
    file_no = file_no.checked_add(1).ok_or(crate::BackupError::Overflow)?;
    let effect_count = first.pin()?.effects_records().len();
    for index in 0..effect_count {
        send_source_file(
            first,
            ssh,
            io,
            &data,
            FileRole::SnapshotEffectsMember,
            index,
            file_no,
        )
        .await?;
        file_no = file_no.checked_add(1).ok_or(crate::BackupError::Overflow)?;
    }
    if file_no != data.source.file_count {
        return Err(original_wire_refused());
    }
    let end = original_control!(SourceDataEnd,io.syntax_original(),Direction::SourceToDestination,{package_roster_sha256:data.source.package_roster_sha256.clone(),captured_roster_sha256:data.source.captured_roster_sha256.clone(),snapshot_effects_roster_sha256:data.source.snapshot_effects_roster_sha256.clone(),files:file_no,
        bytes:data.source.package_bytes.checked_add(data.source.evidence_bytes).ok_or(crate::BackupError::Overflow)?});
    write_original_control(io, &UntrustedControl::SourceDataEnd(end), &[]).await?;
    let ready = {
        let incoming = io.read_frame().await.map_err(|_| original_wire_refused())?;
        let UntrustedMessage::Data(
            UntrustedControl::DestinationReady(ready),
            UntrustedBody::Ready { witness, intent },
        ) = incoming.message
        else {
            return Err(original_wire_refused());
        };
        first
            .original_owner()
            .prepay_wire_work((witness.len() + intent.len()) as u64, 4)?;
        (ready, witness.to_vec(), intent.to_vec())
    };
    first.recheck().await?;
    let ready =
        verify_ready_in_original_source(first, ssh, io, &data, ready.0, &ready.1, &ready.2)?;
    first.settle_original_capture_after_ready(&ready).await?;
    ready.require_original(first)?;
    first.recheck().await?;
    let actor = actor_binding_data(
        &data.source.source_operation_id,
        &ready.binding,
        &ready.s_nonce,
        &ready.d_nonce,
    )
    .map_err(|_| original_wire_refused())?;
    let mut fence = super::source_fence::DurableSourceFence::create_original(
        first.lease_original(),
        first.fence_anchor_data()?,
        first.source_original()?,
        first.pin()?,
        super::source_fence::FenceDecisionData {
            source: &ready.source,
            destination: &ready.destination,
            ready: &ready.ready,
            s_nonce: &ready.s_nonce,
            d_nonce: &ready.d_nonce,
            binding_sha256: &ready.binding,
            source_actor_binding_sha256: &actor,
        },
    )?;
    first.recheck().await?;
    ready.require_original(first)?;
    fence.recheck_held()?;
    let mut final_decision = original_control!(SourceFinalDecision,io.syntax_original(),Direction::SourceToDestination,{attempt_id:ready.ready.attempt_id.clone(),proof_sha256:ready.ready.proof_sha256.clone(),snapshot_effects_proof_sha256:ready.ready.snapshot_effects_proof_sha256.clone(),fence:fence.wire_data()?,
        source_auth_key_sha256:first.fence_metadata()?.source_auth_sha256_data().to_owned(),signature:"0".repeat(128),snapshot_effects_input_sha256:ready.ready.snapshot_effects_input_sha256.clone(),snapshot_effects_intent_sha256:ready.ready.snapshot_effects_intent_sha256.clone()});
    ssh.sign_original_final(first, &mut final_decision, io, &mut fence)?;
    first.recheck().await?;
    fence.recheck_held()?;
    write_original_control(
        io,
        &UntrustedControl::SourceFinalDecision(final_decision),
        &[],
    )
    .await?;
    let result = {
        let incoming = io.read_frame().await.map_err(|_| original_wire_refused())?;
        let UntrustedMessage::Data(
            UntrustedControl::CommittedResult(result),
            UntrustedBody::Receipt(raw),
        ) = incoming.message
        else {
            return Err(original_wire_refused());
        };
        first
            .original_owner()
            .prepay_wire_work(raw.len() as u64, 2)?;
        (result, raw.to_vec())
    };
    first.recheck().await?;
    fence.recheck_held()?;
    ssh.recheck_installed(first)?;
    let trust = ssh.trust_data()?;
    let key = hex::decode(&trust.verifier_public_key_hex).map_err(|_| original_wire_refused())?;
    let receipt_sha = crate::complete::validate_b1_receipt_data(
        &result.1,
        &ready.expected_statement,
        &key,
        first.fence_metadata()?.verifier_public_key_sha256_data(),
        &first.lease_original().budget,
    )?;
    if result.0.receipt_sha256 != receipt_sha
        || result.0.proof_sha256 != crate::digest(&data.proof_raw)
        || result.0.attempt_id != ready.destination.attempt_id
    {
        return Err(original_wire_refused());
    }
    let ack = original_control!(SourceResultAck,io.syntax_original(),Direction::SourceToDestination,{attempt_id:result.0.attempt_id,receipt_sha256:receipt_sha});
    write_original_control(io, &UntrustedControl::SourceResultAck(ack), &[]).await?;
    let end = encode_frame(
        Kind::End,
        Direction::SourceToDestination,
        io.syntax_original().next[Direction::SourceToDestination.index()],
        &[],
    )
    .map_err(|_| original_wire_refused())?;
    io.write_frame(&end)
        .await
        .map_err(|_| original_wire_refused())?;
    io.close_output_after_end()
        .map_err(|_| original_wire_refused())?;
    {
        let frame = io.read_frame().await.map_err(|_| original_wire_refused())?;
        if !matches!(frame.message, UntrustedMessage::End) {
            return Err(original_wire_refused());
        }
    }
    io.expect_eof().await.map_err(|_| original_wire_refused())?;
    if io.finish_untrusted().map_err(|_| original_wire_refused())?
        != io::UntrustedPipeTermination::EndFramesAndObservedEof
    {
        return Err(original_wire_refused());
    }
    first.recheck().await?;
    fence.recheck_held()
}

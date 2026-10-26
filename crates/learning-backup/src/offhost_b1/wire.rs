//! Pure, default-off wire syntax. No IO, keys, authentication or permissions.
//! All decoded messages, phases and transcript heads remain UNTRUSTED DATA.
//! Existing Ready/receipt body semantics and real EOF require later consumers.
mod schema;
mod transcript;

use super::profile;
use crate::registry;
use schema::{FileRole, UntrustedControl};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

const HEADER: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Direction {
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
enum WireError {
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
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Header {
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
            self.phase = SyntaxPhase::Poisoned;
        }
        result
    }
    fn observe_open<'a>(&mut self, raw: &'a [u8]) -> Result<UntrustedFrame<'a>, WireError> {
        match self.phase {
            SyntaxPhase::Poisoned => return Err(WireError::Poisoned),
            SyntaxPhase::BothEndsSeen | SyntaxPhase::Aborted => return Err(WireError::Closed),
            _ => {}
        }
        let h = inspect_header(raw.get(..HEADER).ok_or(WireError::Header)?)?;
        // S End closes S>D, including Abort. Only the still-open direction can
        // supply its remaining termination; an error poisons the whole syntax.
        if self.phase == SyntaxPhase::DestinationEnd
            && h.direction == Direction::SourceToDestination
        {
            return Err(WireError::Closed);
        }
        if h.sequence != self.next[h.direction.index()] {
            return Err(WireError::Sequence);
        }
        let next = h.sequence.checked_add(1).ok_or(WireError::Sequence)?;
        if usize::try_from(h.payload_bytes)
            .ok()
            .and_then(|n| n.checked_add(HEADER))
            != Some(raw.len())
        {
            return Err(WireError::Length);
        }
        // Valid declared frames (including later schema/phase failures) are
        // charged once before typed parsing. A malformed header is terminal.
        self.budget
            .charge_frame(h.kind.budget_kind(), u64::from(h.payload_bytes))
            .map_err(|_| WireError::Budget)?;
        let frame = parse_frame(raw, h)?;
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
        Ok(frame)
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
                if self.source.as_ref() != Some(&c.source)
                    || self.destination.as_ref() != Some(&c.destination)
                {
                    return Err(WireError::Binding);
                }
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

#[cfg(test)]
mod tests;

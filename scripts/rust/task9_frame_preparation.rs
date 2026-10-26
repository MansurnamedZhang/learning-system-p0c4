#![cfg(test)]

//! Unadopted Task9 framing preparation, compiled only as a standalone test crate.
//! No product module imports this file. Parsing grants no identity, liveness,
//! signing, publication, installation, or distributed-commit authority.
//! This wire budget is a preparation-profile limit, not a product package budget.
//! `finish` observes decoder state only; it does not attest to a socket's EOF.

const HEADER_BYTES: usize = 20;
const MAX_METADATA_BYTES: u32 = 16_384;
const MAX_DATA_BYTES: u32 = 65_536;
const MAX_ABORT_BYTES: u32 = 256;
const MAX_FRAMES: u64 = 8_192;
const MAX_WIRE_BYTES: u64 = 536_870_912;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Metadata,
    Data,
    End,
    Abort,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    SourceToDestination,
    DestinationToSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FrameError {
    HeaderSize,
    Magic,
    Version,
    Kind,
    Direction,
    Reserved,
    ExpectedDirection,
    Sequence,
    LengthLimit,
    PayloadSize,
    FrameLimit,
    WireLimit,
    Arithmetic,
    MetadataRequired,
    DuplicateMetadata,
    EmptyData,
    Closed,
    Poisoned,
    UnexpectedEof,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct UntrustedHeader {
    kind: Kind,
    direction: Direction,
    sequence: u64,
    payload_bytes: u32,
}

// Deliberately no Debug implementation: payload bytes must not enter diagnostics.
struct UntrustedFrame<'a> {
    header: UntrustedHeader,
    payload: &'a [u8],
}

/// Inspect exactly one fixed header without reading payload or changing a decoder.
/// A result is untrusted syntax, never authentication or permission. Errors here
/// concern this stateless inspection; `decode` errors poison its decoder.
fn inspect_header(header: &[u8]) -> Result<UntrustedHeader, FrameError> {
    if header.len() != HEADER_BYTES {
        return Err(FrameError::HeaderSize);
    }
    if &header[..4] != b"KWB1" {
        return Err(FrameError::Magic);
    }
    if header[4] != 1 {
        return Err(FrameError::Version);
    }
    let kind = match header[5] {
        1 => Kind::Metadata,
        2 => Kind::Data,
        3 => Kind::End,
        4 => Kind::Abort,
        _ => return Err(FrameError::Kind),
    };
    let direction = match header[6] {
        1 => Direction::SourceToDestination,
        2 => Direction::DestinationToSource,
        _ => return Err(FrameError::Direction),
    };
    if header[7] != 0 {
        return Err(FrameError::Reserved);
    }
    let sequence = u64::from_be_bytes(
        header[8..16]
            .try_into()
            .map_err(|_| FrameError::HeaderSize)?,
    );
    let payload_bytes = u32::from_be_bytes(
        header[16..20]
            .try_into()
            .map_err(|_| FrameError::HeaderSize)?,
    );
    let limit = match kind {
        Kind::Metadata => MAX_METADATA_BYTES,
        Kind::Data => MAX_DATA_BYTES,
        Kind::End => 0,
        Kind::Abort => MAX_ABORT_BYTES,
    };
    if payload_bytes > limit {
        return Err(FrameError::LengthLimit);
    }
    if kind == Kind::Data && payload_bytes == 0 {
        return Err(FrameError::EmptyData);
    }
    Ok(UntrustedHeader {
        kind,
        direction,
        sequence,
        payload_bytes,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Initial,
    Streaming,
    Ended,
    Aborted,
    Poisoned,
}

/// Neither variant is an authenticated remote outcome. In particular, a
/// syntactically valid abort must not be interpreted as a completed transfer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UntrustedTermination {
    End,
    Abort,
}

struct Decoder {
    direction: Direction,
    next_sequence: u64,
    frames: u64,
    wire_bytes: u64,
    phase: Phase,
}

impl Decoder {
    fn new(direction: Direction) -> Self {
        Self {
            direction,
            next_sequence: 0,
            frames: 0,
            wire_bytes: 0,
            phase: Phase::Initial,
        }
    }

    /// Accept exactly one complete frame, borrowing its payload without copying.
    /// Every rejection permanently poisons this instance; it has no reset API.
    fn decode<'a>(&mut self, frame: &'a [u8]) -> Result<UntrustedFrame<'a>, FrameError> {
        let result = self.decode_open(frame);
        if result.is_err() {
            self.phase = Phase::Poisoned;
        }
        result
    }

    fn decode_open<'a>(&mut self, frame: &'a [u8]) -> Result<UntrustedFrame<'a>, FrameError> {
        match self.phase {
            Phase::Poisoned => return Err(FrameError::Poisoned),
            Phase::Ended | Phase::Aborted => return Err(FrameError::Closed),
            Phase::Initial | Phase::Streaming => {}
        }
        let raw_header = frame.get(..HEADER_BYTES).ok_or(FrameError::HeaderSize)?;
        let header = inspect_header(raw_header)?;
        if header.direction != self.direction {
            return Err(FrameError::ExpectedDirection);
        }
        if header.sequence != self.next_sequence {
            return Err(FrameError::Sequence);
        }
        let payload_bytes =
            usize::try_from(header.payload_bytes).map_err(|_| FrameError::Arithmetic)?;
        let expected_bytes = HEADER_BYTES
            .checked_add(payload_bytes)
            .ok_or(FrameError::Arithmetic)?;
        if frame.len() != expected_bytes {
            return Err(FrameError::PayloadSize);
        }
        let next_phase = match (self.phase, header.kind) {
            (_, Kind::Abort) => Phase::Aborted,
            (Phase::Initial, Kind::Metadata) => Phase::Streaming,
            (Phase::Initial, _) => return Err(FrameError::MetadataRequired),
            (Phase::Streaming, Kind::Metadata) => return Err(FrameError::DuplicateMetadata),
            (Phase::Streaming, Kind::Data) => Phase::Streaming,
            (Phase::Streaming, Kind::End) => Phase::Ended,
            _ => return Err(FrameError::Closed),
        };
        let frames = self.frames.checked_add(1).ok_or(FrameError::Arithmetic)?;
        if frames > MAX_FRAMES {
            return Err(FrameError::FrameLimit);
        }
        let frame_bytes = u64::try_from(frame.len()).map_err(|_| FrameError::Arithmetic)?;
        let wire_bytes = self
            .wire_bytes
            .checked_add(frame_bytes)
            .ok_or(FrameError::Arithmetic)?;
        if wire_bytes > MAX_WIRE_BYTES {
            return Err(FrameError::WireLimit);
        }
        let next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(FrameError::Arithmetic)?;
        self.frames = frames;
        self.wire_bytes = wire_bytes;
        self.next_sequence = next_sequence;
        self.phase = next_phase;
        Ok(UntrustedFrame {
            header,
            payload: &frame[HEADER_BYTES..],
        })
    }

    fn finish(&mut self) -> Result<UntrustedTermination, FrameError> {
        match self.phase {
            Phase::Ended => Ok(UntrustedTermination::End),
            Phase::Aborted => Ok(UntrustedTermination::Abort),
            Phase::Poisoned => Err(FrameError::Poisoned),
            Phase::Initial | Phase::Streaming => {
                self.phase = Phase::Poisoned;
                Err(FrameError::UnexpectedEof)
            }
        }
    }
}

mod tests {
    use super::*;

    fn header(kind: u8, direction: u8, sequence: u64, length: u32) -> Vec<u8> {
        let mut bytes = b"KWB1\x01\x00\x00\x00".to_vec();
        bytes[5] = kind;
        bytes[6] = direction;
        bytes.extend_from_slice(&sequence.to_be_bytes());
        bytes.extend_from_slice(&length.to_be_bytes());
        bytes
    }

    fn frame(kind: u8, direction: u8, sequence: u64, payload: &[u8]) -> Vec<u8> {
        let mut bytes = header(
            kind,
            direction,
            sequence,
            u32::try_from(payload.len()).unwrap(),
        );
        bytes.extend_from_slice(payload);
        bytes
    }

    fn source_decoder() -> Decoder {
        Decoder::new(Direction::SourceToDestination)
    }

    fn streaming() -> Decoder {
        let mut decoder = source_decoder();
        decoder.decode(&frame(1, 1, 0, b"metadata")).unwrap();
        decoder
    }

    // Inspect only the error so UntrustedFrame never needs a Debug implementation.
    fn rejected(decoder: &mut Decoder, bytes: &[u8], expected: FrameError) {
        assert_eq!(decoder.decode(bytes).err(), Some(expected));
        assert_eq!(decoder.phase, Phase::Poisoned);
    }

    #[test]
    fn metadata_then_end_borrows_payload_and_finishes_only_decoder_state() {
        let metadata = frame(1, 1, 0, b"untrusted");
        let mut decoder = source_decoder();
        let parsed = decoder.decode(&metadata).unwrap();
        assert_eq!(parsed.header.kind, Kind::Metadata);
        assert_eq!(parsed.payload, b"untrusted");
        assert_eq!(parsed.payload.as_ptr(), metadata[HEADER_BYTES..].as_ptr());
        decoder.decode(&frame(3, 1, 1, b"")).unwrap();
        assert_eq!(decoder.frames, 2);
        assert_eq!(decoder.wire_bytes, 49);
        assert_eq!(decoder.finish(), Ok(UntrustedTermination::End));
    }

    #[test]
    fn stateless_header_inspection_requires_exact_header_and_no_payload() {
        let raw = header(1, 2, 42, MAX_METADATA_BYTES);
        let parsed = inspect_header(&raw).unwrap();
        assert_eq!(parsed.direction, Direction::DestinationToSource);
        assert_eq!(parsed.sequence, 42);
        assert_eq!(parsed.payload_bytes, MAX_METADATA_BYTES);
        assert_eq!(inspect_header(&raw[..19]), Err(FrameError::HeaderSize));
        let mut extra = raw.clone();
        extra.push(0);
        assert_eq!(inspect_header(&extra), Err(FrameError::HeaderSize));
        assert_eq!(inspect_header(&raw), Ok(parsed));
    }

    #[test]
    fn metadata_accepts_exact_cap_and_rejects_cap_plus_one() {
        let payload = vec![0; MAX_METADATA_BYTES as usize];
        let mut decoder = source_decoder();
        assert_eq!(
            decoder
                .decode(&frame(1, 1, 0, &payload))
                .unwrap()
                .payload
                .len(),
            payload.len()
        );
        let oversized = header(1, 1, 0, MAX_METADATA_BYTES + 1);
        assert_eq!(inspect_header(&oversized), Err(FrameError::LengthLimit));
        rejected(&mut source_decoder(), &oversized, FrameError::LengthLimit);
    }

    #[test]
    fn data_accepts_exact_cap_and_rejects_cap_plus_one_and_empty() {
        let payload = vec![0; MAX_DATA_BYTES as usize];
        let mut decoder = streaming();
        assert_eq!(
            decoder
                .decode(&frame(2, 1, 1, &payload))
                .unwrap()
                .payload
                .len(),
            payload.len()
        );
        rejected(
            &mut streaming(),
            &header(2, 1, 1, MAX_DATA_BYTES + 1),
            FrameError::LengthLimit,
        );
        rejected(
            &mut streaming(),
            &frame(2, 1, 1, b""),
            FrameError::EmptyData,
        );
    }

    #[test]
    fn abort_accepts_exact_cap_and_rejects_cap_plus_one() {
        let mut decoder = source_decoder();
        decoder
            .decode(&frame(4, 1, 0, &vec![0; MAX_ABORT_BYTES as usize]))
            .unwrap();
        assert_eq!(decoder.finish(), Ok(UntrustedTermination::Abort));
        rejected(
            &mut source_decoder(),
            &header(4, 1, 0, MAX_ABORT_BYTES + 1),
            FrameError::LengthLimit,
        );
    }

    #[test]
    fn magic_version_and_reserved_substitutions_poison() {
        for (index, replacement, expected) in [
            (0, b'X', FrameError::Magic),
            (4, 2, FrameError::Version),
            (7, 1, FrameError::Reserved),
        ] {
            let mut raw = frame(1, 1, 0, b"");
            raw[index] = replacement;
            rejected(&mut source_decoder(), &raw, expected);
        }
    }

    #[test]
    fn unknown_kind_and_direction_are_not_extensible() {
        for kind in [0, 5, 255] {
            rejected(
                &mut source_decoder(),
                &frame(kind, 1, 0, b""),
                FrameError::Kind,
            );
        }
        for direction in [0, 3, 255] {
            rejected(
                &mut source_decoder(),
                &frame(1, direction, 0, b""),
                FrameError::Direction,
            );
        }
    }

    #[test]
    fn expected_direction_is_fixed_for_each_decoder() {
        rejected(
            &mut source_decoder(),
            &frame(1, 2, 0, b""),
            FrameError::ExpectedDirection,
        );
        let mut decoder = Decoder::new(Direction::DestinationToSource);
        decoder.decode(&frame(1, 2, 0, b"")).unwrap();
        decoder.decode(&frame(3, 2, 1, b"")).unwrap();
        assert_eq!(decoder.finish(), Ok(UntrustedTermination::End));
        let mut decoder = Decoder::new(Direction::DestinationToSource);
        rejected(
            &mut decoder,
            &frame(1, 1, 0, b""),
            FrameError::ExpectedDirection,
        );
    }

    #[test]
    fn sequence_starts_at_zero_and_cannot_skip_or_repeat() {
        rejected(
            &mut source_decoder(),
            &frame(1, 1, 1, b""),
            FrameError::Sequence,
        );
        rejected(
            &mut streaming(),
            &frame(2, 1, 2, b"a"),
            FrameError::Sequence,
        );
        let mut decoder = streaming();
        let data = frame(2, 1, 1, b"a");
        decoder.decode(&data).unwrap();
        rejected(&mut decoder, &data, FrameError::Sequence);
    }

    #[test]
    fn metadata_is_required_first_and_cannot_repeat() {
        rejected(
            &mut source_decoder(),
            &frame(2, 1, 0, b"a"),
            FrameError::MetadataRequired,
        );
        rejected(
            &mut source_decoder(),
            &frame(3, 1, 0, b""),
            FrameError::MetadataRequired,
        );
        rejected(
            &mut streaming(),
            &frame(1, 1, 1, b"new"),
            FrameError::DuplicateMetadata,
        );
    }

    #[test]
    fn end_requires_zero_declared_bytes_and_no_extra_suffix() {
        rejected(
            &mut streaming(),
            &header(3, 1, 1, 1),
            FrameError::LengthLimit,
        );
        let mut extra = frame(3, 1, 1, b"");
        extra.push(0);
        rejected(&mut streaming(), &extra, FrameError::PayloadSize);
    }

    #[test]
    fn abort_is_terminal_before_metadata_or_after_data() {
        for mut decoder in [source_decoder(), streaming()] {
            if decoder.phase == Phase::Streaming {
                decoder.decode(&frame(2, 1, 1, b"a")).unwrap();
            }
            let sequence = decoder.next_sequence;
            decoder
                .decode(&frame(4, 1, sequence, b"fixed rejection"))
                .unwrap();
            assert_eq!(decoder.finish(), Ok(UntrustedTermination::Abort));
            rejected(
                &mut decoder,
                &frame(3, 1, sequence + 1, b""),
                FrameError::Closed,
            );
        }
    }

    #[test]
    fn end_rejects_repeated_terminal_and_every_other_following_kind() {
        for kind in 1..=4 {
            let mut decoder = streaming();
            decoder.decode(&frame(3, 1, 1, b"")).unwrap();
            let payload: &[u8] = if kind == 2 { b"a" } else { b"" };
            rejected(
                &mut decoder,
                &frame(kind, 1, 2, payload),
                FrameError::Closed,
            );
        }
    }

    #[test]
    fn every_truncated_header_is_rejected() {
        let raw = frame(1, 1, 0, b"abc");
        for length in 0..HEADER_BYTES {
            rejected(
                &mut source_decoder(),
                &raw[..length],
                FrameError::HeaderSize,
            );
        }
    }

    #[test]
    fn payload_truncation_length_substitution_and_concatenation_are_rejected() {
        let raw = frame(1, 1, 0, b"abc");
        for length in HEADER_BYTES..raw.len() {
            rejected(
                &mut source_decoder(),
                &raw[..length],
                FrameError::PayloadSize,
            );
        }
        for declared in [0, 2, 4] {
            let mut substituted = raw.clone();
            substituted[16..20].copy_from_slice(&u32::to_be_bytes(declared));
            rejected(&mut source_decoder(), &substituted, FrameError::PayloadSize);
        }
        let mut concatenated = raw;
        concatenated.extend_from_slice(&frame(3, 1, 1, b""));
        rejected(
            &mut source_decoder(),
            &concatenated,
            FrameError::PayloadSize,
        );
    }

    #[test]
    fn a_decode_error_cannot_be_repaired_with_a_valid_frame() {
        let mut decoder = source_decoder();
        rejected(&mut decoder, &frame(1, 1, 7, b""), FrameError::Sequence);
        rejected(&mut decoder, &frame(1, 1, 0, b""), FrameError::Poisoned);
        assert_eq!(decoder.finish(), Err(FrameError::Poisoned));
        assert_eq!(decoder.frames, 0);
        assert_eq!(decoder.wire_bytes, 0);
    }

    #[test]
    fn incomplete_finish_poisoning_is_not_remote_eof_attestation() {
        for mut decoder in [source_decoder(), streaming()] {
            assert_eq!(decoder.finish(), Err(FrameError::UnexpectedEof));
            let sequence = decoder.next_sequence;
            rejected(
                &mut decoder,
                &frame(4, 1, sequence, b""),
                FrameError::Poisoned,
            );
        }
    }

    #[test]
    fn frame_limit_counts_metadata_data_and_terminal() {
        let mut exact = streaming();
        for sequence in 1..(MAX_FRAMES - 1) {
            exact.decode(&frame(2, 1, sequence, b"a")).unwrap();
        }
        exact.decode(&frame(3, 1, MAX_FRAMES - 1, b"")).unwrap();
        assert_eq!(exact.frames, MAX_FRAMES);
        assert_eq!(exact.finish(), Ok(UntrustedTermination::End));

        let mut no_terminal_slot = streaming();
        for sequence in 1..MAX_FRAMES {
            no_terminal_slot
                .decode(&frame(2, 1, sequence, b"a"))
                .unwrap();
        }
        rejected(
            &mut no_terminal_slot,
            &frame(3, 1, MAX_FRAMES, b""),
            FrameError::FrameLimit,
        );
    }

    #[test]
    fn cumulative_wire_limit_includes_the_header_and_fails_at_plus_one() {
        // Private near-limit setup avoids allocating a 512 MiB fixture.
        let mut exact = streaming();
        exact.wire_bytes = MAX_WIRE_BYTES - HEADER_BYTES as u64;
        exact.decode(&frame(3, 1, 1, b"")).unwrap();
        assert_eq!(exact.wire_bytes, MAX_WIRE_BYTES);
        assert_eq!(exact.finish(), Ok(UntrustedTermination::End));

        let mut oversized = streaming();
        oversized.wire_bytes = MAX_WIRE_BYTES - HEADER_BYTES as u64 + 1;
        rejected(&mut oversized, &frame(3, 1, 1, b""), FrameError::WireLimit);
    }

    #[test]
    fn internal_counter_arithmetic_cannot_wrap_into_acceptance() {
        let mut wire = streaming();
        wire.wire_bytes = u64::MAX;
        rejected(&mut wire, &frame(3, 1, 1, b""), FrameError::Arithmetic);

        let mut count = streaming();
        count.frames = u64::MAX;
        rejected(&mut count, &frame(3, 1, 1, b""), FrameError::Arithmetic);

        let mut sequence = streaming();
        sequence.next_sequence = u64::MAX;
        rejected(
            &mut sequence,
            &frame(3, 1, u64::MAX, b""),
            FrameError::Arithmetic,
        );
    }
}

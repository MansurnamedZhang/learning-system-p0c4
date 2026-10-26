//! Fixed task9-b1-limited-v1 limits. No external profile/cap overrides.

use crate::BackupError;

pub(super) const PROFILE: &str = "task9-b1-limited-v1";
pub(super) const HEADER_BYTES: u64 = 20;
pub(super) const PACKAGE_BYTES: u64 = 268_435_456;
pub(super) const EVIDENCE_BYTES: u64 = 100_663_296;
pub(super) const WIRE_BYTES: u64 = 536_870_912;
pub(super) const FRAMES: u64 = 8192;
pub(super) const CONTROL_BYTES: u64 = 16_384;
pub(super) const DATA_BYTES: u64 = 65_536;
pub(super) const CHUNK_CONTROL_BYTES: u64 = 1024;
pub(super) const CHUNK_BODY_BYTES: u64 = 61_440;
pub(super) const RESULT_BYTES: u64 = 8192;
pub(super) const ABORT_BYTES: u64 = 256;
pub(super) const FILES: u64 = 512;
pub(super) const CONCURRENT_FILE_STREAMS: u64 = 1;
pub(super) const PRODUCT_FDS: u64 = 64;
pub(super) const FENCE_OPERATIONS: u64 = 256;
pub(super) const ANCHOR_BYTES: u64 = 4096;
pub(super) const FENCE_OPEN_BYTES: u64 = 16_384;
pub(super) const HANDSHAKE_SECONDS: u64 = 60;
pub(super) const IDLE_SECONDS: u64 = 30;
pub(super) const OPERATION_SECONDS: u64 = 1800;
pub(super) const SETTLEMENT_SECONDS: u64 = 30;
pub(super) const STDERR_BYTES: u64 = 1_048_576;
pub(super) const DIAGNOSTIC_BYTES: u64 = 1_048_576;
pub(super) const SESSION_OPERATIONS: u64 = 256;
pub(super) const SESSION_ENTRIES: u64 = 1536;
pub(super) const SESSION_ADMISSION_BYTES: u64 = 4096;
pub(super) const SESSION_BINDING_BYTES: u64 = 16_384;
pub(super) const SESSION_TRANSPORT_INDEX_BYTES: u64 = 16_777_216;
pub(super) const SESSION_METADATA_BYTES: u64 = 33_554_432;
pub(super) const READY_WITNESS_BYTES: u64 = 16_384;
pub(super) const READY_INTENT_BYTES: u64 = 16_384;

#[derive(Clone, Copy)]
pub(super) enum FrameKind {
    Metadata,
    Data,
    End,
    Abort,
}

#[derive(Clone, Copy)]
pub(super) enum FileClass {
    Package,
    Evidence,
}

/// One local session counter for BOTH directions; it is neither a wire parser
/// nor the original registry ScanBudget. Runtime consumers must retain both.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum PendingWire {
    #[default]
    None,
    Header,
    Payload,
    EofProbe,
}

#[derive(Default)]
pub(super) struct SessionBudget {
    wire_bytes: u64,
    frames: u64,
    package_bytes: u64,
    evidence_bytes: u64,
    files: u64,
    failed: bool,
    pending_wire: PendingWire,
    eof_probe_used: bool,
}

impl SessionBudget {
    pub(super) fn charge_frame(
        &mut self,
        kind: FrameKind,
        payload_bytes: u64,
    ) -> Result<(), BackupError> {
        let cap = match kind {
            FrameKind::Metadata => CONTROL_BYTES,
            FrameKind::Data => DATA_BYTES,
            FrameKind::End => 0,
            FrameKind::Abort => ABORT_BYTES,
        };
        let next_bytes = self
            .wire_bytes
            .checked_add(HEADER_BYTES)
            .and_then(|v| v.checked_add(payload_bytes));
        let next_frames = self.frames.checked_add(1);
        if self.failed
            || self.pending_wire != PendingWire::None
            || payload_bytes > cap
            || (payload_bytes == 0 && matches!(kind, FrameKind::Metadata | FrameKind::Data))
            || next_bytes.is_none_or(|v| v > WIRE_BYTES)
            || next_frames.is_none_or(|v| v > FRAMES)
        {
            self.failed = true;
            return Err(BackupError::Capacity("offhost B1 session wire budget"));
        }
        self.wire_bytes = next_bytes.expect("checked wire bytes");
        self.frames = next_frames.expect("checked frame count");
        Ok(())
    }

    // These reservations belong to the original shared counter. No credit,
    // refund, reset or caller-selected allowance is returned to the codec.
    pub(super) fn reserve_header(&mut self) -> Result<(), BackupError> {
        let bytes = self.wire_bytes.checked_add(HEADER_BYTES);
        let frames = self.frames.checked_add(1);
        if self.failed
            || self.pending_wire != PendingWire::None
            || bytes.is_none_or(|v| v > WIRE_BYTES)
            || frames.is_none_or(|v| v > FRAMES)
        {
            self.failed = true;
            return Err(BackupError::Capacity("offhost B1 header reservation"));
        }
        self.wire_bytes = bytes.expect("checked bytes");
        self.frames = frames.expect("checked frames");
        self.pending_wire = PendingWire::Header;
        Ok(())
    }

    pub(super) fn reserve_payload(
        &mut self,
        kind: FrameKind,
        payload_bytes: u64,
    ) -> Result<(), BackupError> {
        let cap = match kind {
            FrameKind::Metadata => CONTROL_BYTES,
            FrameKind::Data => DATA_BYTES,
            FrameKind::End => 0,
            FrameKind::Abort => ABORT_BYTES,
        };
        let bytes = self.wire_bytes.checked_add(payload_bytes);
        if self.failed
            || self.pending_wire != PendingWire::Header
            || payload_bytes > cap
            || (payload_bytes == 0 && matches!(kind, FrameKind::Metadata | FrameKind::Data))
            || bytes.is_none_or(|v| v > WIRE_BYTES)
        {
            self.failed = true;
            return Err(BackupError::Capacity("offhost B1 payload reservation"));
        }
        self.wire_bytes = bytes.expect("checked bytes");
        self.pending_wire = PendingWire::Payload;
        Ok(())
    }

    pub(super) fn finish_reserved_frame(&mut self) -> Result<(), BackupError> {
        if self.failed || self.pending_wire != PendingWire::Payload {
            self.failed = true;
            return Err(BackupError::Invalid("offhost B1 unfinished reservation"));
        }
        self.pending_wire = PendingWire::None;
        Ok(())
    }

    pub(super) fn reserve_eof_probe(&mut self) -> Result<(), BackupError> {
        let bytes = self.wire_bytes.checked_add(1);
        if self.failed
            || self.eof_probe_used
            || self.pending_wire != PendingWire::None
            || bytes.is_none_or(|v| v > WIRE_BYTES)
        {
            self.failed = true;
            return Err(BackupError::Capacity("offhost B1 EOF probe reservation"));
        }
        self.wire_bytes = bytes.expect("checked byte");
        self.pending_wire = PendingWire::EofProbe;
        self.eof_probe_used = true;
        Ok(())
    }

    pub(super) fn finish_eof_probe(&mut self) -> Result<(), BackupError> {
        if self.failed || self.pending_wire != PendingWire::EofProbe {
            self.failed = true;
            return Err(BackupError::Invalid("offhost B1 unfinished EOF probe"));
        }
        self.pending_wire = PendingWire::None;
        Ok(())
    }

    pub(super) fn poison(&mut self) {
        self.failed = true;
    }

    pub(super) fn charge_file(&mut self, class: FileClass, bytes: u64) -> Result<(), BackupError> {
        let (counter, cap) = match class {
            FileClass::Package => (&mut self.package_bytes, PACKAGE_BYTES),
            FileClass::Evidence => (&mut self.evidence_bytes, EVIDENCE_BYTES),
        };
        let next_bytes = counter.checked_add(bytes);
        let next_files = self.files.checked_add(1);
        if self.failed || next_bytes.is_none_or(|v| v > cap) || next_files.is_none_or(|v| v > FILES)
        {
            self.failed = true;
            return Err(BackupError::Capacity("offhost B1 session file budget"));
        }
        *counter = next_bytes.expect("checked file bytes");
        self.files = next_files.expect("checked file count");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_cap_is_checked_before_counting_and_failure_is_terminal() {
        let mut budget = SessionBudget::default();
        budget.charge_frame(FrameKind::Data, DATA_BYTES).unwrap();
        let before = (budget.wire_bytes, budget.frames);
        assert!(
            budget
                .charge_frame(FrameKind::Data, DATA_BYTES + 1)
                .is_err()
        );
        assert_eq!((budget.wire_bytes, budget.frames), before);
        assert!(budget.charge_frame(FrameKind::End, 0).is_err());
        assert!(budget.charge_file(FileClass::Package, 1).is_err());
    }

    #[test]
    fn wire_limit_counts_headers_and_checked_overflow() {
        let mut exact = SessionBudget {
            wire_bytes: WIRE_BYTES - HEADER_BYTES,
            ..SessionBudget::default()
        };
        exact.charge_frame(FrameKind::End, 0).unwrap();
        assert!(exact.charge_frame(FrameKind::End, 0).is_err());
        let mut overflow = SessionBudget {
            wire_bytes: u64::MAX,
            ..SessionBudget::default()
        };
        assert!(overflow.charge_frame(FrameKind::Abort, 1).is_err());
    }

    #[test]
    fn frame_count_is_shared_across_message_kinds() {
        let mut budget = SessionBudget {
            frames: FRAMES - 2,
            ..SessionBudget::default()
        };
        budget.charge_frame(FrameKind::Metadata, 1).unwrap();
        budget.charge_frame(FrameKind::Data, 1).unwrap();
        assert!(budget.charge_frame(FrameKind::End, 0).is_err());
    }

    #[test]
    fn empty_data_and_nonempty_end_are_rejected() {
        for (kind, bytes) in [(FrameKind::Data, 0), (FrameKind::End, 1)] {
            assert!(SessionBudget::default().charge_frame(kind, bytes).is_err());
        }
        let mut abort = SessionBudget::default();
        abort.charge_frame(FrameKind::Abort, ABORT_BYTES).unwrap();
        assert!(
            abort
                .charge_frame(FrameKind::Abort, ABORT_BYTES + 1)
                .is_err()
        );
    }

    #[test]
    fn package_and_combined_evidence_have_separate_fixed_limits() {
        let mut budget = SessionBudget::default();
        budget
            .charge_file(FileClass::Package, PACKAGE_BYTES)
            .unwrap();
        budget
            .charge_file(FileClass::Evidence, EVIDENCE_BYTES)
            .unwrap();
        assert!(budget.charge_file(FileClass::Evidence, 1).is_err());
        assert_eq!(budget.package_bytes, PACKAGE_BYTES);
        assert_eq!(budget.evidence_bytes, EVIDENCE_BYTES);
    }

    #[test]
    fn logical_file_count_and_size_overflow_do_not_reset() {
        let mut count = SessionBudget {
            files: FILES - 1,
            ..SessionBudget::default()
        };
        count.charge_file(FileClass::Package, 1).unwrap();
        assert!(count.charge_file(FileClass::Evidence, 1).is_err());
        let mut size = SessionBudget {
            package_bytes: 1,
            ..SessionBudget::default()
        };
        assert!(size.charge_file(FileClass::Package, u64::MAX).is_err());
        assert!(size.charge_file(FileClass::Package, 1).is_err());
    }
}

//! Serial, monotone permission model; callers retain the real guard and durable records.
use super::{ImportFailure, protocol::WriterIdentity};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ImportPhase {
    InputFrozen,
    SqlVerified,
    WriterReady,
    AttemptDurable,
    PayloadSent,
    PrecommitVerified,
    CommitAttempted,
    ImportObserved,
    Quarantined,
    Cancelled,
    CommitUnknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ImportEvent {
    SqlVerified,
    HeaderBlank,
    Ready(WriterIdentity),
    AttemptSynced,
    PayloadSent,
    Precommit(WriterIdentity),
    CommitIntentSynced,
    FinalReviewPassed,
    CommitPermitRequested,
    CommitSendFailed,
    Committed,
    ImportObserved,
    Quarantined,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ImportAction {
    Hold,
    SendHeader,
    SendPayload,
    AttemptCommit,
    Abort,
    Finish,
}

pub(super) struct ImportMachine {
    phase: ImportPhase,
    writer: Option<WriterIdentity>,
    header_blank_seen: bool,
    intent_synced: bool,
    final_review_passed: bool,
    committed: bool,
}

impl ImportMachine {
    pub(super) fn new() -> Self {
        Self {
            phase: ImportPhase::InputFrozen,
            writer: None,
            header_blank_seen: false,
            intent_synced: false,
            final_review_passed: false,
            committed: false,
        }
    }

    pub(super) fn phase(&self) -> ImportPhase {
        self.phase
    }

    pub(super) fn accept(&mut self, event: ImportEvent) -> Result<ImportAction, ImportFailure> {
        use ImportAction::{Abort, AttemptCommit, Finish, Hold, SendHeader, SendPayload};
        use ImportEvent as E;
        use ImportPhase as P;
        if self.phase == P::Cancelled {
            return match event {
                E::Quarantined => {
                    self.phase = P::Quarantined;
                    Ok(Finish)
                }
                _ => Err(ImportFailure::Cancelled),
            };
        }
        if self.phase == P::CommitUnknown {
            return match event {
                E::Quarantined => {
                    self.phase = P::Quarantined;
                    Ok(Finish)
                }
                _ => Err(ImportFailure::CommitUnknown),
            };
        }
        if self.phase == P::Quarantined {
            return Err(ImportFailure::Protocol);
        }
        if event == E::Cancelled {
            if self.phase == P::ImportObserved {
                return Ok(Abort);
            }
            self.phase = if self.phase == P::CommitAttempted {
                P::CommitUnknown
            } else {
                P::Cancelled
            };
            return Ok(Abort);
        }
        if event == E::Quarantined {
            self.phase = P::Quarantined;
            return Ok(Finish);
        }
        match (self.phase, event) {
            (P::InputFrozen, E::SqlVerified) => {
                self.phase = P::SqlVerified;
                Ok(SendHeader)
            }
            (P::SqlVerified, E::HeaderBlank) if !self.header_blank_seen => {
                self.header_blank_seen = true;
                Ok(Hold)
            }
            (P::SqlVerified, E::Ready(writer)) if self.header_blank_seen => {
                self.writer = Some(writer);
                self.phase = P::WriterReady;
                Ok(Hold)
            }
            (P::WriterReady, E::AttemptSynced) => {
                self.phase = P::AttemptDurable;
                Ok(SendPayload)
            }
            (P::AttemptDurable, E::PayloadSent) => {
                self.phase = P::PayloadSent;
                Ok(Hold)
            }
            (P::PayloadSent, E::Precommit(writer)) if self.writer == Some(writer) => {
                self.phase = P::PrecommitVerified;
                Ok(Hold)
            }
            (P::PrecommitVerified, E::CommitIntentSynced) if !self.intent_synced => {
                self.intent_synced = true;
                Ok(Hold)
            }
            (P::PrecommitVerified, E::FinalReviewPassed)
                if self.intent_synced && !self.final_review_passed =>
            {
                self.final_review_passed = true;
                Ok(Hold)
            }
            (P::PrecommitVerified, E::CommitPermitRequested)
                if self.intent_synced && self.final_review_passed =>
            {
                self.phase = P::CommitAttempted;
                Ok(AttemptCommit)
            }
            (P::CommitAttempted, E::CommitSendFailed) => {
                self.phase = P::CommitUnknown;
                Ok(Abort)
            }
            (P::CommitAttempted, E::Committed) if !self.committed => {
                self.committed = true;
                Ok(Hold)
            }
            (P::CommitAttempted, E::ImportObserved) if self.committed => {
                self.phase = P::ImportObserved;
                Ok(Hold)
            }
            _ => Err(ImportFailure::Protocol),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::protocol::{Nonce, WriterEvent, parse_writer_line};
    use super::*;

    #[test]
    fn cancellation_and_durable_intent_are_serial_commit_gates() {
        let mut cancelled = ImportMachine::new();
        assert_eq!(
            cancelled.accept(ImportEvent::Cancelled),
            Ok(ImportAction::Abort)
        );
        assert_eq!(
            cancelled.accept(ImportEvent::AttemptSynced),
            Err(ImportFailure::Cancelled)
        );

        let mut precommit = ImportMachine::new();
        assert_eq!(
            precommit.accept(ImportEvent::SqlVerified),
            Ok(ImportAction::SendHeader)
        );
        let nonce = Nonce::random().unwrap();
        let receipt = format!("KW_C4|{}|READY|42|1234567|99\n", nonce.hex());
        let WriterEvent::Ready(writer) = parse_writer_line(receipt.as_bytes(), &nonce).unwrap()
        else {
            panic!("ready")
        };
        assert_eq!(
            precommit.accept(ImportEvent::HeaderBlank),
            Ok(ImportAction::Hold)
        );
        assert_eq!(
            precommit.accept(ImportEvent::Ready(writer)),
            Ok(ImportAction::Hold)
        );
        assert_eq!(
            precommit.accept(ImportEvent::AttemptSynced),
            Ok(ImportAction::SendPayload)
        );
        assert_eq!(
            precommit.accept(ImportEvent::PayloadSent),
            Ok(ImportAction::Hold)
        );
        assert_eq!(
            precommit.accept(ImportEvent::Precommit(writer)),
            Ok(ImportAction::Hold)
        );
        assert!(
            precommit
                .accept(ImportEvent::CommitPermitRequested)
                .is_err()
        );
        assert_eq!(
            precommit.accept(ImportEvent::CommitIntentSynced),
            Ok(ImportAction::Hold)
        );
        assert!(
            precommit
                .accept(ImportEvent::CommitPermitRequested)
                .is_err()
        );
        assert_eq!(
            precommit.accept(ImportEvent::FinalReviewPassed),
            Ok(ImportAction::Hold)
        );
        assert_eq!(
            precommit.accept(ImportEvent::CommitPermitRequested),
            Ok(ImportAction::AttemptCommit)
        );
        assert_eq!(precommit.phase(), ImportPhase::CommitAttempted);
        assert_eq!(
            precommit.accept(ImportEvent::Cancelled),
            Ok(ImportAction::Abort)
        );
        assert_eq!(precommit.phase(), ImportPhase::CommitUnknown);
    }

    #[test]
    fn cancellation_before_commit_never_revives_and_commit_failure_stays_unknown() {
        let nonce = Nonce::random().unwrap();
        let ready = format!("KW_C4|{}|READY|42|1234567|99\n", nonce.hex());
        let WriterEvent::Ready(writer) = parse_writer_line(ready.as_bytes(), &nonce).unwrap()
        else {
            panic!("ready")
        };
        let mut machine = ImportMachine::new();
        machine.accept(ImportEvent::SqlVerified).unwrap();
        assert_eq!(
            machine.accept(ImportEvent::Ready(writer)),
            Err(ImportFailure::Protocol)
        );
        machine.accept(ImportEvent::HeaderBlank).unwrap();
        assert_eq!(
            machine.accept(ImportEvent::HeaderBlank),
            Err(ImportFailure::Protocol)
        );
        machine.accept(ImportEvent::Ready(writer)).unwrap();
        machine.accept(ImportEvent::AttemptSynced).unwrap();
        machine.accept(ImportEvent::PayloadSent).unwrap();
        machine.accept(ImportEvent::Precommit(writer)).unwrap();
        machine.accept(ImportEvent::CommitIntentSynced).unwrap();
        assert_eq!(
            machine.accept(ImportEvent::Cancelled),
            Ok(ImportAction::Abort)
        );
        assert_eq!(machine.phase(), ImportPhase::Cancelled);
        for late in [
            ImportEvent::CommitIntentSynced,
            ImportEvent::FinalReviewPassed,
            ImportEvent::CommitPermitRequested,
        ] {
            assert_eq!(machine.accept(late), Err(ImportFailure::Cancelled));
        }
        let mut attempted = ImportMachine::new();
        attempted.accept(ImportEvent::SqlVerified).unwrap();
        attempted.accept(ImportEvent::HeaderBlank).unwrap();
        attempted.accept(ImportEvent::Ready(writer)).unwrap();
        attempted.accept(ImportEvent::AttemptSynced).unwrap();
        attempted.accept(ImportEvent::PayloadSent).unwrap();
        attempted.accept(ImportEvent::Precommit(writer)).unwrap();
        attempted.accept(ImportEvent::CommitIntentSynced).unwrap();
        attempted.accept(ImportEvent::FinalReviewPassed).unwrap();
        attempted
            .accept(ImportEvent::CommitPermitRequested)
            .unwrap();
        assert_eq!(
            attempted.accept(ImportEvent::CommitSendFailed),
            Ok(ImportAction::Abort)
        );
        assert_eq!(attempted.phase(), ImportPhase::CommitUnknown);
        assert_eq!(
            attempted.accept(ImportEvent::Committed),
            Err(ImportFailure::CommitUnknown)
        );
    }

    #[test]
    fn observed_import_is_not_finished_until_quarantine() {
        let nonce = Nonce::random().unwrap();
        let ready = format!("KW_C4|{}|READY|42|1234567|99\n", nonce.hex());
        let WriterEvent::Ready(writer) = parse_writer_line(ready.as_bytes(), &nonce).unwrap()
        else {
            panic!("ready")
        };
        let other = format!("KW_C4|{}|PRECOMMIT|43|1234567|99\n", nonce.hex());
        let WriterEvent::Precommit(other_writer) =
            parse_writer_line(other.as_bytes(), &nonce).unwrap()
        else {
            panic!("precommit")
        };
        let mut machine = ImportMachine::new();
        machine.accept(ImportEvent::SqlVerified).unwrap();
        machine.accept(ImportEvent::HeaderBlank).unwrap();
        machine.accept(ImportEvent::Ready(writer)).unwrap();
        machine.accept(ImportEvent::AttemptSynced).unwrap();
        machine.accept(ImportEvent::PayloadSent).unwrap();
        assert_eq!(
            machine.accept(ImportEvent::Precommit(other_writer)),
            Err(ImportFailure::Protocol)
        );
        machine.accept(ImportEvent::Precommit(writer)).unwrap();
        machine.accept(ImportEvent::CommitIntentSynced).unwrap();
        machine.accept(ImportEvent::FinalReviewPassed).unwrap();
        machine.accept(ImportEvent::CommitPermitRequested).unwrap();
        assert_eq!(
            machine.accept(ImportEvent::ImportObserved),
            Err(ImportFailure::Protocol)
        );
        machine.accept(ImportEvent::Committed).unwrap();
        assert_eq!(
            machine.accept(ImportEvent::ImportObserved),
            Ok(ImportAction::Hold)
        );
        assert_eq!(machine.phase(), ImportPhase::ImportObserved);
        assert_eq!(
            machine.accept(ImportEvent::Quarantined),
            Ok(ImportAction::Finish)
        );
        assert_eq!(machine.phase(), ImportPhase::Quarantined);
    }
}

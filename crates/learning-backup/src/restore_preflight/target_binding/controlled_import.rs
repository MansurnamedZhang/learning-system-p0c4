//! Private command/receipt models only; no process or restore authority.
mod commands;
mod fixture_sql;
mod protocol;

#[allow(dead_code)] // Shared failure vocabulary for the subsequent private tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportFailure {
    Identity,
    Session,
    Version,
    Protocol,
    Fixture,
    InputLimit,
    Deadline,
    StdoutLimit,
    StderrLimit,
    Stderr,
    Exit,
    Io,
    Journal,
    Cancelled,
    CommitUnknown,
    UnconfirmedIsolation,
}

#[cfg(test)]
mod sibling_contract_usage {
    use super::commands::FixedImportCommand;
    use super::protocol::{Nonce, WriterEvent, WriterExpected, WriterIdentity, parse_writer_line};

    #[test]
    fn enclosing_private_module_can_consume_command_and_receipt_contracts() {
        let id = "a".repeat(64);
        let db = "learning_restore_c4_2b8a1252-54d5-48aa-b176-a9586a86bea3";
        assert!(!FixedImportCommand::decoder(&id).unwrap().argv().is_empty());
        assert!(
            !FixedImportCommand::writer(&id, db)
                .unwrap()
                .argv()
                .is_empty()
        );
        let nonce = Nonce::random().unwrap();
        let receipt = format!("KW_C4|{}|READY|42|1234567|99\n", nonce.hex());
        let ready = parse_writer_line(receipt.as_bytes(), &nonce).unwrap();
        let WriterEvent::Ready(identity): WriterEvent = ready else {
            panic!("READY required")
        };
        let identity_as_contract = |value: WriterIdentity| value;
        let precommit = format!("KW_C4|{}|PRECOMMIT|42|1234567|99\n", nonce.hex());
        assert_eq!(
            parse_writer_line(precommit.as_bytes(), &nonce),
            Ok(WriterEvent::Precommit(identity_as_contract(identity)))
        );
        let _: Option<WriterExpected> = None; // Type reachability, without exposing fields.
    }
}

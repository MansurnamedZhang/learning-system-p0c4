//! Private command/receipt models only; no process or restore authority.
mod commands;
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

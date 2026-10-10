//! Case-owned test observation. No dump bytes or process-global error slot.
use super::rehearsal_diagnostic::Diagnostic;
use crate::BackupError;
use std::process::ExitStatus;

macro_rules! fixed_codes {
    ($name:ident { $($variant:ident => $code:literal),+ $(,)? }) => {
        #[derive(Clone, Copy)]
        pub(super) enum $name { $($variant),+ }
        impl $name {
            pub(super) const ALL: &'static [Self] = &[$(Self::$variant),+];
            pub(super) fn code(self) -> &'static str {
                match self { $(Self::$variant => $code),+ }
            }
        }
    };
}
fixed_codes!(Step {
    EmbeddedContract => "EMBEDDED_CONTRACT",
    DecoderClient => "DECODER_CLIENT",
    FrozenMetadata => "FROZEN_METADATA",
    DecoderOwner => "DECODER_OWNER",
    VersionRun => "VERSION_RUN",
    VersionSeal => "VERSION_SEAL",
    VersionBytes => "VERSION_BYTES",
    TocRun => "TOC_RUN",
    TocSeal => "TOC_SEAL",
    OwnedSchemaRun => "OWNED_SCHEMA_RUN",
    OwnedSchemaSeal => "OWNED_SCHEMA_SEAL",
    DecodeRun => "DECODE_RUN",
    DecodeSeal => "DECODE_SEAL",
    DecoderSeal => "DECODER_SEAL",
    ValidatorJoin => "VALIDATOR_JOIN",
    TocValidation => "TOC_VALIDATION",
    OwnedSchemaValidation => "OWNED_SCHEMA_VALIDATION",
    DecodedHash => "DECODED_HASH",
    SqlValidation => "SQL_VALIDATION",
    CopyValidation => "COPY_VALIDATION",
    MigrationValidation => "MIGRATION_VALIDATION",
    RangeCoverage => "RANGE_COVERAGE",
});
fixed_codes!(Guard {
    ContractLoad => "CONTRACT_LOAD",
    CanonicalJson => "CANONICAL_JSON",
    ContractManifest => "CONTRACT_MANIFEST",
    ContractSourceIdentity => "CONTRACT_SOURCE_IDENTITY",
    ContractMigrations => "CONTRACT_MIGRATIONS",
    ContractFileSet => "CONTRACT_FILE_SET",
    ContractFileHash => "CONTRACT_FILE_HASH",
    ContractTable => "CONTRACT_TABLE",
    ContractColumn => "CONTRACT_COLUMN",
    ContractTableSet => "CONTRACT_TABLE_SET",
    ContractToc => "CONTRACT_TOC",
    ContractTemplate => "CONTRACT_TEMPLATE",
    Preamble => "PREAMBLE",
    RestrictKey => "RESTRICT_KEY",
    TemplateKeyCount => "TEMPLATE_KEY_COUNT",
    InputRead => "INPUT_READ",
    LineRead => "LINE_READ",
    TemplateBytes => "TEMPLATE_BYTES",
    TrailingBytes => "TRAILING_BYTES",
    TocCapacity => "TOC_CAPACITY",
    TocFormat => "TOC_FORMAT",
    TocDate => "TOC_DATE",
    TocDatabase => "TOC_DATABASE",
    TocHeader => "TOC_HEADER",
    TocEntry => "TOC_ENTRY",
    TocSet => "TOC_SET",
    SqlFixedSchema => "SQL_FIXED_SCHEMA",
    SqlTemplateFrame => "SQL_TEMPLATE_FRAME",
    CopyHeader => "COPY_HEADER",
    CopyFrame => "COPY_FRAME",
    MigrationIdentity => "MIGRATION_IDENTITY",
    SqlPostamble => "SQL_POSTAMBLE",
    HashRead => "HASH_READ",
    HashDeadline => "HASH_DEADLINE",
    HashCapacity => "HASH_CAPACITY",
    HashSeek => "HASH_SEEK",
    RangeCoverage => "RANGE_COVERAGE",
    RootMetadata => "ROOT_METADATA",
    ClientComponentMetadata => "CLIENT_COMPONENT_METADATA",
    ClientHash => "CLIENT_HASH",
    ClientIo => "CLIENT_IO",
    FrozenMetadata => "FROZEN_METADATA",
    PrivateDirectory => "PRIVATE_DIRECTORY",
    ClosedBeforeRun => "CLOSED_BEFORE_RUN",
    DeadlineBeforeRun => "DEADLINE_BEFORE_RUN",
    NativeHandleMissing => "NATIVE_HANDLE_MISSING",
    NativeSpawn => "NATIVE_SPAWN",
    NativePredicate => "NATIVE_PREDICATE",
    NativeRun => "NATIVE_RUN",
    VersionBytes => "VERSION_BYTES",
    OutputMissing => "OUTPUT_MISSING",
    OutputCreate => "OUTPUT_CREATE",
    InputSeek => "INPUT_SEEK",
    InputClone => "INPUT_CLONE",
    SealFile => "SEAL_FILE",
    SealDirectory => "SEAL_DIRECTORY",
    OwnerLost => "OWNER_LOST",
    ValidatorTaskLost => "VALIDATOR_TASK_LOST",
    SpoolClone => "SPOOL_CLONE",
});

#[derive(Clone, Copy)]
pub(super) struct Native {
    operation: super::spool::Operation,
    predicate: Option<bool>,
    stderr_nonempty: bool,
}
impl Native {
    pub(super) fn new(
        operation: super::spool::Operation,
        predicate: Option<ExitStatus>,
        stderr_nonempty: bool,
    ) -> Self {
        Self {
            operation,
            predicate: predicate.map(|status| status.success()),
            stderr_nonempty,
        }
    }
    pub(super) fn append_to(self, code: &mut String) {
        use std::fmt::Write;
        let (wait, status) = match self.predicate {
            Some(true) => ("WAIT_OBSERVED", "SUCCESS"),
            Some(false) => ("WAIT_OBSERVED", "NONZERO"),
            None => ("WAIT_NOT_OBSERVED", "STATUS_NOT_OBSERVED"),
        };
        let stderr = if self.stderr_nonempty {
            "STDERR_NONEMPTY"
        } else {
            "STDERR_EMPTY"
        };
        let _ = write!(
            code,
            "__{}__{wait}__{status}__{stderr}",
            self.operation.code()
        );
    }
}

#[derive(Clone)]
pub(crate) struct Observer(Diagnostic);
impl Observer {
    pub(crate) fn new(diagnostic: Diagnostic) -> Self {
        Self(diagnostic)
    }
    pub(super) fn returned<T>(&self, step: Step, guard: Guard, result: &Result<T, BackupError>) {
        if let Err(error) = result {
            self.failed(step, guard, error, None);
        }
    }
    pub(super) fn failed(
        &self,
        step: Step,
        guard: Guard,
        error: &BackupError,
        native: Option<Native>,
    ) {
        self.0.dump_failure(step, guard, error, native);
    }
    pub(super) fn native_result<T>(
        &self,
        operation: super::spool::Operation,
        guard: Guard,
        result: &Result<T, BackupError>,
        predicate: Option<ExitStatus>,
        stderr_nonempty: bool,
    ) {
        if let Err(error) = result {
            self.failed(
                operation.run_step(),
                guard,
                error,
                Some(Native::new(operation, predicate, stderr_nonempty)),
            );
        }
    }
}

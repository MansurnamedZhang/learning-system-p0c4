//! Full content validation and guarded recovery; content alone grants no authority.
// The observer expressions disappear from normal builds, including private
// signatures. Every boundary returns the exact original Result/BackupError.
macro_rules! dump_observe {
    ($observer:expr, $step:expr, $guard:expr, $call:expr) => {{
        let result = $call;
        #[cfg(test)]
        if let Some(observer) = $observer {
            observer.returned($step, $guard, &result);
        }
        result
    }};
}
macro_rules! dump_error {
    ($observer:expr, $step:expr, $guard:expr, $error:expr) => {{
        let error = $error;
        #[cfg(test)]
        if let Some(observer) = $observer {
            observer.failed($step, $guard, &error, None);
        }
        error
    }};
}
#[cfg(any(test, target_os = "linux"))]
pub(crate) mod assets;
mod copy;
#[cfg(any(test, target_os = "linux"))]
mod data;
#[cfg(all(test, target_os = "linux"))]
mod live_tests;
#[cfg(test)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) mod profile;
mod schema;
mod spool;
pub use schema::{ColumnDefinition, SchemaContract};
pub use spool::{FrozenFullDump, freeze_full_dump};

use crate::BackupError;
#[cfg(test)]
use dump_observer::{Guard, Observer, Step};
use learning_assets::backup_fs::BackupDir;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, BufReader, Read, Seek, SeekFrom},
    time::{Duration, Instant},
};

/// Fully validated content, NOT a CompleteBackup or a target-write permit.
pub struct VerifiedFullImport {
    file: File,
    ranges: Vec<schema::Range>,
    tables: Vec<String>,
    decoded_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentKind<'a> {
    Schema,
    Copy { table: &'a str },
}

/// Only the validator can construct a checked range. No raw file descriptor or
/// writable/path-based reopen escapes this object.
pub struct VerifiedSegment<'a> {
    file: &'a File,
    range: &'a schema::Range,
    kind: SegmentKind<'a>,
}
impl VerifiedFullImport {
    pub fn segments(&self) -> impl Iterator<Item = VerifiedSegment<'_>> {
        self.ranges.iter().map(|range| VerifiedSegment {
            file: &self.file,
            range,
            kind: range
                .table
                .map_or(SegmentKind::Schema, |i| SegmentKind::Copy {
                    table: &self.tables[i],
                }),
        })
    }
    pub fn decoded_sha256(&self) -> &str {
        &self.decoded_sha256
    }
}
impl<'a> VerifiedSegment<'a> {
    pub fn kind(&self) -> SegmentKind<'a> {
        self.kind
    }
    pub fn len(&self) -> u64 {
        self.range.len
    }
    pub fn is_empty(&self) -> bool {
        self.range.len == 0
    }
    pub fn reader(&self) -> impl Read + 'a {
        RangeReader {
            file: self.file,
            position: self.range.start,
            remaining: self.range.len,
        }
    }
}
struct RangeReader<'a> {
    file: &'a File,
    position: u64,
    remaining: u64,
}
impl Read for RangeReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let length = self.remaining.min(buffer.len() as u64) as usize;
        if length == 0 {
            return Ok(0);
        }
        #[cfg(not(any(target_os = "linux", all(test, windows))))]
        {
            let _ = (&self.file, self.position);
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "verified spool requires Linux",
            ))
        }
        #[cfg(any(target_os = "linux", all(test, windows)))]
        {
            #[cfg(target_os = "linux")]
            use std::os::unix::fs::FileExt;
            #[cfg(all(test, windows))]
            use std::os::windows::fs::FileExt;
            #[cfg(all(test, windows))]
            let n = self.file.seek_read(&mut buffer[..length], self.position)?;
            #[cfg(target_os = "linux")]
            let n = self.file.read_at(&mut buffer[..length], self.position)?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "checked spool truncated",
                ));
            }
            self.remaining -= n as u64;
            self.position += n as u64;
            Ok(n)
        }
    }
}

/// Decode only with the fixed held PG18 client; no database connection exists.
pub async fn validate_full_dump(
    dump: FrozenFullDump,
    contract: &SchemaContract,
    spool: &BackupDir,
) -> Result<VerifiedFullImport, BackupError> {
    validate_full_dump_inner(
        dump,
        contract,
        spool,
        #[cfg(test)]
        None,
    )
    .await
}
#[cfg(test)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(super) async fn validate_full_dump_observed(
    dump: FrozenFullDump,
    contract: &SchemaContract,
    spool: &BackupDir,
    observer: Observer,
) -> Result<VerifiedFullImport, BackupError> {
    validate_full_dump_inner(dump, contract, spool, Some(observer)).await
}
async fn validate_full_dump_inner(
    dump: FrozenFullDump,
    contract: &SchemaContract,
    spool: &BackupDir,
    #[cfg(test)] observer: Option<Observer>,
) -> Result<VerifiedFullImport, BackupError> {
    let deadline = Instant::now() + Duration::from_secs(900);
    let root = dump_observe!(
        observer.as_ref(),
        Step::DecoderOwner,
        Guard::SpoolClone,
        spool.try_clone().map_err(BackupError::from)
    )?;
    let decoded = spool::decode(
        dump,
        root,
        #[cfg(test)]
        observer.clone(),
    )
    .await?;
    let contract = contract.clone();
    validate_decoded_task(
        decoded,
        contract,
        deadline,
        #[cfg(test)]
        observer,
    )
    .await
}
async fn validate_decoded_task(
    decoded: spool::Decoded,
    contract: SchemaContract,
    deadline: Instant,
    #[cfg(test)] observer: Option<Observer>,
) -> Result<VerifiedFullImport, BackupError> {
    #[cfg(test)]
    let worker_observer = observer.clone();
    let result = tokio::task::spawn_blocking(move || {
        validate_decoded(
            decoded,
            &contract,
            deadline,
            #[cfg(test)]
            worker_observer.as_ref(),
        )
    })
    .await
    .map_err(|_| BackupError::Invalid("full validator task lost"));
    dump_observe!(
        observer.as_ref(),
        Step::ValidatorJoin,
        Guard::ValidatorTaskLost,
        result
    )?
}

fn validate_decoded(
    mut decoded: spool::Decoded,
    contract: &SchemaContract,
    deadline: Instant,
    #[cfg(test)] observer: Option<&Observer>,
) -> Result<VerifiedFullImport, BackupError> {
    let mut toc = Vec::new();
    if dump_observe!(
        observer,
        Step::TocValidation,
        Guard::TocCapacity,
        decoded.toc.metadata().map_err(BackupError::from)
    )?
    .len()
        > 4 * 1024 * 1024
    {
        return Err(dump_error!(
            observer,
            Step::TocValidation,
            Guard::TocCapacity,
            BackupError::Capacity("full TOC")
        ));
    }
    dump_observe!(
        observer,
        Step::TocValidation,
        Guard::InputRead,
        decoded.toc.read_to_end(&mut toc).map_err(BackupError::from)
    )?;
    #[cfg(test)]
    schema::validate_toc_observed(&toc, contract, observer)?;
    #[cfg(not(test))]
    schema::validate_toc(&toc, contract)?;
    #[cfg(test)]
    schema::validate_owned_observed(
        &mut BufReader::with_capacity(
            spool::BUFFER,
            DeadlineRead {
                file: &mut decoded.owned,
                deadline,
            },
        ),
        contract,
        observer,
    )?;
    #[cfg(not(test))]
    schema::validate_owned(
        &mut BufReader::with_capacity(
            spool::BUFFER,
            DeadlineRead {
                file: &mut decoded.owned,
                deadline,
            },
        ),
        contract,
    )?;
    let mut hash = Sha256::new();
    let mut buffer = [0; spool::BUFFER];
    let mut size = 0u64;
    loop {
        if Instant::now() >= deadline {
            return Err(dump_error!(
                observer,
                Step::DecodedHash,
                Guard::HashDeadline,
                BackupError::Invalid("full decode deadline")
            ));
        }
        let n = dump_observe!(
            observer,
            Step::DecodedHash,
            Guard::HashRead,
            decoded.sql.read(&mut buffer).map_err(BackupError::from)
        )?;
        if n == 0 {
            break;
        }
        size += n as u64;
        if size > spool::MAX_DECODE {
            return Err(dump_error!(
                observer,
                Step::DecodedHash,
                Guard::HashCapacity,
                BackupError::Capacity("decoded SQL")
            ));
        }
        hash.update(&buffer[..n]);
    }
    dump_observe!(
        observer,
        Step::DecodedHash,
        Guard::HashSeek,
        decoded
            .sql
            .seek(SeekFrom::Start(0))
            .map_err(BackupError::from)
    )?;
    #[cfg(test)]
    let ranges = schema::validate_sql_observed(
        &mut BufReader::with_capacity(
            spool::BUFFER,
            DeadlineRead {
                file: &mut decoded.sql,
                deadline,
            },
        ),
        contract,
        observer,
    )?;
    #[cfg(not(test))]
    let ranges = schema::validate_sql(
        &mut BufReader::with_capacity(
            spool::BUFFER,
            DeadlineRead {
                file: &mut decoded.sql,
                deadline,
            },
        ),
        contract,
    )?;
    if dump_observe!(
        observer,
        Step::RangeCoverage,
        Guard::RangeCoverage,
        ranges
            .iter()
            .try_fold(0u64, |a, r| a.checked_add(r.len))
            .ok_or(BackupError::Overflow)
    )? != size
    {
        return Err(dump_error!(
            observer,
            Step::RangeCoverage,
            Guard::RangeCoverage,
            BackupError::Invalid("checked range coverage")
        ));
    }
    Ok(VerifiedFullImport {
        file: decoded.sql,
        ranges,
        tables: contract.table_names().map(String::from).collect(),
        decoded_sha256: hex::encode(hash.finalize()),
    })
}
struct DeadlineRead<'a> {
    file: &'a mut File,
    deadline: Instant,
}
impl Read for DeadlineRead<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if Instant::now() >= self.deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "full decode deadline",
            ));
        }
        self.file.read(buffer)
    }
}

#[cfg(target_os = "linux")]
pub use crate::restore_preflight::target_binding::controlled_import::full::{
    AssetRestoreReport, ImportedTarget, RecoveryPending, restore_complete_backup,
};

#[cfg(not(target_os = "linux"))]
pub struct RecoveryPending {
    _private: (),
}
#[cfg(not(target_os = "linux"))]
pub async fn restore_complete_backup(
    complete: crate::CompleteBackup,
    config: crate::RestorePreflightConfig,
    admin: sqlx::PgPool,
) -> Result<RecoveryPending, BackupError> {
    config.validate()?;
    let _ = (complete, admin);
    Err(BackupError::Invalid("full restore requires Linux"))
}
#[cfg(any(test, target_os = "linux"))]
impl VerifiedFullImport {
    pub(crate) fn checked_reader_from(
        &self,
        offset: u64,
    ) -> Result<impl Read + Send + '_, BackupError> {
        let size = self
            .ranges
            .iter()
            .try_fold(0u64, |a, r| a.checked_add(r.len))
            .ok_or(BackupError::Overflow)?;
        if offset > size {
            return Err(BackupError::Invalid("checked full range offset"));
        }
        let mut skip = offset;
        let mut index = 0;
        while index < self.ranges.len() && skip >= self.ranges[index].len {
            skip -= self.ranges[index].len;
            index += 1;
        }
        Ok(CheckedRanges {
            file: &self.file,
            ranges: &self.ranges,
            index,
            within: skip,
        })
    }
}
#[cfg(any(test, target_os = "linux"))]
impl SchemaContract {
    pub(crate) fn ownership_template(&self) -> &[u8] {
        &self.owned_template
    }
    #[cfg(target_os = "linux")]
    pub(crate) fn catalog(&self) -> &serde_json::Value {
        &self.acl
    }
}

#[cfg(target_os = "linux")]
pub use crate::restore_preflight::target_binding::controlled_import::full::verified_complete_role_recipe;
#[cfg(not(target_os = "linux"))]
pub fn verified_complete_role_recipe(_: uuid::Uuid) -> Result<Vec<u8>, BackupError> {
    Err(BackupError::Invalid(
        "installed complete role reader requires Linux",
    ))
}

#[cfg(any(test, target_os = "linux"))]
pub(crate) fn trusted_ownership() -> Result<String, BackupError> {
    let contract = SchemaContract::embedded()?;
    let text = std::str::from_utf8(contract.ownership_template())
        .map_err(|_| BackupError::Invalid("embedded ownership utf8"))?;
    const REVOKE: &str = "REVOKE ALL ON FUNCTION public.lock_space_grant(requested_actor uuid, requested_space uuid) FROM PUBLIC;";
    const GRANT: &str = "GRANT ALL ON FUNCTION public.lock_space_grant(requested_actor uuid, requested_space uuid) TO learning_runtime;";
    if text.lines().filter(|line| *line == REVOKE).count() != 1
        || text.lines().filter(|line| *line == GRANT).count() != 1
    {
        return Err(BackupError::Invalid("embedded auth lock ACL recipe"));
    }
    let mut result = String::new();
    for line in text.lines() {
        if line == REVOKE || line == GRANT {
            continue;
        }
        if (line.starts_with("ALTER ") && line.contains(" OWNER TO "))
            || line.starts_with("GRANT ")
            || line.starts_with("REVOKE ")
        {
            if !line.ends_with(';') {
                return Err(BackupError::Invalid("embedded ownership statement"));
            }
            if line
                == "ALTER FUNCTION public.lock_space_grant(requested_actor uuid, requested_space uuid) OWNER TO learning_auth_lock;"
            {
                result.push_str(REVOKE);
                result.push('\n');
                result.push_str(GRANT);
                result.push('\n');
                result.push_str("GRANT CREATE ON SCHEMA public TO learning_auth_lock;\n");
                result.push_str(line);
                result.push('\n');
                result.push_str("REVOKE CREATE ON SCHEMA public FROM learning_auth_lock;\n");
            } else {
                result.push_str(line);
                result.push('\n');
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
pub(crate) mod dump_observer;
#[cfg(test)]
mod dump_validation_diagnostic_tests;
#[cfg(test)]
mod fix1_tests;
#[cfg(test)]
pub(crate) mod rehearsal_diagnostic;

#[cfg(any(test, target_os = "linux"))]
struct CheckedRanges<'a> {
    file: &'a File,
    ranges: &'a [schema::Range],
    index: usize,
    within: u64,
}
#[cfg(any(test, target_os = "linux"))]
impl Read for CheckedRanges<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        while let Some(range) = self.ranges.get(self.index) {
            if self.within == range.len {
                self.index += 1;
                self.within = 0;
                continue;
            }
            let mut part = RangeReader {
                file: self.file,
                position: range
                    .start
                    .checked_add(self.within)
                    .ok_or_else(|| io::Error::other("checked range overflow"))?,
                remaining: range.len - self.within,
            };
            let n = part.read(buffer)?;
            self.within += n as u64;
            return Ok(n);
        }
        Ok(0)
    }
}

#[cfg(test)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) mod negative;

//! Full, bounded row comparison before any recovery mutation. The expected
//! inventory can only be derived from the already validated frozen dump.
use super::*;
use crate::full_restore::{SegmentKind, VerifiedFullImport};
use learning_assets::backup_fs::BackupDir;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::time::{Duration, Instant};

// Source COPY frames need not be in primary-key order. Spill only keys and row
// digests, never build a second all-row in-memory copy of a 4 GiB dump.
const SORT_MEMORY: usize = 8 * 1024 * 1024;
const SORT_ENTRIES: usize = 65_536;
const MAX_RUNS: usize = 1024;
const MAX_KEY_PARTS: usize = 1600;

#[derive(Debug, Clone)]
pub(crate) struct ExpectedRecoveryRows {
    decoded_sha256: String,
    tables: Vec<TableObservation>,
}

/// The only excluded table is checked separately against the opaque, committed
/// normalization report. No caller can choose exclusions or provide row hashes.
#[cfg(target_os = "linux")]
pub(crate) struct UnchangedRecoveryRows(RecoveryRowsReport);
#[cfg(target_os = "linux")]
impl UnchangedRecoveryRows {
    pub(crate) fn evidence(&self) -> serde_json::Value {
        serde_json::json!({"format_version":1,
            "capability":"recovery_post_normalization_unchanged_rows_v1",
            "expected_sql_sha256":self.0.expected_sql_sha256,"tables":self.0.tables,
            "unchanged_tables":63,"separately_verified_normalized_table":"job",
            "excluded_columns":[],
            "timestamptz_comparison":"exact_microsecond_instant_including_infinities"})
    }
}

#[derive(Debug)]
pub(crate) struct RecoveryRowsReport {
    expected_sql_sha256: String,
    tables: Vec<TableObservation>,
}
impl RecoveryRowsReport {
    pub(crate) fn table_count(&self) -> usize {
        self.tables.len()
    }
    pub(crate) fn evidence(&self) -> serde_json::Value {
        serde_json::json!({
            "format_version":1,
            "capability":"recovery_all_table_rows_verified_v1",
            "expected_sql_sha256":self.expected_sql_sha256,
            "tables":self.tables,
            "excluded_columns":[],
            "timestamptz_comparison":"exact_microsecond_instant_including_infinities"
        })
    }
    pub(crate) fn canonical_sha256(&self) -> String {
        hex::encode(Sha256::digest(
            serde_json::to_vec(&self.evidence()).expect("finite row evidence"),
        ))
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Entry {
    key: Vec<KeyPart>,
    digest: [u8; 32],
    raw_bytes: u64,
}

fn deadline(end: Instant) -> Result<(), BackupError> {
    if Instant::now() >= end {
        return Err(BackupError::Invalid("recovery row comparison deadline"));
    }
    Ok(())
}

struct TimedRead<R> {
    inner: R,
    end: Instant,
}
impl<R: Read> Read for TimedRead<R> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if Instant::now() >= self.end {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "recovery row read deadline",
            ));
        }
        self.inner.read(bytes)
    }
}

fn encode_entry(entry: &Entry, out: &mut impl Write) -> Result<(), BackupError> {
    out.write_all(&(entry.key.len() as u16).to_be_bytes())?;
    for part in &entry.key {
        match part {
            KeyPart::Integer(value) => {
                out.write_all(&[0])?;
                out.write_all(&value.to_be_bytes())?;
            }
            KeyPart::Uuid(value) => {
                out.write_all(&[1])?;
                out.write_all(value)?;
            }
            KeyPart::Text(value) => {
                out.write_all(&[2])?;
                out.write_all(&(value.len() as u64).to_be_bytes())?;
                out.write_all(value)?;
            }
        }
    }
    out.write_all(&entry.digest)?;
    out.write_all(&entry.raw_bytes.to_be_bytes())?;
    Ok(())
}

fn fixed<const N: usize>(reader: &mut impl Read) -> Result<[u8; N], BackupError> {
    let mut bytes = [0; N];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

// BackupDir deliberately creates O_WRONLY files. Reopen a reader through the
// same held directory and bind its identity to the just-synced write handle.
fn reopen_scratch(root: &BackupDir, name: &str, written: &File) -> Result<File, BackupError> {
    let reader = root.open_file(name)?;
    let before = written.metadata()?;
    let after = reader.metadata()?;
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != after.dev()
            || before.ino() != after.ino()
            || before.len() != after.len()
            || before.nlink() != 1
            || after.nlink() != 1
            || before.mtime() != after.mtime()
            || before.mtime_nsec() != after.mtime_nsec()
        {
            return Err(BackupError::Invalid("recovery scratch file replaced"));
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (before, after);
        return Err(BackupError::Invalid("recovery scratch requires Linux"));
    }
    #[cfg(target_os = "linux")]
    Ok(reader)
}

fn decode_entry(reader: &mut impl Read) -> Result<Option<Entry>, BackupError> {
    let mut first = [0];
    if reader.read(&mut first)? == 0 {
        return Ok(None);
    }
    let n = u16::from_be_bytes([first[0], fixed::<1>(reader)?[0]]) as usize;
    if n == 0 || n > MAX_KEY_PARTS {
        return Err(BackupError::Invalid("recovery spill key count"));
    }
    let mut key = Vec::with_capacity(n);
    let mut key_bytes = 0_u64;
    for _ in 0..n {
        key.push(match fixed::<1>(reader)?[0] {
            0 => KeyPart::Integer(i64::from_be_bytes(fixed(reader)?)),
            1 => KeyPart::Uuid(fixed(reader)?),
            2 => {
                let size = u64::from_be_bytes(fixed(reader)?);
                key_bytes = key_bytes.checked_add(size).ok_or(BackupError::Overflow)?;
                if key_bytes > crate::full_restore::spool::MAX_ROW {
                    return Err(BackupError::Capacity("recovery spill key"));
                }
                let mut bytes = vec![0; usize::try_from(size).map_err(|_| BackupError::Overflow)?];
                reader.read_exact(&mut bytes)?;
                std::str::from_utf8(&bytes)
                    .map_err(|_| BackupError::Invalid("recovery spill text key"))?;
                KeyPart::Text(bytes)
            }
            _ => return Err(BackupError::Invalid("recovery spill key type")),
        });
    }
    let digest = fixed(reader)?;
    let raw_bytes = u64::from_be_bytes(fixed(reader)?);
    if raw_bytes == 0 || raw_bytes > crate::full_restore::spool::MAX_ROW {
        return Err(BackupError::Capacity("recovery spill row"));
    }
    Ok(Some(Entry {
        key,
        digest,
        raw_bytes,
    }))
}

fn merge_runs(
    table: &str,
    readers: &mut [impl Read],
    end: Instant,
) -> Result<TableObservation, BackupError> {
    let mut queue = BinaryHeap::new();
    for (i, reader) in readers.iter_mut().enumerate() {
        if let Some(entry) = decode_entry(reader)? {
            queue.push(Reverse((entry, i)));
        }
    }
    let mut previous = None;
    let mut rows = 0_u64;
    let mut raw_bytes = 0_u64;
    let mut hash = Sha256::new();
    hash.update(b"knowweave-recovery-table-v1\0");
    hash.update((table.len() as u64).to_be_bytes());
    hash.update(table.as_bytes());
    while let Some(Reverse((entry, i))) = queue.pop() {
        deadline(end)?;
        if previous.as_ref().is_some_and(|old| old >= &entry.key) {
            return Err(BackupError::Invalid(
                "recovery duplicate or unsorted primary key",
            ));
        }
        hash.update(entry.digest);
        rows = rows.checked_add(1).ok_or(BackupError::Overflow)?;
        raw_bytes = raw_bytes
            .checked_add(entry.raw_bytes)
            .ok_or(BackupError::Overflow)?;
        previous = Some(entry.key);
        if let Some(next) = decode_entry(&mut readers[i])? {
            queue.push(Reverse((next, i)));
        }
    }
    hash.update(rows.to_be_bytes());
    Ok(TableObservation {
        table: table.into(),
        rows,
        raw_bytes,
        digest: hash.finalize().into(),
    })
}

fn flush_run(
    entries: &mut Vec<Entry>,
    runs: &mut Vec<BufReader<File>>,
    scratch: &BackupDir,
    table_index: usize,
) -> Result<(), BackupError> {
    if entries.is_empty() {
        return Ok(());
    }
    if runs.len() >= MAX_RUNS {
        return Err(BackupError::Capacity("recovery sort run count"));
    }
    entries.sort_unstable_by(|a, b| a.key.cmp(&b.key));
    if entries.windows(2).any(|pair| pair[0].key == pair[1].key) {
        return Err(BackupError::Invalid("recovery duplicate primary key"));
    }
    let name = format!("source-{table_index:03}-{:04}.run", runs.len());
    let mut file = scratch.create_file(&name)?;
    for entry in entries.drain(..) {
        encode_entry(&entry, &mut file)?;
    }
    file.sync_all()?;
    scratch.sync()?;
    let reader = reopen_scratch(scratch, &name, &file)?;
    runs.push(BufReader::new(reader));
    Ok(())
}

pub(crate) fn capture_expected(
    input: &VerifiedFullImport,
    scratch: &BackupDir,
) -> Result<ExpectedRecoveryRows, BackupError> {
    let end = Instant::now() + Duration::from_secs(900);
    let contract = SchemaContract::embedded()?;
    let keys = table_keys(&contract)?;
    let mut found = BTreeMap::new();
    let mut budget = DataBudget::default();
    for segment in input.segments() {
        let SegmentKind::Copy { table } = segment.kind() else {
            continue;
        };
        deadline(end)?;
        let (table_index, key) = keys
            .iter()
            .enumerate()
            .find(|(_, k)| k.table.table == table)
            .ok_or(BackupError::Invalid("recovery unknown COPY table"))?;
        if found.contains_key(table) {
            return Err(BackupError::Invalid("recovery duplicate COPY table"));
        }
        let mut reader = BufReader::new(TimedRead {
            inner: segment.reader(),
            end,
        });
        let header = format!(
            "COPY public.{} ({}) FROM stdin;\n",
            key.table.sql_name,
            key.table
                .columns
                .iter()
                .map(|c| c.sql_name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let mut saw_header = false;
        for _ in 0..8 {
            let line = read_row(&mut reader, &mut budget)?
                .ok_or(BackupError::Invalid("recovery missing COPY header"))?;
            if line.len() > 16_384 {
                return Err(BackupError::Capacity("recovery COPY header"));
            }
            if line == header.as_bytes() {
                saw_header = true;
                break;
            }
        }
        if !saw_header {
            return Err(BackupError::Invalid("recovery COPY header"));
        }
        let mut entries = Vec::new();
        let mut memory = 0_usize;
        let mut runs = Vec::new();
        loop {
            deadline(end)?;
            let row = read_row(&mut reader, &mut budget)?
                .ok_or(BackupError::Invalid("recovery missing COPY terminator"))?;
            if row == b"\\.\n" {
                break;
            }
            let entry = Entry {
                key: row_key(key, &row)?,
                digest: row_digest(key.table, &row)?,
                raw_bytes: row.len() as u64,
            };
            memory = memory.checked_add(row.len()).ok_or(BackupError::Overflow)?;
            entries.push(entry);
            if memory >= SORT_MEMORY || entries.len() >= SORT_ENTRIES {
                flush_run(&mut entries, &mut runs, scratch, table_index)?;
                memory = 0;
            }
        }
        let mut tail = Vec::new();
        reader.take(4).read_to_end(&mut tail)?;
        if tail != b"\n\n" {
            return Err(BackupError::Invalid("recovery COPY trailing data"));
        }
        flush_run(&mut entries, &mut runs, scratch, table_index)?;
        let observation = merge_runs(table, &mut runs, end)?;
        found.insert(table.to_owned(), observation);
    }
    if found.len() != contract.tables.len() {
        return Err(BackupError::Invalid(
            "recovery incomplete source table inventory",
        ));
    }
    Ok(ExpectedRecoveryRows {
        decoded_sha256: input.decoded_sha256().into(),
        tables: found.into_values().collect(),
    })
}

#[cfg(target_os = "linux")]
async fn copy_table(
    connection: &mut sqlx::PgConnection,
    query: &str,
    mut file: File,
    remaining: u64,
    end: tokio::time::Instant,
) -> Result<(File, u64), BackupError> {
    let (sender, mut receiver) = tokio::sync::mpsc::channel::<Vec<u8>>(2);
    let writer = tokio::task::spawn_blocking(move || {
        let mut bytes = 0_u64;
        while let Some(chunk) = receiver.blocking_recv() {
            bytes = bytes
                .checked_add(chunk.len() as u64)
                .ok_or(BackupError::Overflow)?;
            if bytes > remaining {
                return Err(BackupError::Capacity("recovery target COPY total"));
            }
            file.write_all(&chunk)?;
        }
        file.sync_all()?;
        Ok::<_, BackupError>((file, bytes))
    });
    let result = async {
        let mut stream = connection.copy_out_raw(query).await?;
        loop {
            let chunk = std::future::poll_fn(|cx| stream.as_mut().poll_next(cx)).await;
            let Some(chunk) = chunk else { break };
            let chunk = chunk?;
            if chunk.len() as u64 > crate::full_restore::spool::MAX_ROW {
                return Err(BackupError::Capacity("recovery target COPY chunk"));
            }
            sender
                .send(chunk.to_vec())
                .await
                .map_err(|_| BackupError::Invalid("recovery COPY writer lost"))?;
        }
        Ok::<_, BackupError>(())
    };
    let result = tokio::time::timeout_at(end, result)
        .await
        .map_err(|_| BackupError::Invalid("recovery target COPY deadline"));
    drop(sender);
    let written = writer
        .await
        .map_err(|_| BackupError::Invalid("recovery COPY writer lost"))?;
    result??;
    written
}

#[cfg(target_os = "linux")]
pub(crate) fn verify_target_rows<'a>(
    connection: &'a mut sqlx::PgConnection,
    expected: &'a ExpectedRecoveryRows,
    scratch: &'a BackupDir,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<RecoveryRowsReport, BackupError>> + Send + 'a>,
> {
    verify_target_rows_inner(connection, expected, scratch, false)
}

#[cfg(target_os = "linux")]
pub(crate) async fn verify_unchanged_rows_after_normalization(
    connection: &mut sqlx::PgConnection,
    expected: &ExpectedRecoveryRows,
    scratch: &BackupDir,
) -> Result<UnchangedRecoveryRows, BackupError> {
    let rows = verify_target_rows_inner(connection, expected, scratch, true).await?;
    if rows.tables.len() != 63 || rows.tables.iter().any(|row| row.table == "job") {
        return Err(BackupError::Invalid("recovery unchanged row coverage"));
    }
    Ok(UnchangedRecoveryRows(rows))
}

#[cfg(target_os = "linux")]
fn verify_target_rows_inner<'a>(
    connection: &'a mut sqlx::PgConnection,
    expected: &'a ExpectedRecoveryRows,
    scratch: &'a BackupDir,
    after_normalization: bool,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<RecoveryRowsReport, BackupError>> + Send + 'a>,
> {
    Box::pin(async move {
        let end = tokio::time::Instant::now() + Duration::from_secs(900);
        let contract = SchemaContract::embedded()?;
        let keys = table_keys(&contract)?;
        if expected.tables.len() != keys.len() {
            return Err(BackupError::Invalid("recovery source inventory count"));
        }
        for statement in [
            "SET LOCAL TimeZone='UTC'",
            "SET LOCAL DateStyle='ISO,YMD'",
            "SET LOCAL IntervalStyle='postgres'",
            "SET LOCAL bytea_output='hex'",
            "SET LOCAL extra_float_digits=3",
            "SET LOCAL statement_timeout='15s'",
        ] {
            tokio::time::timeout_at(end, sqlx::query(statement).execute(&mut *connection))
                .await
                .map_err(|_| BackupError::Invalid("recovery row setup deadline"))??;
        }
        let mut copied = 0_u64;
        let mut tables = Vec::new();
        for (i, key) in keys.iter().enumerate() {
            if after_normalization && key.table.table == "job" {
                continue;
            }
            let columns = key
                .table
                .columns
                .iter()
                .map(|c| c.sql_name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let order = key
                .indices
                .iter()
                .map(|&j| {
                    let c = &key.table.columns[j];
                    if c.ty == "text" {
                        format!("{} COLLATE \"C\"", c.sql_name)
                    } else {
                        c.sql_name.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            let query = format!(
                "COPY (SELECT {columns} FROM public.{} ORDER BY {order}) TO STDOUT",
                key.table.sql_name
            );
            let name = format!("target-{i:03}.copy");
            let file = scratch.create_file(&name)?;
            let (file, bytes) = copy_table(
                connection,
                &query,
                file,
                crate::full_restore::spool::MAX_DECODE - copied,
                end,
            )
            .await?;
            copied = copied.checked_add(bytes).ok_or(BackupError::Overflow)?;
            scratch.sync()?;
            let file = reopen_scratch(scratch, &name, &file)?;
            // CPU parsing and disk reads do not occupy a Tokio worker thread.
            let contract_for_read = contract.clone();
            let read_end =
                Instant::now() + end.saturating_duration_since(tokio::time::Instant::now());
            let observation = tokio::task::spawn_blocking(move || {
                deadline(read_end)?;
                let keys = table_keys(&contract_for_read)?;
                observe_sorted(
                    &keys[i],
                    &mut BufReader::new(TimedRead {
                        inner: file,
                        end: read_end,
                    }),
                    &mut DataBudget::default(),
                )
            })
            .await
            .map_err(|_| BackupError::Invalid("recovery target row reader lost"))??;
            let original = expected
                .tables
                .iter()
                .find(|t| t.table == observation.table)
                .ok_or(BackupError::Invalid("recovery missing expected table"))?;
            if !original.same_data(&observation) {
                return Err(BackupError::Invalid("recovery imported table rows differ"));
            }
            tables.push(observation);
        }
        tables.sort_unstable_by(|a, b| a.table.cmp(&b.table));
        Ok(RecoveryRowsReport {
            expected_sql_sha256: expected.decoded_sha256.clone(),
            tables,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn entry(key: i64, byte: u8) -> Entry {
        Entry {
            key: vec![KeyPart::Integer(key)],
            digest: [byte; 32],
            raw_bytes: 16,
        }
    }
    fn run(entries: &[Entry]) -> Cursor<Vec<u8>> {
        let mut bytes = Vec::new();
        for e in entries {
            encode_entry(e, &mut bytes).unwrap();
        }
        Cursor::new(bytes)
    }
    #[test]
    fn spill_merge_orders_signed_keys_and_includes_every_full_row_digest() {
        let end = Instant::now() + Duration::from_secs(5);
        let a = merge_runs(
            "job",
            &mut [run(&[entry(-1, 1), entry(10, 3)]), run(&[entry(2, 2)])],
            end,
        )
        .unwrap();
        let b = merge_runs(
            "job",
            &mut [run(&[entry(-1, 1), entry(2, 2), entry(10, 3)])],
            end,
        )
        .unwrap();
        assert!(a.same_data(&b));
        let changed = merge_runs(
            "job",
            &mut [run(&[entry(-1, 1), entry(2, 9), entry(10, 3)])],
            end,
        )
        .unwrap();
        assert!(!a.same_data(&changed));
    }
    #[test]
    fn duplicate_across_spill_runs_and_truncated_run_are_rejected() {
        let end = Instant::now() + Duration::from_secs(5);
        assert!(merge_runs("job", &mut [run(&[entry(1, 1)]), run(&[entry(1, 2)])], end).is_err());
        let mut bad = run(&[entry(1, 1)]).into_inner();
        bad.pop();
        assert!(merge_runs("job", &mut [Cursor::new(bad)], end).is_err());
    }
    #[test]
    fn malformed_spill_keys_and_unbounded_lengths_are_rejected_before_allocation() {
        assert!(decode_entry(&mut Cursor::new(vec![0, 0])).is_err());
        let mut bytes = vec![0, 1, 2];
        bytes.extend_from_slice(&u64::MAX.to_be_bytes());
        assert!(decode_entry(&mut Cursor::new(bytes)).is_err());
        assert!(decode_entry(&mut Cursor::new(vec![0, 1, 3])).is_err());
    }
    #[test]
    fn empty_tables_are_hashed_and_table_identity_is_not_interchangeable() {
        let end = Instant::now() + Duration::from_secs(5);
        let a = merge_runs("app_user", &mut [] as &mut [Cursor<Vec<u8>>], end).unwrap();
        let b = merge_runs("job", &mut [] as &mut [Cursor<Vec<u8>>], end).unwrap();
        assert_eq!(a.rows, 0);
        assert!(!a.same_data(&b));
    }
}

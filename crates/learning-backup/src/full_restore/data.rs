//! Deterministic primary-key ordering for the full recovery row barrier.
//! These content observations never grant import or usable authority.
use super::{SchemaContract, schema::Table};
use crate::BackupError;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::BufRead;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum KeyPart {
    Integer(i64),
    Uuid([u8; 16]),
    Text(Vec<u8>),
}

struct TableKey<'a> {
    table: &'a Table,
    indices: Vec<usize>,
}

fn table_keys(contract: &SchemaContract) -> Result<Vec<TableKey<'_>>, BackupError> {
    let bad = || BackupError::Invalid("recovery primary-key contract");
    let template = std::str::from_utf8(&contract.template).map_err(|_| bad())?;
    let lines: Vec<_> = template.lines().collect();
    let mut primary_keys = BTreeMap::new();
    for pair in lines.windows(2) {
        let Some(table) = pair[0].strip_prefix("ALTER TABLE ONLY public.") else {
            continue;
        };
        let constraint = pair[1].trim();
        if !constraint.starts_with("ADD CONSTRAINT ") {
            continue;
        }
        let Some((_, columns)) = constraint.split_once(" PRIMARY KEY (") else {
            continue;
        };
        let columns = columns.strip_suffix(");").ok_or_else(bad)?;
        if primary_keys
            .insert(table, columns.split(", ").collect::<Vec<_>>())
            .is_some()
        {
            return Err(bad());
        }
    }
    if primary_keys.len() != contract.tables.len() {
        return Err(bad());
    }
    contract
        .tables
        .iter()
        .map(|table| {
            let columns = primary_keys
                .remove(table.sql_name.as_str())
                .ok_or_else(bad)?;
            let indices = columns
                .into_iter()
                .map(|name| {
                    let index = table
                        .columns
                        .iter()
                        .position(|column| column.sql_name == name)
                        .ok_or_else(bad)?;
                    let column = &table.columns[index];
                    if !column.not_null
                        || !matches!(
                            column.ty.as_str(),
                            "uuid" | "smallint" | "integer" | "bigint" | "text"
                        )
                    {
                        return Err(bad());
                    }
                    Ok(index)
                })
                .collect::<Result<Vec<_>, _>>()?;
            if indices.is_empty() {
                return Err(bad());
            }
            Ok(TableKey { table, indices })
        })
        .collect()
}

fn row_key(key: &TableKey<'_>, row: &[u8]) -> Result<Vec<KeyPart>, BackupError> {
    let bad = || BackupError::Invalid("recovery COPY primary key");
    if row.len() as u64 > super::spool::MAX_ROW {
        return Err(BackupError::Capacity("recovery COPY row"));
    }
    let payload = row.strip_suffix(b"\n").ok_or_else(bad)?;
    if payload.contains(&b'\n') || payload.contains(&b'\r') || payload.contains(&0) {
        return Err(bad());
    }
    let fields: Vec<_> = payload.split(|byte| *byte == b'\t').collect();
    if fields.len() != key.table.columns.len() {
        return Err(bad());
    }
    key.indices
        .iter()
        .map(|index| {
            let raw = fields[*index];
            if raw == b"\\N" {
                return Err(bad());
            }
            let ty = key.table.columns[*index].ty.as_str();
            if ty == "text" {
                return Ok(KeyPart::Text(decode_text_key(raw)?));
            }
            let value = std::str::from_utf8(raw).map_err(|_| bad())?;
            if ty == "uuid" {
                let id = uuid::Uuid::parse_str(value).map_err(|_| bad())?;
                if id.to_string() != value {
                    return Err(bad());
                }
                return Ok(KeyPart::Uuid(*id.as_bytes()));
            }
            let number: i64 = value.parse().map_err(|_| bad())?;
            if number.to_string() != value
                || (ty == "smallint" && i16::try_from(number).is_err())
                || (ty == "integer" && i32::try_from(number).is_err())
            {
                return Err(bad());
            }
            Ok(KeyPart::Integer(number))
        })
        .collect()
}

fn decode_text_key(raw: &[u8]) -> Result<Vec<u8>, BackupError> {
    let bad = || BackupError::Invalid("recovery COPY text key");
    let mut value = Vec::with_capacity(raw.len());
    let mut bytes = raw.iter().copied();
    while let Some(byte) = bytes.next() {
        value.push(if byte != b'\\' {
            byte
        } else {
            match bytes.next().ok_or_else(bad)? {
                b'\\' => b'\\',
                b'b' => 8,
                b'f' => 12,
                b'n' => b'\n',
                b'r' => b'\r',
                b't' => b'\t',
                b'v' => 11,
                _ => return Err(bad()),
            }
        });
    }
    std::str::from_utf8(&value).map_err(|_| bad())?;
    Ok(value)
}

// Comparison only: imported COPY bytes are never rewritten. PG18's ISO output
// can render the same timestamptz in different offsets on source and target.
fn timestamp_micros(raw: &[u8]) -> Result<i128, BackupError> {
    let bad = || BackupError::Invalid("recovery timestamp text");
    let text = std::str::from_utf8(raw).map_err(|_| bad())?;
    let (text, bc) = text
        .strip_suffix(" BC")
        .map_or((text, false), |s| (s, true));
    let (date, clock) = text.split_once(' ').ok_or_else(bad)?;
    let number = |value: &str| -> Result<i128, BackupError> {
        if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
            return Err(bad());
        }
        // PG's native year/offset fields fit i64; i128 arithmetic below avoids
        // narrowing the PostgreSQL timestamp range to a date-library range.
        Ok(value.parse::<i64>().map_err(|_| bad())?.into())
    };
    let parts: Vec<_> = date.split('-').collect();
    if parts.len() != 3 {
        return Err(bad());
    }
    let y = number(parts[0])?;
    if y == 0 {
        return Err(bad());
    }
    let year = if bc { 1 - y } else { y };
    let month = number(parts[1])?;
    let day = number(parts[2])?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        _ => return Err(bad()),
    };
    if day < 1 || day > max_day {
        return Err(bad());
    }
    let at = clock.rfind(['+', '-']).ok_or_else(bad)?;
    let (clock, zone) = clock.split_at(at);
    let negative = zone.starts_with('-');
    let zone: Vec<_> = zone[1..].split(':').collect();
    if zone.is_empty() || zone.len() > 3 {
        return Err(bad());
    }
    let offset_hour = number(zone[0])?;
    let offset_minute = if zone.len() > 1 { number(zone[1])? } else { 0 };
    let offset_second = if zone.len() > 2 { number(zone[2])? } else { 0 };
    if offset_minute > 59 || offset_second > 59 {
        return Err(bad());
    }
    let offset =
        (offset_hour * 3600 + offset_minute * 60 + offset_second) * if negative { -1 } else { 1 };
    let clock: Vec<_> = clock.split(':').collect();
    if clock.len() != 3 {
        return Err(bad());
    }
    let hour = number(clock[0])?;
    let minute = number(clock[1])?;
    let (second, fraction) = clock[2]
        .split_once('.')
        .map_or((clock[2], None), |(s, f)| (s, Some(f)));
    let second = number(second)?;
    if hour > 23 || minute > 59 || second > 59 {
        return Err(bad());
    }
    let micros = if let Some(fraction) = fraction {
        if fraction.is_empty() || fraction.len() > 6 {
            return Err(bad());
        }
        number(fraction)? * 10_i128.pow(6 - fraction.len() as u32)
    } else {
        0
    };
    // Proleptic Gregorian civil date to Unix days. Euclidean division also
    // covers PostgreSQL's BC dates (astronomical year zero is 1 BC).
    let y = year - i128::from(month <= 2);
    let era = y.div_euclid(400);
    let year_in_era = y - era * 400;
    let month = month + if month > 2 { -3 } else { 9 };
    let day_in_year = (153 * month + 2) / 5 + day - 1;
    let day_in_era = year_in_era * 365 + year_in_era / 4 - year_in_era / 100 + day_in_year;
    let days = era * 146_097 + day_in_era - 719_468;
    Ok((days * 86_400 + hour * 3600 + minute * 60 + second - offset) * 1_000_000 + micros)
}

fn row_digest(table: &Table, row: &[u8]) -> Result<[u8; 32], BackupError> {
    let bad = || BackupError::Invalid("recovery row observation");
    if row.len() as u64 > super::spool::MAX_ROW {
        return Err(BackupError::Capacity("recovery COPY row"));
    }
    let payload = row.strip_suffix(b"\n").ok_or_else(bad)?;
    if payload.contains(&b'\n') || payload.contains(&b'\r') || payload.contains(&0) {
        return Err(bad());
    }
    std::str::from_utf8(payload).map_err(|_| bad())?;
    let fields: Vec<_> = payload.split(|byte| *byte == b'\t').collect();
    if fields.len() != table.columns.len() {
        return Err(bad());
    }
    let mut hash = Sha256::new();
    hash.update(b"knowweave-recovery-row-v1\0");
    hash.update((table.table.len() as u64).to_be_bytes());
    hash.update(table.table.as_bytes());
    hash.update((fields.len() as u64).to_be_bytes());
    for (column, raw) in table.columns.iter().zip(fields) {
        hash.update((column.ty.len() as u64).to_be_bytes());
        hash.update(column.ty.as_bytes());
        if raw == b"\\N" {
            if column.not_null {
                return Err(bad());
            }
            hash.update([0]);
        } else if column.ty == "timestamp with time zone" {
            match raw {
                b"infinity" => hash.update([3, 0]),
                b"-infinity" => hash.update([3, 1]),
                _ => {
                    hash.update([2]);
                    hash.update(timestamp_micros(raw)?.to_be_bytes());
                }
            }
        } else {
            hash.update([1]);
            hash.update((raw.len() as u64).to_be_bytes());
            hash.update(raw);
        }
    }
    Ok(hash.finalize().into())
}

#[derive(Debug)]
struct TableObservation {
    table: String,
    rows: u64,
    raw_bytes: u64,
    digest: [u8; 32],
}
impl TableObservation {
    fn same_data(&self, other: &Self) -> bool {
        // Raw size is an I/O budget observation, not semantic row equality:
        // the ISO text offset alone can change its length without changing a
        // timestamptz. No column or timestamp precision is excluded from hash.
        self.table == other.table && self.rows == other.rows && self.digest == other.digest
    }
}

#[derive(Default)]
struct DataBudget {
    used: u64,
}

fn observe_sorted(
    key: &TableKey<'_>,
    reader: &mut impl BufRead,
    budget: &mut DataBudget,
) -> Result<TableObservation, BackupError> {
    let mut hash = Sha256::new();
    hash.update(b"knowweave-recovery-table-v1\0");
    hash.update((key.table.table.len() as u64).to_be_bytes());
    hash.update(key.table.table.as_bytes());
    let mut previous = None;
    let mut rows = 0_u64;
    let mut raw_bytes = 0_u64;
    while let Some(row) = read_row(reader, budget)? {
        let current = row_key(key, &row)?;
        if previous.as_ref().is_some_and(|old| old >= &current) {
            return Err(BackupError::Invalid("recovery primary-key order"));
        }
        hash.update(row_digest(key.table, &row)?);
        rows = rows.checked_add(1).ok_or(BackupError::Overflow)?;
        raw_bytes = raw_bytes
            .checked_add(row.len() as u64)
            .ok_or(BackupError::Overflow)?;
        previous = Some(current);
    }
    hash.update(rows.to_be_bytes());
    Ok(TableObservation {
        table: key.table.table.clone(),
        rows,
        raw_bytes,
        digest: hash.finalize().into(),
    })
}

fn read_row(
    reader: &mut impl BufRead,
    budget: &mut DataBudget,
) -> Result<Option<Vec<u8>>, BackupError> {
    let mut row = Vec::new();
    loop {
        let bytes = reader.fill_buf()?;
        if bytes.is_empty() {
            return if row.is_empty() {
                Ok(None)
            } else {
                Err(BackupError::Invalid("recovery COPY truncated row"))
            };
        }
        let newline = bytes.iter().position(|b| *b == b'\n');
        let length = newline.map_or(bytes.len(), |p| p + 1);
        let next_row_size = row.len().checked_add(length).ok_or(BackupError::Overflow)?;
        if next_row_size as u64 > super::spool::MAX_ROW {
            return Err(BackupError::Capacity("recovery COPY row"));
        }
        let next_total = budget
            .used
            .checked_add(length as u64)
            .ok_or(BackupError::Overflow)?;
        if next_total > super::spool::MAX_DECODE {
            return Err(BackupError::Capacity("recovery COPY total"));
        }
        budget.used = next_total;
        row.extend_from_slice(&bytes[..length]);
        reader.consume(length);
        if newline.is_some() {
            return Ok(Some(row));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_inventory_has_all_64_tables_including_migrations() {
        let contract = SchemaContract::embedded().unwrap();
        let keys = table_keys(&contract).unwrap();
        assert_eq!(keys.len(), 64);
        assert!(keys.iter().any(|k| k.table.table == "_sqlx_migrations"));
        assert!(keys.iter().all(|k| !k.indices.is_empty()));
    }

    #[test]
    fn migration_key_orders_signed_integers_numerically() {
        let contract = SchemaContract::embedded().unwrap();
        let keys = table_keys(&contract).unwrap();
        let key = keys
            .iter()
            .find(|k| k.table.table == "_sqlx_migrations")
            .unwrap();
        let row = |n| format!("{n}\tdesc\t2026-10-10 00:00:00+00\tt\t\\\\x00\t1\n");
        assert!(
            row_key(key, row(-1).as_bytes()).unwrap() < row_key(key, row(2).as_bytes()).unwrap()
        );
        assert!(
            row_key(key, row(2).as_bytes()).unwrap() < row_key(key, row(10).as_bytes()).unwrap()
        );
    }

    #[test]
    fn composite_uuid_key_keeps_each_identity_component() {
        let contract = SchemaContract::embedded().unwrap();
        let keys = table_keys(&contract).unwrap();
        let key = keys
            .iter()
            .find(|k| k.table.table == "space_grant")
            .unwrap();
        let a = b"00000000-0000-0000-0000-000000000001\t00000000-0000-0000-0000-000000000002\tt\n";
        let b = b"00000000-0000-0000-0000-000000000001\t00000000-0000-0000-0000-000000000003\tt\n";
        assert!(row_key(key, a).unwrap() < row_key(key, b).unwrap());
    }

    #[test]
    fn malformed_null_or_noncanonical_keys_are_rejected() {
        let contract = SchemaContract::embedded().unwrap();
        let keys = table_keys(&contract).unwrap();
        let key = keys.iter().find(|k| k.table.table == "app_user").unwrap();
        for row in [
            b"\\N\n".as_slice(),
            b"bad-uuid\n",
            b"00000000000000000000000000000001\n",
            b"00000000-0000-0000-0000-000000000001\textra\n",
            b"00000000-0000-0000-0000-000000000001",
        ] {
            assert!(row_key(key, row).is_err());
        }
    }

    #[test]
    fn timestamp_comparison_preserves_instants_across_timezone_offsets() {
        assert_eq!(timestamp_micros(b"1970-01-01 00:00:00+00").unwrap(), 0);
        assert_eq!(timestamp_micros(b"1970-01-01 08:00:00+08").unwrap(), 0);
        assert_eq!(timestamp_micros(b"1969-12-31 19:00:00-05").unwrap(), 0);
        assert_eq!(timestamp_micros(b"1970-01-01 00:30:00+00:30").unwrap(), 0);
        assert_eq!(
            timestamp_micros(b"1970-01-01 00:00:01+00:00:01").unwrap(),
            0
        );
    }

    #[test]
    fn timestamp_fraction_and_leap_dates_do_not_lose_precision() {
        assert_eq!(
            timestamp_micros(b"1970-01-01 00:00:00.000001+00").unwrap(),
            1
        );
        assert_eq!(
            timestamp_micros(b"1970-01-01 00:00:00.1+00").unwrap(),
            100_000
        );
        let a = timestamp_micros(b"2000-02-28 00:00:00+00").unwrap();
        let b = timestamp_micros(b"2000-03-01 00:00:00+00").unwrap();
        assert_eq!(b - a, 2 * 86_400_000_000);
        let a = timestamp_micros(b"1900-02-28 00:00:00+00").unwrap();
        let b = timestamp_micros(b"1900-03-01 00:00:00+00").unwrap();
        assert_eq!(b - a, 86_400_000_000);
    }

    #[test]
    fn timestamp_bc_and_extended_years_keep_the_full_pg_range() {
        let a = timestamp_micros(b"0001-01-01 00:00:00+00 BC").unwrap();
        let b = timestamp_micros(b"0001-01-01 00:00:00+00").unwrap();
        assert_eq!(b - a, 366 * 86_400_000_000);
        assert!(timestamp_micros(b"294276-01-01 00:00:00+00").unwrap() > 0);
        assert!(timestamp_micros(b"4713-01-01 00:00:00+00 BC").unwrap() < a);
    }

    #[test]
    fn timestamp_missing_zone_invalid_calendar_or_precision_is_rejected() {
        for raw in [
            b"2026-02-29 00:00:00+00".as_slice(),
            b"2026-01-01 00:00:00",
            b"2026-01-01 00:00:00.1234567+00",
            b"2026-01-01 00:00:00+00:60",
            b"0000-01-01 00:00:00+00",
            b"2026-01-01 24:00:00+00",
        ] {
            assert!(timestamp_micros(raw).is_err());
        }
    }

    #[test]
    fn text_keys_compare_decoded_bytes_without_rewriting_row_data() {
        assert_eq!(decode_text_key(b"a\\tb\\nc\\\\d").unwrap(), b"a\tb\nc\\d");
        assert_eq!(decode_text_key(b"\\\\N").unwrap(), b"\\N");
        for raw in [b"\\N".as_slice(), b"a\\", b"a\\x", &[255]] {
            assert!(decode_text_key(raw).is_err());
        }
    }

    #[test]
    fn all_non_key_columns_participate_in_row_comparison() {
        let contract = SchemaContract::embedded().unwrap();
        let table = contract
            .tables
            .iter()
            .find(|t| t.table == "space_grant")
            .unwrap();
        let a = b"00000000-0000-0000-0000-000000000001\t00000000-0000-0000-0000-000000000002\tt\n";
        let b = b"00000000-0000-0000-0000-000000000001\t00000000-0000-0000-0000-000000000002\tf\n";
        assert_ne!(row_digest(table, a).unwrap(), row_digest(table, b).unwrap());
    }

    #[test]
    fn equivalent_timestamp_renderings_hash_as_the_same_database_value() {
        let contract = SchemaContract::embedded().unwrap();
        let table = contract
            .tables
            .iter()
            .find(|t| t.table == "_sqlx_migrations")
            .unwrap();
        let row = |time| format!("1\tmy\\tdata\\n\\\\connect other\t{time}\tt\t\\\\x00\t1\n");
        assert_eq!(
            row_digest(table, row("1970-01-01 00:00:00+00").as_bytes()).unwrap(),
            row_digest(table, row("1970-01-01 08:00:00+08").as_bytes()).unwrap()
        );
        assert_ne!(
            row_digest(table, row("1970-01-01 00:00:00+00").as_bytes()).unwrap(),
            row_digest(table, row("1970-01-01 00:00:00.000001+00").as_bytes()).unwrap()
        );
    }

    #[test]
    fn row_comparison_rejects_null_required_fields_or_invalid_frames() {
        let contract = SchemaContract::embedded().unwrap();
        let table = contract
            .tables
            .iter()
            .find(|t| t.table == "app_user")
            .unwrap();
        assert!(row_digest(table, b"\\N\n").is_err());
        assert!(row_digest(table, b"00000000-0000-0000-0000-000000000001\textra\n").is_err());
        assert!(row_digest(table, b"00000000-0000-0000-0000-000000000001\n\n").is_err());
    }

    #[test]
    fn timestamp_infinities_remain_distinct_and_valid() {
        let contract = SchemaContract::embedded().unwrap();
        let table = contract
            .tables
            .iter()
            .find(|t| t.table == "_sqlx_migrations")
            .unwrap();
        let row = |time| format!("1\tdesc\t{time}\tt\t\\\\x00\t1\n");
        assert_ne!(
            row_digest(table, row("infinity").as_bytes()).unwrap(),
            row_digest(table, row("-infinity").as_bytes()).unwrap()
        );
    }

    #[test]
    fn sorted_stream_records_every_row_and_empty_tables() {
        let contract = SchemaContract::embedded().unwrap();
        let keys = table_keys(&contract).unwrap();
        let key = keys.iter().find(|k| k.table.table == "app_user").unwrap();
        let mut budget = DataBudget::default();
        let empty = observe_sorted(key, &mut std::io::Cursor::new(b""), &mut budget).unwrap();
        let bytes = b"00000000-0000-0000-0000-000000000001\n00000000-0000-0000-0000-000000000002\n";
        let full = observe_sorted(key, &mut std::io::Cursor::new(bytes), &mut budget).unwrap();
        assert_eq!(empty.rows, 0);
        assert_eq!(full.rows, 2);
        assert_eq!(full.raw_bytes, bytes.len() as u64);
        assert_eq!(budget.used, bytes.len() as u64);
        assert_ne!(full.digest, empty.digest);
    }

    #[test]
    fn sorted_stream_rejects_duplicate_or_reversed_primary_keys() {
        let contract = SchemaContract::embedded().unwrap();
        let keys = table_keys(&contract).unwrap();
        let key = keys.iter().find(|k| k.table.table == "app_user").unwrap();
        for bytes in [
            b"00000000-0000-0000-0000-000000000002\n00000000-0000-0000-0000-000000000001\n"
                .as_slice(),
            b"00000000-0000-0000-0000-000000000001\n00000000-0000-0000-0000-000000000001\n",
        ] {
            assert!(matches!(
                observe_sorted(
                    key,
                    &mut std::io::Cursor::new(bytes),
                    &mut DataBudget::default()
                ),
                Err(BackupError::Invalid("recovery primary-key order"))
            ));
        }
        assert!(matches!(
            observe_sorted(
                key,
                &mut std::io::Cursor::new(b"00000000-0000-0000-0000-000000000001"),
                &mut DataBudget::default()
            ),
            Err(BackupError::Invalid("recovery COPY truncated row"))
        ));
    }

    #[test]
    fn sorted_stream_budget_is_shared_and_never_returns_partial_inventory() {
        let contract = SchemaContract::embedded().unwrap();
        let keys = table_keys(&contract).unwrap();
        let key = keys.iter().find(|k| k.table.table == "app_user").unwrap();
        let mut budget = DataBudget {
            used: super::super::spool::MAX_DECODE - 1,
        };
        assert!(matches!(
            observe_sorted(
                key,
                &mut std::io::Cursor::new(b"00000000-0000-0000-0000-000000000001\n"),
                &mut budget
            ),
            Err(BackupError::Capacity("recovery COPY total"))
        ));
    }

    #[test]
    fn sorted_stream_compares_timestamp_values_without_length_exclusions() {
        let contract = SchemaContract::embedded().unwrap();
        let keys = table_keys(&contract).unwrap();
        let key = keys
            .iter()
            .find(|k| k.table.table == "_sqlx_migrations")
            .unwrap();
        let row = |time| format!("1\tdesc\t{time}\tt\t\\\\x00\t1\n");
        let a = observe_sorted(
            key,
            &mut std::io::Cursor::new(row("1970-01-01 00:00:00+00")),
            &mut DataBudget::default(),
        )
        .unwrap();
        let b = observe_sorted(
            key,
            &mut std::io::Cursor::new(row("1970-01-01 00:30:00+00:30")),
            &mut DataBudget::default(),
        )
        .unwrap();
        assert_eq!(a.rows, b.rows);
        assert_eq!(a.digest, b.digest);
        assert_ne!(a.raw_bytes, b.raw_bytes);
        assert!(a.same_data(&b));
    }
}

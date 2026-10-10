use crate::BackupError;
use std::io::BufRead;

/// The scanner consumes bytes without changing or accumulating ordinary rows.
/// Only the fifteen small SQLx history rows are retained for identity checks.
pub(super) fn scan_payload(
    reader: &mut impl BufRead,
    columns: usize,
    migrations: bool,
) -> Result<(u64, Vec<Vec<u8>>), BackupError> {
    let bad = || BackupError::Invalid("COPY text frame");
    if columns == 0 {
        return Err(bad());
    }
    let mut total = 0u64;
    let mut row = 0u64;
    let mut fields = 1;
    let mut field_len = 0u64;
    let mut escaped = false;
    let mut null = false;
    let mut prefix = [0u8; 2];
    let mut utf = Utf8::default();
    let mut saved = Vec::new();
    let mut rows = Vec::new();
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Err(bad());
        }
        let mut used = 0;
        for &b in buffer {
            used += 1;
            total = total.checked_add(1).ok_or(BackupError::Overflow)?;
            row += 1;
            if row > super::spool::MAX_ROW {
                return Err(BackupError::Capacity("COPY row"));
            }
            if row <= 2 {
                prefix[row as usize - 1] = b;
            }
            if migrations {
                if saved.len() >= 8192 {
                    return Err(BackupError::Capacity("migration row"));
                }
                saved.push(b);
            }
            if b == b'\n' {
                if row == 3 && prefix == *b"\\." {
                    reader.consume(used);
                    return Ok((total, rows));
                }
                if escaped || fields != columns || utf.remaining != 0 {
                    return Err(bad());
                }
                if migrations {
                    if rows.len() >= 15 {
                        return Err(BackupError::Invalid("migration row count"));
                    }
                    rows.push(std::mem::take(&mut saved));
                }
                row = 0;
                fields = 1;
                field_len = 0;
                escaped = false;
                null = false;
                prefix = [0; 2];
                continue;
            }
            if b == 0 || b == b'\r' {
                return Err(bad());
            }
            // A raw terminator prefix cannot become a data field by appending
            // a tab. Literal backslash-dot data must have an escaped slash.
            if row > 2 && prefix == *b"\\." {
                return Err(bad());
            }
            utf.push(b)?;
            if b == b'\t' {
                if escaped || utf.remaining != 0 {
                    return Err(bad());
                }
                fields += 1;
                if fields > columns {
                    return Err(bad());
                }
                field_len = 0;
                null = false;
                continue;
            }
            if null {
                return Err(bad());
            }
            if escaped {
                match b {
                    b'\\' | b'b' | b'f' | b'n' | b'r' | b't' | b'v' => {}
                    b'N' if field_len == 1 => null = true,
                    b'.' if row == 2 => {} // accepted ONLY if the next byte ends this line
                    _ => return Err(bad()),
                }
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            }
            field_len += 1;
        }
        reader.consume(used);
    }
}

#[derive(Default)]
struct Utf8 {
    remaining: u8,
    value: u32,
    minimum: u32,
}
impl Utf8 {
    fn push(&mut self, b: u8) -> Result<(), BackupError> {
        let bad = || BackupError::Invalid("COPY UTF-8");
        if self.remaining == 0 {
            match b {
                0..=0x7f => {}
                0xc2..=0xdf => {
                    self.remaining = 1;
                    self.value = (b & 31) as u32;
                    self.minimum = 0x80;
                }
                0xe0..=0xef => {
                    self.remaining = 2;
                    self.value = (b & 15) as u32;
                    self.minimum = 0x800;
                }
                0xf0..=0xf4 => {
                    self.remaining = 3;
                    self.value = (b & 7) as u32;
                    self.minimum = 0x10000;
                }
                _ => return Err(bad()),
            }
        } else {
            if !(0x80..=0xbf).contains(&b) {
                return Err(bad());
            }
            self.value = (self.value << 6) | (b & 63) as u32;
            self.remaining -= 1;
            if self.remaining == 0
                && (self.value < self.minimum
                    || self.value > 0x10ffff
                    || (0xd800..=0xdfff).contains(&self.value))
            {
                return Err(bad());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};
    fn payload(reader: &mut impl BufRead, columns: usize) -> Result<u64, BackupError> {
        Ok(scan_payload(reader, columns, false)?.0)
    }
    #[test]
    fn full_copy_preserves_unicode_null_backslash_newline_and_sql_looking_data() {
        let bytes="中文\t\\N\t\\\\path\\nline\\twith tab; DROP TABLE x; \\\\connect other\n\\.\nCOMMIT;\n".as_bytes();
        let mut input = BufReader::with_capacity(3, Cursor::new(bytes));
        let end = payload(&mut input, 3).unwrap();
        assert_eq!(&bytes[end as usize..], b"COMMIT;\n");
    }
    #[test]
    fn rejects_copy_command_positions_and_malformed_frames() {
        for raw in [
            b"a\tb\n\\.\n".as_slice(),
            b"\\connect other\n\\.\n",
            b"a\n\\. extra\n",
            b"a\n",
            b"a\0b\n\\.\n",
            b"\\Ntail\n\\.\n",
            b"a\r\n\\.\n",
        ] {
            assert!(payload(&mut Cursor::new(raw), 1).is_err(), "{raw:?}");
        }
        assert!(payload(&mut Cursor::new(b"\\.\t\n\\.\n"), 2).is_err());
    }
    #[test]
    fn copy_row_64mib_boundary_streams_without_row_allocation() {
        use std::io::{Read, repeat};
        let row = repeat(b'a')
            .take(super::super::spool::MAX_ROW - 1)
            .chain(Cursor::new(b"\n\\.\n"));
        assert_eq!(
            payload(&mut BufReader::with_capacity(65536, row), 1).unwrap(),
            super::super::spool::MAX_ROW + 3
        );
        let row = repeat(b'a')
            .take(super::super::spool::MAX_ROW)
            .chain(Cursor::new(b"\n\\.\n"));
        assert!(matches!(
            payload(&mut BufReader::with_capacity(65536, row), 1),
            Err(BackupError::Capacity("COPY row"))
        ));
    }
}

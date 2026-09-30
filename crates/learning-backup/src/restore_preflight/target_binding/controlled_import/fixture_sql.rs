//! Audited PG18 synthetic fixture bytes; private to the test-only import model.
use super::ImportFailure;
use sha2::{Digest, Sha256};
use std::io::Read;

const MAX_FIXTURE_BYTES: usize = 65_536;
const TEMPLATE: &[u8] =
    include_bytes!("../../../../tests/fixtures/c4-controlled-import/pg18-fixture.sql.in");
const KEY_TOKEN: &[u8] = b"{{RESTRICT_KEY}}";
// The audited source split is 670 with a 63-byte captured key. The token is
// 16 bytes, so the equivalent position in the checked-in template is 623.
const TEMPLATE_HEADER_END: usize = 623;

#[allow(dead_code)]
pub(super) struct FrozenDump {
    bytes: Box<[u8]>,
    sha256: [u8; 32],
}

#[allow(dead_code)]
impl FrozenDump {
    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(super) fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
}

#[allow(dead_code)]
pub(super) struct VerifiedFixtureSql {
    header: Box<[u8]>,
    payload: Box<[u8]>,
    raw_sha256: [u8; 32],
    transformed_sha256: [u8; 32],
}

#[allow(dead_code)]
impl VerifiedFixtureSql {
    pub(super) fn header(&self) -> &[u8] {
        &self.header
    }

    pub(super) fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub(super) fn raw_sha256(&self) -> [u8; 32] {
        self.raw_sha256
    }

    pub(super) fn transformed_sha256(&self) -> [u8; 32] {
        self.transformed_sha256
    }
}

pub(super) fn freeze_dump(
    mut reader: impl Read,
    expected_len: u64,
    expected_sha256: [u8; 32],
) -> Result<FrozenDump, ImportFailure> {
    if expected_len > MAX_FIXTURE_BYTES as u64 {
        return Err(ImportFailure::InputLimit);
    }
    let mut bytes = Vec::with_capacity(expected_len as usize);
    let mut buffer = [0u8; 8192];
    loop {
        let remaining = MAX_FIXTURE_BYTES + 1 - bytes.len();
        let read_limit = remaining.min(buffer.len());
        let read = reader
            .read(&mut buffer[..read_limit])
            .map_err(|_| ImportFailure::Io)?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.len() > MAX_FIXTURE_BYTES {
            return Err(ImportFailure::InputLimit);
        }
    }
    if bytes.len() as u64 != expected_len
        || !bytes.starts_with(b"PGDMP")
        || Sha256::digest(&bytes).as_slice() != expected_sha256
    {
        return Err(ImportFailure::Fixture);
    }
    Ok(FrozenDump {
        bytes: bytes.into_boxed_slice(),
        sha256: expected_sha256,
    })
}

pub(super) fn verify_fixture_sql(decoded: &[u8]) -> Result<VerifiedFixtureSql, ImportFailure> {
    if decoded.len() > MAX_FIXTURE_BYTES {
        return Err(ImportFailure::InputLimit);
    }
    let token_positions: Vec<usize> = TEMPLATE
        .windows(KEY_TOKEN.len())
        .enumerate()
        .filter_map(|(index, window)| (window == KEY_TOKEN).then_some(index))
        .collect();
    let [first, second] = token_positions.as_slice() else {
        return Err(ImportFailure::Fixture);
    };
    let prefix = &TEMPLATE[..*first];
    let middle = &TEMPLATE[first + KEY_TOKEN.len()..*second];
    let suffix = &TEMPLATE[second + KEY_TOKEN.len()..];
    let remaining = decoded.strip_prefix(prefix).ok_or(ImportFailure::Fixture)?;
    let key_len = remaining
        .iter()
        .take_while(|byte| byte.is_ascii_alphanumeric())
        .count();
    if !(1..=128).contains(&key_len) {
        return Err(ImportFailure::Fixture);
    }
    let (key, remaining) = remaining.split_at(key_len);
    let remaining = remaining
        .strip_prefix(middle)
        .ok_or(ImportFailure::Fixture)?;
    let remaining = remaining.strip_prefix(key).ok_or(ImportFailure::Fixture)?;
    if remaining != suffix {
        return Err(ImportFailure::Fixture);
    }

    let header_end = TEMPLATE_HEADER_END - KEY_TOKEN.len() + key_len;
    let (raw_header, payload) = decoded.split_at(header_end);
    let replacements: [(&[u8], &[u8]); 4] = [
        (
            b"SET statement_timeout = 0;\n",
            b"SET LOCAL statement_timeout = '10000ms';\n",
        ),
        (
            b"SET lock_timeout = 0;\n",
            b"SET LOCAL lock_timeout = '5000ms';\n",
        ),
        (
            b"SET idle_in_transaction_session_timeout = 0;\n",
            b"SET LOCAL idle_in_transaction_session_timeout = '30000ms';\n",
        ),
        (
            b"SET transaction_timeout = 0;\n",
            b"SET LOCAL transaction_timeout = '60000ms';\n",
        ),
    ];
    let mut header = raw_header.to_vec();
    for (original, replacement) in replacements {
        let matches: Vec<usize> = header
            .windows(original.len())
            .enumerate()
            .filter_map(|(index, window)| (window == original).then_some(index))
            .collect();
        let [offset] = matches.as_slice() else {
            return Err(ImportFailure::Fixture);
        };
        header.splice(
            *offset..offset + original.len(),
            replacement.iter().copied(),
        );
    }
    let raw_sha256 = Sha256::digest(decoded).into();
    let mut transformed = header.clone();
    transformed.extend_from_slice(payload);
    let transformed_sha256 = Sha256::digest(&transformed).into();
    Ok(VerifiedFixtureSql {
        header: header.into_boxed_slice(),
        payload: payload.to_vec().into_boxed_slice(),
        raw_sha256,
        transformed_sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::fs::OpenOptions;
    use std::io::{Cursor, Error, Seek, SeekFrom, Write};

    const CAPTURE_KEY: &[u8] = b"XUiXOMjlMScOdk9ZzeQOcY8XFfEhfRYfmX6zxd5b98iPfVsmzTgJE32K1SuOOui";

    fn digest(bytes: &[u8]) -> [u8; 32] {
        Sha256::digest(bytes).into()
    }

    fn captured(key: &[u8]) -> Vec<u8> {
        let mut sql = Vec::new();
        let first = TEMPLATE
            .windows(KEY_TOKEN.len())
            .position(|part| part == KEY_TOKEN)
            .unwrap();
        let second = TEMPLATE[first + KEY_TOKEN.len()..]
            .windows(KEY_TOKEN.len())
            .position(|part| part == KEY_TOKEN)
            .unwrap()
            + first
            + KEY_TOKEN.len();
        sql.extend_from_slice(&TEMPLATE[..first]);
        sql.extend_from_slice(key);
        sql.extend_from_slice(&TEMPLATE[first + KEY_TOKEN.len()..second]);
        sql.extend_from_slice(key);
        sql.extend_from_slice(&TEMPLATE[second + KEY_TOKEN.len()..]);
        sql
    }

    #[test]
    fn snapshot_stays_fixed_after_same_inode_rewrite() {
        let original = b"PGDMPoriginal-synthetic-dump";
        let path = std::env::temp_dir().join(format!("p0c4-snapshot-{}", uuid::Uuid::new_v4()));
        let mut file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        file.write_all(original).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        let frozen = freeze_dump(&mut file, original.len() as u64, digest(original)).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(b"PGDMPchanged!-synthetic-dump").unwrap();
        file.sync_all().unwrap();
        assert_eq!(frozen.bytes(), original);
        assert_eq!(frozen.sha256(), digest(original));
        drop(file);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn fixture_budget_accepts_65536_rejects_65537() {
        let mut allowed = vec![b'a'; 65_536];
        allowed[..5].copy_from_slice(b"PGDMP");
        assert_eq!(
            freeze_dump(Cursor::new(&allowed), 65_536, digest(&allowed))
                .unwrap()
                .bytes()
                .len(),
            65_536
        );
        allowed.push(b'a');
        assert!(matches!(
            freeze_dump(Cursor::new(&allowed), 65_537, digest(&allowed)),
            Err(ImportFailure::InputLimit)
        ));
        assert!(matches!(
            freeze_dump(Cursor::new(&allowed), 65_536, digest(&allowed)),
            Err(ImportFailure::InputLimit)
        ));
    }

    #[test]
    fn snapshot_rejects_short_invalid_and_io_reader() {
        let bytes = b"PGDMPsmall";
        assert!(matches!(
            freeze_dump(Cursor::new(bytes), 11, digest(bytes)),
            Err(ImportFailure::Fixture)
        ));
        assert!(matches!(
            freeze_dump(Cursor::new(b"WRONGsmall"), 10, digest(b"WRONGsmall")),
            Err(ImportFailure::Fixture)
        ));
        assert!(matches!(
            freeze_dump(Cursor::new(bytes), 10, [0; 32]),
            Err(ImportFailure::Fixture)
        ));
        struct FailingReader;
        impl Read for FailingReader {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                Err(Error::other("redacted test error"))
            }
        }
        assert!(matches!(
            freeze_dump(FailingReader, 10, digest(bytes)),
            Err(ImportFailure::Io)
        ));
    }

    #[test]
    fn audited_golden_accepts_actual_and_key_length_bounds() {
        let original = captured(CAPTURE_KEY);
        assert_eq!(original.len(), 1308);
        assert_eq!(
            hex::encode(digest(&original)),
            "e2c252bfa5d44c5ed9133dcdb7fd8344e0a2471489d0ee410edb6c09baabd44a"
        );
        let verified = verify_fixture_sql(&original).unwrap();
        assert_eq!(verified.raw_sha256(), digest(&original));
        assert_eq!(verified.payload(), &original[670..]);
        for key in [vec![b'A'], vec![b'z'; 128]] {
            let sql = captured(&key);
            let verified = verify_fixture_sql(&sql).unwrap();
            assert_eq!(verified.payload(), &sql[670 + key.len() - 63..]);
        }
        for key in [vec![], vec![b'B'; 129], vec![b'_'], vec![0xff]] {
            assert!(matches!(
                verify_fixture_sql(&captured(&key)),
                Err(ImportFailure::Fixture)
            ));
        }
    }

    #[test]
    fn golden_rejects_key_mismatch_multiple_markers_and_non_key_changes() {
        let original = captured(CAPTURE_KEY);
        let mut mismatch = original.clone();
        mismatch[1243] = b'Z';
        assert!(matches!(
            verify_fixture_sql(&mismatch),
            Err(ImportFailure::Fixture)
        ));
        for needle in [
            b"-- PostgreSQL".as_slice(),
            b"1\talpha".as_slice(),
            b"-- Name:".as_slice(),
        ] {
            let mut changed = original.clone();
            let offset = changed
                .windows(needle.len())
                .position(|part| part == needle)
                .unwrap();
            changed[offset] = b'!';
            assert!(matches!(
                verify_fixture_sql(&changed),
                Err(ImportFailure::Fixture)
            ));
        }
        let mut duplicated = original.clone();
        duplicated.extend_from_slice(b"\\restrict extra\n");
        assert!(matches!(
            verify_fixture_sql(&duplicated),
            Err(ImportFailure::Fixture)
        ));
        assert!(matches!(
            verify_fixture_sql(&original[..original.len() - 1]),
            Err(ImportFailure::Fixture)
        ));
        assert!(matches!(
            verify_fixture_sql(&[original.as_slice(), b"\n"].concat()),
            Err(ImportFailure::Fixture)
        ));
        assert!(matches!(
            verify_fixture_sql(&original.replace_lf_with_crlf()),
            Err(ImportFailure::Fixture)
        ));
    }

    #[test]
    fn golden_rejects_transaction_reconnect_lo_and_early_unrestrict() {
        let original = captured(CAPTURE_KEY);
        for extra in [
            b"COMMIT;\n".as_slice(),
            b"\\connect other\n".as_slice(),
            b"SELECT lo_create(1);\n".as_slice(),
            b"\\unrestrict early\n".as_slice(),
        ] {
            let mut changed = original.clone();
            changed.splice(741..741, extra.iter().copied());
            assert!(matches!(
                verify_fixture_sql(&changed),
                Err(ImportFailure::Fixture)
            ));
        }
    }

    #[test]
    fn header_is_ddl_free_and_timeouts_are_local() {
        let original = captured(CAPTURE_KEY);
        let verified = verify_fixture_sql(&original).unwrap();
        let header = std::str::from_utf8(verified.header()).unwrap();
        let payload = std::str::from_utf8(verified.payload()).unwrap();
        assert!(!header.contains("CREATE TABLE"));
        assert!(!header.contains("COPY public"));
        assert!(header.ends_with("SET default_table_access_method = heap;\n\n"));
        assert!(payload.starts_with("--\n-- Name: c4_import_probe; Type: TABLE"));
        assert!(payload.contains("CREATE TABLE public.c4_import_probe"));
        assert!(payload.contains(
            "COPY public.c4_import_probe (id, label) FROM stdin;\n1\talpha\n2\tbeta\n\\.\n"
        ));
        assert!(!payload.contains("COMMIT"));
        for expected in [
            "SET LOCAL statement_timeout = '10000ms';\n",
            "SET LOCAL lock_timeout = '5000ms';\n",
            "SET LOCAL idle_in_transaction_session_timeout = '30000ms';\n",
            "SET LOCAL transaction_timeout = '60000ms';\n",
        ] {
            assert!(header.contains(expected), "missing {expected}");
        }
        assert!(!header.contains("SET statement_timeout = 0;"));
        assert_eq!(
            header
                .matches("SELECT pg_catalog.set_config('search_path', '', false);\n")
                .count(),
            1
        );
        let transformed = [verified.header(), verified.payload()].concat();
        assert_eq!(verified.transformed_sha256(), digest(&transformed));
        assert_eq!(
            hex::encode(verified.transformed_sha256()),
            "d52fadf2e5d841b57166cd9d6f281034b3b16264b66a677b55aeaab8fcbce138"
        );
    }

    trait CrLf {
        fn replace_lf_with_crlf(&self) -> Vec<u8>;
    }
    impl CrLf for [u8] {
        fn replace_lf_with_crlf(&self) -> Vec<u8> {
            let mut result = Vec::new();
            for byte in self {
                if *byte == b'\n' {
                    result.push(b'\r');
                }
                result.push(*byte);
            }
            result
        }
    }
}

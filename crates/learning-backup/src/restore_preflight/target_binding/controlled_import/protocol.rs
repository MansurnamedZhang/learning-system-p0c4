use super::super::ChallengeKeys;
use super::ImportFailure;
use ring::rand::{SecureRandom, SystemRandom};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Nonce([u8; 16]);
impl Nonce {
    pub(super) fn random() -> Result<Self, ImportFailure> {
        let mut bytes = [0; 16];
        SystemRandom::new()
            .fill(&mut bytes)
            .map_err(|_| ImportFailure::Io)?;
        Ok(Self(bytes))
    }
    pub(super) fn hex(&self) -> String {
        hex::encode(self.0)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct WriterIdentity {
    backend_pid: i32,
    backend_start_micros: i64,
    transaction_id: u64,
}
#[derive(Debug, PartialEq, Eq)]
pub(super) enum WriterEvent {
    Ready(WriterIdentity),
    Precommit(WriterIdentity),
    Committed,
    RolledBack,
}
#[allow(dead_code)] // The subsequent writer task constructs this from its bound guard.
pub(super) struct WriterExpected {
    database: String,
    database_oid: u64,
    system_identifier: u64,
    control_pid: i32,
    keys: ChallengeKeys,
    nonce: Nonce,
}
pub(super) fn parse_writer_line(
    line: &[u8],
    expected_nonce: &Nonce,
) -> Result<WriterEvent, ImportFailure> {
    if line.len() > 256 || !line.is_ascii() {
        return Err(ImportFailure::Protocol);
    }
    let line = std::str::from_utf8(line).map_err(|_| ImportFailure::Protocol)?;
    let fields: Vec<_> = line
        .strip_suffix('\n')
        .ok_or(ImportFailure::Protocol)?
        .split('|')
        .collect();
    if fields.len() < 3 || fields[0] != "KW_C4" || fields[1] != expected_nonce.hex() {
        return Err(ImportFailure::Protocol);
    }
    match fields[2] {
        "COMMITTED" if fields.len() == 3 => Ok(WriterEvent::Committed),
        "ROLLED_BACK" if fields.len() == 3 => Ok(WriterEvent::RolledBack),
        "READY" | "PRECOMMIT" if fields.len() == 6 => {
            let pid = canonical_positive(fields[3])?;
            let start = canonical_positive(fields[4])?;
            let xid = canonical_positive(fields[5])?;
            let identity = WriterIdentity {
                backend_pid: pid.try_into().map_err(|_| ImportFailure::Protocol)?,
                backend_start_micros: start.try_into().map_err(|_| ImportFailure::Protocol)?,
                transaction_id: xid,
            };
            Ok(if fields[2] == "READY" {
                WriterEvent::Ready(identity)
            } else {
                WriterEvent::Precommit(identity)
            })
        }
        _ => Err(ImportFailure::Protocol),
    }
}
fn canonical_positive(value: &str) -> Result<u64, ImportFailure> {
    if value.is_empty() || value.starts_with('0') || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ImportFailure::Protocol);
    }
    value.parse().map_err(|_| ImportFailure::Protocol)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_receipts_bind_nonce_identity_and_terminal_phase() {
        let nonce = Nonce([0xab; 16]);
        let id = WriterIdentity {
            backend_pid: 42,
            backend_start_micros: 1234567,
            transaction_id: 99,
        };
        assert_eq!(
            parse_writer_line(
                b"KW_C4|abababababababababababababababab|READY|42|1234567|99\n",
                &nonce
            ),
            Ok(WriterEvent::Ready(id))
        );
        assert_eq!(
            parse_writer_line(
                b"KW_C4|abababababababababababababababab|PRECOMMIT|42|1234567|99\n",
                &nonce
            ),
            Ok(WriterEvent::Precommit(id))
        );
        assert_eq!(
            parse_writer_line(
                b"KW_C4|abababababababababababababababab|COMMITTED\n",
                &nonce
            ),
            Ok(WriterEvent::Committed)
        );
        assert_eq!(
            parse_writer_line(
                b"KW_C4|abababababababababababababababab|ROLLED_BACK\n",
                &nonce
            ),
            Ok(WriterEvent::RolledBack)
        );
    }
    #[test]
    fn rejects_replay_noncanonical_and_extra_receipt_bytes() {
        let nonce = Nonce([0xab; 16]);
        for line in [
            "KW_C4|aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa|READY|42|1234567|99\n",
            "KW_C4|ABABABABABABABABABABABABABABABAB|READY|42|1234567|99\n",
            "KW_C4|abababababababababababababababab|READY|042|1234567|99\n",
            "KW_C4|abababababababababababababababab|READY|0|1234567|99\n",
            "KW_C4|abababababababababababababababab|READY|42|0|99\n",
            "KW_C4|abababababababababababababababab|READY|42|1234567|0\n",
            "KW_C4|abababababababababababababababab|READY|2147483648|1234567|99\n",
            "KW_C4|abababababababababababababababab|READY|42|1234567|18446744073709551616\n",
            "KW_C4|abababababababababababababababab|READY|42|1234567|99",
            "KW_C4|abababababababababababababababab|READY|42|1234567|99\r\n",
            "KW_C4|abababababababababababababababab|COMMITTED|42|1234567|99\n",
            "KW_C4|abababababababababababababababab|COMMITTED\n\n",
        ] {
            assert_eq!(
                parse_writer_line(line.as_bytes(), &nonce),
                Err(ImportFailure::Protocol)
            );
        }
    }
    #[test]
    fn operation_nonce_is_random_and_canonical() {
        let a = Nonce::random().unwrap();
        let b = Nonce::random().unwrap();
        assert_ne!(a, b);
        assert_eq!(a.hex().len(), 32);
        assert!(
            a.hex()
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
    }
}

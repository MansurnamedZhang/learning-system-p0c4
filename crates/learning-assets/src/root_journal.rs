//! Root-only byte-safe storage, **not** the C3 business journal or a capability.
//!
//! Opaque bodies do not assert a claim, known SQL outcome, write authority,
//! capture cut or NoEffect. A future DB-private exact tagged wrapper must
//! validate those facts before calling this backend. There is no disk reopen,
//! adoption, repair, deletion, business-event parser or writer integration.

use std::{
    io,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::RootJournalBytes;

pub const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_STORED_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SCAN_BYTES: u64 = 512 * 1024 * 1024;
const MAX_ENTRIES: u64 = 100_000;
const HEADER_BYTES: usize = 120;
#[cfg(any(target_os = "linux", test))]
const MAGIC: &[u8; 8] = b"KWSJBYT2";

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Root journal bytes rejected")
}
fn poisoned() -> io::Error {
    io::Error::other("Root journal bytes permanently poisoned; no replay or adoption")
}

/// Storage identity only. These UUIDs and hashes do not certify an enrolled
/// binding, an original broker epoch, a SQL claim or any business authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteJournalIdentity {
    epoch: Uuid,
    binding: Uuid,
}
impl ByteJournalIdentity {
    pub fn new(epoch: Uuid, binding: Uuid) -> io::Result<Self> {
        if epoch.get_version_num() != 4
            || binding.get_version_num() != 4
            || epoch.get_variant() != uuid::Variant::RFC4122
            || binding.get_variant() != uuid::Variant::RFC4122
        {
            return Err(invalid());
        }
        Ok(Self { epoch, binding })
    }
    pub fn epoch_id(&self) -> Uuid {
        self.epoch
    }
    pub fn binding_id(&self) -> Uuid {
        self.binding
    }
    #[cfg(any(target_os = "linux", test))]
    fn encode(self) -> Vec<u8> {
        let mut b = Vec::with_capacity(40);
        b.extend_from_slice(b"KWSJEPO2");
        b.extend_from_slice(self.epoch.as_bytes());
        b.extend_from_slice(self.binding.as_bytes());
        b
    }
}

/// Uninterpreted storage bytes. Constructing or appending this value grants no
/// C3 authority; it deliberately has no serde or JobLease conversion.
pub struct OpaqueJournalBody<'a>(&'a [u8]);
impl<'a> OpaqueJournalBody<'a> {
    pub fn from_uninterpreted_bytes(bytes: &'a [u8]) -> io::Result<Self> {
        if bytes.len() > MAX_FRAME_BYTES - HEADER_BYTES {
            return Err(invalid());
        }
        Ok(Self(bytes))
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteJournalHead {
    pub sequence: u64,
    pub sha256: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JournalBudgetUsage {
    pub charged_bytes: u64,
    pub charged_entries: u64,
    pub exhausted: bool,
}

struct Meter {
    usage: JournalBudgetUsage,
    bytes_limit: u64,
    entries_limit: u64,
}

/// Clone shares one cumulative meter; no public reset or mutable inner meter.
/// Encoding, file writes, directory entries and every prefix/readback consume
/// the same operation budget. An exceeded/poisoned meter stays exhausted.
#[derive(Clone)]
pub struct SharedJournalBudget(Arc<Mutex<Meter>>);
impl Default for SharedJournalBudget {
    fn default() -> Self {
        Self::limited(MAX_SCAN_BYTES, MAX_ENTRIES)
    }
}
impl SharedJournalBudget {
    fn limited(bytes_limit: u64, entries_limit: u64) -> Self {
        Self(Arc::new(Mutex::new(Meter {
            usage: JournalBudgetUsage {
                charged_bytes: 0,
                charged_entries: 0,
                exhausted: false,
            },
            bytes_limit,
            entries_limit,
        })))
    }
    pub fn usage(&self) -> io::Result<JournalBudgetUsage> {
        Ok(self.0.lock().map_err(|_| poisoned())?.usage)
    }
    /// Share cumulative charging with the future typed wrapper. Charging only
    /// consumes a limit; it cannot authorize bytes or recover an exhausted meter.
    pub fn charge(&self, bytes: u64, entries: u64) -> io::Result<()> {
        let mut m = self.0.lock().map_err(|_| poisoned())?;
        if m.usage.exhausted {
            return Err(poisoned());
        }
        let b = m.usage.charged_bytes.checked_add(bytes);
        let e = m.usage.charged_entries.checked_add(entries);
        if b.is_none_or(|n| n > m.bytes_limit) || e.is_none_or(|n| n > m.entries_limit) {
            m.usage.exhausted = true;
            return Err(poisoned());
        }
        m.usage.charged_bytes = b.expect("checked");
        m.usage.charged_entries = e.expect("checked");
        Ok(())
    }
}

#[cfg(any(target_os = "linux", test))]
fn digest(b: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(b).into()
}

#[cfg(any(target_os = "linux", test))]
#[derive(Debug, PartialEq, Eq)]
struct FrameHeader {
    identity: ByteJournalIdentity,
    sequence: u64,
    previous: [u8; 32],
    body_sha256: [u8; 32],
    body_len: u64,
}
#[cfg(any(target_os = "linux", test))]
impl FrameHeader {
    fn parse(b: &[u8; HEADER_BYTES]) -> io::Result<Self> {
        if &b[..8] != MAGIC {
            return Err(invalid());
        }
        let sequence = u64::from_be_bytes(b[8..16].try_into().map_err(|_| invalid())?);
        let identity = ByteJournalIdentity::new(
            Uuid::from_slice(&b[16..32]).map_err(|_| invalid())?,
            Uuid::from_slice(&b[32..48]).map_err(|_| invalid())?,
        )?;
        let body_len = u64::from_be_bytes(b[112..120].try_into().map_err(|_| invalid())?);
        if sequence == 0 || body_len > (MAX_FRAME_BYTES - HEADER_BYTES) as u64 {
            return Err(invalid());
        }
        Ok(Self {
            identity,
            sequence,
            previous: b[48..80].try_into().map_err(|_| invalid())?,
            body_sha256: b[80..112].try_into().map_err(|_| invalid())?,
            body_len,
        })
    }
}

#[cfg(any(target_os = "linux", test))]
fn encode_frame(
    identity: ByteJournalIdentity,
    head: ByteJournalHead,
    body: &OpaqueJournalBody<'_>,
) -> io::Result<Vec<u8>> {
    let sequence = head.sequence.checked_add(1).ok_or_else(invalid)?;
    let mut b = Vec::with_capacity(HEADER_BYTES + body.0.len());
    b.extend_from_slice(MAGIC);
    b.extend_from_slice(&sequence.to_be_bytes());
    b.extend_from_slice(identity.epoch.as_bytes());
    b.extend_from_slice(identity.binding.as_bytes());
    b.extend_from_slice(&head.sha256);
    b.extend_from_slice(&digest(body.0));
    b.extend_from_slice(&(body.0.len() as u64).to_be_bytes());
    b.extend_from_slice(body.0);
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_budget_aggregates_clones_and_is_sticky_after_overflow() {
        let a = SharedJournalBudget::limited(256, 3);
        let b = a.clone();
        a.charge(120, 1).unwrap();
        b.charge(120, 1).unwrap();
        assert!(a.charge(17, 1).is_err());
        assert!(b.charge(0, 0).is_err());
        assert_eq!(
            a.usage().unwrap(),
            JournalBudgetUsage {
                charged_bytes: 240,
                charged_entries: 2,
                exhausted: true
            }
        );
        let entries = SharedJournalBudget::limited(256, 1);
        entries.charge(0, 1).unwrap();
        assert!(entries.charge(0, 1).is_err());
    }
    #[test]
    fn bounded_opaque_frames_chain_exact_bytes_without_business_interpretation() {
        let id = ByteJournalIdentity::new(Uuid::new_v4(), Uuid::new_v4()).unwrap();
        let head = ByteJournalHead {
            sequence: 0,
            sha256: digest(&id.encode()),
        };
        let body =
            OpaqueJournalBody::from_uninterpreted_bytes(b"not a C3 event or authority").unwrap();
        let frame = encode_frame(id, head, &body).unwrap();
        let parsed = FrameHeader::parse(frame[..HEADER_BYTES].try_into().unwrap()).unwrap();
        assert_eq!(parsed.identity, id);
        assert_eq!(parsed.sequence, 1);
        assert_eq!(parsed.previous, head.sha256);
        assert_eq!(parsed.body_sha256, digest(&frame[HEADER_BYTES..]));
        assert_eq!(parsed.body_len as usize, frame.len() - HEADER_BYTES);
        let next = encode_frame(
            id,
            ByteJournalHead {
                sequence: 1,
                sha256: digest(&frame),
            },
            &body,
        )
        .unwrap();
        assert_eq!(
            FrameHeader::parse(next[..HEADER_BYTES].try_into().unwrap())
                .unwrap()
                .previous,
            digest(&frame)
        );
        let mut corrupt: [u8; HEADER_BYTES] = frame[..HEADER_BYTES].try_into().unwrap();
        corrupt[0] ^= 1;
        assert!(FrameHeader::parse(&corrupt).is_err());
    }
    #[test]
    fn oversized_body_nil_identity_and_sequence_wrap_are_rejected() {
        assert!(ByteJournalIdentity::new(Uuid::nil(), Uuid::new_v4()).is_err());
        let mut non_rfc = Uuid::new_v4().into_bytes();
        non_rfc[8] &= 0x3f;
        let non_rfc = Uuid::from_bytes(non_rfc);
        assert_eq!(non_rfc.get_version_num(), 4);
        assert_ne!(non_rfc.get_variant(), uuid::Variant::RFC4122);
        assert!(ByteJournalIdentity::new(non_rfc, Uuid::new_v4()).is_err());
        assert!(ByteJournalIdentity::new(Uuid::new_v4(), non_rfc).is_err());
        assert!(
            OpaqueJournalBody::from_uninterpreted_bytes(&vec![
                0;
                MAX_FRAME_BYTES - HEADER_BYTES + 1
            ])
            .is_err()
        );
        let id = ByteJournalIdentity::new(Uuid::new_v4(), Uuid::new_v4()).unwrap();
        assert!(
            encode_frame(
                id,
                ByteJournalHead {
                    sequence: u64::MAX,
                    sha256: [0; 32]
                },
                &OpaqueJournalBody::from_uninterpreted_bytes(b"").unwrap()
            )
            .is_err()
        );
    }
}

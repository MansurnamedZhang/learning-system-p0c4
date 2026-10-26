//! Deterministic candidate DATA hashes, never an authenticated transcript.
use super::{Direction, WireError, schema};
use crate::registry;
use serde::Serialize;
use sha2::{Digest, Sha256};

const DOMAIN: &[u8] = b"KnowWeave/Task9/B1/wire/v1";
const AUTH_DOMAIN: &[u8] = b"KnowWeave/Task9/B1/source-auth/v1";
const FINAL_DOMAIN: &[u8] = b"KnowWeave/Task9/B1/source-final/v1";
const ACTOR_DOMAIN: &[u8] = b"KnowWeave/Task9/B1/source-actor/v1";

pub(super) struct CandidateTranscript {
    heads: [[u8; 32]; 2],
}
impl Default for CandidateTranscript {
    fn default() -> Self {
        Self {
            heads: [
                initial(Direction::SourceToDestination),
                initial(Direction::DestinationToSource),
            ],
        }
    }
}
impl CandidateTranscript {
    pub(super) fn head(&self, direction: Direction) -> [u8; 32] {
        self.heads[direction.index()]
    }
    pub(super) fn observe_untrusted(
        &mut self,
        direction: Direction,
        header: &[u8],
        payload: &[u8],
    ) -> Result<(), WireError> {
        self.heads[direction.index()] = next(self.head(direction), header, payload)?;
        Ok(())
    }
}

fn initial(direction: Direction) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(DOMAIN);
    hash.update([0, direction.octet()]);
    hash.finalize().into()
}
fn next(previous: [u8; 32], header: &[u8], payload: &[u8]) -> Result<[u8; 32], WireError> {
    if header.len() != 20 || payload.len() > super::profile::DATA_BYTES as usize {
        return Err(WireError::Length);
    }
    let mut hash = Sha256::new();
    hash.update(previous);
    hash.update(20_u32.to_be_bytes());
    hash.update(header);
    hash.update(
        u32::try_from(payload.len())
            .map_err(|_| WireError::Length)?
            .to_be_bytes(),
    );
    hash.update(payload);
    Ok(hash.finalize().into())
}

pub(super) fn frame_sha256(header: &[u8], payload: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(header);
    hash.update(payload);
    hex::encode(hash.finalize())
}

pub(super) fn binding(
    source: &schema::SourceBinding,
    destination: &schema::DestinationBinding,
) -> Result<String, WireError> {
    #[derive(Serialize)]
    struct Binding<'a> {
        source: &'a schema::SourceBinding,
        destination: &'a schema::DestinationBinding,
    }
    Ok(crate::digest(
        &registry::canonical(&Binding {
            source,
            destination,
        })
        .map_err(|_| WireError::Json)?,
    ))
}

pub(super) fn actor_binding(
    source_operation_id: &str,
    binding_sha256: &str,
    s_nonce: &str,
    d_nonce: &str,
) -> Result<String, WireError> {
    #[derive(Serialize)]
    struct Actor<'a> {
        source_operation_id: &'a str,
        binding_sha256: &'a str,
        s_nonce: &'a str,
        d_nonce: &'a str,
    }
    let raw = registry::canonical(&Actor {
        source_operation_id,
        binding_sha256,
        s_nonce,
        d_nonce,
    })
    .map_err(|_| WireError::Json)?;
    let mut hash = Sha256::new();
    hash.update(ACTOR_DOMAIN);
    hash.update([0]);
    hash.update(raw);
    Ok(hex::encode(hash.finalize()))
}

/// Only the two fixed unsigned DATA objects are supported. No key or signer is
/// reachable; a peer-supplied signature string is neither verified nor trusted.
pub(super) fn candidate_preimage(control: &schema::UntrustedControl) -> Result<Vec<u8>, WireError> {
    let domain = match control {
        schema::UntrustedControl::SourceAuth(_) => AUTH_DOMAIN,
        schema::UntrustedControl::SourceFinalDecision(_) => FINAL_DOMAIN,
        _ => return Err(WireError::Unsupported),
    };
    let c = control.common();
    let header = super::Header {
        kind: super::Kind::Data,
        direction: c.direction,
        sequence: c.sequence,
        payload_bytes: 0,
    };
    if !control.valid(header) {
        return Err(WireError::Fields);
    }
    // The exact typed object was already parsed/validated. Do not use a map as
    // an inbound parser, and never accept a caller-selected signature domain.
    let mut object: serde_json::Value =
        serde_json::from_slice(&control.canonical()?).map_err(|_| WireError::Json)?;
    if object
        .as_object_mut()
        .ok_or(WireError::Json)?
        .remove("signature")
        .is_none()
    {
        return Err(WireError::Fields);
    }
    let unsigned = registry::canonical(&object).map_err(|_| WireError::Json)?;
    let mut bytes = Vec::with_capacity(domain.len() + 1 + unsigned.len());
    bytes.extend_from_slice(domain);
    bytes.push(0);
    bytes.extend_from_slice(&unsigned);
    Ok(bytes)
}

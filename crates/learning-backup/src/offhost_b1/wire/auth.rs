//! Candidate-policy signature DATA for exactly one SourceAuth or Final frame.
//! This does not authenticate a session, prove a genuine S/D, or release any
//! clock/lease/fence. A checked Final never upgrades an earlier syntax-only
//! SourceAuth to a verified message. No signing, key IO or positive authority.
use super::{
    UntrustedBody, UntrustedControl, UntrustedFrame, UntrustedMessage, WireError, WireSyntax,
    profile, transcript,
};
use crate::offhost_b1::policy::UntrustedDestinationInputs;
use ring::signature::{ED25519, UnparsedPublicKey};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SourceSignatureDataKind {
    SourceAuth,
    SourceFinalDecision,
}

// No Deserialize/Serialize/Clone/Default/Debug or caller constructor. Fields
// describe this exact checked frame only, never a permit or an actor slot.
pub(super) struct CheckedSourceSignatureData<'a> {
    frame: UntrustedFrame<'a>,
    kind: SourceSignatureDataKind,
    public_key_sha256: String,
    preimage_sha256: String,
}
impl CheckedSourceSignatureData<'_> {
    pub(super) fn frame_data(&self) -> &UntrustedFrame<'_> {
        &self.frame
    }
    pub(super) fn kind_data(&self) -> SourceSignatureDataKind {
        self.kind
    }
    pub(super) fn public_key_sha256_data(&self) -> &str {
        &self.public_key_sha256
    }
    pub(super) fn preimage_sha256_data(&self) -> &str {
        &self.preimage_sha256
    }
}

pub(super) fn verify_frame_data<'a>(
    owner: &WireSyntax,
    frame: UntrustedFrame<'a>,
    destination: &UntrustedDestinationInputs,
) -> Result<CheckedSourceSignatureData<'a>, WireError> {
    let UntrustedMessage::Data(control, UntrustedBody::Empty) = &frame.message else {
        return Err(WireError::Unsupported);
    };
    // These expected values belong to the current syntax object and precede
    // this frame. A message's own nonce/prev is never its own expectation.
    owner.check_common(control)?;
    let (kind, claimed_key, signature_hex) = match control {
        UntrustedControl::SourceAuth(c) => {
            owner.check_source_auth_data(c)?;
            (
                SourceSignatureDataKind::SourceAuth,
                &c.source_auth_key_sha256,
                &c.signature,
            )
        }
        UntrustedControl::SourceFinalDecision(c) => {
            owner.check_source_final_data(c)?;
            (
                SourceSignatureDataKind::SourceFinalDecision,
                &c.source_auth_key_sha256,
                &c.signature,
            )
        }
        _ => return Err(WireError::Unsupported),
    };
    let pins = destination.signature_data_pins();
    let source = owner.source.as_ref().ok_or(WireError::Phase)?;
    let dest = owner.destination.as_ref().ok_or(WireError::Phase)?;
    if (
        source.deployment_id.as_str(),
        source.source_group_id.as_str(),
        source.manager_release_sha256.as_str(),
        source.application_build_sha256.as_str(),
    ) != pins.source_data()
        || (
            dest.deployment_id.as_str(),
            dest.destination_group_id.as_str(),
            dest.registry_authority_sha256.as_str(),
            dest.receiver_release_sha256.as_str(),
        ) != pins.destination_data()
    {
        return Err(WireError::Binding);
    }
    // Candidate policy has no external expectation for source application
    // commit or actual root/current-generation facts; do not invent one here.
    let (public_hex, expected_key) = pins.public_key_data();
    if !crate::valid_hex(public_hex, 64) || !crate::valid_hex(signature_hex, 128) {
        return Err(WireError::SignatureData);
    }
    let mut public = [0_u8; 32];
    let mut signature = [0_u8; 64];
    hex::decode_to_slice(public_hex, &mut public).map_err(|_| WireError::SignatureData)?;
    hex::decode_to_slice(signature_hex, &mut signature).map_err(|_| WireError::SignatureData)?;
    let public_key_sha256 = crate::digest(&public);
    if public_key_sha256 != expected_key || public_key_sha256 != *claimed_key {
        return Err(WireError::Binding);
    }
    // Only the two existing fixed domains, typed objects and bounded canonical
    // preimages are reachable. No caller-selected algorithm/domain/bytes API.
    let preimage = transcript::candidate_preimage(control)?;
    let domain_bytes = match kind {
        SourceSignatureDataKind::SourceAuth => b"KnowWeave/Task9/B1/source-auth/v1\0".len(),
        SourceSignatureDataKind::SourceFinalDecision => {
            b"KnowWeave/Task9/B1/source-final/v1\0".len()
        }
    };
    if preimage.len() > profile::CONTROL_BYTES as usize + domain_bytes {
        return Err(WireError::Length);
    }
    // This is ring0.17.14 ED25519 verification semantics, not a general strict
    // key-admission claim. Operational key issuance/PKCS#8/C remain separate.
    UnparsedPublicKey::new(&ED25519, &public)
        .verify(&preimage, &signature)
        .map_err(|_| WireError::SignatureData)?;
    let preimage_sha256 = crate::digest(&preimage);
    Ok(CheckedSourceSignatureData {
        frame,
        kind,
        public_key_sha256,
        preimage_sha256,
    })
}

#[cfg(test)]
mod tests;

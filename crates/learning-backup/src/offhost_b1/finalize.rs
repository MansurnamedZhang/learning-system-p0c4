//! Fixed original-D finalization bridge. Only the concrete original receiver
//! can consume the opaque crypto-before-apply token after rechecking its real
//! Ready and current native/metadata owners. Signature DATA has no constructor.

use crate::{
    BackupError,
    destination::{
        CommittedRemoteOriginal, OperationAuthority, OriginalRemoteFinalization,
        RemoteFinalBindingData,
    },
    valid_digest,
};
use std::sync::{Arc, Mutex};

// Exact existing profile constants; no new configuration or resource limit.
pub(crate) const FILES: u64 = super::profile::FILES;
pub(crate) const PACKAGE_BYTES: u64 = super::profile::PACKAGE_BYTES;
pub(crate) const EVIDENCE_BYTES: u64 = super::profile::EVIDENCE_BYTES;

/// Only the actual installed D and its retained Ready can consume the opaque
/// VerifiedFinalInOriginalD produced by the single fixed wire owner. No public
/// or test-success constructor, Local conversion, DTO, path or bool exists.
#[allow(
    dead_code,
    reason = "default-off original D; installation, CODE and runtime acceptance remain separate"
)]
pub(crate) struct AuthenticatedFinalInOriginalD {
    operation: Arc<Mutex<OperationAuthority>>,
    binding: RemoteFinalBindingData,
    final_frame_sha256: String,
    fence_open_record_sha256: String,
}
impl AuthenticatedFinalInOriginalD {
    /// Only the fixed original D can promote the opaque crypto-before-apply
    /// token after rechecking its real Ready, current installed policy, native
    /// owner, nonce/transcript/session joins and durable negative admission.
    pub(super) fn from_original_destination(
        original: &mut super::destination_actor::OriginalDestinationSession<'_>,
        verified: &super::wire::VerifiedFinalInOriginalD,
    ) -> Result<Self, BackupError> {
        let (operation, binding) = original.require_original_final_data(verified)?;
        let final_data = verified.control_data();
        Ok(Self {
            operation,
            binding,
            final_frame_sha256: verified.frame_sha256_data().to_owned(),
            fence_open_record_sha256: final_data.fence.open_record_sha256.clone(),
        })
    }
    pub(crate) fn binding_data(&self) -> &RemoteFinalBindingData {
        &self.binding
    }

    pub(crate) fn require_original(
        &self,
        operation: &Arc<Mutex<OperationAuthority>>,
        binding: &RemoteFinalBindingData,
    ) -> Result<(), BackupError> {
        self.binding.validate_data()?;
        binding.validate_data()?;
        if !Arc::ptr_eq(&self.operation, operation)
            || &self.binding != binding
            || !valid_digest(&self.final_frame_sha256)
            || !valid_digest(&self.fence_open_record_sha256)
        {
            return Err(BackupError::Invalid(
                "authenticated Final belongs to a different original D",
            ));
        }
        Ok(())
    }
}

#[allow(
    dead_code,
    reason = "default-off original D; installation, CODE and runtime acceptance remain separate"
)]
pub(super) fn finalize_original(
    original: OriginalRemoteFinalization,
    decision: AuthenticatedFinalInOriginalD,
) -> Result<CommittedRemoteOriginal, BackupError> {
    original.finish(decision)
}

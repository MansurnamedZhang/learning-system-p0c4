//! Disk integrity is distinct from original-operation publication authority.
use crate::{
    BackupError, digest,
    registry::{MAX_ENTRIES, canonical, canonical_record},
    valid_digest,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, MutexGuard},
};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Intent {
    pub format_version: u32,
    pub capability: String,
    pub backup_id: Uuid,
    pub attempt_id: Uuid,
    pub destination_dev: u64,
    pub destination_ino: u64,
    pub evidence_dev: u64,
    pub evidence_ino: u64,
    pub manifest_sha256: String,
    pub proof_sha256: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Terminal {
    pub format_version: u32,
    pub capability: String,
    pub backup_id: Uuid,
    pub attempt_id: Uuid,
    pub intent_sha256: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CommitIdentity {
    pub intent: Intent,
}
impl Intent {
    pub(super) fn terminal(&self) -> Result<Terminal, BackupError> {
        Ok(Terminal {
            format_version: 1,
            capability: "destination_evidence_commit_v1".into(),
            backup_id: self.backup_id,
            attempt_id: self.attempt_id,
            intent_sha256: digest(&canonical(self)?),
        })
    }
}
pub(super) fn validate_committed(
    intent: &[u8],
    terminal: &[u8],
) -> Result<CommitIdentity, BackupError> {
    let intent: Intent = canonical_record(intent, 4096)?;
    let terminal: Terminal = canonical_record(terminal, 4096)?;
    if intent.format_version != 1
        || intent.capability != "destination_evidence_attempt_v1"
        || !crate::registry::v4(&intent.backup_id.to_string())
        || !crate::registry::v4(&intent.attempt_id.to_string())
        || [
            intent.destination_dev,
            intent.destination_ino,
            intent.evidence_dev,
            intent.evidence_ino,
        ]
        .contains(&0)
        || !valid_digest(&intent.manifest_sha256)
        || !valid_digest(&intent.proof_sha256)
        || terminal != intent.terminal()?
    {
        return Err(BackupError::Invalid("destination evidence attempt linkage"));
    }
    Ok(CommitIdentity { intent })
}
pub(super) fn matching_residue(names: &[String], id: Uuid) -> Vec<String> {
    let prefix = id.to_string();
    names
        .iter()
        .filter(|s| s.starts_with(&prefix))
        .cloned()
        .collect()
}
pub(super) fn require_fresh(names: &[String], id: Uuid) -> Result<(), BackupError> {
    if !matching_residue(names, id).is_empty() {
        return Err(BackupError::Invalid(
            "destination evidence attempt already exists",
        ));
    }
    Ok(())
}
pub(super) fn require_committed_names(names: &[String], id: Uuid) -> Result<(), BackupError> {
    let mut expected = vec![id.to_string(), format!("{id}.attempt")];
    expected.sort();
    if matching_residue(names, id) != expected {
        return Err(BackupError::Invalid(
            "destination evidence incomplete or extra residue",
        ));
    }
    Ok(())
}

#[derive(Debug, Default)]
pub(crate) struct OperationAuthority {
    failed: bool,
    attempted: BTreeSet<Uuid>,
    successful: BTreeMap<Uuid, CommitIdentity>,
    #[cfg(all(target_os = "linux", feature = "offhost-b1"))]
    remote_final: BTreeMap<Uuid, remote::Phase>,
}
impl OperationAuthority {
    fn begin(&mut self, id: Uuid) -> Result<(), BackupError> {
        if self.failed || self.attempted.len() >= MAX_ENTRIES || !self.attempted.insert(id) {
            return Err(BackupError::Invalid(
                "destination operation unavailable or repeated",
            ));
        }
        Ok(())
    }
    pub(super) fn require(&self, identity: &CommitIdentity) -> Result<(), BackupError> {
        #[cfg(all(target_os = "linux", feature = "offhost-b1"))]
        self.require_local_publication(identity.intent.backup_id)?;
        if self.failed || self.successful.get(&identity.intent.backup_id) != Some(identity) {
            return Err(BackupError::Invalid(
                "original destination publication operation required",
            ));
        }
        Ok(())
    }
    pub(super) fn require_id(&self, id: Uuid) -> Result<(), BackupError> {
        #[cfg(all(target_os = "linux", feature = "offhost-b1"))]
        self.require_local_publication(id)?;
        if self.failed || !self.successful.contains_key(&id) {
            return Err(BackupError::Invalid(
                "original destination publication operation required",
            ));
        }
        Ok(())
    }
    fn invalidate(&mut self) {
        self.failed = true;
        self.successful.clear();
        #[cfg(all(target_os = "linux", feature = "offhost-b1"))]
        for phase in self.remote_final.values_mut() {
            // A later transport failure cannot erase a completed disk commit.
            // It still invalidates future uses of this original operation.
            if *phase != remote::Phase::Committed {
                *phase = remote::Phase::Failed;
            }
        }
    }
    #[cfg(all(target_os = "linux", feature = "offhost-b1"))]
    fn require_local_publication(&self, id: Uuid) -> Result<(), BackupError> {
        if self.remote_final.contains_key(&id) {
            return Err(BackupError::Invalid(
                "remote original requires its one-shot Final consumer",
            ));
        }
        Ok(())
    }
}
pub(crate) struct ErrorFence {
    context: Arc<Mutex<OperationAuthority>>,
    success: bool,
}
impl ErrorFence {
    pub(crate) fn new(context: &Arc<Mutex<OperationAuthority>>) -> Self {
        Self {
            context: Arc::clone(context),
            success: false,
        }
    }
    pub(crate) fn complete(mut self) {
        self.success = true;
    }
}
impl Drop for ErrorFence {
    fn drop(&mut self) {
        if !self.success {
            self.context
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .invalidate();
        }
    }
}

// The final authority-consuming closure must not call helpers that lock or
// invalidate this context; evidence reads and their error fences stay outside.
pub(super) fn finalize_use<T>(
    context: &Arc<Mutex<OperationAuthority>>,
    identity: &CommitIdentity,
    action: impl FnOnce() -> Result<T, BackupError>,
) -> Result<T, BackupError> {
    serialized_commit(
        context,
        |authority| authority.require(identity),
        |_| action(),
    )
}

struct SerializedCommit<'a> {
    authority: MutexGuard<'a, OperationAuthority>,
    committed: bool,
}
impl Drop for SerializedCommit<'_> {
    fn drop(&mut self) {
        // Already own the mutex. Re-locking through ErrorFence here would
        // deadlock on an error or unwind from the consuming closure.
        if !self.committed {
            self.authority.invalidate();
        }
    }
}
fn serialized_commit<T>(
    context: &Arc<Mutex<OperationAuthority>>,
    validate: impl FnOnce(&OperationAuthority) -> Result<(), BackupError>,
    action: impl FnOnce(&mut OperationAuthority) -> Result<T, BackupError>,
) -> Result<T, BackupError> {
    let authority = match context.lock() {
        Ok(authority) => authority,
        Err(poison) => {
            poison.into_inner().invalidate();
            return Err(BackupError::Invalid(
                "destination operation unwind invalidated authority",
            ));
        }
    };
    let mut commit = SerializedCommit {
        authority,
        committed: false,
    };
    validate(&commit.authority)?;
    let result = action(&mut commit.authority)?;
    // Linearization point: invalidation uses this same mutex. Revocation
    // after this point affects future uses, not an already committed result.
    commit.committed = true;
    Ok(result)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    Admit,
    Prepare,
    Stage,
    BeforeRename,
    Rename,
    ParentSync,
    FinalReadback,
    TerminalWrite,
    TerminalSync,
    TerminalReadback,
    FinalRecheck,
}
pub(super) const STEPS: [Step; 11] = [
    Step::Admit,
    Step::Prepare,
    Step::Stage,
    Step::BeforeRename,
    Step::Rename,
    Step::ParentSync,
    Step::FinalReadback,
    Step::TerminalWrite,
    Step::TerminalSync,
    Step::TerminalReadback,
    Step::FinalRecheck,
];
pub(super) trait PublicationIo {
    type Output;
    fn step(&mut self, step: Step) -> Result<(), BackupError>;
    fn outcome(&mut self) -> Result<(Self::Output, CommitIdentity), BackupError>;
}
pub(super) fn publish<I: PublicationIo>(
    context: &Arc<Mutex<OperationAuthority>>,
    id: Uuid,
    io: &mut I,
) -> Result<I::Output, BackupError> {
    let fence = ErrorFence::new(context);
    context.lock().expect("destination operation").begin(id)?;
    for step in STEPS {
        io.step(step)?;
    }
    let (output, identity) = io.outcome()?;
    if identity.intent.backup_id != id {
        return Err(BackupError::Invalid(
            "destination publication result identity",
        ));
    }
    let output = serialized_commit(
        context,
        |operation| {
            if operation.failed
                || !operation.attempted.contains(&id)
                || operation.successful.contains_key(&id)
            {
                return Err(BackupError::Invalid(
                    "destination operation invalidated during publication",
                ));
            }
            Ok(())
        },
        |operation| {
            operation.successful.insert(id, identity);
            Ok(output)
        },
    )?;
    fence.complete();
    Ok(output)
}

// The concrete original receiver selects this fixed, non-generic remote writer
// before its first step. Neither disk DATA, a callback, a caller mode flag nor
// a successful Local operation can construct its genuine authenticated owner.
#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
#[allow(
    dead_code,
    reason = "default-off original D; installation, CODE and runtime acceptance remain separate"
)]
pub(super) mod remote {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum Phase {
        Pending,
        Consuming,
        Committed,
        Failed,
    }

    pub(in crate::destination) fn publish_original(
        original: &crate::offhost_b1::destination_actor::OriginalAuthenticatedSourceInD,
        io: &mut crate::destination::linux::OriginalRemoteWriter<'_>,
    ) -> Result<(crate::CapturedSourceProof, CommitIdentity), BackupError> {
        original.require_destination(original.destination_original())?;
        // Pay original attempted/success/final BTreeMap nodes and the intent
        // clone before the first publication step, not at later consumption.
        original.prepay_original_work(16_384, 8)?;
        let context = &original.destination_original().lease.destination_operation;
        let id = original.backup_id_data()?;
        let error = ErrorFence::new(context);
        {
            let mut authority = context.lock().map_err(|poison| {
                poison.into_inner().invalidate();
                BackupError::Invalid("remote original operation poisoned")
            })?;
            authority.begin(id)?;
            authority.remote_final.insert(id, Phase::Pending);
        }
        for step in STEPS {
            io.step(step)?;
        }
        let (output, identity) = io.outcome()?;
        if identity.intent.backup_id != id {
            return Err(BackupError::Invalid("remote original evidence identity"));
        }
        let output = serialized_commit(
            context,
            |authority| {
                if authority.failed
                    || !authority.attempted.contains(&id)
                    || authority.successful.contains_key(&id)
                    || authority.remote_final.get(&id) != Some(&Phase::Pending)
                {
                    return Err(BackupError::Invalid("remote original evidence invalidated"));
                }
                Ok(())
            },
            |authority| {
                authority.successful.insert(id, identity);
                Ok(output)
            },
        )?;
        error.complete();
        Ok(output)
    }

    /// A borrow of the actual, still-held original authority mutex. Only the
    /// fixed consumer below can acquire it; there is no callback or constructor
    /// for a caller, an authenticated-DATA object, or a recovered disk record.
    pub(crate) struct Consumption<'a> {
        context: &'a Arc<Mutex<OperationAuthority>>,
        identity: &'a CommitIdentity,
        authority: MutexGuard<'a, OperationAuthority>,
        completed: bool,
    }
    impl<'a> Consumption<'a> {
        fn lock_pending(
            context: &'a Arc<Mutex<OperationAuthority>>,
            identity: &'a CommitIdentity,
        ) -> Result<Self, BackupError> {
            let authority = context.lock().map_err(|poison| {
                poison.into_inner().invalidate();
                BackupError::Invalid("remote original operation poisoned")
            })?;
            let owner = Self {
                context,
                identity,
                authority,
                completed: false,
            };
            owner.require_pending()?;
            Ok(owner)
        }
        fn require_pending(&self) -> Result<(), BackupError> {
            let id = self.identity.intent.backup_id;
            if self.authority.failed
                || self.authority.successful.get(&id) != Some(self.identity)
                || self.authority.remote_final.get(&id) != Some(&Phase::Pending)
            {
                return Err(BackupError::Invalid(
                    "remote Final is not original Pending success",
                ));
            }
            Ok(())
        }
        fn enter(&mut self) -> Result<(), BackupError> {
            self.require_pending()?;
            *self
                .authority
                .remote_final
                .get_mut(&self.identity.intent.backup_id)
                .ok_or(BackupError::Invalid("remote original Final state"))? = Phase::Consuming;
            Ok(())
        }
        pub(in crate::destination) fn require_original(
            &self,
            context: &Arc<Mutex<OperationAuthority>>,
            identity: &CommitIdentity,
        ) -> Result<(), BackupError> {
            let id = identity.intent.backup_id;
            if !Arc::ptr_eq(self.context, context)
                || self.identity != identity
                || self.authority.failed
                || self.authority.successful.get(&id) != Some(identity)
                || self.authority.remote_final.get(&id) != Some(&Phase::Consuming)
            {
                return Err(BackupError::Invalid(
                    "same consuming remote original required",
                ));
            }
            Ok(())
        }
        fn commit(mut self) -> Result<(), BackupError> {
            self.require_original(self.context, self.identity)?;
            *self
                .authority
                .remote_final
                .get_mut(&self.identity.intent.backup_id)
                .ok_or(BackupError::Invalid("remote original Final state"))? = Phase::Committed;
            self.completed = true;
            Ok(())
        }
    }
    impl Drop for Consumption<'_> {
        fn drop(&mut self) {
            if !self.completed {
                // Already own the same mutex; an ErrorFence here would relock.
                self.authority.invalidate();
            }
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    pub(in crate::destination) enum RejectionObservation {
        RejectedPending,
        RetainedConsumption,
        AlreadyFailed,
    }
    pub(in crate::destination) fn observe_rejection(
        context: &Arc<Mutex<OperationAuthority>>,
        id: Uuid,
    ) -> Result<RejectionObservation, BackupError> {
        let mut authority = context.lock().map_err(|poison| {
            poison.into_inner().invalidate();
            BackupError::Invalid("remote original operation poisoned")
        })?;
        match authority.remote_final.get(&id) {
            Some(Phase::Pending) => {
                authority.invalidate();
                Ok(RejectionObservation::RejectedPending)
            }
            Some(Phase::Consuming | Phase::Committed) => {
                Ok(RejectionObservation::RetainedConsumption)
            }
            Some(Phase::Failed) => Ok(RejectionObservation::AlreadyFailed),
            None => {
                authority.invalidate();
                Err(BackupError::Invalid("remote original rejection identity"))
            }
        }
    }

    pub(in crate::destination) fn require_original_pending(
        original: &crate::offhost_b1::destination_actor::OriginalAuthenticatedSourceInD,
        identity: &CommitIdentity,
    ) -> Result<(), BackupError> {
        original.require_destination(original.destination_original())?;
        let authority = original
            .destination_original()
            .lease
            .destination_operation
            .lock()
            .map_err(|_| BackupError::Invalid("remote original Pending poisoned"))?;
        if authority.failed
            || authority.successful.get(&identity.intent.backup_id) != Some(identity)
            || authority.remote_final.get(&identity.intent.backup_id) != Some(&Phase::Pending)
            || identity.intent.backup_id != original.backup_id_data()?
        {
            return Err(BackupError::Invalid(
                "same original remote Pending required",
            ));
        }
        Ok(())
    }

    pub(in crate::destination) fn require_committed_original(
        context: &Arc<Mutex<OperationAuthority>>,
        identity: &CommitIdentity,
    ) -> Result<(), BackupError> {
        let authority = context
            .lock()
            .map_err(|_| BackupError::Invalid("committed original operation poisoned"))?;
        if authority.failed
            || authority.remote_final.get(&identity.intent.backup_id) != Some(&Phase::Committed)
            || authority.successful.get(&identity.intent.backup_id) != Some(identity)
        {
            return Err(BackupError::Invalid(
                "same actual committed original required",
            ));
        }
        Ok(())
    }

    pub(in crate::destination) fn consume_original(
        original: &crate::destination::OriginalRemoteFinalization,
        decision: &crate::offhost_b1::finalize::AuthenticatedFinalInOriginalD,
    ) -> Result<crate::complete::OriginalRemoteReceipt, BackupError> {
        let mut owner = Consumption::lock_pending(original.operation(), original.identity())?;
        original.prepay_consumer_entry()?;
        decision.require_original(original.operation(), original.final_binding_data())?;
        original.require_pending_inputs()?;
        owner.enter()?;
        let receipt = original.sign_and_publish_fixed(&owner)?;
        original.check_deadline()?;
        // Disk success is committed here, before the opaque receipt can escape.
        owner.commit()?;
        Ok(receipt)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        // Pure state-model fixtures only. These cannot construct an original
        // D owner, an authenticated Final, a signer, or a committed receipt.
        fn pending_model() -> (Arc<Mutex<OperationAuthority>>, CommitIdentity) {
            let id = Uuid::new_v4();
            let identity = CommitIdentity {
                intent: Intent {
                    format_version: 1,
                    capability: "destination_evidence_attempt_v1".into(),
                    backup_id: id,
                    attempt_id: Uuid::new_v4(),
                    destination_dev: 1,
                    destination_ino: 2,
                    evidence_dev: 1,
                    evidence_ino: 3,
                    manifest_sha256: "a".repeat(64),
                    proof_sha256: "b".repeat(64),
                },
            };
            let mut model = OperationAuthority::default();
            model.begin(id).unwrap();
            model.remote_final.insert(id, Phase::Pending);
            model.successful.insert(id, identity.clone());
            (Arc::new(Mutex::new(model)), identity)
        }

        #[test]
        fn remote_state_model_never_uses_legacy_local_signing_authority() {
            let (context, identity) = pending_model();
            assert!(context.lock().unwrap().require(&identity).is_err());
            assert!(
                context
                    .lock()
                    .unwrap()
                    .require_id(identity.intent.backup_id)
                    .is_err()
            );
            assert!(
                finalize_use::<()>(&context, &identity, || {
                    panic!("legacy signer must not run for Remote")
                })
                .is_err()
            );
        }

        #[test]
        fn remote_state_model_requires_original_success_and_exact_attempt() {
            let (context, identity) = pending_model();
            let other_context = Arc::new(Mutex::new(OperationAuthority::default()));
            assert!(Consumption::lock_pending(&other_context, &identity).is_err());
            let mut changed = identity.clone();
            changed.intent.attempt_id = Uuid::new_v4();
            assert!(Consumption::lock_pending(&context, &changed).is_err());
            assert_eq!(
                context.lock().unwrap().remote_final[&identity.intent.backup_id],
                Phase::Failed
            );
        }

        #[test]
        fn remote_state_model_pending_rejection_wins_before_consumption() {
            let (context, identity) = pending_model();
            assert_eq!(
                observe_rejection(&context, identity.intent.backup_id).unwrap(),
                RejectionObservation::RejectedPending
            );
            assert!(Consumption::lock_pending(&context, &identity).is_err());
        }

        #[test]
        fn remote_state_model_drop_or_local_error_never_resets_consuming() {
            let (context, identity) = pending_model();
            {
                let mut owner = Consumption::lock_pending(&context, &identity).unwrap();
                owner.enter().unwrap();
                owner.require_original(&context, &identity).unwrap();
            }
            assert_eq!(
                context.lock().unwrap().remote_final[&identity.intent.backup_id],
                Phase::Failed
            );
            assert!(Consumption::lock_pending(&context, &identity).is_err());
        }

        #[test]
        fn remote_state_model_late_remote_event_keeps_committed_and_denies_reuse() {
            let (context, identity) = pending_model();
            let mut owner = Consumption::lock_pending(&context, &identity).unwrap();
            owner.enter().unwrap();
            owner.commit().unwrap();
            assert_eq!(
                observe_rejection(&context, identity.intent.backup_id).unwrap(),
                RejectionObservation::RetainedConsumption
            );
            assert_eq!(
                context.lock().unwrap().remote_final[&identity.intent.backup_id],
                Phase::Committed
            );
            assert!(Consumption::lock_pending(&context, &identity).is_err());
            assert_eq!(
                context.lock().unwrap().remote_final[&identity.intent.backup_id],
                Phase::Committed
            );
        }

        #[test]
        fn remote_state_model_consuming_holds_same_mutex_until_terminal_decision() {
            use std::sync::{TryLockError, mpsc};
            let (context, identity) = pending_model();
            let cloned = Arc::clone(&context);
            let worker_identity = identity.clone();
            let (entered_tx, entered_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            let worker = std::thread::spawn(move || {
                let mut owner = Consumption::lock_pending(&cloned, &worker_identity).unwrap();
                owner.enter().unwrap();
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                owner.commit().unwrap();
            });
            entered_rx.recv().unwrap();
            let held = matches!(context.try_lock(), Err(TryLockError::WouldBlock));
            release_tx.send(()).unwrap();
            worker.join().unwrap();
            assert!(held);
            assert_eq!(
                observe_rejection(&context, identity.intent.backup_id).unwrap(),
                RejectionObservation::RetainedConsumption
            );
        }

        #[test]
        fn remote_state_model_unwind_poison_is_terminal_without_recursive_lock() {
            use std::panic::{AssertUnwindSafe, catch_unwind};
            let (context, identity) = pending_model();
            assert!(
                catch_unwind(AssertUnwindSafe(|| {
                    let mut owner = Consumption::lock_pending(&context, &identity).unwrap();
                    owner.enter().unwrap();
                    panic!("pure model local unwind");
                }))
                .is_err()
            );
            let authority = context.lock().unwrap_err().into_inner();
            assert_eq!(
                authority.remote_final[&identity.intent.backup_id],
                Phase::Failed
            );
            drop(authority);
            assert!(Consumption::lock_pending(&context, &identity).is_err());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Store {
        fail: Option<Step>,
        names: Vec<String>,
        intent: Intent,
        terminal: Option<Vec<u8>>,
        prior: Vec<u8>,
    }
    impl Store {
        fn new() -> Self {
            Self {
                fail: None,
                names: vec![Uuid::new_v4().to_string()],
                intent: Intent {
                    format_version: 1,
                    capability: "destination_evidence_attempt_v1".into(),
                    backup_id: Uuid::new_v4(),
                    attempt_id: Uuid::new_v4(),
                    destination_dev: 1,
                    destination_ino: 2,
                    evidence_dev: 1,
                    evidence_ino: 3,
                    manifest_sha256: "a".repeat(64),
                    proof_sha256: "b".repeat(64),
                },
                terminal: None,
                prior: b"previous unrelated immutable package".to_vec(),
            }
        }
    }
    impl PublicationIo for Store {
        type Output = ();
        fn step(&mut self, step: Step) -> Result<(), BackupError> {
            if self.fail == Some(step) {
                return Err(BackupError::Invalid(
                    "injected destination publication failure",
                ));
            }
            let id = self.intent.backup_id;
            match step {
                Step::Admit => {
                    require_fresh(&self.names, id)?;
                    self.names.push(format!("{id}.attempt"));
                }
                Step::Stage => self
                    .names
                    .push(format!("{id}.staging-{}", self.intent.attempt_id)),
                Step::Rename => {
                    self.names
                        .retain(|v| v != &format!("{id}.staging-{}", self.intent.attempt_id));
                    self.names.push(id.to_string());
                }
                Step::TerminalWrite => self.terminal = Some(canonical(&self.intent.terminal()?)?),
                _ => (),
            }
            self.names.sort();
            Ok(())
        }
        fn outcome(&mut self) -> Result<((), CommitIdentity), BackupError> {
            Ok((
                (),
                validate_committed(&canonical(&self.intent)?, self.terminal.as_ref().unwrap())?,
            ))
        }
    }
    #[test]
    fn publication_failure_revokes_prior_operation_capability_even_with_visible_terminal() {
        let context = Arc::new(Mutex::new(OperationAuthority::default()));
        let mut good = Store::new();
        publish(&context, good.intent.backup_id, &mut good).unwrap();
        let token = good.outcome().unwrap().1;
        context.lock().unwrap().require(&token).unwrap();
        let mut failed = Store::new();
        failed.fail = Some(Step::TerminalSync);
        assert!(publish(&context, failed.intent.backup_id, &mut failed).is_err());
        assert!(failed.terminal.is_some());
        assert!(context.lock().unwrap().require(&token).is_err());
    }
    #[test]
    fn every_seam_retains_residue_blocks_same_id_and_never_reconstructs_live_authority() {
        for fail in [
            Step::BeforeRename,
            Step::ParentSync,
            Step::FinalReadback,
            Step::TerminalSync,
            Step::TerminalReadback,
            Step::FinalRecheck,
        ] {
            let context = Arc::new(Mutex::new(OperationAuthority::default()));
            let mut store = Store::new();
            let prior = store.prior.clone();
            store.fail = Some(fail);
            let id = store.intent.backup_id;
            assert!(publish(&context, id, &mut store).is_err());
            assert!(require_fresh(&store.names, id).is_err());
            store.fail = None;
            let fresh = Arc::new(Mutex::new(OperationAuthority::default()));
            assert!(publish(&fresh, id, &mut store).is_err());
            assert_eq!(store.prior, prior);
            if let Some(terminal) = &store.terminal {
                let identity =
                    validate_committed(&canonical(&store.intent).unwrap(), terminal).unwrap();
                assert!(fresh.lock().unwrap().require(&identity).is_err());
            }
        }
    }
    #[test]
    fn only_original_successful_operation_and_clones_can_prepare_signing() {
        let context = Arc::new(Mutex::new(OperationAuthority::default()));
        let cloned = Arc::clone(&context);
        let mut store = Store::new();
        let id = store.intent.backup_id;
        publish(&context, id, &mut store).unwrap();
        let identity = store.outcome().unwrap().1;
        context.lock().unwrap().require(&identity).unwrap();
        cloned.lock().unwrap().require(&identity).unwrap();
        assert!(require_fresh(&store.names, id).is_err());
        require_committed_names(&store.names, id).unwrap();
        assert!(OperationAuthority::default().require(&identity).is_err());
        let mut extra = store.names.clone();
        extra.push(format!("{id}.staging-{}", Uuid::new_v4()));
        extra.sort();
        assert!(require_committed_names(&extra, id).is_err());
        assert!(publish(&context, id, &mut store).is_err());
        assert!(context.lock().unwrap().require(&identity).is_err());
        drop(context);
        drop(cloned);
        assert!(OperationAuthority::default().require(&identity).is_err());
    }
    #[test]
    fn altered_disk_identity_is_not_live_authority_and_terminal_linkage_is_exact() {
        let context = Arc::new(Mutex::new(OperationAuthority::default()));
        let mut store = Store::new();
        publish(&context, store.intent.backup_id, &mut store).unwrap();
        let original = store.outcome().unwrap().1;
        let mut changed = original.intent.clone();
        changed.proof_sha256 = "c".repeat(64);
        let changed_identity = validate_committed(
            &canonical(&changed).unwrap(),
            &canonical(&changed.terminal().unwrap()).unwrap(),
        )
        .unwrap();
        assert!(context.lock().unwrap().require(&changed_identity).is_err());
        let mut terminal = original.intent.terminal().unwrap();
        terminal.attempt_id = Uuid::new_v4();
        assert!(
            validate_committed(
                &canonical(&original.intent).unwrap(),
                &canonical(&terminal).unwrap()
            )
            .is_err()
        );
        let mut names = store.names.clone();
        names.push(format!("{}.unexpected", store.intent.backup_id));
        names.sort();
        assert!(require_committed_names(&names, store.intent.backup_id).is_err());
    }
    #[test]
    fn signing_or_receipt_error_fence_revokes_success_without_editing_disk() {
        let context = Arc::new(Mutex::new(OperationAuthority::default()));
        let mut store = Store::new();
        publish(&context, store.intent.backup_id, &mut store).unwrap();
        let identity = store.outcome().unwrap().1;
        let terminal = store.terminal.clone();
        let names = store.names.clone();
        {
            let _error = ErrorFence::new(&context);
        }
        assert!(context.lock().unwrap().require(&identity).is_err());
        assert_eq!(store.terminal, terminal);
        assert_eq!(store.names, names);
        let parsed = validate_committed(
            &canonical(&store.intent).unwrap(),
            terminal.as_ref().unwrap(),
        )
        .unwrap();
        assert_eq!(
            parsed, identity,
            "storage integrity is still readable, not new authority"
        );
    }
    #[test]
    fn final_use_refuses_revocation_after_early_read_before_signing() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            mpsc,
        };
        let context = Arc::new(Mutex::new(OperationAuthority::default()));
        let mut store = Store::new();
        publish(&context, store.intent.backup_id, &mut store).unwrap();
        let identity = store.outcome().unwrap().1;
        let entered = Arc::new(AtomicBool::new(false));
        let (early_tx, early_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let worker_context = Arc::clone(&context);
        let worker_entered = Arc::clone(&entered);
        let signer = std::thread::spawn(move || {
            worker_context.lock().unwrap().require(&identity).unwrap();
            early_tx.send(()).unwrap();
            resume_rx.recv().unwrap();
            finalize_use(&worker_context, &identity, || {
                worker_entered.store(true, Ordering::SeqCst);
                Ok(b"committed witness result".to_vec())
            })
        });
        early_rx.recv().unwrap();
        drop(ErrorFence::new(&context)); // Revocation completes before final use.
        resume_tx.send(()).unwrap();
        assert!(signer.join().unwrap().is_err());
        assert!(
            !entered.load(Ordering::SeqCst),
            "the signing closure must never run"
        );
    }
    #[test]
    fn final_use_commits_before_late_revocation_and_future_use_is_refused() {
        use std::sync::{TryLockError, mpsc};
        let context = Arc::new(Mutex::new(OperationAuthority::default()));
        let mut store = Store::new();
        publish(&context, store.intent.backup_id, &mut store).unwrap();
        let identity = store.outcome().unwrap().1;
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker_context = Arc::clone(&context);
        let worker_identity = identity.clone();
        let signer = std::thread::spawn(move || {
            finalize_use(&worker_context, &worker_identity, || {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(b"committed witness result".to_vec())
            })
        });
        entered_rx.recv().unwrap();
        let held = matches!(context.try_lock(), Err(TryLockError::WouldBlock));
        let revoke_context = Arc::clone(&context);
        let (ready_tx, ready_rx) = mpsc::channel();
        let revoker = std::thread::spawn(move || {
            ready_tx.send(()).unwrap();
            drop(ErrorFence::new(&revoke_context));
        });
        ready_rx.recv().unwrap();
        release_tx.send(()).unwrap();
        let result = signer.join().unwrap();
        revoker.join().unwrap();
        assert!(
            held,
            "the actual consuming closure owns the shared authority mutex"
        );
        assert_eq!(result.unwrap(), b"committed witness result");
        assert!(
            finalize_use::<()>(&context, &identity, || panic!("revoked future use escaped"))
                .is_err()
        );
    }
    #[test]
    fn final_use_error_invalidates_and_releases_before_outer_error_fence() {
        let context = Arc::new(Mutex::new(OperationAuthority::default()));
        let mut store = Store::new();
        publish(&context, store.intent.backup_id, &mut store).unwrap();
        let identity = store.outcome().unwrap().1;
        let result = {
            let _outer = ErrorFence::new(&context);
            finalize_use::<()>(&context, &identity, || {
                Err(BackupError::Invalid("controlled signing failure"))
            })
        };
        assert!(result.is_err());
        assert!(
            context
                .try_lock()
                .expect("guard released without reentrant locking")
                .require(&identity)
                .is_err()
        );
        assert!(
            finalize_use::<()>(&context, &identity, || panic!("failed operation escaped")).is_err()
        );
    }
    #[test]
    fn final_use_unwind_invalidates_and_releases_before_outer_error_fence() {
        use std::panic::{AssertUnwindSafe, catch_unwind};
        use std::sync::TryLockError;
        let context = Arc::new(Mutex::new(OperationAuthority::default()));
        let mut store = Store::new();
        publish(&context, store.intent.backup_id, &mut store).unwrap();
        let identity = store.outcome().unwrap().1;
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _outer = ErrorFence::new(&context);
            finalize_use::<()>(&context, &identity, || panic!("controlled signing unwind"))
        }));
        assert!(result.is_err());
        {
            let authority = match context.try_lock() {
                Ok(authority) => authority,
                Err(TryLockError::Poisoned(poison)) => poison.into_inner(),
                Err(TryLockError::WouldBlock) => panic!("unwind retained operation lock"),
            };
            assert!(authority.require(&identity).is_err());
        }
        assert!(
            finalize_use::<()>(&context, &identity, || panic!("poisoned operation escaped"))
                .is_err()
        );
    }

    fn task9_pre_action_failure(step: Step, admission_persisted: bool) {
        let context = Arc::new(Mutex::new(OperationAuthority::default()));
        let mut store = Store::new();
        let id = store.intent.backup_id;
        let names_before = store.names.clone();
        let prior_before = store.prior.clone();
        store.fail = Some(step);

        assert!(publish(&context, id, &mut store).is_err());
        let original = context.lock().unwrap();
        assert!(original.failed);
        assert!(original.successful.is_empty());
        assert!(original.require_id(id).is_err());
        drop(original);
        assert_eq!(store.prior, prior_before);

        if !admission_persisted {
            // Failure precedes the Store's Admit action. The failed operation
            // is closed, but it must not invent a durable same-ID residue.
            assert_eq!(store.names, names_before);
            assert!(store.terminal.is_none());
            assert!(require_fresh(&store.names, id).is_ok());
            return;
        }

        assert!(require_fresh(&store.names, id).is_err());
        let retained_names = store.names.clone();
        let retained_terminal = store.terminal.clone();
        let previous_attempt = store.intent.attempt_id;
        store.intent.attempt_id = Uuid::new_v4();
        assert_ne!(store.intent.attempt_id, previous_attempt);
        store.fail = None;
        let reopened = Arc::new(Mutex::new(OperationAuthority::default()));
        // A fresh attempt_id cannot evade persisted admission for backup_id.
        assert!(publish(&reopened, id, &mut store).is_err());
        assert!(reopened.lock().unwrap().require_id(id).is_err());
        assert_eq!(store.names, retained_names);
        assert_eq!(store.terminal, retained_terminal);
        assert_eq!(store.prior, prior_before);
    }

    #[test]
    fn task9_admit_prewrite_failure_keeps_disk_identity_unconsumed() {
        task9_pre_action_failure(Step::Admit, false);
    }

    #[test]
    fn task9_prepare_failure_preserves_admission_and_rejects_fresh_attempt() {
        task9_pre_action_failure(Step::Prepare, true);
    }

    #[test]
    fn task9_stage_failure_preserves_admission_and_rejects_fresh_attempt() {
        task9_pre_action_failure(Step::Stage, true);
    }

    #[test]
    fn task9_rename_failure_retains_stage_and_rejects_fresh_attempt() {
        task9_pre_action_failure(Step::Rename, true);
    }

    #[test]
    fn task9_terminal_write_preaction_failure_retains_final_without_authority() {
        task9_pre_action_failure(Step::TerminalWrite, true);
    }

    enum Task9Outcome {
        Error,
        WrongBackup,
        Revoke,
        Unwind,
    }

    struct Task9OutcomeIo {
        store: Store,
        context: Arc<Mutex<OperationAuthority>>,
        mode: Task9Outcome,
    }

    impl PublicationIo for Task9OutcomeIo {
        type Output = ();

        fn step(&mut self, step: Step) -> Result<(), BackupError> {
            self.store.step(step)
        }

        fn outcome(&mut self) -> Result<((), CommitIdentity), BackupError> {
            let result = self.store.outcome()?;
            match self.mode {
                Task9Outcome::Error => Err(BackupError::Invalid("controlled outcome error")),
                Task9Outcome::WrongBackup => {
                    // A self-consistent alternate identity is still not this
                    // publication's identity. Do not alter the retained bytes.
                    let mut other = result.1.intent;
                    other.backup_id = Uuid::new_v4();
                    let identity =
                        validate_committed(&canonical(&other)?, &canonical(&other.terminal()?)?)?;
                    Ok(((), identity))
                }
                Task9Outcome::Revoke => {
                    drop(ErrorFence::new(&self.context));
                    Ok(result)
                }
                Task9Outcome::Unwind => panic!("controlled outcome unwind"),
            }
        }
    }

    fn task9_outcome_rejection(mode: Task9Outcome) {
        use std::panic::{AssertUnwindSafe, catch_unwind};

        let context = Arc::new(Mutex::new(OperationAuthority::default()));
        let cloned = Arc::clone(&context);
        let mut prior = Store::new();
        publish(&context, prior.intent.backup_id, &mut prior).unwrap();
        let prior_identity = prior.outcome().unwrap().1;
        let prior_names = prior.names.clone();
        let prior_terminal = prior.terminal.clone();
        let prior_bytes = prior.prior.clone();
        let mut io = Task9OutcomeIo {
            store: Store::new(),
            context: Arc::clone(&context),
            mode,
        };
        let id = io.store.intent.backup_id;
        let unrelated_bytes = io.store.prior.clone();
        let unwinds = matches!(io.mode, Task9Outcome::Unwind);
        let result = catch_unwind(AssertUnwindSafe(|| publish(&context, id, &mut io)));
        if unwinds {
            assert!(result.is_err());
        } else {
            assert!(result.unwrap().is_err());
        }

        // Every Store step completed, but its readable terminal is not new
        // signing authority. The earlier successful operation is revoked too.
        require_committed_names(&io.store.names, id).unwrap();
        let retained_identity = io.store.outcome().unwrap().1;
        {
            let authority = context.try_lock().expect("outcome must release the lock");
            assert!(authority.failed);
            assert!(authority.successful.is_empty());
            assert!(authority.require(&retained_identity).is_err());
            assert!(authority.require(&prior_identity).is_err());
        }
        assert!(cloned.lock().unwrap().require_id(id).is_err());
        let names = io.store.names.clone();
        let terminal = io.store.terminal.clone();
        io.store.intent.attempt_id = Uuid::new_v4();
        let fresh = Arc::new(Mutex::new(OperationAuthority::default()));
        assert!(publish(&fresh, id, &mut io.store).is_err());
        assert!(fresh.lock().unwrap().require_id(id).is_err());
        assert_eq!(io.store.names, names);
        assert_eq!(io.store.terminal, terminal);
        assert_eq!(io.store.prior, unrelated_bytes);
        assert_eq!(prior.names, prior_names);
        assert_eq!(prior.terminal, prior_terminal);
        assert_eq!(prior.prior, prior_bytes);
    }

    #[test]
    fn task9_outcome_error_revokes_authority_and_retains_committed_residue() {
        task9_outcome_rejection(Task9Outcome::Error);
    }

    #[test]
    fn task9_outcome_wrong_backup_revokes_authority_without_rewriting_residue() {
        task9_outcome_rejection(Task9Outcome::WrongBackup);
    }

    #[test]
    fn task9_outcome_revocation_wins_before_success_registration() {
        task9_outcome_rejection(Task9Outcome::Revoke);
    }

    #[test]
    fn task9_outcome_unwind_revokes_authority_and_releases_lock() {
        task9_outcome_rejection(Task9Outcome::Unwind);
    }

    #[test]
    fn task9_self_consistent_identity_substitutions_never_run_final_use() {
        let context = Arc::new(Mutex::new(OperationAuthority::default()));
        let mut store = Store::new();
        publish(&context, store.intent.backup_id, &mut store).unwrap();
        let original = store.outcome().unwrap().1;
        let names = store.names.clone();
        let terminal = store.terminal.clone();
        let prior = store.prior.clone();

        // Each substitute has a valid canonical intent/terminal linkage. An
        // integrity parser accepting it must not authorize a consuming closure.
        for field in 0..8 {
            let mut changed = original.intent.clone();
            match field {
                0 => changed.backup_id = Uuid::new_v4(),
                1 => changed.attempt_id = Uuid::new_v4(),
                2 => changed.destination_dev += 1,
                3 => changed.destination_ino += 1,
                4 => changed.evidence_dev += 1,
                5 => changed.evidence_ino += 1,
                6 => changed.manifest_sha256 = "c".repeat(64),
                7 => changed.proof_sha256 = "d".repeat(64),
                _ => unreachable!(),
            }
            let changed_identity = validate_committed(
                &canonical(&changed).unwrap(),
                &canonical(&changed.terminal().unwrap()).unwrap(),
            )
            .unwrap();
            assert_ne!(changed_identity, original);
            assert!(context.lock().unwrap().require(&changed_identity).is_err());
            context.lock().unwrap().require(&original).unwrap();

            // Use a separate original context for final-use rejection: failure
            // correctly closes that whole operation, including its prior ID.
            let isolated = Arc::new(Mutex::new(OperationAuthority::default()));
            let mut same = Store::new();
            same.intent = original.intent.clone();
            publish(&isolated, same.intent.backup_id, &mut same).unwrap();
            let mut entered = false;
            assert!(
                finalize_use(&isolated, &changed_identity, || {
                    entered = true;
                    Ok(())
                })
                .is_err()
            );
            assert!(!entered, "substituted field {field} reached final use");
            assert!(isolated.lock().unwrap().require(&original).is_err());
        }
        context.lock().unwrap().require(&original).unwrap();
        assert_eq!(store.names, names);
        assert_eq!(store.terminal, terminal);
        assert_eq!(store.prior, prior);
    }
}

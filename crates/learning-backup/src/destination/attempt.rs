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
        if self.failed || self.successful.get(&identity.intent.backup_id) != Some(identity) {
            return Err(BackupError::Invalid(
                "original destination publication operation required",
            ));
        }
        Ok(())
    }
    pub(super) fn require_id(&self, id: Uuid) -> Result<(), BackupError> {
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
}

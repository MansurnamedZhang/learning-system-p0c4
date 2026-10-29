//! Actual Capture work, using only the genuine peer from the fixed entry.
//! Control stdio/FD3 belong exclusively to learning-db's original owner.
use super::{capture_child, capture_export::OriginalB1CaptureExport};
use crate::{
    BackupError, SourceLocalPin,
    source::{
        B1PrepareOwners,
        snapshot_effects::{
            B1InstallOwners, B1QuiesceOwners, CaptureInput, CapturedSnapshotEffectsV2,
            EnrolledSnapshotBroker, prepare_source_backup_with_snapshot_effects_b1,
        },
    },
};
use learning_assets::{FsAssetStore, backup_fs::BackupDir};
use learning_db::snapshot_broker::B1InstalledSourceProcess;
use std::{path::Path, sync::Arc};

/// Concrete actual resources, stored in the original Book before any work
/// future is polled. Futures borrow these fields; they never own this state.
pub(super) struct OriginalCaptureOwners {
    assets: Option<FsAssetStore>,
    broker: Option<EnrolledSnapshotBroker>,
    install: B1InstallOwners,
    quiesce: B1QuiesceOwners,
    input: Option<CaptureInput>,
    prepare: B1PrepareOwners,
    pin: Option<SourceLocalPin>,
    effects: Option<CapturedSnapshotEffectsV2>,
    export: Option<OriginalB1CaptureExport>,
}
impl OriginalCaptureOwners {
    pub(super) fn empty() -> Self {
        Self {
            assets: None,
            broker: None,
            install: B1InstallOwners::empty(),
            quiesce: B1QuiesceOwners::empty(),
            input: None,
            prepare: B1PrepareOwners::empty(),
            pin: None,
            effects: None,
            export: None,
        }
    }
    pub(super) async fn settle_helpers_negative(&mut self) -> Result<(), BackupError> {
        let helpers = self.prepare.settle_helpers_negative().await;
        self.install.close_admissions_negative().await;
        helpers
    }
    async fn run(&mut self, peer: &Arc<B1InstalledSourceProcess>) -> Result<(), BackupError> {
        capture_child::require_original_interest(peer)?;
        if self.assets.is_some() {
            return Err(BackupError::Invalid("B1 Capture phase already entered"));
        }
        let backup_id = peer
            .capture_backup_id_data()
            .map_err(|_| BackupError::Invalid("B1 original Capture backup ID"))?;
        self.assets = Some(open_fixed_capture_assets(peer)?);
        EnrolledSnapshotBroker::install_fresh_b1(
            Arc::clone(peer),
            self.assets.as_ref().expect("stored original assets"),
            &mut self.install,
        )
        .await?;
        self.install.validate_wrapper_transfer()?;
        self.broker = Some(self.install.transfer_validated());
        capture_child::require_original_interest(peer)?;
        if !self.quiesce.is_empty() {
            return Err(BackupError::Invalid("B1 original Q0 already entered"));
        }
        self.broker
            .as_ref()
            .expect("stored original broker")
            .request_quiesce()?;
        let capture_id = uuid::Uuid::new_v4();
        // Move only the DB broker into its own consuming original Q0. The
        // wrapper's actual enrollment/listeners/FDs stay in this Book phase.
        let broker = self
            .quiesce
            .retain_wrapper(self.broker.take().expect("validated original broker"));
        self.quiesce.quiesce(broker, backup_id, capture_id).await?;
        self.quiesce.validate_input_transfer()?;
        self.input = Some(self.quiesce.transfer_validated());
        capture_child::require_original_interest(peer)?;
        prepare_source_backup_with_snapshot_effects_b1(
            self.assets.as_ref().expect("stored original assets"),
            &mut self.input,
            &mut self.prepare,
            &mut self.pin,
            &mut self.effects,
        )
        .await?;
        if self.export.is_some() || self.pin.is_none() || self.effects.is_none() {
            return Err(BackupError::Invalid("B1 original M5 transfer state"));
        }
        // Prior validation precedes this allocation-free, infallible transfer.
        self.export = Some(OriginalB1CaptureExport::retain(
            self.pin.take().expect("validated original pin"),
            self.effects.take().expect("validated original effects"),
        ));
        capture_child::require_original_interest(peer)?;
        let digest = self
            .export
            .as_mut()
            .expect("stored complete original M5")
            .publish_from_original(peer)?;
        capture_child::require_original_interest(peer)?;
        let stop = peer
            .report_captured_digest_and_wait_for_settle(digest)
            .await
            .map_err(|_| BackupError::Invalid("B1 original Capture Root notification"))?;
        self.export
            .as_ref()
            .expect("stored complete original M5")
            .require_stop_data(&stop)?;
        capture_child::require_original_interest(peer)
    }
}

pub(super) async fn run_original_capture_job(
    peer: Arc<B1InstalledSourceProcess>,
) -> Result<(), BackupError> {
    let original = capture_child::original_phase_owners(&peer)?;
    let mut owners = original.lock().await;
    owners.borrow_mut()?.run(&peer).await
}

fn open_fixed_capture_assets(peer: &B1InstalledSourceProcess) -> Result<FsAssetStore, BackupError> {
    const ASSETS: &str = "/var/lib/knowweave-source/assets";
    const STAGING: &str = "/var/lib/knowweave-source/staging";
    peer.revalidate()
        .map_err(|_| BackupError::Invalid("B1 original Capture process"))?;
    let scan = peer.scan_accounting_meter();
    scan.charge(4096, 48)?;
    // Require both pre-existing no-follow mounted roots before FsAssetStore's
    // legacy constructor, so it cannot create a missing directory. The real
    // install lookup subsequently joins their held identities to current
    // Registry RootRecords before any broker installation or source mutation.
    let assets = BackupDir::open_trusted_private_root(Path::new(ASSETS))?;
    let staging = BackupDir::open_trusted_private_root(Path::new(STAGING))?;
    if assets.identity()? == staging.identity()? {
        return Err(BackupError::Invalid("B1 Capture asset roots alias"));
    }
    let store = FsAssetStore::new(ASSETS.into(), STAGING.into())?;
    let (held_assets, held_staging) = store.backup_root_handles()?;
    if held_assets.identity()? != assets.identity()?
        || held_staging.identity()? != staging.identity()?
    {
        return Err(BackupError::Invalid("B1 Capture asset roots replaced"));
    }
    peer.revalidate()
        .map_err(|_| BackupError::Invalid("B1 original Capture process"))?;
    Ok(store)
}

use super::require_admin_owner;
use crate::BackupError;
use sqlx::{PgConnection, PgPool, Postgres, pool::PoolConnection};

/// Owns the admitted session through every SQL and filesystem phase. Closing
/// on Drop releases only session resources; it never reopens a source gate.
pub(crate) struct SourceAdmission {
    connection: AdmissionConnection,
    database: String,
}

enum AdmissionConnection {
    Legacy(PoolConnection<Postgres>),
    #[cfg(feature = "offhost-b1")]
    B1 {
        session: Option<learning_db::snapshot_broker::OwnedB1AdminSession>,
        peer: std::sync::Arc<learning_db::snapshot_broker::B1InstalledSourceProcess>,
        input: Box<super::binding::B1SourceInputs>,
        budget: std::sync::Arc<std::sync::Mutex<crate::registry::ScanBudget>>,
        null_passfile: std::fs::File,
        failed: std::sync::Arc<std::sync::atomic::AtomicBool>,
        registry_opened: std::sync::atomic::AtomicBool,
        closed: bool,
    },
}

/// Original pre-admission acquisition storage. The fixed M3 phase owns this
/// before the actual DB session or any held input descriptors are produced.
#[cfg(feature = "offhost-b1")]
pub(in crate::source) struct B1AdmissionAcquisition {
    peer: Option<std::sync::Arc<learning_db::snapshot_broker::B1InstalledSourceProcess>>,
    budget: Option<std::sync::Arc<std::sync::Mutex<crate::registry::ScanBudget>>>,
    session: Option<learning_db::snapshot_broker::OwnedB1AdminSession>,
    input: Option<Box<super::binding::B1SourceInputs>>,
    null_passfile: Option<std::fs::File>,
    database: Option<String>,
    failed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    attempted: bool,
}
#[cfg(feature = "offhost-b1")]
impl B1AdmissionAcquisition {
    pub(in crate::source) fn empty() -> Self {
        Self {
            peer: None,
            budget: None,
            session: None,
            input: None,
            null_passfile: None,
            database: None,
            failed: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            attempted: false,
        }
    }
    pub(in crate::source) async fn acquire(
        &mut self,
        peer: std::sync::Arc<learning_db::snapshot_broker::B1InstalledSourceProcess>,
    ) -> Result<(), BackupError> {
        if std::mem::replace(&mut self.attempted, true) {
            return Err(BackupError::Invalid(
                "B1 original admission acquisition already entered",
            ));
        }
        self.peer = Some(peer);
        let peer = self.peer.as_ref().expect("stored original peer");
        peer.revalidate()
            .map_err(|_| BackupError::Invalid("B1 original source process"))?;
        self.budget = Some(std::sync::Arc::new(std::sync::Mutex::new(
            crate::registry::ScanBudget::from_b1_scan(peer.scan_accounting_meter()),
        )));
        self.session = Some(
            peer.acquire_fixed_admin()
                .await
                .map_err(|_| BackupError::Invalid("B1 original admin session"))?,
        );
        let session = self.session.as_mut().expect("stored original session");
        let data = session
            .isolation_input_data()
            .map_err(|_| BackupError::Invalid("B1 current isolation input"))?;
        self.input = Some(Box::new(super::binding::B1SourceInputs::read(
            peer,
            &data,
            self.budget.as_ref().expect("stored scan"),
        )?));
        let input = self.input.as_ref().expect("stored original inputs");
        let (current, login, authenticated, database, owner, local): (String, String, Option<String>, String, String, bool) = sqlx::query_as(
            "SELECT current_user::text, session_user::text, system_user, current_database()::text, pg_get_userbyid(d.datdba)::text, pg_catalog.inet_client_addr() IS NULL AND pg_catalog.inet_server_addr() IS NULL FROM pg_catalog.pg_database d WHERE d.datname=current_database()"
        ).fetch_one(session.connection_in_original_owner()).await?;
        if current != "learning_admin"
            || login != "learning_admin"
            || owner != "learning_admin"
            || authenticated.as_deref() != Some("peer:root")
            || !local
        {
            return Err(BackupError::Invalid(
                "B1 original authenticated management owner",
            ));
        }
        if database != session.database() || database != input.database() {
            return Err(BackupError::Invalid(
                "B1 source admission database identity",
            ));
        }
        self.database = Some(database);
        let acquired: bool = sqlx::query_scalar("SELECT pg_catalog.pg_try_advisory_lock($1, $2)")
            .bind(0x4b57_4334_i32)
            .bind(0x5352_4345_i32)
            .fetch_one(session.connection_in_original_owner())
            .await?;
        if !acquired {
            return Err(BackupError::Invalid("source maintenance admission busy"));
        }
        self.null_passfile = Some(SourceAdmission::open_null_passfile(
            self.budget.as_ref().expect("stored scan"),
        )?);
        self.validate_transfer()
    }
    pub(in crate::source) fn validate_transfer(&self) -> Result<(), BackupError> {
        let peer = self
            .peer
            .as_ref()
            .ok_or(BackupError::Invalid("original peer missing"))?;
        let session = self
            .session
            .as_ref()
            .ok_or(BackupError::Invalid("original session missing"))?;
        let budget = self
            .budget
            .as_ref()
            .ok_or(BackupError::Invalid("original scan missing"))?;
        let data = session
            .isolation_input_data()
            .map_err(|_| BackupError::Invalid("B1 current isolation input"))?;
        let input = self
            .input
            .as_ref()
            .ok_or(BackupError::Invalid("original inputs missing"))?;
        input.recheck(peer, &data, budget)?;
        SourceAdmission::check_null_passfile(
            self.null_passfile
                .as_ref()
                .ok_or(BackupError::Invalid("original null passfile missing"))?,
            budget,
        )?;
        if self.database.as_deref() != Some(input.database())
            || self.database.as_deref() != Some(session.database())
        {
            return Err(BackupError::Invalid("original database changed"));
        }
        Ok(())
    }
    pub(in crate::source) fn transfer_validated(&mut self) -> SourceAdmission {
        SourceAdmission {
            database: self.database.take().expect("validated original database"),
            connection: AdmissionConnection::B1 {
                session: self.session.take(),
                peer: self.peer.take().expect("validated original peer"),
                input: self.input.take().expect("validated original inputs"),
                budget: self.budget.take().expect("validated original scan"),
                null_passfile: self
                    .null_passfile
                    .take()
                    .expect("validated original null passfile"),
                failed: std::sync::Arc::clone(&self.failed),
                registry_opened: std::sync::atomic::AtomicBool::new(false),
                closed: false,
            },
        }
    }
    pub(in crate::source) async fn close_for_cleanup(&mut self) -> Result<(), BackupError> {
        match self.session.take() {
            Some(session) => session
                .close()
                .await
                .map_err(|_| BackupError::Invalid("B1 original acquisition close negative")),
            None => Ok(()),
        }
    }
}

/// Live Install custody after the original admin session naturally closed.
/// Only the actual in-place close and its validated field transfer can construct it. Copied DATA cannot create it, and
/// it deliberately retains all current original input descriptors and budget.
#[cfg(feature = "offhost-b1")]
pub(in crate::source) struct B1RetainedInstallInputs {
    peer: std::sync::Arc<learning_db::snapshot_broker::B1InstalledSourceProcess>,
    input: Box<super::binding::B1SourceInputs>,
    budget: std::sync::Arc<std::sync::Mutex<crate::registry::ScanBudget>>,
    null_passfile: std::fs::File,
    failed: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// The same Capture session's inputs after that session really closed. It is
/// never manufactured from the earlier Install issuance or copied Root DATA.
#[cfg(feature = "offhost-b1")]
pub(in crate::source) struct B1RetainedCaptureInputs {
    peer: std::sync::Arc<learning_db::snapshot_broker::B1InstalledSourceProcess>,
    input: Box<super::binding::B1SourceInputs>,
    budget: std::sync::Arc<std::sync::Mutex<crate::registry::ScanBudget>>,
    null_passfile: std::fs::File,
    failed: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(feature = "offhost-b1")]
impl B1RetainedCaptureInputs {
    pub(in crate::source) fn require_original_peer(
        &self,
        peer: &std::sync::Arc<learning_db::snapshot_broker::B1InstalledSourceProcess>,
    ) -> Result<(), BackupError> {
        if !std::sync::Arc::ptr_eq(peer, &self.peer) {
            self.failed
                .store(true, std::sync::atomic::Ordering::Release);
            return Err(BackupError::Invalid("B1 retained Capture process changed"));
        }
        self.recheck()
    }
    pub(in crate::source) fn recheck(&self) -> Result<(), BackupError> {
        use std::sync::atomic::Ordering;
        if self.failed.load(Ordering::Acquire) {
            return Err(BackupError::Invalid(
                "B1 retained Capture inputs permanently failed",
            ));
        }
        let observed = (|| {
            self.input.capture_identity_data()?;
            self.input.recheck_retained(&self.peer, &self.budget)?;
            SourceAdmission::check_null_passfile(&self.null_passfile, &self.budget)
        })();
        if observed.is_err() {
            self.failed.store(true, Ordering::Release);
        }
        observed
    }
    pub(in crate::source) fn identity_data(
        &self,
    ) -> Result<(uuid::Uuid, uuid::Uuid, [u8; 32], [u8; 32]), BackupError> {
        self.recheck()?;
        self.input.capture_identity_data()
    }
}

#[cfg(feature = "offhost-b1")]
impl B1RetainedInstallInputs {
    pub(in crate::source) fn peer(
        &self,
    ) -> &std::sync::Arc<learning_db::snapshot_broker::B1InstalledSourceProcess> {
        &self.peer
    }
    pub(in crate::source) fn budget(
        &self,
    ) -> &std::sync::Arc<std::sync::Mutex<crate::registry::ScanBudget>> {
        &self.budget
    }
    pub(in crate::source) fn record_result<T>(
        &self,
        observed: Result<T, BackupError>,
    ) -> Result<T, BackupError> {
        use std::sync::atomic::Ordering;
        if observed.is_err() {
            self.failed.store(true, Ordering::Release);
        }
        if observed.is_ok() && self.failed.load(Ordering::Acquire) {
            return Err(BackupError::Invalid(
                "B1 retained Install inputs permanently failed",
            ));
        }
        observed
    }
    pub(in crate::source) fn recheck(&self) -> Result<(), BackupError> {
        use std::sync::atomic::Ordering;
        if self.failed.load(Ordering::Acquire) {
            return Err(BackupError::Invalid(
                "B1 retained Install inputs permanently failed",
            ));
        }
        let observed = (|| {
            self.input.recheck_retained(&self.peer, &self.budget)?;
            SourceAdmission::check_null_passfile(&self.null_passfile, &self.budget)
        })();
        if observed.is_err() {
            self.failed.store(true, Ordering::Release);
        }
        observed
    }
    pub(in crate::source) fn recheck_enrollment(
        &self,
        lease: &crate::registry::RegistryLease,
        enrolled: &crate::registry::EnrolledSource,
    ) -> Result<(), BackupError> {
        self.recheck()?;
        crate::registry::require_shared_scan_budget(&self.budget, &lease.budget)?;
        let observed = self.input.require_enrollment(lease, enrolled);
        if observed.is_err() {
            self.failed
                .store(true, std::sync::atomic::Ordering::Release);
        }
        observed?;
        self.recheck()
    }
    pub(in crate::source) fn install_observation(
        &self,
        broker: &learning_db::snapshot_broker::RootSnapshotBroker,
    ) -> Result<super::binding::B1InstallCutoverObservation, BackupError> {
        self.recheck()?;
        let observed = self.input.install_observation(
            broker.descriptor_bytes(),
            broker.epoch_id(),
            broker.binding_id(),
            &self.budget,
        );
        if observed.is_err() {
            self.failed
                .store(true, std::sync::atomic::Ordering::Release);
        }
        self.recheck()?;
        observed
    }
    pub(in crate::source) fn recheck_enrollment_for_capture(
        &self,
        lease: &crate::registry::RegistryLease,
        enrolled: &crate::registry::EnrolledSource,
    ) -> Result<(), BackupError> {
        self.recheck()?;
        crate::registry::require_fresh_b1_wrapper_same_backing(&self.budget, &lease.budget)?;
        // A new Capture admission has a fresh wrapper on the same cumulative
        // backing. Never relax the original within-lease Arc::ptr_eq rule.
        let observed = self.input.require_enrollment(lease, enrolled);
        if observed.is_err() {
            self.failed
                .store(true, std::sync::atomic::Ordering::Release);
        }
        observed?;
        self.recheck()
    }
}

impl SourceAdmission {
    pub(crate) async fn try_acquire(
        pool: &PgPool,
        expected_database: &str,
    ) -> Result<Self, BackupError> {
        let mut connection = pool.acquire().await?;
        // Set before the first fallible SQL operation: errors/cancellation must
        // never return an admitted (or unverified) session to the pool.
        connection.close_on_drop();
        let database = require_admin_owner(&mut connection).await?;
        if database != expected_database {
            return Err(BackupError::Invalid("source admission database identity"));
        }
        let acquired: bool = sqlx::query_scalar("SELECT pg_catalog.pg_try_advisory_lock($1, $2)")
            .bind(0x4b57_4334_i32)
            .bind(0x5352_4345_i32)
            .fetch_one(&mut *connection)
            .await?;
        if !acquired {
            return Err(BackupError::Invalid("source maintenance admission busy"));
        }
        Ok(Self {
            connection: AdmissionConnection::Legacy(connection),
            database,
        })
    }

    pub(crate) fn revalidate(&self) -> Result<(), BackupError> {
        match &self.connection {
            AdmissionConnection::Legacy(_) => Ok(()),
            #[cfg(feature = "offhost-b1")]
            AdmissionConnection::B1 {
                session,
                peer,
                input,
                budget,
                null_passfile,
                failed,
                ..
            } => {
                use std::sync::atomic::Ordering;
                if failed.load(Ordering::Acquire) {
                    return Err(BackupError::Invalid(
                        "B1 source admission permanently failed",
                    ));
                }
                let observed = (|| {
                    peer.revalidate()
                        .map_err(|_| BackupError::Invalid("B1 original source process"))?;
                    let data = session
                        .as_ref()
                        .ok_or(BackupError::Invalid("B1 original session already consumed"))?
                        .isolation_input_data()
                        .map_err(|_| BackupError::Invalid("B1 current isolation input"))?;
                    input.recheck(peer, &data, budget)?;
                    Self::check_null_passfile(null_passfile, budget)
                })();
                if observed.is_err() {
                    failed.store(true, Ordering::Release);
                }
                observed
            }
        }
    }

    #[cfg(feature = "offhost-b1")]
    pub(crate) fn b1_budget(
        &self,
    ) -> Result<&std::sync::Arc<std::sync::Mutex<crate::registry::ScanBudget>>, BackupError> {
        match &self.connection {
            AdmissionConnection::B1 { budget, failed, .. } => {
                if failed.load(std::sync::atomic::Ordering::Acquire) {
                    return Err(BackupError::Invalid(
                        "B1 source admission permanently failed",
                    ));
                }
                Ok(budget)
            }
            AdmissionConnection::Legacy(_) => {
                Err(BackupError::Invalid("B1 original admission required"))
            }
        }
    }

    #[cfg(feature = "offhost-b1")]
    fn checked_b1<T>(
        &self,
        observe: impl FnOnce(
            &super::binding::B1SourceInputs,
            &std::sync::Arc<std::sync::Mutex<crate::registry::ScanBudget>>,
        ) -> Result<T, BackupError>,
    ) -> Result<T, BackupError> {
        match &self.connection {
            AdmissionConnection::B1 {
                input,
                budget,
                failed,
                ..
            } => {
                use std::sync::atomic::Ordering;
                if failed.load(Ordering::Acquire) {
                    return Err(BackupError::Invalid(
                        "B1 source admission permanently failed",
                    ));
                }
                let observed = observe(input, budget);
                if observed.is_err() {
                    failed.store(true, Ordering::Release);
                }
                observed
            }
            AdmissionConnection::Legacy(_) => {
                Err(BackupError::Invalid("B1 original admission required"))
            }
        }
    }

    #[cfg(feature = "offhost-b1")]
    pub(crate) fn take_b1_registry_scope(
        &self,
    ) -> Result<
        (
            std::sync::Arc<std::sync::Mutex<crate::registry::ScanBudget>>,
            std::sync::Arc<std::sync::atomic::AtomicBool>,
        ),
        BackupError,
    > {
        self.revalidate()?;
        match &self.connection {
            AdmissionConnection::B1 {
                budget,
                failed,
                registry_opened,
                ..
            } => {
                use std::sync::atomic::Ordering;
                if registry_opened.swap(true, Ordering::AcqRel) {
                    failed.store(true, Ordering::Release);
                    return Err(BackupError::Invalid(
                        "B1 registry already opened for original admission",
                    ));
                }
                Ok((std::sync::Arc::clone(budget), std::sync::Arc::clone(failed)))
            }
            AdmissionConnection::Legacy(_) => {
                Err(BackupError::Invalid("B1 original admission required"))
            }
        }
    }

    /// A failed B1 observation is sticky for this exact admission. The fixed
    /// entry still must await peer.close_original in its original finally;
    /// this local flag is no substitute for that process-owner settlement.
    pub(crate) fn record_result<T>(
        &self,
        observed: Result<T, BackupError>,
    ) -> Result<T, BackupError> {
        #[cfg(feature = "offhost-b1")]
        if let AdmissionConnection::B1 { failed, .. } = &self.connection {
            if observed.is_err() {
                failed.store(true, std::sync::atomic::Ordering::Release);
            }
            if observed.is_ok() && failed.load(std::sync::atomic::Ordering::Acquire) {
                return Err(BackupError::Invalid(
                    "B1 source admission permanently failed",
                ));
            }
        }
        observed
    }

    #[cfg(feature = "offhost-b1")]
    pub(super) fn require_b1_claim(
        &self,
        claim: super::binding::B1ClaimRefs<'_>,
    ) -> Result<(), BackupError> {
        self.revalidate()?;
        self.checked_b1(|input, _| input.require_claim(claim))
    }

    #[cfg(feature = "offhost-b1")]
    pub(super) fn b1_configuration(
        &self,
        enrolled: &crate::registry::EnrolledSource,
    ) -> Result<super::SourceBackupConfig, BackupError> {
        self.revalidate()?;
        self.checked_b1(|input, _| input.configuration(enrolled))
    }
    #[cfg(feature = "offhost-b1")]
    pub(super) fn require_same_b1_capture_peer(
        &self,
        original: &std::sync::Arc<learning_db::snapshot_broker::B1InstalledSourceProcess>,
    ) -> Result<(), BackupError> {
        self.revalidate()?;
        let observed = match &self.connection {
            AdmissionConnection::B1 { peer, input, .. }
                if std::sync::Arc::ptr_eq(peer, original) && input.phase() == "Capture" =>
            {
                Ok(())
            }
            _ => Err(BackupError::Invalid(
                "B1 original Capture process Arc or generation changed",
            )),
        };
        self.record_result(observed)
    }

    #[cfg(feature = "offhost-b1")]
    pub(super) fn b1_isolation(
        &self,
        config: &super::SourceBackupConfig,
        require_fresh: bool,
    ) -> Result<super::endpoint::VerifiedSourceIsolation, BackupError> {
        self.revalidate()?;
        let observed =
            self.checked_b1(|input, budget| input.isolation(config, require_fresh, budget));
        self.revalidate()?;
        observed
    }

    #[cfg(feature = "offhost-b1")]
    pub(crate) fn require_b1_enrollment(
        &self,
        lease: &crate::registry::RegistryLease,
        enrolled: &crate::registry::EnrolledSource,
    ) -> Result<(), BackupError> {
        self.revalidate()?;
        self.checked_b1(|input, _| input.require_enrollment(lease, enrolled))
    }

    #[cfg(feature = "offhost-b1")]
    pub(super) fn read_b1_control_root(
        &self,
        path: &std::path::Path,
    ) -> Result<Option<(learning_assets::backup_fs::BackupDir, Vec<u8>)>, BackupError> {
        match &self.connection {
            AdmissionConnection::Legacy(_) => Ok(None),
            AdmissionConnection::B1 { .. } => {
                self.revalidate()?;
                self.checked_b1(|input, budget| input.read_control_root(path, budget))
                    .map(Some)
            }
        }
    }

    pub(super) fn is_b1(&self) -> bool {
        match &self.connection {
            AdmissionConnection::Legacy(_) => false,
            #[cfg(feature = "offhost-b1")]
            AdmissionConnection::B1 { .. } => true,
        }
    }

    #[cfg(feature = "offhost-b1")]
    pub(super) fn b1_failure_flag(
        &self,
    ) -> Result<std::sync::Arc<std::sync::atomic::AtomicBool>, BackupError> {
        match &self.connection {
            AdmissionConnection::B1 { failed, .. } => Ok(std::sync::Arc::clone(failed)),
            AdmissionConnection::Legacy(_) => {
                Err(BackupError::Invalid("B1 original admission required"))
            }
        }
    }

    pub(super) fn prepare_fixed_tool(
        &self,
        command: &mut tokio::process::Command,
    ) -> Result<(), BackupError> {
        self.revalidate()?;
        if self.is_b1() {
            command
                .env("PGCONNECT_TIMEOUT", "5")
                .env("PGPASSFILE", "/dev/null")
                .env("PGSSLMODE", "disable");
        }
        Ok(())
    }

    #[cfg(feature = "offhost-b1")]
    fn open_null_passfile(
        budget: &std::sync::Arc<std::sync::Mutex<crate::registry::ScanBudget>>,
    ) -> Result<std::fs::File, BackupError> {
        use std::os::unix::fs::OpenOptionsExt;
        budget
            .lock()
            .map_err(|_| BackupError::Invalid("B1 scan budget"))?
            .entries(3)?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open("/dev/null")?;
        Self::check_null_passfile(&file, budget)?;
        Ok(file)
    }

    #[cfg(feature = "offhost-b1")]
    fn check_null_passfile(
        file: &std::fs::File,
        budget: &std::sync::Arc<std::sync::Mutex<crate::registry::ScanBudget>>,
    ) -> Result<(), BackupError> {
        use std::os::unix::fs::{FileTypeExt, MetadataExt};
        budget
            .lock()
            .map_err(|_| BackupError::Invalid("B1 scan budget"))?
            .entries(2)?;
        let held = file.metadata()?;
        let named = std::fs::symlink_metadata("/dev/null")?;
        if !held.file_type().is_char_device()
            || !named.file_type().is_char_device()
            || held.rdev() != libc::makedev(1, 3)
            || named.rdev() != held.rdev()
            || (named.dev(), named.ino()) != (held.dev(), held.ino())
        {
            return Err(BackupError::Invalid(
                "B1 fixed null passfile character identity",
            ));
        }
        Ok(())
    }

    pub(crate) fn database(&self) -> &str {
        &self.database
    }

    pub(crate) fn connection(&mut self) -> &mut PgConnection {
        match &mut self.connection {
            AdmissionConnection::Legacy(connection) => connection,
            #[cfg(feature = "offhost-b1")]
            AdmissionConnection::B1 { session, .. } => session
                .as_mut()
                .expect("active original B1 session")
                .connection_in_original_owner(),
        }
    }

    pub(crate) async fn close(self) -> Result<(), BackupError> {
        match self.connection {
            AdmissionConnection::Legacy(connection) => connection.close().await?,
            #[cfg(feature = "offhost-b1")]
            AdmissionConnection::B1 {
                session,
                peer,
                input,
                budget,
                null_passfile,
                failed,
                ..
            } => {
                use std::sync::atomic::Ordering;
                // Qualification poison never blocks original resource cleanup.
                // Retain the same failure Arc and every input while close runs;
                // resource success cannot revive a locally failed admission.
                let failed_before_close = failed.load(Ordering::Acquire);
                let closed = session
                    .ok_or(BackupError::Invalid("B1 original session already consumed"))?
                    .close()
                    .await;
                let checked = peer.revalidate();
                let failed_after_close = failed.load(Ordering::Acquire);
                drop((input, budget, peer, null_passfile));
                closed.map_err(|_| BackupError::Invalid("B1 original admin close"))?;
                if failed_before_close || failed_after_close {
                    return Err(BackupError::Invalid(
                        "B1 source admission permanently failed before or during close",
                    ));
                }
                checked.map_err(|_| BackupError::Invalid("B1 source process after close"))?;
            }
        }
        Ok(())
    }

    /// Only the DB wrapper moves into its already rooted DB close. All backup
    /// inputs stay in this externally retained admission through that await.
    #[cfg(feature = "offhost-b1")]
    async fn close_b1_current_in_place(&mut self) -> Result<(), BackupError> {
        self.revalidate()?;
        match &mut self.connection {
            AdmissionConnection::B1 {
                session,
                failed,
                closed,
                ..
            } => {
                let actual = session
                    .take()
                    .ok_or(BackupError::Invalid("B1 original close already attempted"))?;
                match actual.close().await {
                    Ok(()) => {
                        *closed = true;
                        Ok(())
                    }
                    Err(_) => {
                        failed.store(true, std::sync::atomic::Ordering::Release);
                        Err(BackupError::Invalid("B1 original close unknown"))
                    }
                }
            }
            AdmissionConnection::Legacy(_) => {
                Err(BackupError::Invalid("B1 original admission required"))
            }
        }
    }
    #[cfg(feature = "offhost-b1")]
    pub(in crate::source) async fn close_for_cleanup_in_place(
        &mut self,
    ) -> Result<(), BackupError> {
        match &mut self.connection {
            AdmissionConnection::B1 {
                session, failed, ..
            } => {
                failed.store(true, std::sync::atomic::Ordering::Release);
                match session.take() {
                    Some(session) => session
                        .close()
                        .await
                        .map_err(|_| BackupError::Invalid("B1 original cleanup close negative")),
                    None => Ok(()),
                }
            }
            AdmissionConnection::Legacy(_) => {
                Err(BackupError::Invalid("B1 original cleanup required"))
            }
        }
    }
    #[cfg(feature = "offhost-b1")]
    pub(in crate::source) async fn close_install_in_place(&mut self) -> Result<(), BackupError> {
        self.checked_b1(|input, _| {
            if input.phase() == "Install" {
                Ok(())
            } else {
                Err(BackupError::Invalid("B1 original Install phase required"))
            }
        })?;
        self.close_b1_current_in_place().await
    }
    #[cfg(feature = "offhost-b1")]
    pub(in crate::source) async fn close_capture_in_place(&mut self) -> Result<(), BackupError> {
        self.checked_b1(|input, _| {
            if input.phase() == "Capture" {
                Ok(())
            } else {
                Err(BackupError::Invalid("B1 original Capture phase required"))
            }
        })?;
        self.close_b1_current_in_place().await
    }
    #[cfg(feature = "offhost-b1")]
    pub(in crate::source) fn validate_closed_inputs(&self) -> Result<(), BackupError> {
        match &self.connection {
            AdmissionConnection::B1 {
                session,
                closed,
                peer,
                input,
                budget,
                null_passfile,
                failed,
                ..
            } if *closed
                && session.is_none()
                && !failed.load(std::sync::atomic::Ordering::Acquire) =>
            {
                input.recheck_retained(peer, budget)?;
                Self::check_null_passfile(null_passfile, budget)
            }
            _ => Err(BackupError::Invalid(
                "B1 original inputs not naturally closed",
            )),
        }
    }
    #[cfg(feature = "offhost-b1")]
    pub(in crate::source) fn transfer_closed_install(self) -> B1RetainedInstallInputs {
        match self.connection {
            AdmissionConnection::B1 {
                peer,
                input,
                budget,
                null_passfile,
                failed,
                ..
            } => B1RetainedInstallInputs {
                peer,
                input,
                budget,
                null_passfile,
                failed,
            },
            AdmissionConnection::Legacy(_) => unreachable!("validated original Install admission"),
        }
    }
    #[cfg(feature = "offhost-b1")]
    pub(in crate::source) fn transfer_closed_capture(self) -> B1RetainedCaptureInputs {
        match self.connection {
            AdmissionConnection::B1 {
                peer,
                input,
                budget,
                null_passfile,
                failed,
                ..
            } => B1RetainedCaptureInputs {
                peer,
                input,
                budget,
                null_passfile,
                failed,
            },
            AdmissionConnection::Legacy(_) => unreachable!("validated original Capture admission"),
        }
    }

    /// Negative resource cleanup is independent of qualification. In
    /// particular a poisoned ctx must not prevent this original close await.
    #[cfg(feature = "offhost-b1")]
    pub(in crate::source) async fn close_b1_negative(self) -> Result<(), BackupError> {
        match self.connection {
            AdmissionConnection::Legacy(connection) => connection.close().await?,
            AdmissionConnection::B1 {
                session,
                peer,
                input,
                budget,
                null_passfile,
                failed,
                ..
            } => {
                failed.store(true, std::sync::atomic::Ordering::Release);
                let closed = session
                    .ok_or(BackupError::Invalid("B1 original session already consumed"))?
                    .close()
                    .await;
                drop((peer, input, budget, null_passfile));
                closed.map_err(|_| BackupError::Invalid("B1 original negative admin close"))?;
            }
        }
        Ok(())
    }
}

/// Publication is deliberately separate from the Install/Capture admission.
/// This original slot lives in Source Book before session acquisition starts;
/// dropping a borrowed future never drops this session or any unknown owner.
#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
pub(crate) struct B1PublicationOwners {
    session: Option<learning_db::snapshot_broker::OwnedB1AdminSession>,
    input: Option<Box<super::binding::B1PublicationInputs>>,
    budget: std::sync::Arc<std::sync::Mutex<crate::registry::ScanBudget>>,
    failed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    registry_opened: std::sync::atomic::AtomicBool,
    null_passfile: Option<std::fs::File>,
    database: Option<String>,
    postmaster: Option<String>,
    attempted: bool,
    closed: bool,
    close_outcome: Option<bool>,
}
#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
impl B1PublicationOwners {
    pub(crate) fn original_slot(
        original: &'static crate::offhost_b1::SourceFrontOriginal,
    ) -> Result<Self, BackupError> {
        let (budget, failed) = original.take_publication_scope()?;
        // All fixed storage and Arc/Box allocations were reserved before the
        // new scope was allocated, against the same genuine process backing.
        Ok(Self {
            session: None,
            input: None,
            budget,
            failed,
            registry_opened: std::sync::atomic::AtomicBool::new(false),
            null_passfile: None,
            database: None,
            postmaster: None,
            attempted: false,
            closed: false,
            close_outcome: None,
        })
    }
    pub(crate) async fn acquire_in_place(
        &mut self,
        original: &'static crate::offhost_b1::SourceFrontOriginal,
    ) -> Result<(), BackupError> {
        if std::mem::replace(&mut self.attempted, true) {
            original.reject();
            return Err(publication_refused());
        }
        original.revalidate()?;
        // The real wrapper is assigned directly into the rooted original slot
        // before any local metadata read, SQL, allocation, or positive check.
        self.session = Some(original.acquire_publication_admin().await?);
        self.input = Some(Box::new(original.read_publication_inputs(&self.budget)?));
        original.bind_publication_root_clock(
            self.input.as_ref().expect("stored Publication inputs"),
            &self.budget,
        )?;
        self.null_passfile = Some(SourceAdmission::open_null_passfile(&self.budget)?);
        self.reserve_sql()?;
        let session = self.session.as_mut().expect("stored Publication session");
        let row = publication_sql(original, session.connection_in_original_owner()).await?;
        publication_owner_row(&row)?;
        let input = self.input.as_ref().expect("stored Publication inputs");
        input.require_database(&row.3, row.6, &row.7)?;
        if row.3 != session.database() {
            return Err(publication_refused());
        }
        self.database = Some(row.3);
        self.postmaster = Some(row.8);
        self.reserve_sql()?;
        let acquired = publication_acquire_lock(
            original,
            self.session
                .as_mut()
                .expect("stored Publication session")
                .connection_in_original_owner(),
        )
        .await?;
        if !acquired {
            return Err(BackupError::Invalid("source maintenance admission busy"));
        }
        B1PublicationAdmission {
            original,
            owners: self,
        }
        .revalidate()
    }
    fn reserve_sql(&self) -> Result<(), BackupError> {
        let mut scan = self.budget.lock().map_err(|_| publication_refused())?;
        scan.charge(16384)?;
        scan.entries(12)
    }
    pub(crate) async fn close_in_place(&mut self) -> Result<(), BackupError> {
        if self.closed {
            return if self.close_outcome == Some(true) {
                Ok(())
            } else {
                Err(publication_refused())
            };
        }
        self.closed = true;
        let observed = match self.session.take() {
            Some(session) => session.close().await.map_err(|_| publication_refused()),
            None if !self.attempted => Ok(()),
            None => Err(publication_refused()),
        };
        // A cancelled/unwound close leaves None: never reconstruct a wrapper
        // or label that unknown close naturally settled from the started bit.
        self.close_outcome = Some(observed.is_ok());
        observed
    }
}

/// Borrowing this private original slot creates no S. No clone, serde,
/// caller-selected connection, path, raw FD, budget, phase or constructor.
#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
pub(crate) struct B1PublicationAdmission<'a> {
    original: &'static crate::offhost_b1::SourceFrontOriginal,
    owners: &'a mut B1PublicationOwners,
}
#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
impl<'a> B1PublicationAdmission<'a> {
    pub(crate) fn borrow_original(
        original: &'static crate::offhost_b1::SourceFrontOriginal,
        owners: &'a mut B1PublicationOwners,
    ) -> Result<Self, BackupError> {
        let admission = Self { original, owners };
        admission.revalidate()?;
        Ok(admission)
    }
    pub(crate) fn revalidate(&self) -> Result<(), BackupError> {
        use std::sync::atomic::Ordering;
        let observed = (|| {
            if self.owners.failed.load(Ordering::Acquire) || self.owners.closed {
                return Err(publication_refused());
            }
            self.original.revalidate()?;
            self.original.first_s_clock()?.check()?;
            let input = self.input()?;
            self.original
                .recheck_publication_inputs(input, &self.owners.budget)?;
            SourceAdmission::check_null_passfile(
                self.owners
                    .null_passfile
                    .as_ref()
                    .ok_or_else(publication_refused)?,
                &self.owners.budget,
            )?;
            if self
                .owners
                .session
                .as_ref()
                .ok_or_else(publication_refused)?
                .database()
                != input.database()
                || self.owners.database.as_deref() != Some(input.database())
            {
                return Err(publication_refused());
            }
            self.original.first_s_clock()?.check()?;
            self.original.revalidate()
        })();
        if observed.is_err() {
            self.fail();
        }
        observed
    }
    pub(crate) async fn revalidate_sql(&mut self) -> Result<(), BackupError> {
        self.revalidate()?;
        self.owners.reserve_sql()?;
        let original = self.original;
        let observed = async {
            let row = publication_sql(original, self.connection()).await?;
            publication_owner_row(&row)?;
            self.input()?.require_database(&row.3, row.6, &row.7)?;
            if self.owners.database.as_deref() != Some(row.3.as_str())
                || self.owners.postmaster.as_deref() != Some(row.8.as_str())
            {
                return Err(publication_refused());
            }
            self.owners.reserve_sql()?;
            let locked = publication_lock_observation(original, self.connection()).await?;
            if !locked {
                return Err(publication_refused());
            }
            self.revalidate()
        }
        .await;
        if observed.is_err() {
            self.fail();
        }
        observed
    }
    pub(crate) async fn require_manifest_source(
        &mut self,
        manifest: &crate::BackupManifestV1,
    ) -> Result<(), BackupError> {
        self.revalidate_sql().await?;
        let maximum = super::MIGRATOR.iter().count();
        {
            let mut scan = self.budget().lock().map_err(|_| publication_refused())?;
            scan.charge(
                (maximum as u64)
                    .checked_mul(2048)
                    .and_then(|n| n.checked_add(16384))
                    .ok_or(BackupError::Overflow)?,
            )?;
            scan.entries(maximum.checked_add(8).ok_or(BackupError::Overflow)?)?;
        }
        let original = self.original;
        let observed = async {
            let version = publication_version(original, self.connection()).await?;
            if version.parse::<u32>().ok().map(|v| v / 10000) != Some(18) {
                return Err(publication_refused());
            }
            let rows = publication_migration_rows(original, self.connection()).await?;
            if rows.len() != maximum {
                return Err(publication_refused());
            }
            let mut migrations = Vec::new();
            migrations
                .try_reserve_exact(maximum)
                .map_err(|_| publication_refused())?;
            for ((version, bytes, checksum, success), embedded) in
                rows.into_iter().zip(super::MIGRATOR.iter())
            {
                if !success
                    || bytes != 48
                    || version != embedded.version
                    || checksum != hex::encode(embedded.checksum.as_ref())
                {
                    return Err(publication_refused());
                }
                migrations.push(crate::MigrationRecord {
                    version: u64::try_from(version).map_err(|_| BackupError::Overflow)?,
                    checksum_hex: checksum,
                });
            }
            let source = crate::SourceIdentity::from_migrations(
                super::compiled_build_id(option_env!("KNOWWEAVE_BUILD_ID_SHA256"))?,
                option_env!("KNOWWEAVE_SOURCE_COMMIT")
                    .ok_or_else(publication_refused)?
                    .into(),
                18,
                migrations,
            )?;
            if source != manifest.source {
                return Err(publication_refused());
            }
            self.revalidate_sql().await
        }
        .await;
        if observed.is_err() {
            self.fail();
        }
        observed
    }
    pub(crate) fn fail(&self) {
        self.owners
            .failed
            .store(true, std::sync::atomic::Ordering::Release);
        self.original.reject();
    }
    pub(crate) fn input(&self) -> Result<&super::binding::B1PublicationInputs, BackupError> {
        self.owners.input.as_deref().ok_or_else(publication_refused)
    }
    pub(crate) fn budget(&self) -> &std::sync::Arc<std::sync::Mutex<crate::registry::ScanBudget>> {
        &self.owners.budget
    }
    pub(crate) fn postmaster(&self) -> Result<&str, BackupError> {
        self.owners
            .postmaster
            .as_deref()
            .ok_or_else(publication_refused)
    }
    pub(crate) fn require_enrollment(
        &self,
        lease: &crate::RegistryLease,
        source: &crate::EnrolledSource,
    ) -> Result<(), BackupError> {
        self.revalidate()?;
        crate::registry::require_shared_scan_budget(&lease.budget, self.budget())?;
        self.input()?.require_enrollment(lease, source)?;
        self.input()?.require_control(lease, source)?;
        self.revalidate()
    }
    pub(crate) fn take_registry_scope(
        &self,
    ) -> Result<
        (
            std::sync::Arc<std::sync::Mutex<crate::registry::ScanBudget>>,
            std::sync::Arc<std::sync::atomic::AtomicBool>,
        ),
        BackupError,
    > {
        self.revalidate()?;
        if self
            .owners
            .registry_opened
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            self.fail();
            return Err(publication_refused());
        }
        Ok((
            std::sync::Arc::clone(&self.owners.budget),
            std::sync::Arc::clone(&self.owners.failed),
        ))
    }
    pub(crate) fn connection(&mut self) -> &mut PgConnection {
        self.owners
            .session
            .as_mut()
            .expect("admitted original Publication session")
            .connection_in_original_owner()
    }
}
#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
fn publication_refused() -> BackupError {
    BackupError::Invalid("B1 original Publication admission")
}
#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
type PublicationOwnerRow = (
    String,
    String,
    Option<String>,
    String,
    String,
    bool,
    i64,
    String,
    String,
);
#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
async fn publication_sql(
    original: &'static crate::offhost_b1::SourceFrontOriginal,
    connection: &mut PgConnection,
) -> Result<PublicationOwnerRow, BackupError> {
    let deadline = original.first_s_clock()?.deadline()?;
    let query = sqlx::query_as("SELECT current_user::text, session_user::text, system_user, current_database()::text, pg_get_userbyid(d.datdba)::text, pg_catalog.inet_client_addr() IS NULL AND pg_catalog.inet_server_addr() IS NULL, d.oid::bigint, (pg_catalog.pg_control_system()).system_identifier::text, pg_catalog.pg_postmaster_start_time()::text FROM pg_catalog.pg_database d WHERE d.datname=current_database()").fetch_one(connection);
    tokio::pin!(query);
    let returned = tokio::select! {
        result = &mut query => result,
        _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {
            original.reject();
            // Do not drop/replace the started query. The actual retained worker
            // keeps this very future and its session until natural return.
            (&mut query).await
        }
    };
    publication_query_returned(original)?;
    Ok(returned?)
}
#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
async fn publication_acquire_lock(
    original: &'static crate::offhost_b1::SourceFrontOriginal,
    connection: &mut PgConnection,
) -> Result<bool, BackupError> {
    let deadline = original.first_s_clock()?.deadline()?;
    let query = sqlx::query_scalar::<_, bool>("SELECT pg_catalog.pg_try_advisory_lock($1, $2)")
        .bind(0x4b57_4334_i32)
        .bind(0x5352_4345_i32)
        .fetch_one(connection);
    tokio::pin!(query);
    let returned = tokio::select! { result = &mut query => result,
    _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => { original.reject(); (&mut query).await } };
    publication_query_returned(original)?;
    Ok(returned?)
}
#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
async fn publication_lock_observation(
    original: &'static crate::offhost_b1::SourceFrontOriginal,
    connection: &mut PgConnection,
) -> Result<bool, BackupError> {
    let deadline = original.first_s_clock()?.deadline()?;
    let query = sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_locks WHERE pid=pg_catalog.pg_backend_pid() AND locktype='advisory' AND classid=$1::bigint::oid AND objid=$2::bigint::oid AND objsubid=2 AND granted)")
        .bind(i64::from(0x4b57_4334_i32)).bind(i64::from(0x5352_4345_i32)).fetch_one(connection);
    tokio::pin!(query);
    let returned = tokio::select! { result = &mut query => result,
    _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => { original.reject(); (&mut query).await } };
    publication_query_returned(original)?;
    Ok(returned?)
}
#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
async fn publication_version(
    original: &'static crate::offhost_b1::SourceFrontOriginal,
    connection: &mut PgConnection,
) -> Result<String, BackupError> {
    let deadline = original.first_s_clock()?.deadline()?;
    let query = sqlx::query_scalar("SHOW server_version_num").fetch_one(connection);
    tokio::pin!(query);
    let returned = tokio::select! { result = &mut query => result,
    _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => { original.reject(); (&mut query).await } };
    publication_query_returned(original)?;
    Ok(returned?)
}
#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
async fn publication_migration_rows(
    original: &'static crate::offhost_b1::SourceFrontOriginal,
    connection: &mut PgConnection,
) -> Result<Vec<(i64, i32, String, bool)>, BackupError> {
    let deadline = original.first_s_clock()?.deadline()?;
    let maximum = i64::try_from(
        super::MIGRATOR
            .iter()
            .count()
            .checked_add(1)
            .ok_or(BackupError::Overflow)?,
    )
    .map_err(|_| BackupError::Overflow)?;
    let query = sqlx::query_as("SELECT version,pg_catalog.octet_length(checksum),pg_catalog.encode(pg_catalog.substring(checksum,1,49),'hex'),success FROM public._sqlx_migrations ORDER BY version LIMIT $1").bind(maximum).fetch_all(connection);
    tokio::pin!(query);
    let returned = tokio::select! { result = &mut query => result,
    _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => { original.reject(); (&mut query).await } };
    publication_query_returned(original)?;
    Ok(returned?)
}
#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
fn publication_query_returned(
    original: &crate::offhost_b1::SourceFrontOriginal,
) -> Result<(), BackupError> {
    original.first_s_clock()?.check()?;
    original.revalidate()
}
#[cfg(all(target_os = "linux", feature = "offhost-b1"))]
fn publication_owner_row(row: &PublicationOwnerRow) -> Result<(), BackupError> {
    if row.0 != "learning_admin"
        || row.1 != "learning_admin"
        || row.4 != "learning_admin"
        || row.2.as_deref() != Some("peer:root")
        || !row.5
        || row.3.len() > 63
        || row.6 <= 0
        || row.7.is_empty()
        || row.7.len() > 32
        || row.8.is_empty()
        || row.8.len() > 64
    {
        return Err(publication_refused());
    }
    Ok(())
}

#[cfg(all(test, target_os = "linux", feature = "offhost-b1"))]
mod publication_owner_data_tests {
    use super::*;

    #[test]
    fn fixed_sql_tuple_requires_peer_root_unix_owner_and_bounded_identity() {
        // Closed SQL-result DATA predicates only; this makes no session or S.
        let mut row: PublicationOwnerRow = (
            "learning_admin".into(),
            "learning_admin".into(),
            Some("peer:root".into()),
            "learning_backup_fixture".into(),
            "learning_admin".into(),
            true,
            12,
            "123456".into(),
            "2026-01-01 00:00:00+00".into(),
        );
        assert!(publication_owner_row(&row).is_ok());
        row.2 = Some("password:root".into());
        assert!(publication_owner_row(&row).is_err());
        row.2 = Some("peer:root".into());
        row.5 = false;
        assert!(publication_owner_row(&row).is_err());
        row.5 = true;
        row.4 = "postgres".into();
        assert!(publication_owner_row(&row).is_err());
        row.4 = "learning_admin".into();
        row.7 = "1".repeat(33);
        assert!(publication_owner_row(&row).is_err());
    }
}

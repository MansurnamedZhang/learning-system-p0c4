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
        session: learning_db::snapshot_broker::OwnedB1AdminSession,
        peer: std::sync::Arc<learning_db::snapshot_broker::B1InstalledSourceProcess>,
        input: Box<super::binding::B1SourceInputs>,
        budget: std::sync::Arc<std::sync::Mutex<crate::registry::ScanBudget>>,
        null_passfile: std::fs::File,
        failed: std::sync::Arc<std::sync::atomic::AtomicBool>,
        registry_opened: std::sync::atomic::AtomicBool,
    },
}

/// Live Install custody after the original admin session naturally closed.
/// Only close_b1_install can construct it. Copied DATA cannot create it, and
/// it deliberately retains all current original input descriptors and budget.
#[cfg(feature = "offhost-b1")]
pub(in crate::source) struct B1RetainedInstallInputs {
    peer: std::sync::Arc<learning_db::snapshot_broker::B1InstalledSourceProcess>,
    input: Box<super::binding::B1SourceInputs>,
    budget: std::sync::Arc<std::sync::Mutex<crate::registry::ScanBudget>>,
    null_passfile: std::fs::File,
    failed: std::sync::Arc<std::sync::atomic::AtomicBool>,
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

    /// The actual M3 owner calls this constructor. An external session, DATA
    /// value, database name, path or second process Arc cannot be supplied.
    #[cfg(feature = "offhost-b1")]
    pub(crate) async fn try_acquire_b1(
        peer: std::sync::Arc<learning_db::snapshot_broker::B1InstalledSourceProcess>,
    ) -> Result<Self, BackupError> {
        peer.revalidate()
            .map_err(|_| BackupError::Invalid("B1 original source process"))?;
        // Establish the wrapper before the first Catalog ancestor open/stat.
        // The journal meter is separate and is never created or reset here.
        let budget = std::sync::Arc::new(std::sync::Mutex::new(
            crate::registry::ScanBudget::from_b1_scan(peer.scan_accounting_meter()),
        ));
        let mut session = peer
            .acquire_fixed_admin()
            .await
            .map_err(|_| BackupError::Invalid("B1 original admin session"))?;
        let data = session
            .isolation_input_data()
            .map_err(|_| BackupError::Invalid("B1 current isolation input"))?;
        let input = super::binding::B1SourceInputs::read(&peer, &data, &budget)?;
        let (current, login, authenticated, database, owner, local):
            (String, String, Option<String>, String, String, bool) = sqlx::query_as(
                "SELECT current_user::text, session_user::text, system_user, current_database()::text, pg_get_userbyid(d.datdba)::text, pg_catalog.inet_client_addr() IS NULL AND pg_catalog.inet_server_addr() IS NULL FROM pg_catalog.pg_database d WHERE d.datname=current_database()",
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
        let acquired: bool = sqlx::query_scalar("SELECT pg_catalog.pg_try_advisory_lock($1, $2)")
            .bind(0x4b57_4334_i32)
            .bind(0x5352_4345_i32)
            .fetch_one(session.connection_in_original_owner())
            .await?;
        if !acquired {
            return Err(BackupError::Invalid("source maintenance admission busy"));
        }
        let null_passfile = Self::open_null_passfile(&budget)?;
        let admission = Self {
            connection: AdmissionConnection::B1 {
                session,
                peer,
                input: Box::new(input),
                budget,
                null_passfile,
                failed: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                registry_opened: std::sync::atomic::AtomicBool::new(false),
            },
            database,
        };
        admission.revalidate()?;
        Ok(admission)
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
            AdmissionConnection::B1 { session, .. } => session.connection_in_original_owner(),
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
                let closed = session.close().await;
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

    #[cfg(feature = "offhost-b1")]
    pub(in crate::source) async fn close_b1_install(
        self,
    ) -> Result<B1RetainedInstallInputs, BackupError> {
        self.revalidate()?;
        match self.connection {
            AdmissionConnection::Legacy(connection) => {
                connection.close().await?;
                Err(BackupError::Invalid(
                    "B1 original Install admission required",
                ))
            }
            AdmissionConnection::B1 {
                session,
                peer,
                input,
                budget,
                null_passfile,
                failed,
                ..
            } => {
                if input.phase() != "Install" {
                    failed.store(true, std::sync::atomic::Ordering::Release);
                    return Err(BackupError::Invalid("B1 original Install phase required"));
                }
                // This future remains in the fixed original M3 owner. Keep
                // all input custody through the true DB close; no Drop/ACK
                // is interpreted as completion or permission to install.
                if session.close().await.is_err() {
                    failed.store(true, std::sync::atomic::Ordering::Release);
                    return Err(BackupError::Invalid("B1 original Install admin close"));
                }
                let retained = B1RetainedInstallInputs {
                    peer,
                    input,
                    budget,
                    null_passfile,
                    failed,
                };
                retained.recheck()?;
                Ok(retained)
            }
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
                let closed = session.close().await;
                drop((peer, input, budget, null_passfile));
                closed.map_err(|_| BackupError::Invalid("B1 original negative admin close"))?;
            }
        }
        Ok(())
    }
}

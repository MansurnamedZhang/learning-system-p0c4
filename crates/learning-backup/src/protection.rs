//! Durable additive protection; none of these values grants deletion authority.
use crate::{
    BackupError, BackupPlan, SourceLocalPin,
    registry::{EnrolledSource, RegistryLease},
};
use learning_assets::{FsAssetStore, ReconcileReport};
use sqlx::PgPool;
use std::{collections::BTreeMap, time::SystemTime};
use uuid::Uuid;

#[cfg(any(test, target_os = "linux"))]
pub(crate) fn require_finite_protection(protect_all: bool) -> Result<(), BackupError> {
    if protect_all {
        return Err(BackupError::Invalid(
            "registry protection incomplete catalog",
        ));
    }
    Ok(())
}
#[cfg(any(test, target_os = "linux"))]
pub(crate) fn record_backup_keys(
    budget: &mut crate::registry::ScanBudget,
    counts: &mut BTreeMap<String, usize>,
    plan: &BackupPlan,
) -> Result<(), BackupError> {
    for asset in plan.asset_files() {
        budget.protect(&asset.sha256)?;
        *counts.entry(asset.sha256.clone()).or_default() += 1;
    }
    Ok(())
}

#[derive(Debug)]
pub struct CaptureProtection {
    #[cfg(target_os = "linux")]
    inner: linux::Capture,
}
#[derive(Debug)]
pub struct ProtectionSnapshot {
    #[cfg(target_os = "linux")]
    inner: linux::Snapshot,
}
#[cfg(target_os = "linux")]
pub(crate) fn verify_registered_package(
    lease: &RegistryLease,
    dir: &learning_assets::backup_fs::BackupDir,
    id: Uuid,
) -> Result<(crate::BackupManifestV1, Vec<u8>), BackupError> {
    linux::verify_pin(lease, dir, id)
}
impl ProtectionSnapshot {
    #[cfg(target_os = "linux")]
    pub(crate) fn require_same_operation(&self, lease: &RegistryLease) -> Result<(), BackupError> {
        crate::registry::require_shared_scan_budget(&self.inner.lease.budget, &lease.budget)?;
        if !std::sync::Arc::ptr_eq(
            &self.inner.lease.destination_operation,
            &lease.destination_operation,
        ) {
            return Err(BackupError::Invalid(
                "captured evidence original registry operation required",
            ));
        }
        if self.inner.lease.path != lease.path
            || self.inner.lease.authority != lease.authority
            || self.inner.lease.root.identity()? != lease.root.identity()?
        {
            return Err(BackupError::Invalid(
                "captured evidence registry operation identity",
            ));
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    pub(crate) fn captured_files(
        &self,
        id: Uuid,
    ) -> Result<std::collections::BTreeMap<String, Vec<u8>>, BackupError> {
        self.inner.captured_files(id)
    }
    #[cfg(target_os = "linux")]
    pub(crate) fn recheck_held(&self) -> Result<(), BackupError> {
        self.inner.recheck(&self.inner.lease)
    }
    pub fn recheck(&self, lease: &RegistryLease) -> Result<(), BackupError> {
        #[cfg(target_os = "linux")]
        {
            self.inner.recheck(lease)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = lease;
            Err(BackupError::Invalid("registry requires Linux"))
        }
    }
    /// Separate backup references remain visible even when their digest is shared.
    pub fn backup_reference_counts(&self) -> BTreeMap<String, usize> {
        #[cfg(target_os = "linux")]
        {
            self.inner.counts.clone()
        }
        #[cfg(not(target_os = "linux"))]
        {
            BTreeMap::new()
        }
    }
}
pub fn begin_capture_protection(
    lease: &RegistryLease,
    source: &EnrolledSource,
    backup_id: Uuid,
) -> Result<CaptureProtection, BackupError> {
    #[cfg(target_os = "linux")]
    {
        Ok(CaptureProtection {
            inner: linux::Capture::begin(lease, source, backup_id)?,
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (lease, source, backup_id);
        Err(BackupError::Invalid("registry requires Linux"))
    }
}
impl CaptureProtection {
    /// Validate currently held capture evidence without issuing finite
    /// protection, release authority, a report or a new record.
    #[cfg(target_os = "linux")]
    pub(crate) fn recheck_capture(&self, group: &str, id: Uuid) -> Result<(), BackupError> {
        self.inner.recheck_capture(group, id)
    }
    pub fn bind_catalog(&mut self, plan: &BackupPlan) -> Result<(), BackupError> {
        #[cfg(target_os = "linux")]
        {
            self.inner.bind_catalog(plan)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = plan;
            Err(BackupError::Invalid("registry requires Linux"))
        }
    }
    pub fn retain_pin(&mut self, pin: &SourceLocalPin) -> Result<(), BackupError> {
        #[cfg(target_os = "linux")]
        {
            self.inner.retain_pin(pin)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = pin;
            Err(BackupError::Invalid("registry requires Linux"))
        }
    }
    pub fn recheck_before_release(&self) -> Result<(), BackupError> {
        #[cfg(target_os = "linux")]
        {
            self.inner.recheck_before_release()
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(BackupError::Invalid("registry requires Linux"))
        }
    }
    #[cfg(target_os = "linux")]
    pub(crate) fn reopen(
        lease: &RegistryLease,
        source: &EnrolledSource,
        id: Uuid,
    ) -> Result<Self, BackupError> {
        Ok(Self {
            inner: linux::Capture::reopen(lease, source, id)?,
        })
    }
    #[cfg(target_os = "linux")]
    pub(crate) fn abandon_ready(
        &mut self,
        ready: &[u8],
        journal: &crate::SourceGateRecord,
    ) -> Result<(), BackupError> {
        self.inner.abandon_ready(ready, journal)
    }
    #[cfg(target_os = "linux")]
    pub(crate) fn abandoned(&mut self, terminal: &[u8]) -> Result<(), BackupError> {
        self.inner.abandoned(terminal)
    }
}
pub fn discover_backup_protection(
    lease: &RegistryLease,
    source: &EnrolledSource,
) -> Result<ProtectionSnapshot, BackupError> {
    #[cfg(target_os = "linux")]
    {
        let snapshot = linux::scan(lease, source)?;
        require_finite_protection(snapshot.protect_all)?;
        Ok(ProtectionSnapshot { inner: snapshot })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (lease, source);
        Err(BackupError::Invalid("registry requires Linux"))
    }
}
pub async fn reconcile_registered_assets(
    admin: &PgPool,
    assets: &FsAssetStore,
    older_than: SystemTime,
) -> Result<ReconcileReport, BackupError> {
    #[cfg(target_os = "linux")]
    {
        let options = admin.connect_options();
        let db = options
            .get_database()
            .ok_or(BackupError::Invalid("source database required"))?;
        let mut admission = crate::source::SourceAdmission::try_acquire(admin, db).await?;
        let lease = crate::ManagementRegistry::open_installed()?.try_lock()?;
        let source = lease.lookup_source_group(&mut admission, assets).await?;
        let snapshot = discover_backup_protection(&lease, &source)?;
        let plan = crate::catalog::plan_assets_on(admission.connection()).await?;
        let mut keys = std::collections::HashSet::new();
        for digest in snapshot
            .inner
            .counts
            .keys()
            .chain(plan.assets().iter().map(|a| &a.sha256))
        {
            lease.budget.lock().expect("scan budget").protect(digest)?;
            keys.insert(crate::asset_key(digest));
        }
        // There is no DB upload-in-progress table in the fixed schema; all
        // persisted ready originals are included, rather than only live uses.
        snapshot.recheck(&lease)?;
        let report = assets.reconcile_registered_dry_run(&keys, older_than, 100000)?;
        snapshot.recheck(&lease)?;
        admission.close().await?;
        Ok(report)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (admin, assets, older_than);
        Err(BackupError::Invalid("registry requires Linux"))
    }
}
#[cfg(target_os = "linux")]
pub(crate) fn validate_relevant_inventory(
    lease: &RegistryLease,
    source: &EnrolledSource,
) -> Result<(), BackupError> {
    linux::scan(lease, source).map(|_| ())
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use crate::{
        digest,
        registry::{self, RootRecord, SourceGroup, canonical, canonical_record, v4},
        valid_digest,
    };
    use learning_assets::backup_fs::{BackupDir, BackupEntryKind};
    use serde::{Deserialize, Serialize};
    use std::{collections::BTreeSet, io::Write, path::Path};

    const STAGES: [(&str, &str); 5] = [
        ("00-pending.json", "capture_pending"),
        ("10-catalog.json", "catalog_durable"),
        ("20-retained.json", "retained"),
        ("30-abandon-ready.json", "abandon_keep_all_ready"),
        ("40-abandoned.json", "abandoned_keep_all"),
    ];
    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Record {
        format_version: u32,
        capability: String,
        deployment_id: String,
        group_id: String,
        backup_id: String,
        registry_generation: u64,
        registry_generation_sha256: String,
        source_binding_sha256: String,
        application_commit: String,
        application_build_sha256: String,
        state: String,
        previous_sha256: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        roots: Option<BTreeMap<String, String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        asset_index: Option<crate::FileRecord>,
        #[serde(skip_serializing_if = "Option::is_none")]
        logical_asset_count: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        asset_index_sha256: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        manifest_sha256: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        sealed_name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        sealed_dev: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        sealed_ino: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        abandonment_ready_sha256: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        source_journal_sha256: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        abandonment_terminal_sha256: Option<String>,
    }
    impl Record {
        fn pending(
            lease: &RegistryLease,
            source: &EnrolledSource,
            id: Uuid,
        ) -> Result<Self, BackupError> {
            Ok(Self {
                format_version: 1,
                capability: "source_protection_v1".into(),
                deployment_id: lease.authority.deployment_id.clone(),
                group_id: source.group.group_id.clone(),
                backup_id: id.to_string(),
                registry_generation: lease.generation().generation,
                registry_generation_sha256: digest(&canonical(lease.generation())?),
                source_binding_sha256: source.group.source_binding_sha256.clone(),
                application_commit: source.group.application_commit.clone(),
                application_build_sha256: source.group.application_build_sha256.clone(),
                state: "capture_pending".into(),
                previous_sha256: None,
                roots: Some(source.group.roots.clone()),
                asset_index: None,
                logical_asset_count: None,
                asset_index_sha256: None,
                manifest_sha256: None,
                sealed_name: None,
                sealed_dev: None,
                sealed_ino: None,
                abandonment_ready_sha256: None,
                source_journal_sha256: None,
                abandonment_terminal_sha256: None,
            })
        }
        fn next(&self, state: &str, previous: &[u8]) -> Self {
            let mut r = self.clone();
            r.state = state.into();
            r.previous_sha256 = Some(digest(previous));
            r.roots = None;
            r.asset_index = None;
            r.logical_asset_count = None;
            r.asset_index_sha256 = None;
            r.manifest_sha256 = None;
            r.sealed_name = None;
            r.sealed_dev = None;
            r.sealed_ino = None;
            r.abandonment_ready_sha256 = None;
            r.source_journal_sha256 = None;
            r.abandonment_terminal_sha256 = None;
            r
        }
        fn validate(
            &self,
            lease: &RegistryLease,
            source: &EnrolledSource,
            id: &str,
            stage: usize,
            previous: Option<&[u8]>,
        ) -> Result<(), BackupError> {
            let g = &source.group;
            let predecessor = previous
                .map(|raw| canonical_record::<Record>(raw, 16384))
                .transpose()?;
            registry::validate_capture_generation(
                &lease.generations,
                g,
                &source.roots,
                self.registry_generation,
                &self.registry_generation_sha256,
                predecessor
                    .as_ref()
                    .map(|r| (r.registry_generation, r.registry_generation_sha256.as_str())),
            )?;
            if self.format_version != 1
                || self.capability != "source_protection_v1"
                || self.deployment_id != lease.authority.deployment_id
                || self.group_id != g.group_id
                || self.backup_id != id
                || !v4(id)
                || self.registry_generation == 0
                || self.registry_generation > lease.generation().generation
                || self.registry_generation_sha256
                    != digest(&canonical(
                        &lease.generations[(self.registry_generation - 1) as usize],
                    )?)
                || self.source_binding_sha256 != g.source_binding_sha256
                || self.application_commit != g.application_commit
                || self.application_build_sha256 != g.application_build_sha256
                || self.state != STAGES[stage].1
                || self.previous_sha256 != previous.map(digest)
            {
                return Err(BackupError::Invalid("registry protection chain identity"));
            }
            let mut expected = self.clone();
            expected.roots = None;
            expected.asset_index = None;
            expected.logical_asset_count = None;
            expected.asset_index_sha256 = None;
            expected.manifest_sha256 = None;
            expected.sealed_name = None;
            expected.sealed_dev = None;
            expected.sealed_ino = None;
            expected.abandonment_ready_sha256 = None;
            expected.source_journal_sha256 = None;
            expected.abandonment_terminal_sha256 = None;
            match stage {
                0 => {
                    expected.roots = Some(g.roots.clone());
                }
                1 => {
                    let f = self
                        .asset_index
                        .clone()
                        .filter(|f| {
                            f.path == "asset-index.json"
                                && f.size > 0
                                && f.size <= crate::MAX_ASSET_INDEX_BYTES as u64
                                && valid_digest(&f.sha256)
                        })
                        .ok_or(BackupError::Invalid("registry catalog index"))?;
                    expected.asset_index = Some(f);
                    expected.logical_asset_count = Some(
                        self.logical_asset_count
                            .filter(|n| *n <= 100000)
                            .ok_or(BackupError::Invalid("registry catalog count"))?,
                    );
                }
                2 => {
                    expected.asset_index_sha256 = Some(
                        self.asset_index_sha256
                            .clone()
                            .filter(|s| valid_digest(s))
                            .ok_or(BackupError::Invalid("registry retained index"))?,
                    );
                    expected.manifest_sha256 = Some(
                        self.manifest_sha256
                            .clone()
                            .filter(|s| valid_digest(s))
                            .ok_or(BackupError::Invalid("registry retained manifest"))?,
                    );
                    expected.sealed_name = Some(format!("{id}.sealed"));
                    expected.sealed_dev = Some(
                        self.sealed_dev
                            .filter(|n| *n > 0)
                            .ok_or(BackupError::Invalid("registry sealed identity"))?,
                    );
                    expected.sealed_ino = Some(
                        self.sealed_ino
                            .filter(|n| *n > 0)
                            .ok_or(BackupError::Invalid("registry sealed identity"))?,
                    );
                }
                3 => {
                    expected.abandonment_ready_sha256 = Some(
                        self.abandonment_ready_sha256
                            .clone()
                            .filter(|s| valid_digest(s))
                            .ok_or(BackupError::Invalid("registry abandonment ready"))?,
                    );
                    expected.source_journal_sha256 = Some(
                        self.source_journal_sha256
                            .clone()
                            .filter(|s| valid_digest(s))
                            .ok_or(BackupError::Invalid("registry abandonment journal"))?,
                    );
                }
                4 => {
                    expected.abandonment_terminal_sha256 = Some(
                        self.abandonment_terminal_sha256
                            .clone()
                            .filter(|s| valid_digest(s))
                            .ok_or(BackupError::Invalid("registry abandonment terminal"))?,
                    );
                }
                _ => unreachable!(),
            }
            if canonical(self)? != canonical(&expected)? {
                return Err(BackupError::Invalid(
                    "registry protection exact stage fields",
                ));
            }
            Ok(())
        }
    }
    type Observations = BTreeMap<String, (Option<Vec<u8>>, (u64, u64))>;
    pub(super) struct Snapshot {
        pub(super) lease: RegistryLease,
        pub(super) group: SourceGroup,
        roots: BTreeMap<String, RootRecord>,
        handles: BTreeMap<String, BackupDir>,
        observations: Observations,
        pub(super) counts: BTreeMap<String, usize>,
        pub(super) protect_all: bool,
    }
    impl std::fmt::Debug for Snapshot {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Snapshot")
                .field("group", &self.group.group_id)
                .field("protected_digests", &self.counts.len())
                .field("protect_all", &self.protect_all)
                .finish()
        }
    }
    impl Snapshot {
        pub(super) fn captured_files(
            &self,
            id: Uuid,
        ) -> Result<BTreeMap<String, Vec<u8>>, BackupError> {
            self.recheck(&self.lease)?;
            require_finite_protection(self.protect_all)?;
            let mut files = BTreeMap::new();
            for (name, raw) in [
                ("source-authority.json", canonical(&self.lease.authority)?),
                (
                    "source-generation.json",
                    canonical(self.lease.generation())?,
                ),
                ("source-group.json", canonical(&self.group)?),
            ] {
                files.insert(name.into(), raw);
            }
            for record in self.roots.values() {
                files.insert(
                    format!("roots/{}.json", record.enrollment_id),
                    canonical(record)?,
                );
            }
            for (name, original) in [
                (
                    "source-released.json",
                    format!("control/{id}.control/released.json"),
                ),
                (
                    "protection-pending.json",
                    format!("protection/{id}/00-pending.json"),
                ),
                (
                    "protection-catalog.json",
                    format!("protection/{id}/10-catalog.json"),
                ),
                (
                    "protection-retained.json",
                    format!("protection/{id}/20-retained.json"),
                ),
            ] {
                let raw = self
                    .observations
                    .get(&original)
                    .and_then(|v| v.0.as_ref())
                    .ok_or(BackupError::Invalid(
                        "captured source proof missing retained release",
                    ))?;
                files.insert(name.into(), raw.clone());
            }
            let mut records = BTreeMap::new();
            for (path, (raw, _)) in &self.lease.metadata {
                records.insert(format!("registry/{path}"), raw.as_slice());
            }
            for (path, (raw, _)) in &self.observations {
                if let Some(raw) = raw {
                    records.insert(path.clone(), raw.as_slice());
                }
            }
            let snapshot = crate::destination::SnapshotRecord {
                format_version: 1,
                capability: "protection_snapshot_v1".into(),
                source_group_id: self.group.group_id.clone(),
                registry_generation_sha256: digest(&canonical(self.lease.generation())?),
                records: records
                    .into_iter()
                    .map(|(path, raw)| crate::FileRecord {
                        path,
                        size: raw.len() as u64,
                        sha256: digest(raw),
                    })
                    .collect(),
                protected_digest_count: self.counts.len() as u64,
                scan_bytes: self.lease.budget.lock().expect("scan budget").bytes,
            };
            let raw = canonical(&snapshot)?;
            if raw.len() > crate::MAX_ASSET_INDEX_BYTES {
                return Err(BackupError::Capacity("captured protection snapshot"));
            }
            self.lease
                .budget
                .lock()
                .expect("scan budget")
                .charge(raw.len() as u64)?;
            files.insert("protection-snapshot.json".into(), raw);
            Ok(files)
        }
        pub(super) fn recheck(&self, lease: &RegistryLease) -> Result<(), BackupError> {
            lease.recheck()?;
            let source = EnrolledSource {
                group: self.group.clone(),
                roots: self.roots.clone(),
                handles: clone_handles(&self.handles)?,
            };
            let now = scan(lease, &source)?;
            if now.observations != self.observations
                || now.counts != self.counts
                || now.protect_all != self.protect_all
            {
                return Err(BackupError::Invalid("registry protection snapshot changed"));
            }
            Ok(())
        }
    }
    fn clone_handles(
        handles: &BTreeMap<String, BackupDir>,
    ) -> Result<BTreeMap<String, BackupDir>, BackupError> {
        handles
            .iter()
            .map(|(key, h)| Ok((key.clone(), h.try_clone()?)))
            .collect()
    }
    fn group_protection(
        lease: &RegistryLease,
        source: &EnrolledSource,
    ) -> Result<BackupDir, BackupError> {
        let parent = lease.root.open_dir("protection")?;
        let dir = parent.open_dir(&source.group.group_id)?;
        dir.require_private_directory()?;
        Ok(dir)
    }
    fn plan(bytes: &[u8]) -> Result<BackupPlan, BackupError> {
        let index: crate::AssetIndexV1 = serde_json::from_slice(bytes)?;
        if index.format_version != 1 {
            return Err(BackupError::Invalid("registry asset index format"));
        }
        let p = BackupPlan::from_rows(index.assets)?;
        if p.asset_index_bytes() != bytes {
            return Err(BackupError::Invalid("registry noncanonical asset index"));
        }
        Ok(p)
    }
    fn pin_metadata(
        lease: &RegistryLease,
        dir: &BackupDir,
        name: &str,
    ) -> Result<Vec<u8>, BackupError> {
        use std::os::unix::fs::MetadataExt;
        let mut file = dir.open_file(name)?;
        let m = file.metadata()?;
        if m.uid() != 0
            || m.mode() & 0o7777 != 0o400
            || m.nlink() != 1
            || m.len() == 0
            || m.len() > crate::MAX_ASSET_INDEX_BYTES as u64
        {
            return Err(BackupError::Invalid("registry sealed metadata"));
        }
        let raw = registry::read_metadata(&mut file, m.len(), &lease.budget)?;
        if file.metadata()?.len() != m.len() {
            return Err(BackupError::Invalid("registry sealed metadata size"));
        }
        Ok(raw)
    }
    pub(super) fn verify_pin(
        lease: &RegistryLease,
        dir: &BackupDir,
        id: Uuid,
    ) -> Result<(crate::BackupManifestV1, Vec<u8>), BackupError> {
        let raw = pin_metadata(lease, dir, "manifest.json")?;
        let manifest: crate::BackupManifestV1 = serde_json::from_slice(&raw)?;
        if manifest.backup_id != id || manifest.canonical_bytes()? != raw {
            return Err(BackupError::Invalid("registry pin manifest identity"));
        }
        let index = pin_metadata(lease, dir, "asset-index.json")?;
        manifest.validate_with_index(&index)?;
        let mut expected_files = BTreeSet::from(["manifest.json".to_owned()]);
        let mut expected_dirs = BTreeSet::new();
        for record in &manifest.files {
            expected_files.insert(record.path.clone());
            let parts = record.path.split('/').collect::<Vec<_>>();
            for n in 1..parts.len() {
                expected_dirs.insert(parts[..n].join("/"));
            }
        }
        fn walk(
            lease: &RegistryLease,
            dir: &BackupDir,
            prefix: &str,
            files: &BTreeSet<String>,
            dirs: &BTreeSet<String>,
            found_files: &mut BTreeSet<String>,
            found_dirs: &mut BTreeSet<String>,
        ) -> Result<(), BackupError> {
            for name in registry::list(dir, lease)? {
                let path = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}/{name}")
                };
                match dir.kind(&name)? {
                    BackupEntryKind::File if files.contains(&path) => {
                        found_files.insert(path);
                    }
                    BackupEntryKind::Directory if dirs.contains(&path) => {
                        found_dirs.insert(path.clone());
                        walk(
                            lease,
                            &dir.open_dir(&name)?,
                            &path,
                            files,
                            dirs,
                            found_files,
                            found_dirs,
                        )?;
                    }
                    _ => return Err(BackupError::Invalid("registry pin exact inventory")),
                }
            }
            Ok(())
        }
        let mut found_files = BTreeSet::new();
        let mut found_dirs = BTreeSet::new();
        walk(
            lease,
            dir,
            "",
            &expected_files,
            &expected_dirs,
            &mut found_files,
            &mut found_dirs,
        )?;
        if found_files != expected_files || found_dirs != expected_dirs {
            return Err(BackupError::Invalid("registry missing pin entries"));
        }
        for record in &manifest.files {
            if ["asset-index.json", "roles.json"].contains(&record.path.as_str()) {
                let bytes = pin_metadata(lease, dir, &record.path)?;
                if bytes.len() as u64 != record.size || digest(&bytes) != record.sha256 {
                    return Err(BackupError::Invalid("registry pin metadata digest"));
                }
            } else {
                crate::sealed::check_file(dir, record)?;
            }
        }
        if pin_metadata(lease, dir, "manifest.json")? != raw {
            return Err(BackupError::Invalid("registry pin metadata digest"));
        }
        Ok((manifest, index))
    }
    fn observe(
        lease: &RegistryLease,
        dir: &BackupDir,
        name: &str,
        path: String,
        cap: u64,
        observations: &mut Observations,
    ) -> Result<Vec<u8>, BackupError> {
        let (raw, id) = registry::read(dir, name, cap, &lease.budget)?;
        observations.insert(path, (Some(raw.clone()), id));
        Ok(raw)
    }
    // Authority-only inventory: source dump and originals are streaming/full-byte
    // proof, not metadata. Their names/held identities are captured here.
    fn inventory(
        lease: &RegistryLease,
        dir: &BackupDir,
        prefix: &str,
        depth: usize,
        observations: &mut Observations,
    ) -> Result<(), BackupError> {
        if depth > 6 {
            return Err(BackupError::Invalid("registry source inventory depth"));
        }
        if depth == 0 || !prefix.starts_with("local_pins") {
            dir.require_private_directory()?;
        }
        observations.insert(prefix.into(), (None, dir.identity()?));
        for name in registry::list(dir, lease)? {
            let path = format!("{prefix}/{name}");
            match dir.kind(&name)? {
                BackupEntryKind::Directory => {
                    inventory(lease, &dir.open_dir(&name)?, &path, depth + 1, observations)?
                }
                BackupEntryKind::File => {
                    use std::os::unix::fs::MetadataExt;
                    let mut file = dir.open_file(&name)?;
                    let m = file.metadata()?;
                    let read_content = prefix.starts_with("control") && name != "database.dump"
                        || prefix.starts_with("local_pins")
                            && (name == "manifest.json" || name == "asset-index.json");
                    if read_content {
                        // Sealed records are 0400; clone/read bounded original files,
                        // retain metadata identity and exact contents.
                        if m.uid() != 0
                            || ![0o400, 0o600].contains(&(m.mode() & 0o7777))
                            || m.nlink() != 1
                            || (m.len() == 0
                                && !(prefix.starts_with("control/")
                                    && (prefix.contains(".source")
                                        || prefix.contains(".journal-staging")
                                        || (prefix.contains(".abandonment")
                                            && crate::source::lifecycle::temp_name(&name)))))
                            || m.len() > crate::MAX_ASSET_INDEX_BYTES as u64
                        {
                            return Err(BackupError::Invalid("registry source inventory file"));
                        }
                        let raw = registry::read_metadata(&mut file, m.len(), &lease.budget)?;
                        if file.metadata()?.len() != m.len() {
                            return Err(BackupError::Invalid("registry source inventory size"));
                        }
                        observations.insert(path, (Some(raw), (m.dev(), m.ino())));
                    } else {
                        observations.insert(path, (None, (m.dev(), m.ino())));
                    }
                }
                _ => return Err(BackupError::Invalid("registry unknown source entry")),
            }
        }
        Ok(())
    }
    pub(super) fn scan(
        lease: &RegistryLease,
        source: &EnrolledSource,
    ) -> Result<Snapshot, BackupError> {
        lease.recheck()?;
        let mut snapshot = Snapshot {
            lease: lease.clone_held()?,
            group: source.group.clone(),
            roots: source.roots.clone(),
            handles: clone_handles(&source.handles)?,
            observations: BTreeMap::new(),
            counts: BTreeMap::new(),
            protect_all: false,
        };
        for (name, r) in &source.roots {
            registry::matches_root(r, Path::new(&r.path), source.handle(name))?;
        }
        let (binding, _) = registry::read(
            source.handle("control"),
            "source-binding.json",
            4096,
            &lease.budget,
        )?;
        if digest(&binding) != source.group.source_binding_sha256 {
            return Err(BackupError::Invalid("registry original source binding"));
        }
        // Strict legacy control scanner still owns journal/terminal semantics.
        // It cannot be used here because pending capture and finish may own an
        // unfinished journal; recover/validate every journal individually below.
        for name in ["control", "local_pins"] {
            inventory(
                lease,
                source.handle(name),
                name,
                0,
                &mut snapshot.observations,
            )?;
        }
        let group = group_protection(lease, source)?;
        snapshot
            .observations
            .insert("protection".into(), (None, group.identity()?));
        let attempt_names = registry::list(&group, lease)?;
        let mut protected_attempts = BTreeSet::new();
        for id in attempt_names {
            if !v4(&id) || group.kind(&id)? != BackupEntryKind::Directory {
                return Err(BackupError::Invalid("registry protection attempt name"));
            }
            protected_attempts.insert(id.clone());
            let dir = group.open_dir(&id)?;
            dir.require_private_directory()?;
            snapshot
                .observations
                .insert(format!("protection/{id}"), (None, dir.identity()?));
            let names = registry::list(&dir, lease)?;
            if !names.iter().any(|n| n == "00-pending.json")
                || names
                    .iter()
                    .any(|n| n != "asset-index.json" && !STAGES.iter().any(|s| s.0 == n))
            {
                return Err(BackupError::Invalid("registry protection exact inventory"));
            }
            let mut last: Option<Vec<u8>> = None;
            let mut catalog: Option<(Record, BackupPlan)> = None;
            let mut abandoned_ready: Option<Vec<u8>> = None;
            let mut retained = false;
            for (n, (name, _)) in STAGES.iter().enumerate() {
                if !names.iter().any(|s| s == name) {
                    continue;
                }
                if (n == 1 && last.is_none())
                    || (n == 2 && catalog.is_none())
                    || (n == 4 && abandoned_ready.is_none())
                {
                    return Err(BackupError::Invalid(
                        "registry protection predecessor missing",
                    ));
                }
                let raw = observe(
                    lease,
                    &dir,
                    name,
                    format!("protection/{id}/{name}"),
                    16384,
                    &mut snapshot.observations,
                )?;
                let record: Record = canonical_record(&raw, 16384)?;
                record.validate(lease, source, &id, n, last.as_deref())?;
                if n == 1 {
                    if !names.iter().any(|s| s == "asset-index.json") {
                        return Err(BackupError::Invalid("registry missing catalog index"));
                    }
                    let index = observe(
                        lease,
                        &dir,
                        "asset-index.json",
                        format!("protection/{id}/asset-index.json"),
                        crate::MAX_ASSET_INDEX_BYTES as u64,
                        &mut snapshot.observations,
                    )?;
                    let p = plan(&index)?;
                    if record.asset_index.as_ref() != Some(p.asset_index_file())
                        || record.logical_asset_count != Some(p.logical_asset_count())
                    {
                        return Err(BackupError::Invalid("registry catalog index linkage"));
                    }
                    record_backup_keys(
                        &mut lease.budget.lock().expect("scan budget"),
                        &mut snapshot.counts,
                        &p,
                    )?;
                    catalog = Some((record.clone(), p));
                }
                if n == 2 {
                    let pin = source
                        .handle("local_pins")
                        .open_dir(&format!("{id}.sealed"))?;
                    if pin.identity()? != (record.sealed_dev.unwrap(), record.sealed_ino.unwrap()) {
                        return Err(BackupError::Invalid(
                            "registry original sealed pin identity",
                        ));
                    }
                    // Metadata uses the shared budget, full declared originals
                    // use streaming hashing, and enumeration stays bounded.
                    let (m, pin_index) =
                        verify_pin(lease, &pin, Uuid::parse_str(&id).expect("v4"))?;
                    let manifest = m.canonical_bytes()?;
                    let (_, p) = catalog.as_ref().expect("catalog");
                    if m.backup_id.to_string() != id
                        || m.canonical_bytes()? != manifest
                        || digest(&manifest) != record.manifest_sha256.clone().unwrap()
                        || digest(&manifest)
                            != record
                                .manifest_sha256
                                .as_deref()
                                .expect("validated manifest")
                        || record.asset_index_sha256.as_deref()
                            != Some(p.asset_index_file().sha256.as_str())
                        || m.source.application_commit != source.group.application_commit
                        || m.source.application_build_sha256
                            != source.group.application_build_sha256
                    {
                        return Err(BackupError::Invalid("registry retained pin linkage"));
                    }
                    if pin_index != p.asset_index_bytes() {
                        return Err(BackupError::Invalid("registry retained catalog bytes"));
                    }
                    m.validate_with_index(p.asset_index_bytes())?;
                    retained = true;
                }
                if n == 3 {
                    let side = source
                        .handle("control")
                        .open_dir(&format!("{id}.abandonment"))?;
                    let ready = observe(
                        lease,
                        &side,
                        "ready.json",
                        format!("control/{id}.abandonment/ready.json"),
                        4096,
                        &mut snapshot.observations,
                    )?;
                    let journal = crate::SourceGateJournal::recover_in_metered(
                        source.handle("control"),
                        Uuid::parse_str(&id).expect("v4"),
                        &lease.budget,
                    )?;
                    if record.abandonment_ready_sha256 != Some(digest(&ready))
                        || record.source_journal_sha256
                            != Some(digest(&serde_json::to_vec(journal.record())?))
                    {
                        return Err(BackupError::Invalid("registry abandonment ready linkage"));
                    }
                    crate::source::lifecycle::parse_ready(
                        &ready,
                        Uuid::parse_str(&id).expect("v4"),
                        &source.group.source_binding_sha256,
                        journal.record(),
                    )?;
                    abandoned_ready = Some(ready);
                }
                if n == 4 {
                    let side = source
                        .handle("control")
                        .open_dir(&format!("{id}.abandonment"))?;
                    let terminal = observe(
                        lease,
                        &side,
                        "abandoned.json",
                        format!("control/{id}.abandonment/abandoned.json"),
                        4096,
                        &mut snapshot.observations,
                    )?;
                    if record.abandonment_terminal_sha256 != Some(digest(&terminal)) {
                        return Err(BackupError::Invalid(
                            "registry abandonment terminal linkage",
                        ));
                    }
                    crate::source::lifecycle::parse_terminal(
                        &terminal,
                        Uuid::parse_str(&id).expect("v4"),
                        &source.group.source_binding_sha256,
                        abandoned_ready.as_deref().expect("ready"),
                    )?;
                }
                last = Some(raw);
            }
            if catalog.is_none() {
                snapshot.protect_all = true;
                if names.iter().any(|s| s == "asset-index.json") {
                    return Err(BackupError::Invalid("registry uncommitted catalog index"));
                }
            }
            if source
                .handle("control")
                .kind(&format!("{id}.control"))
                .is_ok()
            {
                let journal = crate::SourceGateJournal::recover_in_metered(
                    source.handle("control"),
                    Uuid::parse_str(&id).expect("v4"),
                    &lease.budget,
                )?;
                if journal.record().phase() == crate::GatePhase::Released && !retained {
                    return Err(BackupError::Invalid(
                        "registry released without retained protection",
                    ));
                }
            }
        }
        for name in registry::list(source.handle("control"), lease)? {
            if name == "source-binding.json" {
                continue;
            }
            let id = [
                ".control",
                ".source",
                ".journal-staging",
                ".abandonment",
                ".release-recovery",
            ]
            .iter()
            .find_map(|suffix| name.strip_suffix(suffix))
            .ok_or(BackupError::Invalid("registry unknown source authority"))?;
            if !v4(id) || !protected_attempts.contains(id) {
                return Err(BackupError::Invalid(
                    "registry unprotected source authority",
                ));
            }
        }
        for name in registry::list(source.handle("local_pins"), lease)? {
            let id = name
                .strip_suffix(".sealed")
                .or_else(|| name.strip_suffix(".staging"))
                .ok_or(BackupError::Invalid("registry unknown pin authority"))?;
            if !v4(id) || !protected_attempts.contains(id) {
                return Err(BackupError::Invalid("registry unprotected pin authority"));
            }
        }
        // Only a completely validated scan can finish the missing directory
        // durability from a visible rename. Retain all partial transactions.
        for id in protected_attempts {
            hook("protection_reopen_before_sync")?;
            group.open_dir(&id)?.sync()?;
        }
        hook("protection_reopen_before_sync")?;
        group.sync()?;
        hook("protection_reopen_before_sync")?;
        lease.root.open_dir("protection")?.sync()?;
        hook("protection_reopen_before_sync")?;
        lease.root.open_dir("staging")?.sync()?;
        hook("protection_reopen_before_sync")?;
        lease.root.sync()?;
        Ok(snapshot)
    }
    #[derive(Debug)]
    pub(super) struct Capture {
        lease: RegistryLease,
        source: EnrolledSource,
        id: Uuid,
        dir: BackupDir,
    }
    fn hook(point: &'static str) -> Result<(), BackupError> {
        #[cfg(test)]
        crate::source::lifecycle_tests::hook(point)?;
        #[cfg(not(test))]
        let _ = point;
        Ok(())
    }
    fn write(dir: &BackupDir, name: &str, raw: &[u8]) -> Result<(), BackupError> {
        let mut f = dir.create_file(name)?;
        f.write_all(raw)?;
        hook("protection_before_file_sync")?;
        f.sync_all()?;
        Ok(())
    }
    impl Capture {
        pub(super) fn recheck_capture(&self, group: &str, id: Uuid) -> Result<(), BackupError> {
            if self.source.group_id() != group || self.id != id {
                return Err(BackupError::Invalid("source capture protection identity"));
            }
            self.lease.recheck()?;
            self.dir.require_private_directory()?;
            if group_protection(&self.lease, &self.source)?
                .open_dir(&self.id.to_string())?
                .identity()?
                != self.dir.identity()?
            {
                return Err(BackupError::Invalid(
                    "source capture protection directory changed",
                ));
            }
            for (name, record) in &self.source.roots {
                registry::matches_root(record, Path::new(&record.path), self.source.handle(name))?;
            }
            let (binding, _) = registry::read(
                self.source.handle("control"),
                "source-binding.json",
                4096,
                &self.lease.budget,
            )?;
            if digest(&binding) != self.source.group.source_binding_sha256 {
                return Err(BackupError::Invalid("registry original source binding"));
            }
            // Own-chain metadata only: never hash every retained original on
            // the child's supervision loop. Full release scan remains separate.
            let names = registry::list(&self.dir, &self.lease)?;
            if !names.iter().any(|name| name == "00-pending.json")
                || names.iter().any(|name| {
                    name != "asset-index.json" && !STAGES.iter().any(|stage| stage.0 == name)
                })
            {
                return Err(BackupError::Invalid("registry protection exact inventory"));
            }
            let mut previous: Option<Vec<u8>> = None;
            let mut observed = Vec::new();
            let mut catalog = false;
            let mut ready = false;
            for (stage, (name, _)) in STAGES.iter().enumerate() {
                if !names.iter().any(|entry| entry == name) {
                    continue;
                }
                if (stage == 1 && previous.is_none())
                    || (stage == 2 && !catalog)
                    || (stage == 4 && !ready)
                {
                    return Err(BackupError::Invalid(
                        "registry protection predecessor missing",
                    ));
                }
                let (raw, identity) = registry::read(&self.dir, name, 16384, &self.lease.budget)?;
                let record: Record = canonical_record(&raw, 16384)?;
                record.validate(
                    &self.lease,
                    &self.source,
                    &self.id.to_string(),
                    stage,
                    previous.as_deref(),
                )?;
                if stage == 1 {
                    let (index, index_id) = registry::read(
                        &self.dir,
                        "asset-index.json",
                        crate::MAX_ASSET_INDEX_BYTES as u64,
                        &self.lease.budget,
                    )?;
                    let plan = plan(&index)?;
                    if record.asset_index.as_ref() != Some(plan.asset_index_file())
                        || record.logical_asset_count != Some(plan.logical_asset_count())
                    {
                        return Err(BackupError::Invalid("registry catalog index linkage"));
                    }
                    record_backup_keys(
                        &mut self.lease.budget.lock().expect("scan budget"),
                        &mut BTreeMap::new(),
                        &plan,
                    )?;
                    observed.push((
                        "asset-index.json",
                        index,
                        index_id,
                        crate::MAX_ASSET_INDEX_BYTES as u64,
                    ));
                    catalog = true;
                }
                if stage == 3 {
                    ready = true;
                }
                observed.push((*name, raw.clone(), identity, 16384));
                previous = Some(raw);
            }
            if !catalog && names.iter().any(|name| name == "asset-index.json") {
                return Err(BackupError::Invalid("registry uncommitted catalog index"));
            }
            for (name, expected, identity, cap) in observed {
                let (raw, current) = registry::read(&self.dir, name, cap, &self.lease.budget)?;
                if raw != expected || current != identity {
                    return Err(BackupError::Invalid("source capture protection changed"));
                }
            }
            if registry::list(&self.dir, &self.lease)? != names {
                return Err(BackupError::Invalid(
                    "source capture protection inventory changed",
                ));
            }
            self.lease.recheck()
        }
        pub(super) fn begin(
            lease: &RegistryLease,
            source: &EnrolledSource,
            id: Uuid,
        ) -> Result<Self, BackupError> {
            if !v4(&id.to_string()) {
                return Err(BackupError::Invalid("registry backup UUID"));
            }
            scan(lease, source)?;
            let pending = canonical(&Record::pending(lease, source, id)?)?;
            let staging = lease.root.open_dir("staging")?;
            let token = Uuid::new_v4().to_string();
            let transaction = staging.create_dir(&token)?;
            write(&transaction, "00-pending.json", &pending)?;
            transaction.sync()?;
            staging.sync()?;
            hook("protection_pending_before_publish")?;
            let group = group_protection(lease, source)?;
            staging.rename_entry_to_noreplace_without_sync(&token, &group, &id.to_string())?;
            hook("protection_pending_after_publish")?;
            staging.sync()?;
            group.sync()?;
            let capture = Self::reopen(lease, source, id)?;
            let (raw, _) = registry::read(&capture.dir, "00-pending.json", 16384, &lease.budget)?;
            if raw != pending {
                return Err(BackupError::Invalid("registry pending readback"));
            }
            Ok(capture)
        }
        pub(super) fn reopen(
            lease: &RegistryLease,
            source: &EnrolledSource,
            id: Uuid,
        ) -> Result<Self, BackupError> {
            scan(lease, source)?;
            let dir = group_protection(lease, source)?.open_dir(&id.to_string())?;
            Ok(Self {
                lease: lease.clone_held()?,
                source: EnrolledSource {
                    group: source.group.clone(),
                    roots: source.roots.clone(),
                    handles: clone_handles(&source.handles)?,
                },
                id,
                dir,
            })
        }
        fn latest(&self) -> Result<(Record, Vec<u8>), BackupError> {
            for (name, _) in STAGES.iter().rev() {
                match registry::read(&self.dir, name, 16384, &self.lease.budget) {
                    Ok((raw, _)) => return Ok((canonical_record(&raw, 16384)?, raw)),
                    Err(BackupError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e),
                }
            }
            Err(BackupError::Invalid("registry pending missing"))
        }
        fn publish(&self, files: &[(&str, &[u8])]) -> Result<(), BackupError> {
            self.lease.recheck()?;
            let staging = self.lease.root.open_dir("staging")?;
            let token = Uuid::new_v4().to_string();
            let transaction = staging.create_dir(&token)?;
            for (name, raw) in files {
                write(&transaction, name, raw)?;
            }
            transaction.sync()?;
            staging.sync()?;
            for (name, raw) in files {
                hook("protection_before_record_publish")?;
                transaction.rename_entry_to_noreplace_without_sync(name, &self.dir, name)?;
                hook("protection_after_record_publish")?;
                transaction.sync()?;
                self.dir.sync()?;
                let cap = if *name == "asset-index.json" {
                    crate::MAX_ASSET_INDEX_BYTES as u64
                } else {
                    16384
                };
                let (readback, _) = registry::read(&self.dir, name, cap, &self.lease.budget)?;
                if readback != *raw {
                    return Err(BackupError::Invalid("registry protection readback"));
                }
                if *name == "asset-index.json" {
                    hook("protection_index_before_catalog")?;
                }
            }
            staging.remove_empty_dir(&token)?;
            self.lease.recheck()?;
            Ok(())
        }
        pub(super) fn bind_catalog(&mut self, p: &BackupPlan) -> Result<(), BackupError> {
            let (last, raw) = self.latest()?;
            if last.state != "capture_pending" {
                return Err(BackupError::Invalid("registry catalog phase"));
            }
            let mut next = last.next("catalog_durable", &raw);
            next.asset_index = Some(p.asset_index_file().clone());
            next.logical_asset_count = Some(p.logical_asset_count());
            let record = canonical(&next)?;
            self.publish(&[
                ("asset-index.json", p.asset_index_bytes()),
                ("10-catalog.json", &record),
            ])
        }
        pub(super) fn retain_pin(&mut self, pin: &SourceLocalPin) -> Result<(), BackupError> {
            scan(&self.lease, &self.source)?;
            if pin.sealed().backup_id() != self.id
                || pin.manifest().source.application_build_sha256
                    != self.source.group.application_build_sha256
                || pin.manifest().source.application_commit != self.source.group.application_commit
            {
                return Err(BackupError::Invalid("registry pin source identity"));
            }
            let (last, raw) = self.latest()?;
            if last.state == "retained" {
                self.recheck_before_release()?;
                return Ok(());
            }
            if last.state != "catalog_durable" {
                return Err(BackupError::Invalid("registry retain phase"));
            }
            let name = format!("{}.sealed", self.id);
            let sealed = self.source.handle("local_pins").open_dir(&name)?;
            let (verified, pin_index) = verify_pin(&self.lease, &sealed, self.id)?;
            let (index, _) = registry::read(
                &self.dir,
                "asset-index.json",
                crate::MAX_ASSET_INDEX_BYTES as u64,
                &self.lease.budget,
            )?;
            pin.manifest().validate_with_index(&index)?;
            if pin_index != index
                || verified.canonical_sha256()? != pin.sealed().manifest_sha256()
                || pin.manifest() != &verified
            {
                return Err(BackupError::Invalid("registry verified pin mismatch"));
            }
            let mut next = last.next("retained", &raw);
            next.asset_index_sha256 = Some(digest(&index));
            next.manifest_sha256 = Some(verified.canonical_sha256()?);
            next.sealed_name = Some(name);
            let (dev, ino) = sealed.identity()?;
            next.sealed_dev = Some(dev);
            next.sealed_ino = Some(ino);
            self.publish(&[("20-retained.json", &canonical(&next)?)])?;
            hook("protection_retained_before_release")?;
            Ok(())
        }
        pub(super) fn abandon_ready(
            &mut self,
            ready: &[u8],
            journal: &crate::SourceGateRecord,
        ) -> Result<(), BackupError> {
            let (last, raw) = self.latest()?;
            if last.state == "abandon_keep_all_ready" {
                if last.abandonment_ready_sha256 != Some(digest(ready)) {
                    return Err(BackupError::Invalid("registry abandonment changed"));
                }
                return Ok(());
            }
            if !["capture_pending", "catalog_durable", "retained"].contains(&last.state.as_str()) {
                return Err(BackupError::Invalid("registry abandon phase"));
            }
            crate::source::lifecycle::parse_ready(
                ready,
                self.id,
                &self.source.group.source_binding_sha256,
                journal,
            )?;
            let mut next = last.next("abandon_keep_all_ready", &raw);
            next.abandonment_ready_sha256 = Some(digest(ready));
            next.source_journal_sha256 = Some(digest(&serde_json::to_vec(journal)?));
            self.publish(&[("30-abandon-ready.json", &canonical(&next)?)])?;
            hook("protection_abandon_ready_before_release")?;
            Ok(())
        }
        pub(super) fn abandoned(&mut self, terminal: &[u8]) -> Result<(), BackupError> {
            let (last, raw) = self.latest()?;
            if last.state == "abandoned_keep_all" {
                if last.abandonment_terminal_sha256 != Some(digest(terminal)) {
                    return Err(BackupError::Invalid("registry abandonment changed"));
                }
                return self.recheck_before_release();
            }
            if last.state != "abandon_keep_all_ready" {
                return Err(BackupError::Invalid("registry terminal abandon phase"));
            }
            let (ready, _) = registry::read(
                &self
                    .source
                    .handle("control")
                    .open_dir(&format!("{}.abandonment", self.id))?,
                "ready.json",
                4096,
                &self.lease.budget,
            )?;
            crate::source::lifecycle::parse_terminal(
                terminal,
                self.id,
                &self.source.group.source_binding_sha256,
                &ready,
            )?;
            let mut next = last.next("abandoned_keep_all", &raw);
            next.abandonment_terminal_sha256 = Some(digest(terminal));
            self.publish(&[("40-abandoned.json", &canonical(&next)?)])
        }
        pub(super) fn recheck_before_release(&self) -> Result<(), BackupError> {
            let snapshot = scan(&self.lease, &self.source)?;
            let (last, _) = self.latest()?;
            if !["retained", "abandon_keep_all_ready", "abandoned_keep_all"]
                .contains(&last.state.as_str())
            {
                return Err(BackupError::Invalid("registry release protection missing"));
            }
            snapshot.recheck(&self.lease)
        }
    }
}

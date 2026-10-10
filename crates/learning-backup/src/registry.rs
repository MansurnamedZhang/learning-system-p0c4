//! Independently installed management authority. Operations never enroll paths.
#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]
use crate::{BackupError, digest, valid_digest, valid_hex};
use learning_assets::{FsAssetStore, backup_fs::BackupDir};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use uuid::{Uuid, Variant, Version};

pub(crate) const REGISTRY_PATH: &str = "/var/lib/knowweave-c4/registry";
pub(crate) const MAX_SCAN_BYTES: u64 = 536870912;
pub(crate) const MAX_ENTRIES: usize = 100000;
pub(crate) const MAX_RECORD: u64 = 16384;
pub(crate) const DIRECTORY_NAMES: [&str; 5] =
    ["generations", "groups", "protection", "roots", "staging"];

#[cfg(any(test, target_os = "linux"))]
pub(crate) fn require_shared_scan_budget(
    left: &Arc<Mutex<ScanBudget>>,
    right: &Arc<Mutex<ScanBudget>>,
) -> Result<(), BackupError> {
    if !Arc::ptr_eq(left, right) {
        return Err(BackupError::Invalid(
            "captured evidence requires original registry operation budget",
        ));
    }
    Ok(())
}

#[test]
fn capture_evidence_requires_original_shared_operation_budget() {
    let original = Arc::new(Mutex::new(ScanBudget::default()));
    let cloned = Arc::clone(&original);
    let unrelated = Arc::new(Mutex::new(ScanBudget::default()));
    require_shared_scan_budget(&original, &cloned).unwrap();
    assert!(require_shared_scan_budget(&original, &unrelated).is_err());
    assert_eq!(original.lock().unwrap().bytes, 0);
    assert_eq!(unrelated.lock().unwrap().bytes, 0);
}

#[derive(Default, Debug)]
pub(crate) struct ScanBudget {
    pub(crate) bytes: u64,
    entries: usize,
    keys: BTreeSet<String>,
}
impl ScanBudget {
    pub(crate) fn charge(&mut self, bytes: u64) -> Result<(), BackupError> {
        let next = self
            .bytes
            .checked_add(bytes)
            .ok_or(BackupError::Capacity("registry scan bytes"))?;
        if next > MAX_SCAN_BYTES {
            return Err(BackupError::Capacity("registry scan bytes"));
        }
        self.bytes = next;
        Ok(())
    }
    pub(crate) fn entries(&mut self, count: usize) -> Result<(), BackupError> {
        self.entries = self
            .entries
            .checked_add(count)
            .ok_or(BackupError::Capacity("registry entries"))?;
        if self.entries > MAX_ENTRIES {
            return Err(BackupError::Capacity("registry entries"));
        }
        Ok(())
    }
    pub(crate) fn remaining_entries(&self) -> usize {
        MAX_ENTRIES.saturating_sub(self.entries)
    }
    pub(crate) fn protect(&mut self, key: &str) -> Result<(), BackupError> {
        if !valid_digest(key) {
            return Err(BackupError::Invalid("registry protected digest"));
        }
        if !self.keys.contains(key) && self.keys.len() == MAX_ENTRIES {
            return Err(BackupError::Capacity("registry protected digests"));
        }
        self.keys.insert(key.to_owned());
        Ok(())
    }
}
/// Charge the entire requested read before allocating or touching the reader.
/// Callers validate the held file's length again after this bounded read.
pub(crate) fn read_metadata(
    reader: &mut impl std::io::Read,
    size: u64,
    budget: &Arc<Mutex<ScanBudget>>,
) -> Result<Vec<u8>, BackupError> {
    budget.lock().expect("scan budget").charge(size)?;
    let size = usize::try_from(size).map_err(|_| BackupError::Capacity("registry scan bytes"))?;
    let mut raw = vec![0; size];
    #[cfg(test)]
    READ_OBSERVATION.with(|value| {
        let (calls, bytes) = value.get();
        value.set((calls + 1, bytes + size as u64));
    });
    reader.read_exact(&mut raw)?;
    Ok(raw)
}
#[cfg(test)]
thread_local! { static READ_OBSERVATION: std::cell::Cell<(u64,u64)> = const { std::cell::Cell::new((0,0)) }; }
pub(crate) fn canonical_record<T: DeserializeOwned + Serialize>(
    bytes: &[u8],
    cap: u64,
) -> Result<T, BackupError> {
    if bytes.is_empty() || bytes.len() as u64 > cap {
        return Err(BackupError::Invalid("registry record size"));
    }
    let value: T = serde_json::from_slice(bytes)?;
    let json = serde_json::to_value(&value)?;
    fn integers(v: &serde_json::Value) -> bool {
        match v {
            serde_json::Value::Number(n) => n.is_i64() || n.is_u64(),
            serde_json::Value::Array(a) => a.iter().all(integers),
            serde_json::Value::Object(o) => o.values().all(integers),
            _ => true,
        }
    }
    // Re-serialization detects duplicate fields even for Value, whitespace,
    // alternative escapes and ordering; typed records also reject extra keys.
    if !integers(&json) || serde_json::to_vec(&json)? != bytes {
        return Err(BackupError::Invalid("registry noncanonical record"));
    }
    Ok(value)
}
pub(crate) fn canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, BackupError> {
    Ok(serde_json::to_vec(&serde_json::to_value(value)?)?)
}
pub(crate) fn v4(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|id| {
        id.to_string() == value
            && id.get_variant() == Variant::RFC4122
            && id.get_version() == Some(Version::Random)
    })
}
pub(crate) fn canonical_path(value: &str) -> bool {
    value.starts_with('/')
        && value != "/"
        && !value.contains('\0')
        && value[1..]
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != "..")
}
pub(crate) fn validate_root_identity(
    record: &RootRecord,
    path: &Path,
    identity: (u64, u64),
) -> Result<(), BackupError> {
    if path.to_str() != Some(record.path.as_str()) || identity != (record.dev, record.ino) {
        return Err(BackupError::Invalid("registry root identity"));
    }
    Ok(())
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Identity {
    pub(crate) dev: u64,
    pub(crate) ino: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Authority {
    pub(crate) format_version: u32,
    pub(crate) capability: String,
    pub(crate) deployment_id: String,
    pub(crate) registry_path: String,
    pub(crate) registry_dev: u64,
    pub(crate) registry_ino: u64,
    pub(crate) lock_dev: u64,
    pub(crate) lock_ino: u64,
    pub(crate) directories: BTreeMap<String, Identity>,
    pub(crate) initial_generation: u64,
    pub(crate) initial_generation_sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct RootRecord {
    pub(crate) format_version: u32,
    pub(crate) capability: String,
    pub(crate) deployment_id: String,
    pub(crate) enrollment_id: String,
    pub(crate) group_id: String,
    pub(crate) kind: String,
    pub(crate) path: String,
    pub(crate) dev: u64,
    pub(crate) ino: u64,
    pub(crate) uid: u32,
    pub(crate) mode: u32,
    pub(crate) enrolled_generation: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceGroup {
    pub(crate) format_version: u32,
    pub(crate) capability: String,
    pub(crate) deployment_id: String,
    pub(crate) group_id: String,
    pub(crate) database: String,
    pub(crate) database_oid: u32,
    pub(crate) system_identifier: String,
    pub(crate) source_binding_sha256: String,
    pub(crate) application_commit: String,
    pub(crate) application_build_sha256: String,
    pub(crate) roots: BTreeMap<String, String>,
}
pub(crate) fn unique_source<'a>(
    mut matches: impl Iterator<Item = &'a SourceGroup>,
) -> Result<&'a SourceGroup, BackupError> {
    let first = matches
        .next()
        .ok_or(BackupError::Invalid("registry unique source group"))?;
    if matches.next().is_some() {
        return Err(BackupError::Invalid("registry unique source group"));
    }
    Ok(first)
}
pub(crate) fn validate_build_binding(
    group: &SourceGroup,
    binding: Option<&str>,
    commit: Option<&str>,
    build: Option<&str>,
) -> Result<(), BackupError> {
    if binding != Some(group.source_binding_sha256.as_str())
        || commit != Some(group.application_commit.as_str())
        || build != Some(group.application_build_sha256.as_str())
    {
        return Err(BackupError::Invalid("registry original source build"));
    }
    Ok(())
}
pub(crate) fn validate_capture_generation(
    generations: &[Generation],
    group: &SourceGroup,
    roots: &BTreeMap<String, RootRecord>,
    number: u64,
    hash: &str,
    previous: Option<(u64, &str)>,
) -> Result<(), BackupError> {
    let generation = number
        .checked_sub(1)
        .and_then(|n| usize::try_from(n).ok())
        .and_then(|n| generations.get(n))
        .ok_or(BackupError::Invalid(
            "registry protection captured generation",
        ))?;
    let group_hash = digest(&canonical(group)?);
    if digest(&canonical(generation)?) != hash
        || previous.is_some_and(|pair| pair != (number, hash))
        || !generation
            .groups
            .iter()
            .any(|r| r.id == group.group_id && r.sha256 == group_hash)
        || roots.len() != 4
        || roots.len() != group.roots.len()
    {
        return Err(BackupError::Invalid(
            "registry protection captured generation",
        ));
    }
    for (name, id) in &group.roots {
        let root = roots
            .get(name)
            .ok_or(BackupError::Invalid("registry protection captured root"))?;
        let hash = digest(&canonical(root)?);
        if root.enrollment_id != *id
            || !generation
                .roots
                .iter()
                .any(|r| r.id == *id && r.sha256 == hash)
        {
            return Err(BackupError::Invalid("registry protection captured root"));
        }
    }
    Ok(())
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct DestinationGroup {
    pub(crate) format_version: u32,
    pub(crate) capability: String,
    pub(crate) deployment_id: String,
    pub(crate) group_id: String,
    pub(crate) destination_host_id: String,
    pub(crate) destination_storage_id: String,
    pub(crate) roots: BTreeMap<String, String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub(crate) enum Group {
    Source(SourceGroup),
    Destination(DestinationGroup),
}
impl Group {
    pub(crate) fn id(&self) -> &str {
        match self {
            Self::Source(g) => &g.group_id,
            Self::Destination(g) => &g.group_id,
        }
    }
    fn roots(&self) -> &BTreeMap<String, String> {
        match self {
            Self::Source(g) => &g.roots,
            Self::Destination(g) => &g.roots,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Roster {
    pub(crate) id: String,
    pub(crate) sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Generation {
    pub(crate) format_version: u32,
    pub(crate) capability: String,
    pub(crate) deployment_id: String,
    pub(crate) generation: u64,
    pub(crate) previous_generation_sha256: Option<String>,
    pub(crate) roots: Vec<Roster>,
    pub(crate) groups: Vec<Roster>,
}
pub(crate) fn validate_generation_chain(
    generations: &[Generation],
    roots: &[Roster],
    groups: &[Roster],
) -> Result<(), BackupError> {
    if generations.is_empty() || generations.len() > 128 || roots.len() > 1024 || groups.len() > 256
    {
        return Err(BackupError::Invalid("registry generation roster"));
    }
    fn roster(r: &[Roster], cap: usize) -> bool {
        r.len() <= cap
            && r.iter().all(|v| v4(&v.id) && valid_digest(&v.sha256))
            && r.windows(2).all(|w| w[0].id < w[1].id)
    }
    let mut previous: Option<&Generation> = None;
    for (i, g) in generations.iter().enumerate() {
        if g.format_version != 1
            || g.capability != "backup_registry_generation_v1"
            || !v4(&g.deployment_id)
            || g.generation != i as u64 + 1
            || !roster(&g.roots, 1024)
            || !roster(&g.groups, 256)
        {
            return Err(BackupError::Invalid("registry generation identity"));
        }
        match previous {
            None if g.previous_generation_sha256.is_none() => {}
            Some(p)
                if g.deployment_id == p.deployment_id
                    && g.previous_generation_sha256.as_deref()
                        == Some(digest(&canonical(p)?).as_str())
                    && p.roots.iter().all(|r| g.roots.contains(r))
                    && p.groups.iter().all(|r| g.groups.contains(r)) => {}
            _ => return Err(BackupError::Invalid("registry stale generation")),
        }
        previous = Some(g);
    }
    let last = generations.last().expect("nonempty");
    if last.roots != roots || last.groups != groups {
        return Err(BackupError::Invalid("registry loose or missing record"));
    }
    Ok(())
}

#[derive(Debug)]
pub struct ManagementRegistry {
    pub(crate) root: BackupDir,
    path: PathBuf,
    budget: Arc<Mutex<ScanBudget>>,
}
#[derive(Debug)]
pub struct RegistryLease {
    pub(crate) root: BackupDir,
    pub(crate) path: PathBuf,
    pub(crate) lock: std::fs::File,
    pub(crate) authority: Authority,
    pub(crate) generations: Vec<Generation>,
    pub(crate) roots: BTreeMap<String, RootRecord>,
    pub(crate) groups: BTreeMap<String, Group>,
    pub(crate) metadata: BTreeMap<String, (Vec<u8>, (u64, u64))>,
    pub(crate) budget: Arc<Mutex<ScanBudget>>,
    pub(crate) destination_operation: Arc<Mutex<crate::destination::OperationAuthority>>,
}
#[derive(Debug)]
pub struct EnrolledSource {
    pub(crate) group: SourceGroup,
    pub(crate) handles: BTreeMap<String, BackupDir>,
    pub(crate) roots: BTreeMap<String, RootRecord>,
}
impl EnrolledSource {
    pub fn group_id(&self) -> &str {
        &self.group.group_id
    }
    pub(crate) fn handle(&self, name: &str) -> &BackupDir {
        &self.handles[name]
    }
}
#[derive(Debug)]
pub struct EnrolledDestination {
    pub(crate) group: DestinationGroup,
    pub(crate) handles: BTreeMap<String, BackupDir>,
    pub(crate) lease: RegistryLease,
}
impl EnrolledDestination {
    pub(crate) fn recheck(&self) -> Result<(), BackupError> {
        self.lease.recheck()?;
        #[cfg(target_os = "linux")]
        for (kind, id) in &self.group.roots {
            let record = &self.lease.roots[id];
            matches_root(record, Path::new(&record.path), &self.handles[kind])?;
        }
        Ok(())
    }
    pub(crate) fn destination_path(&self) -> &Path {
        Path::new(&self.lease.roots[&self.group.roots["destination"]].path)
    }
    pub fn group_id(&self) -> &str {
        &self.group.group_id
    }
    pub fn destination(&self) -> &BackupDir {
        &self.handles["destination"]
    }
    pub fn evidence(&self) -> &BackupDir {
        &self.handles["evidence"]
    }
}
impl ManagementRegistry {
    pub fn open_installed() -> Result<Self, BackupError> {
        Self::open(Path::new(REGISTRY_PATH))
    }
    fn open(path: &Path) -> Result<Self, BackupError> {
        let root = BackupDir::open_trusted_private_root(path)?;
        #[allow(unused_mut)]
        let mut registry = Self {
            root,
            path: path.to_owned(),
            budget: Arc::new(Mutex::new(ScanBudget::default())),
        };
        // Full authority is validated on open too, without taking/waiting for a lock.
        #[cfg(target_os = "linux")]
        {
            linux::read_snapshot(&registry.root, &registry.path, &registry.budget)?;
        }
        Ok(registry)
    }
    pub fn try_lock(&self) -> Result<RegistryLease, BackupError> {
        #[cfg(target_os = "linux")]
        {
            linux::try_lock(self)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(BackupError::Invalid("registry requires Linux"))
        }
    }
    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn open_test(path: &Path) -> Result<Self, BackupError> {
        Self::open(path)
    }
}
impl RegistryLease {
    pub(crate) fn clone_held(&self) -> Result<Self, BackupError> {
        Ok(Self {
            root: self.root.try_clone()?,
            path: self.path.clone(),
            lock: self.lock.try_clone()?,
            authority: self.authority.clone(),
            generations: self.generations.clone(),
            roots: self.roots.clone(),
            groups: self.groups.clone(),
            metadata: self.metadata.clone(),
            budget: Arc::clone(&self.budget),
            destination_operation: Arc::clone(&self.destination_operation),
        })
    }

    pub(crate) fn generation(&self) -> &Generation {
        self.generations.last().expect("validated generation")
    }
    pub(crate) fn recheck(&self) -> Result<(), BackupError> {
        #[cfg(target_os = "linux")]
        {
            linux::recheck(self)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(BackupError::Invalid("registry requires Linux"))
        }
    }
    pub(crate) fn admit_control_source(
        &self,
        control: &BackupDir,
        expected_database: &str,
    ) -> Result<EnrolledSource, BackupError> {
        #[cfg(target_os = "linux")]
        {
            linux::admit_control(self, control, expected_database)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (control, expected_database);
            Err(BackupError::Invalid("registry requires Linux"))
        }
    }
    pub(crate) fn admit_existing_source(
        &self,
        control: &BackupDir,
        config: &crate::SourceBackupConfig,
    ) -> Result<EnrolledSource, BackupError> {
        let source = self.admit_control_source(control, &config.expected_database)?;
        #[cfg(target_os = "linux")]
        {
            linux::matches_root(
                &source.roots["local_pins"],
                &config.local_pin_root,
                &BackupDir::open_trusted_private_root(&config.local_pin_root)?,
            )?;
        }
        Ok(source)
    }
    pub(crate) fn admit_source(
        &self,
        control: &BackupDir,
        assets: &FsAssetStore,
        config: &crate::SourceBackupConfig,
    ) -> Result<EnrolledSource, BackupError> {
        let source = self.admit_existing_source(control, config)?;
        #[cfg(target_os = "linux")]
        {
            linux::matches_assets(&source, assets)?;
        }
        #[cfg(not(target_os = "linux"))]
        let _ = assets;
        Ok(source)
    }
    #[cfg(target_os = "linux")]
    pub(crate) async fn lookup_source_group(
        &self,
        admission: &mut crate::source::SourceAdmission,
        assets: &FsAssetStore,
    ) -> Result<EnrolledSource, BackupError> {
        linux::lookup(self, admission, assets).await
    }
    pub fn admit_destination(
        &self,
        destination_root: &Path,
    ) -> Result<EnrolledDestination, BackupError> {
        #[cfg(target_os = "linux")]
        {
            linux::destination(self, destination_root)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = destination_root;
            Err(BackupError::Invalid("registry requires Linux"))
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::os::unix::{fs::MetadataExt, io::AsRawFd};
    pub(crate) fn list(dir: &BackupDir, lease: &RegistryLease) -> Result<Vec<String>, BackupError> {
        list_cap(dir, lease, MAX_ENTRIES)
    }
    fn list_cap(
        dir: &BackupDir,
        lease: &RegistryLease,
        cap: usize,
    ) -> Result<Vec<String>, BackupError> {
        let remaining = lease
            .budget
            .lock()
            .expect("scan budget")
            .remaining_entries();
        let mut names = dir.list_bounded(cap.min(remaining)).map_err(|e| {
            if e.kind() == std::io::ErrorKind::FileTooLarge {
                BackupError::Capacity("registry entries")
            } else {
                e.into()
            }
        })?;
        lease
            .budget
            .lock()
            .expect("scan budget")
            .entries(names.len())?;
        names.sort();
        Ok(names)
    }
    pub(crate) fn read(
        dir: &BackupDir,
        name: &str,
        cap: u64,
        budget: &Arc<Mutex<ScanBudget>>,
    ) -> Result<(Vec<u8>, (u64, u64)), BackupError> {
        let mut file = dir.open_file(name)?;
        let m = file.metadata()?;
        if m.uid() != 0
            || m.mode() & 0o7777 != 0o600
            || m.nlink() != 1
            || m.len() == 0
            || m.len() > cap
        {
            return Err(BackupError::Invalid("registry private record"));
        }
        let raw = read_metadata(&mut file, m.len(), budget)?;
        if file.metadata()?.len() != m.len() {
            return Err(BackupError::Invalid("registry changed record length"));
        }
        Ok((raw, (m.dev(), m.ino())))
    }
    pub(super) fn read_snapshot(
        root: &BackupDir,
        path: &Path,
        budget: &Arc<Mutex<ScanBudget>>,
    ) -> Result<RegistryLease, BackupError> {
        root.require_private_directory()?;
        let remaining = budget.lock().expect("scan budget").remaining_entries();
        let mut names = root.list_bounded(7.min(remaining))?;
        budget.lock().expect("scan budget").entries(names.len())?;
        names.sort();
        if names
            != [
                "authority.json",
                "generations",
                "groups",
                "protection",
                "registry.lock",
                "roots",
                "staging",
            ]
        {
            return Err(BackupError::Invalid("registry exact layout"));
        }
        let (raw, ident) = read(root, "authority.json", 4096, budget)?;
        let authority: Authority = canonical_record(&raw, 4096)?;
        if authority.format_version != 1
            || authority.capability != "backup_registry_v1"
            || !v4(&authority.deployment_id)
            || authority.registry_path != REGISTRY_PATH
            || authority.initial_generation != 1
            || !valid_digest(&authority.initial_generation_sha256)
            || authority
                .directories
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>()
                != DIRECTORY_NAMES
            || !canonical_path(path.to_str().unwrap_or(""))
        {
            return Err(BackupError::Invalid("registry original authority"));
        }
        if root.identity()? != (authority.registry_dev, authority.registry_ino) {
            return Err(BackupError::Invalid("registry original dev/inode"));
        }
        let reopened = BackupDir::open_trusted_private_root(path)?;
        if reopened.identity()? != root.identity()? {
            return Err(BackupError::Invalid("registry replaced root"));
        }
        let lock = root.open_file("registry.lock")?;
        let lm = lock.metadata()?;
        if lm.uid() != 0
            || lm.mode() & 0o7777 != 0o600
            || lm.nlink() != 1
            || lm.len() != 0
            || (lm.dev(), lm.ino()) != (authority.lock_dev, authority.lock_ino)
        {
            return Err(BackupError::Invalid("registry lock identity"));
        }
        let mut lease = RegistryLease {
            root: root.try_clone()?,
            path: path.to_owned(),
            lock,
            authority,
            generations: vec![],
            roots: BTreeMap::new(),
            groups: BTreeMap::new(),
            metadata: BTreeMap::from([("authority.json".into(), (raw, ident))]),
            budget: Arc::clone(budget),
            destination_operation: Arc::new(Mutex::new(
                crate::destination::OperationAuthority::default(),
            )),
        };
        for name in DIRECTORY_NAMES {
            let dir = root.open_dir(name)?;
            dir.require_private_directory()?;
            let id = &lease.authority.directories[name];
            if dir.identity()? != (id.dev, id.ino) {
                return Err(BackupError::Invalid("registry metadata directory identity"));
            }
        }
        if !list(&root.open_dir("staging")?, &lease)?.is_empty() {
            return Err(BackupError::Invalid("registry publication incomplete"));
        }
        let generations = root.open_dir("generations")?;
        let entries = list_cap(&generations, &lease, 128)?;
        if entries.len() > 128 {
            return Err(BackupError::Capacity("registry generations"));
        }
        for (n, name) in entries.iter().enumerate() {
            if *name != format!("{:020}.json", n + 1) {
                return Err(BackupError::Invalid("registry generation gap"));
            }
            let (raw, id) = read(&generations, name, 524288, &lease.budget)?;
            lease.generations.push(canonical_record(&raw, 524288)?);
            lease
                .metadata
                .insert(format!("generations/{name}"), (raw, id));
        }
        let mut rr = vec![];
        let mut gr = vec![];
        for (dir_name, cap) in [("roots", 1024), ("groups", 256)] {
            let dir = root.open_dir(dir_name)?;
            let entries = list_cap(&dir, &lease, cap)?;
            if entries.len() > cap {
                return Err(BackupError::Capacity("registry roster"));
            }
            for name in entries {
                let id = name
                    .strip_suffix(".json")
                    .filter(|s| v4(s))
                    .ok_or(BackupError::Invalid("registry record filename"))?;
                let (raw, identity) = read(&dir, &name, MAX_RECORD, &lease.budget)?;
                let r = Roster {
                    id: id.into(),
                    sha256: digest(&raw),
                };
                if dir_name == "roots" {
                    let record: RootRecord = canonical_record(&raw, MAX_RECORD)?;
                    if record.enrollment_id != id {
                        return Err(BackupError::Invalid("registry enrollment filename"));
                    }
                    lease.roots.insert(id.into(), record);
                    rr.push(r);
                } else {
                    let g: Group = canonical_record(&raw, MAX_RECORD)?;
                    if g.id() != id {
                        return Err(BackupError::Invalid("registry group filename"));
                    }
                    lease.groups.insert(id.into(), g);
                    gr.push(r);
                }
                lease
                    .metadata
                    .insert(format!("{dir_name}/{name}"), (raw, identity));
            }
        }
        validate_generation_chain(&lease.generations, &rr, &gr)?;
        if digest(&canonical(&lease.generations[0])?) != lease.authority.initial_generation_sha256
            || lease
                .generations
                .iter()
                .any(|g| g.deployment_id != lease.authority.deployment_id)
        {
            return Err(BackupError::Invalid("registry generation authority"));
        }
        let mut paths = BTreeSet::new();
        let mut identities = BTreeSet::new();
        let mut root_ids = BTreeSet::new();
        for r in lease.roots.values() {
            if r.format_version != 1
                || r.capability != "backup_root_v1"
                || r.deployment_id != lease.authority.deployment_id
                || !v4(&r.group_id)
                || !canonical_path(&r.path)
                || r.uid != 0
                || r.mode != 448
                || r.dev == 0
                || r.ino == 0
                || r.enrolled_generation == 0
                || r.enrolled_generation > lease.generation().generation
                || !paths.insert(&r.path)
                || !identities.insert((r.dev, r.ino))
                || lease
                    .generations
                    .iter()
                    .position(|g| g.roots.iter().any(|v| v.id == r.enrollment_id))
                    != Some((r.enrolled_generation - 1) as usize)
            {
                return Err(BackupError::Invalid("registry enrolled root fields"));
            }
        }
        for g in lease.groups.values() {
            let kinds: &[(&str, &str)] = match g {
                Group::Source(g) => {
                    if g.format_version != 1
                        || g.capability != "backup_source_group_v1"
                        || g.deployment_id != lease.authority.deployment_id
                        || !crate::maintenance::valid_c4_database(&g.database)
                        || g.database_oid == 0
                        || !g
                            .system_identifier
                            .parse::<u64>()
                            .is_ok_and(|n| n > 0 && n.to_string() == g.system_identifier)
                        || !valid_digest(&g.source_binding_sha256)
                        || !valid_digest(&g.application_build_sha256)
                        || !valid_hex(&g.application_commit, 40)
                    {
                        return Err(BackupError::Invalid("registry source group fields"));
                    }
                    &[
                        ("assets", "source_assets"),
                        ("control", "source_control"),
                        ("local_pins", "local_pins"),
                        ("staging", "source_staging"),
                    ]
                }
                Group::Destination(g) => {
                    if g.format_version != 1
                        || g.capability != "backup_destination_group_v1"
                        || g.deployment_id != lease.authority.deployment_id
                        || g.destination_host_id.is_empty()
                        || g.destination_storage_id.is_empty()
                    {
                        return Err(BackupError::Invalid("registry destination group fields"));
                    }
                    &[
                        ("destination", "backup_destination"),
                        ("evidence", "destination_evidence"),
                    ]
                }
            };
            if g.roots().len() != kinds.len() {
                return Err(BackupError::Invalid("registry group roots"));
            }
            for (key, kind) in kinds {
                let id = g
                    .roots()
                    .get(*key)
                    .ok_or(BackupError::Invalid("registry group root missing"))?;
                let r = lease
                    .roots
                    .get(id)
                    .ok_or(BackupError::Invalid("registry unknown root"))?;
                if r.group_id != g.id() || r.kind != *kind || !root_ids.insert(id) {
                    return Err(BackupError::Invalid("registry root group alias"));
                }
            }
        }
        if root_ids.len() != lease.roots.len() {
            return Err(BackupError::Invalid("registry orphan enrollment"));
        }
        // Protection roots must be independently provisioned for every source group.
        let protection = root.open_dir("protection")?;
        let actual = list_cap(&protection, &lease, 256)?;
        let expected = lease
            .groups
            .values()
            .filter_map(|g| match g {
                Group::Source(g) => Some(g.group_id.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        if actual != expected {
            return Err(BackupError::Invalid("registry protection group roster"));
        }
        for name in actual {
            protection.open_dir(&name)?.require_private_directory()?;
        }
        // Fully validate first, then finish rename-visible publication durability
        // on the held directories. No incomplete staging transaction is removed.
        for name in DIRECTORY_NAMES {
            #[cfg(test)]
            crate::source::lifecycle_tests::hook("registry_reopen_before_sync")?;
            root.open_dir(name)?.sync()?;
        }
        #[cfg(test)]
        crate::source::lifecycle_tests::hook("registry_reopen_before_sync")?;
        root.sync()?;
        Ok(lease)
    }
    pub(super) fn try_lock(registry: &ManagementRegistry) -> Result<RegistryLease, BackupError> {
        let before = read_snapshot(&registry.root, &registry.path, &registry.budget)?;
        if unsafe { libc::flock(before.lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } < 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::EWOULDBLOCK) {
                return Err(BackupError::Invalid("registry busy"));
            }
            return Err(e.into());
        }
        // Do not reacquire: lock is on this exact held descriptor through release.
        recheck(&before)?;
        Ok(before)
    }
    pub(super) fn recheck(lease: &RegistryLease) -> Result<(), BackupError> {
        let now = read_snapshot(&lease.root, &lease.path, &lease.budget)?;
        if now.authority != lease.authority
            || now.generations != lease.generations
            || now.metadata != lease.metadata
        {
            return Err(BackupError::Invalid("registry snapshot changed"));
        }
        Ok(())
    }
    pub(crate) fn matches_root(
        record: &RootRecord,
        path: &Path,
        handle: &BackupDir,
    ) -> Result<(), BackupError> {
        handle.require_private_directory()?;
        validate_root_identity(record, path, handle.identity()?)?;
        if BackupDir::open_trusted_private_root(path)?.identity()? != handle.identity()? {
            return Err(BackupError::Invalid("registry root identity"));
        }
        Ok(())
    }
    fn source(lease: &RegistryLease, g: &SourceGroup) -> Result<EnrolledSource, BackupError> {
        let mut handles = BTreeMap::new();
        let mut roots = BTreeMap::new();
        let mut entries = g.roots.iter().collect::<Vec<_>>();
        entries.sort_by_key(|(_, id)| *id);
        for (key, id) in entries {
            let r = &lease.roots[id];
            let dir = BackupDir::open_trusted_private_root(Path::new(&r.path))?;
            matches_root(r, Path::new(&r.path), &dir)?;
            handles.insert(key.clone(), dir);
            roots.insert(key.clone(), r.clone());
        }
        Ok(EnrolledSource {
            group: g.clone(),
            handles,
            roots,
        })
    }
    pub(super) fn admit_control(
        lease: &RegistryLease,
        control: &BackupDir,
        database: &str,
    ) -> Result<EnrolledSource, BackupError> {
        lease.recheck()?;
        let identity = control.identity()?;
        let matches = lease
            .groups
            .values()
            .filter_map(|v| match v {
                Group::Source(g)
                    if g.database == database
                        && (
                            lease.roots[&g.roots["control"]].dev,
                            lease.roots[&g.roots["control"]].ino,
                        ) == identity =>
                {
                    Some(g)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let g = unique_source(matches.into_iter())?;
        validate_build_binding(
            g,
            option_env!("KNOWWEAVE_C4_SOURCE_CONTROL_BINDING_SHA256"),
            option_env!("KNOWWEAVE_SOURCE_COMMIT"),
            option_env!("KNOWWEAVE_BUILD_ID_SHA256"),
        )?;
        let source = source(lease, g)?;
        let (binding, _) = read(control, "source-binding.json", 4096, &lease.budget)?;
        if digest(&binding) != g.source_binding_sha256 {
            return Err(BackupError::Invalid("registry original source binding"));
        }
        let facts: serde_json::Value = serde_json::from_slice(&binding)?;
        if facts["database"] != serde_json::json!(g.database)
            || facts["database_oid"] != serde_json::json!(g.database_oid)
            || facts["system_identifier"] != serde_json::json!(g.system_identifier)
        {
            return Err(BackupError::Invalid(
                "registry source binding database facts",
            ));
        }
        crate::protection::validate_relevant_inventory(lease, &source)?;
        Ok(source)
    }
    pub(crate) fn matches_assets(
        source: &EnrolledSource,
        assets: &FsAssetStore,
    ) -> Result<(), BackupError> {
        let (a, s) = assets.backup_root_handles()?;
        matches_root(&source.roots["assets"], assets.backup_assets_path(), &a)?;
        matches_root(&source.roots["staging"], assets.backup_staging_path(), &s)
    }
    pub(super) async fn lookup(
        lease: &RegistryLease,
        admission: &mut crate::source::SourceAdmission,
        assets: &FsAssetStore,
    ) -> Result<EnrolledSource, BackupError> {
        let (db,oid,system):(String,i64,String)=sqlx::query_as("SELECT current_database()::text, oid::bigint, (pg_catalog.pg_control_system()).system_identifier::text FROM pg_catalog.pg_database WHERE datname=current_database()").fetch_one(admission.connection()).await?;
        let (a, s) = assets.backup_root_handles()?;
        let ai = a.identity()?;
        let si = s.identity()?;
        let matches = lease
            .groups
            .values()
            .filter_map(|v| match v {
                Group::Source(g)
                    if g.database == db
                        && db == admission.database()
                        && i64::from(g.database_oid) == oid
                        && g.system_identifier == system
                        && (
                            lease.roots[&g.roots["assets"]].dev,
                            lease.roots[&g.roots["assets"]].ino,
                        ) == ai
                        && (
                            lease.roots[&g.roots["staging"]].dev,
                            lease.roots[&g.roots["staging"]].ino,
                        ) == si =>
                {
                    Some(g)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let selected = unique_source(matches.into_iter())?;
        let control = crate::source::binding::admit_control_root(
            admission,
            Path::new(&lease.roots[&selected.roots["control"]].path),
        )
        .await?;
        let source = admit_control(lease, &control, &db)?;
        matches_assets(&source, assets)?;
        Ok(source)
    }
    pub(super) fn destination(
        lease: &RegistryLease,
        path: &Path,
    ) -> Result<EnrolledDestination, BackupError> {
        lease.recheck()?;
        let dir = BackupDir::open_trusted_private_root(path)?;
        let identity = dir.identity()?;
        let matches = lease
            .groups
            .values()
            .filter_map(|v| match v {
                Group::Destination(g)
                    if lease.roots[&g.roots["destination"]].path == path.to_str().unwrap_or("")
                        && (
                            lease.roots[&g.roots["destination"]].dev,
                            lease.roots[&g.roots["destination"]].ino,
                        ) == identity =>
                {
                    Some(g)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(BackupError::Invalid("registry unique destination group"));
        }
        let g = matches[0];
        let mut handles = BTreeMap::new();
        let mut entries = g.roots.iter().collect::<Vec<_>>();
        entries.sort_by_key(|(_, id)| *id);
        for (key, id) in entries {
            let r = &lease.roots[id];
            let d = BackupDir::open_trusted_private_root(Path::new(&r.path))?;
            matches_root(r, Path::new(&r.path), &d)?;
            handles.insert(key.clone(), d);
        }
        Ok(EnrolledDestination {
            group: g.clone(),
            handles,
            lease: lease.clone_held()?,
        })
    }
}
#[cfg(target_os = "linux")]
pub(crate) use linux::{list, matches_root, read};

#[cfg(all(test, target_os = "linux"))]
mod durability_tests {
    use super::*;
    use std::{io::Write, os::unix::fs::MetadataExt};
    fn write(dir: &BackupDir, name: &str, raw: &[u8]) {
        let mut file = dir.create_file(name).unwrap();
        file.write_all(raw).unwrap();
        file.sync_all().unwrap();
        dir.sync().unwrap();
    }
    #[test]
    #[ignore = "actual root plus fresh TEST_C4_LIFECYCLE_FS_ROOT; no PG or source compile pin"]
    fn registry_reopen_resync_and_shared_journal_budget_actual_files() {
        assert_eq!(unsafe { libc::geteuid() }, 0);
        let base = PathBuf::from(
            std::env::var_os("TEST_C4_LIFECYCLE_FS_ROOT").expect("fresh private FS root"),
        );
        let parent = BackupDir::open_trusted_private_root(&base).unwrap();
        let token = Uuid::new_v4().to_string();
        let root = parent.create_dir(&token).unwrap();
        let path = base.join(&token);
        let deployment = Uuid::new_v4().to_string();
        let mut directories = BTreeMap::new();
        for name in DIRECTORY_NAMES {
            let dir = root.create_dir(name).unwrap();
            let (dev, ino) = dir.identity().unwrap();
            directories.insert(name.to_owned(), Identity { dev, ino });
        }
        let lock = root.create_file("registry.lock").unwrap();
        lock.sync_all().unwrap();
        let lm = lock.metadata().unwrap();
        let generation = Generation {
            format_version: 1,
            capability: "backup_registry_generation_v1".into(),
            deployment_id: deployment.clone(),
            generation: 1,
            previous_generation_sha256: None,
            roots: vec![],
            groups: vec![],
        };
        let generation_raw = canonical(&generation).unwrap();
        write(
            &root.open_dir("generations").unwrap(),
            "00000000000000000001.json",
            &generation_raw,
        );
        let (dev, ino) = root.identity().unwrap();
        let authority = Authority {
            format_version: 1,
            capability: "backup_registry_v1".into(),
            deployment_id: deployment,
            registry_path: REGISTRY_PATH.into(),
            registry_dev: dev,
            registry_ino: ino,
            lock_dev: lm.dev(),
            lock_ino: lm.ino(),
            directories,
            initial_generation: 1,
            initial_generation_sha256: digest(&generation_raw),
        };
        let authority_raw = canonical(&authority).unwrap();
        write(&root, "authority.json", &authority_raw);
        let record_bytes = (authority_raw.len() + generation_raw.len()) as u64;
        READ_OBSERVATION.with(|v| v.set((0, 0)));
        let registry = ManagementRegistry::open_test(&path).unwrap();
        let lease = registry.try_lock().unwrap();
        assert_eq!(lease.budget.lock().unwrap().bytes, 3 * record_bytes);
        assert_eq!(
            READ_OBSERVATION.with(|v| v.get()),
            (6, 3 * record_bytes),
            "open, try_lock, and held recheck share actual reads"
        );
        lease.budget.lock().unwrap().bytes = MAX_SCAN_BYTES - record_bytes;
        READ_OBSERVATION.with(|v| v.set((0, 0)));
        lease.recheck().unwrap();
        assert_eq!(READ_OBSERVATION.with(|v| v.get()), (2, record_bytes));
        assert!(matches!(
            lease.recheck(),
            Err(BackupError::Capacity("registry scan bytes"))
        ));
        assert_eq!(READ_OBSERVATION.with(|v| v.get()), (2, record_bytes));
        lease.budget.lock().unwrap().bytes = MAX_SCAN_BYTES - record_bytes + 1;
        READ_OBSERVATION.with(|v| v.set((0, 0)));
        assert!(matches!(
            lease.recheck(),
            Err(BackupError::Capacity("registry scan bytes"))
        ));
        assert_eq!(
            READ_OBSERVATION.with(|v| v.get()),
            (1, authority_raw.len() as u64),
            "cap+1 refuses generation reader before allocation"
        );
        drop(lease);
        drop(registry);
        for skip in 0..6 {
            crate::source::lifecycle_tests::fault("registry_reopen_before_sync", skip);
            assert!(matches!(
                ManagementRegistry::open_test(&path),
                Err(BackupError::Invalid("controlled lifecycle test fault"))
            ));
            assert_eq!(
                std::fs::read(path.join("authority.json")).unwrap(),
                authority_raw
            );
            assert_eq!(
                std::fs::read(path.join("generations/00000000000000000001.json")).unwrap(),
                generation_raw
            );
        }
        ManagementRegistry::open_test(&path)
            .unwrap()
            .try_lock()
            .unwrap()
            .recheck()
            .unwrap();
        let control = parent
            .create_dir(&format!("journal-{}", Uuid::new_v4()))
            .unwrap();
        let id = Uuid::new_v4();
        let mut journal = crate::SourceGateJournal::start_in(&control, id).unwrap();
        journal.advance(crate::GatePhase::Closed, None).unwrap();
        journal.advance(crate::GatePhase::Drained, None).unwrap();
        let phases = [
            crate::GatePhase::Intent,
            crate::GatePhase::Closed,
            crate::GatePhase::Drained,
        ];
        let directory = control.open_dir(&format!("{id}.control")).unwrap();
        let bytes = phases
            .iter()
            .map(|p| {
                directory
                    .open_file(p.file_name())
                    .unwrap()
                    .metadata()
                    .unwrap()
                    .len()
            })
            .sum::<u64>();
        let budget = Arc::new(Mutex::new(ScanBudget::default()));
        budget.lock().unwrap().bytes = MAX_SCAN_BYTES - bytes;
        READ_OBSERVATION.with(|v| v.set((0, 0)));
        let recovered =
            crate::SourceGateJournal::recover_in_metered(&control, id, &budget).unwrap();
        assert_eq!(recovered.record(), journal.record());
        assert_eq!(READ_OBSERVATION.with(|v| v.get()), (3, bytes));
        assert!(matches!(
            crate::SourceGateJournal::recover_in_metered(&control, id, &budget),
            Err(BackupError::Capacity("registry scan bytes"))
        ));
        assert_eq!(READ_OBSERVATION.with(|v| v.get()), (3, bytes));
        assert_eq!(
            crate::SourceGateJournal::recover_in(&control, id)
                .unwrap()
                .record(),
            journal.record(),
            "original unmetered public contract remains independent"
        );
        // Preserve this exact fresh authority and journal, including every inode.
    }
}
